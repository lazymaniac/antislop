use super::*;
use crate::lean::core_task_execution::task_execution::worker::writer_worktree;

struct Fixture {
    _directory: tempfile::TempDir,
    project: PathBuf,
    core: Arc<Core>,
    conversation: String,
    tasks: Vec<Task>,
    specs: Vec<TaskContract>,
    head: String,
    index: Vec<u8>,
}

fn fixture(output: &str, legacy: bool) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&project)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(project.join("source"), "user source").unwrap();
    std::fs::write(project.join("user.md"), "user document").unwrap();
    std::fs::write(project.join(".gitignore"), ".antislop/\n").unwrap();
    let head = git::commit_all(&project, "base").unwrap();
    let config = Config::from_toml(include_str!("../../../examples/antislop.toml")).unwrap();
    let core = Core::open(&directory.path().join("state"), config).unwrap();
    let conversation = core
        .store
        .create_conversation(project.to_str().unwrap())
        .unwrap()
        .id;
    let values = [
        json!({"id":"report","title":"Report","goal":"Write report","kind":"analysis","writes":true,"change_kind":"artifact","delivery":"untracked","outputs":[output]}),
        json!({"id":"correction","title":"Correction","goal":"Correct report","kind":"analysis","writes":true,"change_kind":"artifact","delivery":"untracked","outputs":[output],"depends_on":["report"]}),
        json!({"id":"other-correction","title":"Correction","goal":"Correct report","kind":"analysis","writes":true,"change_kind":"artifact","delivery":"untracked","outputs":[output],"depends_on":["report"]}),
    ];
    let specs = values
        .iter()
        .cloned()
        .map(serde_json::from_value)
        .collect::<std::result::Result<Vec<TaskContract>, _>>()
        .unwrap();
    let encoded = specs
        .iter()
        .map(|spec| serde_json::to_value(spec).unwrap())
        .collect::<Vec<_>>();
    let mut tasks = core.store.add_plan(&conversation, &encoded).unwrap();
    let worktree = writer_worktree(&core, &mut tasks[0], &specs[0], &project)
        .unwrap()
        .unwrap();
    std::fs::create_dir_all(worktree.join(output).parent().unwrap()).unwrap();
    std::fs::write(worktree.join(output), "old report").unwrap();
    let change = git::commit_artifacts(&worktree, &specs[0].outputs, "report", true).unwrap();
    tasks[0].result = Some(change.clone());
    core.store.update_task(&tasks[0]).unwrap();
    let base = tasks[0].base.clone().unwrap();
    if legacy {
        assert_eq!(
            git::deliver_untracked(&project, &base, &change).unwrap(),
            git::Integration::Applied
        );
    } else {
        assert_eq!(
            core.deliver_artifact_change(&tasks[0], &specs[0], &project, &base, &change)
                .unwrap(),
            git::Integration::Applied
        );
    }
    core.record_plan_landing(&tasks[0].plan_id, &tasks[0].id, &change)
        .unwrap();
    tasks[0].status = status::APPROVED.into();
    tasks[0].report = Some("full report".into());
    core.store.update_task(&tasks[0]).unwrap();
    let index = std::fs::read(project.join(".git/index")).unwrap();
    Fixture {
        _directory: directory,
        project,
        core,
        conversation,
        tasks,
        specs,
        head,
        index,
    }
}

fn correction(f: &mut Fixture, n: usize, text: &str) -> PathBuf {
    let path = writer_worktree(&f.core, &mut f.tasks[n], &f.specs[n], &f.project)
        .unwrap()
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(path.join(&f.specs[n].outputs[0])).unwrap(),
        "old report"
    );
    std::fs::write(path.join(&f.specs[n].outputs[0]), text).unwrap();
    f.tasks[n].result =
        Some(git::commit_artifacts(&path, &f.specs[n].outputs, "correction", true).unwrap());
    f.core.store.update_task(&f.tasks[n]).unwrap();
    f.core
        .validate_artifact_delta(&f.tasks[n], &f.specs[n], &path)
        .unwrap();
    path
}

