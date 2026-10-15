//! Durable private workspace preparation. Source refs/index/working files are read
//! only; Git state and detached worktrees belong to the application's data root.
use super::snapshot::{self, FileKind, Snapshot};
use crate::{Result, domain::Run, hash, id, require};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Intent,
    Unknown,
    Captured,
    Active,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Origin {
    Head { root: String, object_id: String },
    Working { root: String, path: String },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub mode: String,
    pub content_hash: String,
    pub length: u64,
    pub origin: Origin,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub source: Snapshot,
    pub selected: BTreeSet<String>,
    pub project_prefix: String,
    pub files: BTreeMap<String, File>,
    pub scope_hash: String,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub directory_inputs: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub selectors: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_identity: Option<super::delivery::Identity>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub commit: String,
    pub tree: String,
    pub candidate: String,
    pub project_root: String,
    pub observed: Snapshot,
    pub git_pointer_hash: String,
    pub repository_config_hash: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    pub id: String,
    pub workspace_id: String,
    pub state: State,
    pub root: String,
    pub original_spec: crate::domain::RunSpec,
    pub original_revision: u64,
    pub plan: Plan,
    pub receipt: Option<Receipt>,
    pub diagnostic: Option<String>,
}
fn path_valid(path: &str) -> Result<()> {
    require(!path.is_empty()&&!path.contains('\0')&&Path::new(path).components().all(|c|matches!(c,Component::Normal(n) if !n.as_encoded_bytes().eq_ignore_ascii_case(b".git"))),"managed file path escapes source scope or Git administration")
}
fn data(origin: &Origin, token: &AtomicBool) -> Result<Vec<u8>> {
    require(
        !token.load(Ordering::SeqCst),
        "workspace preparation cancelled",
    )?;
    match origin {
        Origin::Head { root, object_id } => snapshot::required_git(
            Path::new(root),
            &["cat-file", "blob", object_id],
            None,
            token,
        ),
        Origin::Working { root, path } => {
            let root = Path::new(root);
            path_valid(path)?;
            let target = root.join(path);
            let metadata = target.symlink_metadata()?;
            if metadata.file_type().is_symlink() {
                Ok(std::fs::read_link(target)?
                    .as_os_str()
                    .as_encoded_bytes()
                    .to_vec())
            } else {
                super::read_input_bytes(root, path)
            }
        }
    }
}
fn quoted(path: &str) -> String {
    let mut result = String::from("\"");
    for byte in path.bytes() {
        if (32..127).contains(&byte) && byte != b'"' && byte != b'\\' {
            result.push(char::from(byte))
        } else {
            result.push_str(&format!("\\{byte:03o}"));
        }
    }
    result.push('"');
    result
}
pub fn plan(run: &Run, state: &Path, selected: &BTreeSet<String>) -> Result<Plan> {
    plan_cancelled(run, state, selected, &AtomicBool::new(false))
}
pub(crate) fn plan_cancelled(
    run: &Run,
    state: &Path,
    selected: &BTreeSet<String>,
    token: &AtomicBool,
) -> Result<Plan> {
    eligible(run)?;
    let source_root = Path::new(&run.spec.workspace).canonicalize()?;
    let state = state.canonicalize()?;
    let preliminary = snapshot::repository(&source_root, token)?;
    let root = preliminary
        .as_ref()
        .map_or(source_root.clone(), |r| PathBuf::from(&r.root));
    plan_source(run, &source_root, &root, &state, selected, token)
}

pub fn eligible(run: &Run) -> Result<()> {
    require(
        !run.intake
            && run.team.is_none()
            && run.artifacts.is_empty()
            && !run.reconciliation_required
            && !matches!(
                run.status,
                crate::domain::Status::Running
                    | crate::domain::Status::Paused
                    | crate::domain::Status::Completed
                    | crate::domain::Status::Cancelled
            ),
        "prepare workspace before task/team execution, after contract acceptance and reconciliation",
    )?;
    require(
        run.managed_workspace.is_none(),
        "workspace preparation already exists; adjudicate it without replay",
    )?;
    require(
        run.spec
            .config
            .as_ref()
            .is_some_and(|c| c.config.workspace.mode == crate::config::WorkspaceMode::Worktree),
        "explicit worktree configuration is required",
    )?;
    Ok(())
}

