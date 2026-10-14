use architect::{
    config::WorkspaceMode,
    domain::*,
    id,
    runtime::{Command, Runtime},
    storage::Store,
    workspace::managed,
};
use std::{
    collections::BTreeSet,
    path::Path,
    process::Command as Process,
    sync::Arc,
    time::{Duration, Instant},
};
fn git(root: &Path, args: &[&str]) -> Vec<u8> {
    let o = Process::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@localhost",
            "-c",
            "commit.gpgSign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    o.stdout
}
struct Fixture {
    root: tempfile::TempDir,
    runtime: Arc<Runtime>,
    run: Run,
}
impl Fixture {
    fn new(with_git: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let state = root.path().join("state");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&state).unwrap();
        std::fs::write(source.join("input.txt"), "base").unwrap();
        if with_git {
            git(&source, &["init", "-b", "main"]);
            git(&source, &["add", "input.txt"]);
            git(&source, &["commit", "-m", "base"]);
        }
        let mut spec: RunSpec =
            serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
        spec.workspace = source.to_str().unwrap().into();
        spec.input_files = vec!["input.txt".into()];
        spec.editable_files.clear();
        spec.check_command.clear();
        let mut config = architect::runtime::intake::bootstrap("codex".into())
            .unwrap()
            .config;
        config.workspace.mode = WorkspaceMode::Worktree;
        spec.config = Some(config.pin().unwrap());
        let runtime =
            Arc::new(Runtime::new(Store::open(&state.join("state.db")).unwrap()).unwrap());
        let run = runtime.dispatch(&id(), Command::Create { spec }).unwrap();
        Self { root, runtime, run }
    }
    fn state(&self) -> std::path::PathBuf {
        self.root.path().join("state")
    }
    fn wait(&self) -> Run {
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            let r = self
                .runtime
                .store
                .lock()
                .unwrap()
                .get(&self.run.id)
                .unwrap();
            if r.status != Status::Running
                && architect::recovery::WorkspaceLease::acquire(&r.spec.workspace).is_ok()
            {
                return r;
            }
            assert!(Instant::now() < end, "{:?}", r.diagnostic);
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn prepare(&self, selected: BTreeSet<String>) -> Run {
        let p = managed::plan(&self.run, &self.state(), &selected).unwrap();
        self.runtime
            .dispatch(
                &id(),
                Command::PrepareWorkspace {
                    run_id: self.run.id.clone(),
                    selected,
                    scope_hash: p.scope_hash,
                },
            )
            .unwrap();
        self.wait()
    }
}
#[test]
fn private_git_workspace_preserves_source_and_activation_creates_new_scoped_revision() {
    let f = Fixture::new(true);
    let source = Path::new(&f.run.spec.workspace);
    let index = std::fs::read(source.join(".git/index")).unwrap();
    let head = git(source, &["rev-parse", "HEAD"]);
    let prepared = f.prepare(BTreeSet::new());
    let w = prepared.managed_workspace.as_ref().unwrap();
    assert_eq!(
        w.state,
        managed::State::Captured,
        "{:?}",
        prepared.diagnostic
    );
    let receipt = w.receipt.as_ref().unwrap();
    assert_eq!(
        std::fs::read_to_string(Path::new(&receipt.project_root).join("input.txt")).unwrap(),
        "base"
    );
    assert_eq!(std::fs::read(source.join(".git/index")).unwrap(), index);
    assert_eq!(git(source, &["rev-parse", "HEAD"]), head);
    assert!(!source.join(".git/worktrees").exists());
    let policy = f.run.spec.config.as_ref().unwrap().config.policy.clone();
    let scope = managed::activation_scope(&prepared).unwrap();
    let activated = f
        .runtime
        .dispatch(
            &id(),
            Command::ActivateWorkspace {
                run_id: prepared.id.clone(),
                scope_hash: scope,
            },
        )
        .unwrap();
    assert_eq!(activated.spec_revision, prepared.spec_revision + 1);
    assert_ne!(activated.spec.workspace, f.run.spec.workspace);
    assert_eq!(
        activated.managed_workspace.as_ref().unwrap().state,
        managed::State::Active
    );
    assert_eq!(
        activated.spec.config.as_ref().unwrap().config.policy,
        policy
    );
    assert!(activated.artifacts.is_empty() && activated.quality_gates.is_empty());
    assert_eq!(activated.input_snapshot, f.run.input_snapshot);
}
#[test]
fn nongit_selected_inputs_and_local_changes_are_copied_without_initializing_user_source() {
    let f = Fixture::new(false);
    assert!(managed::plan(&f.run, &f.state(), &BTreeSet::new()).is_err());
    let prepared = f.prepare(BTreeSet::from(["input.txt".into()]));
    assert_eq!(
        prepared.managed_workspace.as_ref().unwrap().state,
        managed::State::Captured,
        "{:?}",
        prepared.diagnostic
    );
    assert!(!Path::new(&f.run.spec.workspace).join(".git").exists());
}
#[test]
fn failed_intent_commit_starts_no_git_and_failed_result_checkpoint_restores_without_replay() {
    let f = Fixture::new(true);
    let db = rusqlite::Connection::open(f.state().join("state.db")).unwrap();
    db.execute_batch("CREATE TRIGGER deny_workspace_intent BEFORE INSERT ON events WHEN NEW.kind='workspace_prepare_intent' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    let p = managed::plan(&f.run, &f.state(), &BTreeSet::new()).unwrap();
    let c = Command::PrepareWorkspace {
        run_id: f.run.id.clone(),
        selected: BTreeSet::new(),
        scope_hash: p.scope_hash,
    };
    let key = id();
    assert!(f.runtime.dispatch(&key, c.clone()).is_err());
    assert!(!f.state().join("workspaces").exists());
    db.execute_batch("DROP TRIGGER deny_workspace_intent; CREATE TRIGGER deny_workspace_finish BEFORE INSERT ON events WHEN NEW.kind='workspace_prepare_finished' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    f.runtime.dispatch(&key, c).unwrap();
    let unknown = f.wait();
    assert_eq!(
        unknown.managed_workspace.as_ref().unwrap().state,
        managed::State::Unknown
    );
    let before = managed::observed_result(unknown.managed_workspace.as_ref().unwrap()).unwrap();
    let scope = managed::recovery_scope(&unknown).unwrap();
    db.execute_batch("DROP TRIGGER deny_workspace_finish")
        .unwrap();
    let reopened =
        Arc::new(Runtime::new(Store::open(&f.state().join("state.db")).unwrap()).unwrap());
    let restored = reopened
        .dispatch(
            &id(),
            Command::RestoreWorkspaceResult {
                run_id: unknown.id.clone(),
                scope_hash: scope,
            },
        )
        .unwrap();
    let after = restored
        .managed_workspace
        .as_ref()
        .unwrap()
        .receipt
        .as_ref()
        .unwrap();
    assert_eq!(after.commit, before.commit);
    assert_eq!(after.candidate, before.candidate);
    assert!(!restored.reconciliation_required);
    assert_eq!(
        std::fs::read_dir(f.state().join("workspaces"))
            .unwrap()
            .count(),
        1
    );
}