fn deliver(f: &Fixture, n: usize) -> git::Integration {
    f.core
        .deliver_artifact_change(
            &f.tasks[n],
            &f.specs[n],
            &f.project,
            f.tasks[n].base.as_deref().unwrap(),
            f.tasks[n].result.as_deref().unwrap(),
        )
        .unwrap()
}

#[test]
fn a_new_writer_can_update_its_delivered_dependency_without_touching_head_or_index() {
    for output in ["report.md", ".antislop/analysis.md"] {
        for legacy in [false, true] {
            let mut f = fixture(output, legacy);
            let path = correction(&mut f, 1, "corrected report");
            assert_eq!(deliver(&f, 1), git::Integration::Applied);
            assert_eq!(
                std::fs::read_to_string(f.project.join(output)).unwrap(),
                "corrected report"
            );
            assert_eq!(f.head, git::head(&f.project).unwrap());
            assert_eq!(
                f.index,
                std::fs::read(f.project.join(".git/index")).unwrap()
            );
            assert_eq!(
                deliver(&f, 1),
                git::Integration::Applied,
                "replay must be idempotent"
            );
            let receipt = f
                .core
                .store
                .artifact_receipt(&f.conversation, output)
                .unwrap()
                .unwrap();
            assert_eq!(receipt.task, f.tasks[1].id);
            // Durable receipts/leases survive daemon reconstruction.
            let state = f.core.state.clone();
            let config = f.core.config_snapshot().as_ref().clone();
            drop(f.core);
            let reopened = Core::open(&state, config).unwrap();
            assert_eq!(
                reopened
                    .store
                    .artifact_receipt(&f.conversation, output)
                    .unwrap(),
                Some(receipt)
            );
            assert_eq!(
                std::fs::read_to_string(path.join(output)).unwrap(),
                "corrected report"
            );
        }
    }
}

#[test]
fn user_changes_or_a_competing_delivery_invalidate_the_frozen_artifact_version() {
    let mut f = fixture("report.md", false);
    let worktree = correction(&mut f, 1, "correction one");
    std::fs::write(f.project.join("report.md"), "USER edit").unwrap();
    assert!(matches!(deliver(&f, 1), git::Integration::UserEdits(_)));
    assert_eq!(
        std::fs::read_to_string(f.project.join("report.md")).unwrap(),
        "USER edit"
    );
    assert_eq!(
        std::fs::read_to_string(worktree.join("report.md")).unwrap(),
        "correction one"
    );
    std::fs::write(f.project.join("report.md"), "old report").unwrap();
    correction(&mut f, 2, "correction two");
    assert_eq!(deliver(&f, 1), git::Integration::Applied);
    assert!(matches!(deliver(&f, 2), git::Integration::UserEdits(_)));
    assert_eq!(
        std::fs::read_to_string(f.project.join("report.md")).unwrap(),
        "correction one"
    );
    assert_eq!(f.head, git::head(&f.project).unwrap());
    assert_eq!(
        f.index,
        std::fs::read(f.project.join(".git/index")).unwrap()
    );
}

#[test]
fn receipt_allows_same_conversation_without_an_old_edge_but_denies_foreign_conversations() {
    let f = fixture("report.md", false);
    for foreign in [false, true] {
        let conversation = if foreign {
            f.core
                .store
                .create_conversation(f.project.to_str().unwrap())
                .unwrap()
                .id
        } else {
            f.conversation.clone()
        };
        let mut spec = f.specs[1].clone();
        spec.id = "unrelated".into();
        spec.depends_on.clear();
        let mut task = f
            .core
            .store
            .add_plan(&conversation, &[serde_json::to_value(&spec).unwrap()])
            .unwrap()
            .remove(0);
        let result = writer_worktree(&f.core, &mut task, &spec, &f.project);
        assert_eq!(result.is_err(), foreign);
        assert_eq!(
            std::fs::read_to_string(f.project.join("report.md")).unwrap(),
            "old report"
        );
    }
}