fn plan_source(
    run: &Run,
    source_root: &Path,
    root: &Path,
    state: &Path,
    selected: &BTreeSet<String>,
    token: &AtomicBool,
) -> Result<Plan> {
    let prefix = source_root
        .strip_prefix(root)
        .map_err(|_| crate::Error::Invalid("workspace root mapping failed".into()))?;
    let project_prefix = if prefix.as_os_str().is_empty() {
        String::new()
    } else {
        format!(
            "{}/",
            prefix
                .to_str()
                .ok_or_else(|| crate::Error::Invalid("non-UTF-8 project prefix".into()))?
        )
    };
    let selectors: BTreeSet<_> = selected
        .iter()
        .map(|p| {
            let relative = super::inputs::relative(source_root, p)?;
            if relative != "." {
                path_valid(&relative)?;
            }
            Ok(relative)
        })
        .collect::<Result<_>>()?;
    let excluded = state
        .strip_prefix(root)
        .ok()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| {
            p.to_str()
                .map(str::to_owned)
                .ok_or_else(|| crate::Error::Invalid("non-UTF-8 data root".into()))
        })
        .transpose()?
        .into_iter()
        .collect::<BTreeSet<_>>();
    require(state != root, "daemon data root cannot be the source root")?;
    let (_, members) = snapshot::source_paths(root, &excluded, token)?;
    let mut additional = BTreeSet::new();
    let mut directories = BTreeSet::new();
    for selector in &selectors {
        if source_root.join(selector).is_dir() {
            super::validate_inputs(source_root, std::slice::from_ref(selector))?;
            let prefix = if selector == "." {
                project_prefix.clone()
            } else {
                format!("{project_prefix}{selector}/")
            };
            directories.insert(prefix.clone());
            additional.extend(
                members
                    .iter()
                    .filter(|path| path.starts_with(&prefix))
                    .cloned(),
            );
        } else {
            additional.insert(format!("{project_prefix}{selector}"));
        }
    }
    let source = snapshot::observe_excluding(root, &additional, &excluded, token)?;
    let selected: BTreeSet<_> = additional
        .iter()
        .filter(|path| {
            !directories.iter().any(|prefix| path.starts_with(prefix))
                || source.local_changes.contains(*path)
        })
        .cloned()
        .collect();
    if let Some(repo) = &source.repository {
        require(
            !repo
                .head_entries
                .keys()
                .chain(repo.index_entries.keys())
                .any(|p| {
                    excluded
                        .iter()
                        .any(|e| p == e || p.starts_with(&format!("{e}/")))
                }),
            "daemon data directory overlaps tracked project files",
        )?;
    }
    source.inclusion_scope(&selected)?;
    let mut recipes = BTreeMap::new();
    if let Some(repo) = &source.repository {
        for (path, entry) in &repo.head_entries {
            path_valid(path)?;
            require(
                entry.mode != "160000",
                "submodule requires recursive admission",
            )?;
            recipes.insert(
                path.clone(),
                (
                    entry.mode.clone(),
                    Origin::Head {
                        root: source.root.clone(),
                        object_id: entry.object_id.clone(),
                    },
                ),
            );
        }
    }
    for path in &selected {
        match source
            .files
            .get(path)
            .ok_or_else(|| crate::Error::Invalid("selected path absent from observation".into()))?
        {
            None => {
                recipes.remove(path);
            }
            Some(file) => {
                require(
                    matches!(file.kind, FileKind::Regular | FileKind::Symlink),
                    "unsupported selected source entry",
                )?;
                let mode = if file.kind == FileKind::Symlink {
                    "120000"
                } else if file.permissions & 0o100 != 0 {
                    "100755"
                } else {
                    "100644"
                };
                recipes.insert(
                    path.clone(),
                    (
                        mode.into(),
                        Origin::Working {
                            root: source.root.clone(),
                            path: path.clone(),
                        },
                    ),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    for (path, (mode, origin)) in recipes {
        let bytes = data(&origin, token)?;
        if mode == "120000" {
            require(
                !Path::new(std::ffi::OsStr::new(std::str::from_utf8(&bytes).map_err(
                    |_| {
                        crate::Error::Invalid(
                            "non-UTF-8 link target needs explicit admission".into(),
                        )
                    },
                )?))
                .is_absolute(),
                "absolute source symlink needs explicit admission",
            )?;
            let mut depth = Path::new(&path)
                .parent()
                .map_or(0, |p| p.components().count());
            for part in Path::new(std::str::from_utf8(&bytes).unwrap()).components() {
                match part {
                    Component::ParentDir => {
                        require(depth > 0, "source symlink escapes candidate")?;
                        depth -= 1;
                    }
                    Component::Normal(_) => depth += 1,
                    Component::CurDir => (),
                    _ => {
                        return Err(crate::Error::Invalid(
                            "source symlink escapes candidate".into(),
                        ));
                    }
                }
            }
        }
        files.insert(
            path,
            File {
                mode,
                content_hash: hash(&bytes),
                length: bytes.len() as u64,
                origin,
            },
        );
    }
    source.validate_current(token)?;
    validate_candidate_inputs(run, &files, &project_prefix)?;
    let directory_inputs = run
        .spec
        .input_files
        .iter()
        .filter(|name| source_root.join(name).is_dir())
        .map(|name| super::inputs::relative(source_root, name))
        .collect::<Result<BTreeSet<_>>>()?;
    let mut plan = Plan {
        source,
        selected,
        project_prefix,
        files,
        scope_hash: String::new(),
        directory_inputs,
        selectors,
        project_identity: Some(super::delivery::Identity::read(source_root)?),
    };
    plan.scope_hash = preparation_scope(run, &plan, state)?;
    Ok(plan)
}

fn preparation_scope(run: &Run, plan: &Plan, state: &Path) -> Result<String> {
    Ok(hash(serde_json::to_vec(&(
        "prepare_workspace_v2",
        &run.id,
        &run.spec,
        run.spec_revision,
        &run.input_snapshot,
        run.diff_baseline(),
        &plan.source,
        &plan.selected,
        &plan.files,
        &plan.project_prefix,
        state,
        &plan.selectors,
        &plan.directory_inputs,
        &plan.project_identity,
    ))?))
}

/// A runtime-produced proposal can be authenticated without rerunning Git while
/// the UI waits on IPC. Source drift is rechecked by the preparation worker.
pub fn pending<'a>(run: &'a Run, state: &Path) -> Result<&'a Plan> {
    eligible(run)?;
    let plan = run.workspace_plan.as_ref().ok_or_else(|| {
        crate::Error::Invalid("inspect workspace inputs before preparing the candidate".into())
    })?;
    require(
        plan.scope_hash == preparation_scope(run, plan, &state.canonicalize()?)?,
        "workspace preparation proposal no longer matches the task",
    )?;
    Ok(plan)
}

fn validate_candidate_inputs(
    run: &Run,
    files: &BTreeMap<String, File>,
    prefix: &str,
) -> Result<()> {
    let root = Path::new(&run.spec.workspace);
    require(
        super::read_inputs(root, &run.spec.input_files)? == run.input_snapshot,
        "task sources changed before candidate preparation",
    )?;
    for (name, text) in &run.input_snapshot {
        let relative = super::inputs::relative(root, name)?;
        let path = format!("{prefix}{relative}");
        if root.join(&relative).is_dir() {
            let entries =
                super::inputs::directory_diff_entries(&relative, text)?.ok_or_else(|| {
                    crate::Error::Invalid("directory input observation missing".into())
                })?;
            let scope = if relative == "." {
                prefix.to_owned()
            } else {
                format!("{path}/")
            };
            require(
                !files.contains_key(path.trim_end_matches("/.")),
                "candidate directory became a file",
            )?;
            let candidate: BTreeMap<_, _> = files
                .iter()
                .filter(|(p, _)| p.starts_with(&scope))
                .collect();
            require(
                candidate.len() == entries.len(),
                "candidate directory membership differs from accepted input; include its local changes",
            )?;
            for (member, content) in entries {
                let file = files.get(&format!("{prefix}{member}")).ok_or_else(|| {
                    crate::Error::Invalid(
                        "candidate directory member is absent; include its local change".into(),
                    )
                })?;
                let (digest, link) = match content {
                    super::inputs::DiffContent::Text(text) => (hash(text), false),
                    super::inputs::DiffContent::Metadata(value) => (
                        value["sha256"]
                            .as_str()
                            .ok_or_else(|| {
                                crate::Error::Invalid("captured member hash missing".into())
                            })?
                            .to_owned(),
                        value["kind"] == "symlink",
                    ),
                };
                require(
                    file.content_hash == digest && (file.mode == "120000") == link,
                    "candidate directory content differs from accepted input; include its local change",
                )?;
            }
        } else {
            require(
                files
                    .get(&path)
                    .is_some_and(|file| file.content_hash == hash(text)),
                "candidate task input differs from accepted contract; explicitly include its local change",
            )?;
        }
    }
    Ok(())
}
fn command(
    root: &Path,
    args: &[&str],
    input: Option<std::fs::File>,
    token: &AtomicBool,
) -> Result<String> {
    require(
        !token.load(Ordering::SeqCst),
        "workspace preparation cancelled",
    )?;
    let mut c = crate::process::command("git")?;
    c.env_clear();
    for name in ["PATH", "HOME", "TMPDIR"] {
        if let Some(v) = std::env::var_os(name) {
            c.env(name, v);
        }
    }
    c.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "commit.gpgSign=false",
            "-c",
            "core.autocrlf=false",
            "-C",
        ])
        .arg(root)
        .args(args)
        .stdin(input.map_or_else(Stdio::null, Stdio::from))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = crate::process::SupervisedChild::spawn(&mut c)?;
    let mut out = child.child.stdout.take().unwrap();
    let mut err = child.child.stderr.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        out.read_to_end(&mut bytes).map(|_| bytes)
    });
    let errors = std::thread::spawn(move || std::io::copy(&mut err, &mut std::io::sink()));
    let status = loop {
        if token.load(Ordering::SeqCst) {
            child.cancel()?;
            break None;
        }
        if let Some(s) = child.child.try_wait()? {
            break Some(s);
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let output = reader
        .join()
        .map_err(|_| crate::Error::Invalid("workspace Git output reader failed".into()))??;
    errors
        .join()
        .map_err(|_| crate::Error::Invalid("workspace Git error reader failed".into()))??;
    require(
        !token.load(Ordering::SeqCst),
        "workspace preparation cancelled",
    )?;
    require(
        status.is_some_and(|s| s.success()),
        &format!("workspace Git operation failed: {}", args[0]),
    )?;
    String::from_utf8(output)
        .map_err(|_| crate::Error::Invalid("invalid workspace Git response".into()))
}
pub(crate) fn intent(
    run: &Run,
    state: &Path,
    plan: Plan,
    workspace_id: String,
) -> Result<Workspace> {
    let op = id();
    let base = state.join("workspaces");
    let root = base.join(&op);
    require(!root.exists(), "workspace destination already exists")?;
    Ok(Workspace {
        id: op,
        workspace_id,
        state: State::Intent,
        root: root
            .to_str()
            .ok_or_else(|| crate::Error::Invalid("non-UTF-8 workspace destination".into()))?
            .into(),
        original_spec: run.spec.clone(),
        original_revision: run.spec_revision,
        plan,
        receipt: None,
        diagnostic: None,
    })
}
pub(crate) fn prepare(workspace: &Workspace, token: &AtomicBool) -> Result<Receipt> {
    require(
        workspace.state == State::Intent && workspace.receipt.is_none(),
        "workspace intent is not fresh; no replay",
    )?;
    workspace.plan.source.validate_current(token)?;
    let root = Path::new(&workspace.root);
    let base = root.parent().unwrap();
    crate::ipc::private_directory(base)?;
    std::fs::DirBuilder::new().mode(0o700).create(root)?;
    let marker = root.join("owner.json");
    std::fs::write(
        &marker,
        serde_json::to_vec(&(
            "managed_workspace_v1",
            &workspace.id,
            &workspace.workspace_id,
            &workspace.plan.scope_hash,
        ))?,
    )?;
    std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o600))?;
    let repository = root.join("repository.git");
    command(
        root,
        &[
            "init",
            "--bare",
            "--template=",
            repository.to_str().unwrap(),
        ],
        None,
        token,
    )?;
    let stream = root.join("snapshot.import");
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&stream)?;
    output.write_all(b"commit refs/heads/source\ncommitter Architect Workspace <workspace@localhost> 1700000000 +0000\ndata 24\nManaged source snapshot\n\n")?;
    let staged = root.join("files");
    std::fs::DirBuilder::new().mode(0o700).create(&staged)?;
    for (path, file) in &workspace.plan.files {
        require(
            !token.load(Ordering::SeqCst),
            "workspace preparation cancelled",
        )?;
        path_valid(path)?;
        let bytes = data(&file.origin, token)?;
        require(
            hash(&bytes) == file.content_hash && bytes.len() as u64 == file.length,
            "workspace source changed before materialization",
        )?;
        let destination = staged.join(path);
        std::fs::create_dir_all(destination.parent().unwrap())?;
        let mut saved = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&destination)?;
        saved.write_all(&bytes)?;
        saved.sync_all()?;
        write!(
            output,
            "M {} inline {}\ndata {}\n",
            file.mode,
            quoted(path),
            file.length
        )?;
        output.write_all(&bytes)?;
        output.write_all(b"\n")?;
    }
    output.write_all(b"\ndone\n")?;
    output.sync_all()?;
    drop(output);
    command(
        &repository,
        &["fast-import", "--quiet"],
        Some(std::fs::File::open(&stream)?),
        token,
    )?;
    let commit = command(
        &repository,
        &["rev-parse", "refs/heads/source"],
        None,
        token,
    )?
    .trim()
    .to_string();
    let tree = command(
        &repository,
        &["rev-parse", "refs/heads/source^{tree}"],
        None,
        token,
    )?
    .trim()
    .to_string();
    let candidate = root.join("candidate");
    command(
        &repository,
        &[
            "worktree",
            "add",
            "--detach",
            "--no-checkout",
            candidate.to_str().unwrap(),
            &commit,
        ],
        None,
        token,
    )?;
    command(&candidate, &["read-tree", &commit], None, token)?;
    for (path, file) in &workspace.plan.files {
        let target = candidate.join(path);
        std::fs::create_dir_all(target.parent().unwrap())?;
        let bytes = std::fs::read(staged.join(path))?;
        require(
            hash(&bytes) == file.content_hash,
            "private staged source corrupted",
        )?;
        if file.mode == "120000" {
            std::os::unix::fs::symlink(
                std::ffi::OsStr::new(std::str::from_utf8(&bytes).unwrap()),
                &target,
            )?;
        } else {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(if file.mode == "100755" { 0o700 } else { 0o600 })
                .open(&target)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
        }
    }
    let project_root = candidate.join(&workspace.plan.project_prefix);
    if !project_root.exists() {
        std::fs::create_dir_all(&project_root)?;
    }
    for scope in &workspace.plan.directory_inputs {
        let mut path = project_root.clone();
        for part in Path::new(scope).components() {
            if let Component::Normal(part) = part {
                path.push(part);
                match path.symlink_metadata() {
                    Ok(metadata) => require(
                        metadata.is_dir() && !metadata.file_type().is_symlink(),
                        "prepared input directory contains a link or file",
                    )?,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
    }
    workspace.plan.source.validate_current(token)?;
    let observed = snapshot::observe(&candidate, &BTreeSet::new(), token)?;
    require(
        observed.local_changes.is_empty()
            && observed
                .repository
                .as_ref()
                .is_some_and(|r| r.head.as_ref() == Some(&commit)),
        "prepared candidate does not match its committed snapshot",
    )?;
    for (path, file) in &workspace.plan.files {
        require(
            observed
                .files
                .get(path)
                .and_then(|f| f.as_ref())
                .and_then(|f| f.content_hash.as_ref())
                == Some(&file.content_hash),
            "prepared file does not match approved source",
        )?;
    }
    let receipt = Receipt {
        commit,
        tree,
        candidate: candidate.to_str().unwrap().into(),
        project_root: project_root.to_str().unwrap().into(),
        git_pointer_hash: hash(std::fs::read(candidate.join(".git"))?),
        repository_config_hash: hash(std::fs::read(repository.join("config"))?),
        observed,
    };
    let result = root.join("result.json");
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&result)?;
    f.write_all(&serde_json::to_vec(&receipt)?)?;
    f.sync_all()?;
    std::fs::File::open(root)?.sync_all()?;
    Ok(receipt)
}
pub fn observed_result(workspace: &Workspace) -> Result<Receipt> {
    let root = Path::new(&workspace.root);
    crate::ipc::private_directory(root)?;
    require(
        serde_json::from_slice::<(String, String, String, String)>(&std::fs::read(
            root.join("owner.json"),
        )?)? == (
            "managed_workspace_v1".into(),
            workspace.id.clone(),
            workspace.workspace_id.clone(),
            workspace.plan.scope_hash.clone(),
        ),
        "workspace ownership marker changed",
    )?;
    let receipt: Receipt = crate::transport::decode(&std::fs::read(root.join("result.json"))?)?;
    require(
        Path::new(&receipt.candidate) == root.join("candidate")
            && Path::new(&receipt.project_root)
                == root.join("candidate").join(&workspace.plan.project_prefix),
        "workspace result paths changed",
    )?;
    require(
        receipt.git_pointer_hash
            == hash(std::fs::read(Path::new(&receipt.candidate).join(".git"))?)
            && receipt.repository_config_hash
                == hash(std::fs::read(root.join("repository.git/config"))?),
        "workspace Git administration changed",
    )?;
    receipt.observed.validate_current(&AtomicBool::new(false))?;
    Ok(receipt)
}

pub fn recovery_scope(run: &Run) -> Result<String> {
    let workspace = run
        .managed_workspace
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("no managed workspace".into()))?;
    require(
        matches!(workspace.state, State::Intent | State::Unknown),
        "workspace output already captured",
    )?;
    let receipt = observed_result(workspace)?;
    Ok(hash(serde_json::to_vec(&(
        "restore_workspace_result_v1",
        run,
        receipt,
    ))?))
}
pub fn activation_scope(run: &Run) -> Result<String> {
    let workspace = run
        .managed_workspace
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("no managed workspace".into()))?;
    require(
        workspace.state == State::Captured
            && !run.reconciliation_required
            && !matches!(
                run.status,
                crate::domain::Status::Running
                    | crate::domain::Status::Paused
                    | crate::domain::Status::Completed
                    | crate::domain::Status::Cancelled
            ),
        "workspace is not captured and stopped",
    )?;
    let receipt = workspace
        .receipt
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("captured workspace receipt missing".into()))?;
    Ok(hash(serde_json::to_vec(&(
        "activate_workspace_v2",
        &run.id,
        &run.spec,
        run.spec_revision,
        &run.input_snapshot,
        &workspace.id,
        &workspace.plan.scope_hash,
        receipt,
    ))?))
}
pub(crate) fn activate(run: &mut Run, scope: &str) -> Result<()> {
    require(
        activation_scope(run)? == scope,
        "workspace activation scope changed",
    )?;
    let workspace = run.managed_workspace.as_ref().unwrap();
    let observed = observed_result(workspace)?;
    require(
        workspace
            .receipt
            .as_ref()
            .is_some_and(|r| serde_json::to_value(r).ok() == serde_json::to_value(&observed).ok()),
        "captured workspace result changed",
    )?;
    require(
        run.team.is_none()
            && run.artifacts.is_empty()
            && run.spec_revision == workspace.original_revision
            && serde_json::to_value(&run.spec)? == serde_json::to_value(&workspace.original_spec)?,
        "task contract changed after workspace preparation",
    )?;
    let receipt = workspace.receipt.as_ref().unwrap();
    let original_root = Path::new(&run.spec.workspace);
    let inputs: Vec<_> = run
        .spec
        .input_files
        .iter()
        .map(|name| super::inputs::relative(original_root, name))
        .collect::<Result<_>>()?;
    let expected: BTreeMap<_, _> = run
        .input_snapshot
        .iter()
        .map(|(name, text)| Ok((super::inputs::relative(original_root, name)?, text.clone())))
        .collect::<Result<_>>()?;
    let mut baseline: BTreeMap<_, _> = run
        .diff_baseline()
        .iter()
        .map(|(name, text)| Ok((super::inputs::relative(original_root, name)?, text.clone())))
        .collect::<Result<_>>()?;
    let candidate_inputs = super::read_inputs(Path::new(&receipt.project_root), &inputs)?;
    require(
        super::inputs::equivalent_sources(&expected, &candidate_inputs)?,
        "prepared inputs differ from accepted task",
    )?;
    // Before any task executes, cover the complete prepared project so unrelated
    // unchanged HEAD files are not falsely reported as additions by the diff.
    if !baseline.contains_key(".") {
        let whole = super::read_inputs(Path::new(&receipt.project_root), &[".".into()])?;
        baseline.insert(".".into(), whole["."].clone());
    }
    let editable = run
        .spec
        .editable_files
        .iter()
        .map(|name| super::inputs::relative(original_root, name))
        .collect::<Result<_>>()?;
    run.spec.input_files = inputs;
    run.spec.editable_files = editable;
    run.input_snapshot = candidate_inputs;
    run.diff_baseline = Some(baseline);
    run.spec.workspace = receipt.project_root.clone();
    run.spec_revision = run
        .spec_revision
        .checked_add(1)
        .ok_or_else(|| crate::Error::Invalid("workspace revision overflow".into()))?;
    run.managed_workspace.as_mut().unwrap().state = State::Active;
    run.communication_ledger.cancel_queued()?;
    run.active_session = None;
    run.stage = crate::domain::Stage::Discovery;
    run.status = crate::domain::Status::AwaitingStart;
    run.approvals.push(crate::domain::Approval {
        action: "activate_workspace".into(),
        scope_hash: scope.into(),
        spec_revision: run.spec_revision,
        actor: "local_user".into(),
    });
    crate::verification::refresh_provenance(run);
    run.diagnostic=Some("Private workspace activated in a new revision; old session directories/history retained. Team planning and execution need separate approval.".into());
    Ok(())
}
pub(crate) fn require_active(run: &Run) -> Result<()> {
    if run
        .spec
        .config
        .as_ref()
        .is_none_or(|c| c.config.workspace.mode != crate::config::WorkspaceMode::Worktree)
    {
        return Ok(());
    }
    let workspace = run.managed_workspace.as_ref().ok_or_else(|| {
        crate::Error::Invalid(
            "prepare and activate the configured worktree before executing tasks".into(),
        )
    })?;
    require(
        workspace.state == State::Active,
        "managed workspace is not active",
    )?;
    let receipt = workspace
        .receipt
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("active workspace receipt missing".into()))?;
    require(
        run.spec.workspace == receipt.project_root
            && Path::new(&receipt.candidate).canonicalize()?.to_str()
                == Some(receipt.observed.root.as_str()),
        "active workspace identity changed",
    )?;
    require(
        receipt.git_pointer_hash
            == hash(std::fs::read(Path::new(&receipt.candidate).join(".git"))?)
            && receipt.repository_config_hash
                == hash(std::fs::read(
                    Path::new(&workspace.root).join("repository.git/config"),
                )?),
        "active workspace Git administration changed",
    )
}