#[test]
fn legacy_provenance_never_adopts_a_user_document_from_the_starting_snapshot() {
    let mut f = fixture("report.md", true);
    f.specs[0].outputs.push("user.md".into());
    f.tasks[0].spec = serde_json::to_value(&f.specs[0]).unwrap();
    f.core.store.update_task(&f.tasks[0]).unwrap();
    f.specs[1].outputs = vec!["user.md".into()];
    f.tasks[1].spec = serde_json::to_value(&f.specs[1]).unwrap();
    f.core.store.update_task(&f.tasks[1]).unwrap();
    assert!(writer_worktree(&f.core, &mut f.tasks[1], &f.specs[1], &f.project).is_err());
    assert!(
        f.core
            .store
            .artifact_receipt(&f.conversation, "user.md")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        std::fs::read_to_string(f.project.join("user.md")).unwrap(),
        "user document"
    );
}

#[test]
fn a_previously_unknown_owned_report_can_be_recovered_without_a_dependency_edge() {
    for legacy in [false, true] {
        let mut f = fixture("report.md", legacy);
        f.specs[1].depends_on.clear();
        f.core
            .store
            .freeze_artifact_input(&f.tasks[1], "report.md", None)
            .unwrap();
        let mut task = f.tasks[1].clone();
        let path = writer_worktree(&f.core, &mut task, &f.specs[1], &f.project)
            .unwrap()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(path.join("report.md")).unwrap(),
            "old report"
        );
        assert!(
            f.core
                .store
                .artifact_input(&task, "report.md")
                .unwrap()
                .unwrap()
                .is_some()
        );
        assert_eq!(git::head(&f.project).unwrap(), f.head);
        assert_eq!(
            std::fs::read(f.project.join(".git/index")).unwrap(),
            f.index
        );
    }
}

#[test]
fn recovered_ownership_never_adopts_user_edits_or_symlink_targets() {
    use std::os::unix::fs::symlink;
    for link in [false, true] {
        let mut f = fixture("report.md", false);
        f.specs[1].depends_on.clear();
        f.core
            .store
            .freeze_artifact_input(&f.tasks[1], "report.md", None)
            .unwrap();
        if link {
            std::fs::remove_file(f.project.join("report.md")).unwrap();
            symlink("user.md", f.project.join("report.md")).unwrap();
        } else {
            std::fs::write(f.project.join("report.md"), "user correction").unwrap();
        }
        let mut task = f.tasks[1].clone();
        assert!(writer_worktree(&f.core, &mut task, &f.specs[1], &f.project).is_err());
        assert!(
            f.core
                .store
                .artifact_input(&task, "report.md")
                .unwrap()
                .unwrap()
                .is_none()
        );
        assert_eq!(
            std::fs::read_to_string(f.project.join("user.md")).unwrap(),
            "user document"
        );
        assert_eq!(git::head(&f.project).unwrap(), f.head);
        assert_eq!(
            std::fs::read(f.project.join(".git/index")).unwrap(),
            f.index
        );
    }
}

#[test]
fn continuation_keeps_integrated_work_and_new_upstream_changes() {
    let mut f = fixture("report.md", false);
    std::fs::write(f.project.join("source"), "new upstream content").unwrap();
    f.tasks[1].base = f.tasks[0].base.clone();
    f.tasks[1].result = f.tasks[0].result.clone();
    f.tasks[1].feedback = Some(json!({"continuation_review":true,"resume_role":"reviewer"}));
    let path = writer_worktree(&f.core, &mut f.tasks[1], &f.specs[1], &f.project)
        .unwrap()
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(path.join("source")).unwrap(),
        "new upstream content"
    );
    assert_eq!(
        std::fs::read_to_string(path.join("report.md")).unwrap(),
        "old report"
    );
    assert_eq!(
        std::fs::read_to_string(f.project.join("report.md")).unwrap(),
        "old report"
    );
    assert_eq!(git::head(&f.project).unwrap(), f.head);
    assert_eq!(
        std::fs::read(f.project.join(".git/index")).unwrap(),
        f.index
    );
}
