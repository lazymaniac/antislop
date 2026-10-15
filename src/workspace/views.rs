//! Per-task Git views for team runs. A private repository in the runtime data
//! root holds the run base commit, one branch per writer task and the commits
//! its verified results produce. The user's checkout is only read here; it is
//! written by integration (`runtime::team::integrate`), never by task work.
use crate::{Result, require};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
};

/// Git object-database and worktree-admin updates are serialized per process;
/// task workers may prepare views concurrently.
static GIT: Mutex<()> = Mutex::new(());

/// Runtime-private directories that are never project content.
pub const RUNTIME_STATE: [&str; 2] = [".antislop", ".architect"];

#[derive(Debug, Clone)]
pub struct Repo {
    pub dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: String,
    pub mode: String,
    pub blob: String,
    pub size: u64,
}

pub struct Output {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

fn isolated(command: &mut Command) {
    command.env_clear();
    for name in ["PATH", "HOME", "TMPDIR"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.autocrlf=false",
            "-c",
            "commit.gpgSign=false",
            "-c",
            "gc.auto=0",
            "-c",
            "user.name=antislop",
            "-c",
            "user.email=runtime@antislop.invalid",
        ]);
}

/// Run Git with an isolated configuration. Nonzero exit is returned, not raised.
pub fn git(
    cwd: &Path,
    args: &[&str],
    env: &[(&str, &Path)],
    input: Option<&[u8]>,
) -> Result<Output> {
    let mut command = crate::process::command("git")?;
    isolated(&mut command);
    for (name, value) in env {
        command.env(name, value);
    }
    command
        .args(args)
        .current_dir(cwd)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = crate::process::SupervisedChild::spawn(&mut command)?;
    let mut stdout = child.child.stdout.take().expect("git stdout");
    let mut stderr = child.child.stderr.take().expect("git stderr");
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let errors = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });
    if let Some(bytes) = input {
        let mut stdin = child.child.stdin.take().expect("git stdin");
        stdin.write_all(bytes)?;
    }
    let status = child.child.wait()?;
    let stdout = reader
        .join()
        .map_err(|_| crate::Error::Invalid("task view Git output reader failed".into()))??;
    let stderr = errors
        .join()
        .map_err(|_| crate::Error::Invalid("task view Git error reader failed".into()))??;
    Ok(Output {
        status: status.code().unwrap_or(-1),
        stdout,
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

fn checked(output: Output, what: &str) -> Result<String> {
    require(
        output.status == 0,
        &format!("task view Git {what} failed: {}", output.stderr.trim()),
    )?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

impl Repo {
    pub fn open_or_init(dir: &Path) -> Result<Self> {
        let _guard = GIT.lock().expect("git mutex");
        if !dir.join("HEAD").is_file() {
            std::fs::create_dir_all(dir)?;
            checked(
                git(dir, &["init", "--bare", "--quiet", "."], &[], None)?,
                "init",
            )?;
        }
        Ok(Self {
            dir: dir.canonicalize()?,
        })
    }
    fn run(&self, args: &[&str], input: Option<&[u8]>) -> Result<Output> {
        let mut full = vec!["--git-dir", self.dir.to_str().unwrap()];
        full.extend_from_slice(args);
        git(&self.dir, &full, &[], input)
    }
    fn ok(&self, args: &[&str], what: &str) -> Result<String> {
        checked(self.run(args, None)?, what)
    }
    /// Capture the user's current files (tracked, nonignored untracked and local
    /// edits) as one parentless commit. Ignored build output is not project source.
    pub fn capture(&self, root: &Path, parent: Option<&str>, message: &str) -> Result<String> {
        let root = root.canonicalize()?;
        let excluded: BTreeSet<String> = RUNTIME_STATE.iter().map(|s| s.to_string()).collect();
        let (_, paths) = super::snapshot::source_paths(
            &root,
            &excluded,
            &std::sync::atomic::AtomicBool::new(false),
        )?;
        let mut list = Vec::new();
        for path in paths
            .iter()
            .filter(|p| root.join(p).symlink_metadata().is_ok())
        {
            list.extend_from_slice(path.as_bytes());
            list.push(0);
        }
        let _guard = GIT.lock().expect("git mutex");
        let index = self.dir.join(format!("capture-{}.index", crate::id()));
        let result = (|| {
            let env = [("GIT_INDEX_FILE", index.as_path())];
            let git_dir = self.dir.to_str().unwrap().to_owned();
            let work_tree = root.to_str().unwrap().to_owned();
            let add = git(
                &root,
                &[
                    "--git-dir",
                    &git_dir,
                    "--work-tree",
                    &work_tree,
                    "add",
                    "--force",
                    "--pathspec-from-file=-",
                    "--pathspec-file-nul",
                ],
                &env,
                Some(&list),
            )?;
            checked(add, "capture")?;
            let tree = checked(
                git(&root, &["--git-dir", &git_dir, "write-tree"], &env, None)?,
                "write-tree",
            )?;
            let mut args = vec!["--git-dir", &git_dir, "commit-tree", &tree, "-m", message];
            if let Some(parent) = parent {
                args.extend(["-p", parent]);
            }
            checked(git(&root, &args, &env, None)?, "commit-tree")
        })();
        let _ = std::fs::remove_file(&index);
        result
    }
    /// Keep a commit reachable; the private repository never relies on reflogs.
    pub fn pin(&self, name: &str, commit: &str) -> Result<()> {
        self.ok(
            &["update-ref", &format!("refs/antislop/{name}"), commit],
            "update-ref",
        )?;
        Ok(())
    }
    /// Files of a commit with blob IDs and sizes.
    pub fn files(&self, commit: &str) -> Result<Vec<Entry>> {
        let out = self.run(
            &["ls-tree", "-r", "-z", "--long", "--full-tree", commit],
            None,
        )?;
        let text = String::from_utf8_lossy(&checked_bytes(out, "ls-tree")?).into_owned();
        Ok(text
            .split('\0')
            .filter(|line| !line.is_empty())
            .filter_map(|line| {
                let (meta, path) = line.split_once('\t')?;
                let mut fields = meta.split_whitespace();
                let mode = fields.next()?.to_owned();
                let kind = fields.next()?;
                let blob = fields.next()?.to_owned();
                let size = fields.next()?.parse().unwrap_or(0);
                (kind == "blob").then(|| Entry {
                    path: path.to_owned(),
                    mode,
                    blob,
                    size,
                })
            })
            .collect())
    }
    /// Read several blobs in one Git process.
    pub fn blobs(&self, ids: &[&str]) -> Result<Vec<Vec<u8>>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let mut input = Vec::new();
        for id in ids {
            input.extend_from_slice(id.as_bytes());
            input.push(b'\n');
        }
        let out = checked_bytes(
            self.run(&["cat-file", "--batch"], Some(&input))?,
            "cat-file",
        )?;
        let mut blobs = Vec::with_capacity(ids.len());
        let mut rest = out.as_slice();
        for _ in ids {
            let newline = rest
                .iter()
                .position(|b| *b == b'\n')
                .ok_or_else(|| crate::Error::Invalid("truncated Git blob batch".into()))?;
            let header = String::from_utf8_lossy(&rest[..newline]).into_owned();
            let size: usize = header
                .rsplit(' ')
                .next()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| crate::Error::Invalid(format!("missing Git blob: {header}")))?;
            let start = newline + 1;
            require(rest.len() > start + size, "truncated Git blob batch")?;
            blobs.push(rest[start..start + size].to_vec());
            rest = &rest[start + size + 1..];
        }
        Ok(blobs)
    }
    /// Commit `files` (None deletes) on top of `parent`; `merged` records another
    /// parent whose result these files carry.
    pub fn apply_files(
        &self,
        parent: &str,
        merged: Option<&str>,
        files: &[(String, Option<Vec<u8>>)],
        message: &str,
    ) -> Result<String> {
        let _guard = GIT.lock().expect("git mutex");
        let index = self.dir.join(format!("apply-{}.index", crate::id()));
        let git_dir = self.dir.to_str().unwrap().to_owned();
        let result = (|| {
            let env = [("GIT_INDEX_FILE", index.as_path())];
            let run = |args: &[&str], input: Option<&[u8]>| -> Result<String> {
                let mut full = vec!["--git-dir", git_dir.as_str()];
                full.extend_from_slice(args);
                checked(git(&self.dir, &full, &env, input)?, args[0])
            };
            run(&["read-tree", parent], None)?;
            for (path, content) in files {
                match content {
                    Some(bytes) => {
                        let blob = run(&["hash-object", "-w", "--stdin"], Some(bytes))?;
                        let mode = run(&["ls-files", "--stage", "--", path], None)?
                            .split_whitespace()
                            .next()
                            .filter(|mode| mode.starts_with("100"))
                            .unwrap_or("100644")
                            .to_owned();
                        run(
                            &[
                                "update-index",
                                "--add",
                                "--cacheinfo",
                                &format!("{mode},{blob},{path}"),
                            ],
                            None,
                        )?;
                    }
                    None => {
                        run(&["update-index", "--force-remove", "--", path], None)?;
                    }
                }
            }
            let tree = run(&["write-tree"], None)?;
            let mut args = vec!["commit-tree", tree.as_str(), "-p", parent, "-m", message];
            if let Some(merged) = merged {
                args.extend(["-p", merged]);
            }
            run(&args, None)
        })();
        let _ = std::fs::remove_file(&index);
        result
    }
    /// Create (or confirm) a checkout of `commit`. Writers get a branch; readers
    /// get a detached view. An existing checkout must be at the same commit.
    pub fn checkout(&self, path: &Path, commit: &str, branch: Option<&str>) -> Result<()> {
        let _guard = GIT.lock().expect("git mutex");
        if path.exists() {
            let head = checked(git(path, &["rev-parse", "HEAD"], &[], None)?, "rev-parse")?;
            require(
                head == commit || branch.is_some(),
                "existing task view is at another commit",
            )?;
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let target = path.to_str().unwrap();
        let mut args = vec!["worktree", "add", "--quiet"];
        if let Some(branch) = branch {
            args.extend(["-B", branch]);
        } else {
            args.push("--detach");
        }
        args.extend([target, commit]);
        self.ok(&args, "worktree add")?;
        Ok(())
    }
    pub fn head(&self, path: &Path) -> Result<String> {
        checked(git(path, &["rev-parse", "HEAD"], &[], None)?, "rev-parse")
    }
    /// Commit all tracked/nonignored changes of a writer checkout. Unchanged
    /// checkouts keep their current commit.
    pub fn commit(&self, path: &Path, message: &str) -> Result<String> {
        let _guard = GIT.lock().expect("git mutex");
        checked(git(path, &["add", "--all"], &[], None)?, "add")?;
        if git(path, &["diff", "--cached", "--quiet"], &[], None)?.status != 0 {
            checked(
                git(
                    path,
                    &["commit", "--quiet", "--no-verify", "-m", message],
                    &[],
                    None,
                )?,
                "commit",
            )?;
        }
        checked(git(path, &["rev-parse", "HEAD"], &[], None)?, "rev-parse")
    }
    /// Discard uncommitted (never ignored) changes, e.g. partial writes of an
    /// interrupted turn or files a check modified.
    pub fn reset(&self, path: &Path, commit: &str) -> Result<()> {
        let _guard = GIT.lock().expect("git mutex");
        checked(
            git(path, &["reset", "--quiet", "--hard", commit], &[], None)?,
            "reset",
        )?;
        checked(git(path, &["clean", "--quiet", "-fd"], &[], None)?, "clean")?;
        Ok(())
    }
    pub fn clean(&self, path: &Path) -> Result<bool> {
        let status = checked(
            git(
                path,
                &["status", "--porcelain", "--untracked-files=all"],
                &[],
                None,
            )?,
            "status",
        )?;
        Ok(status.is_empty())
    }
    /// Combine several result commits into one view. Returns conflicting paths
    /// instead of a commit when their changes overlap.
    pub fn combine(&self, commits: &[String]) -> Result<std::result::Result<String, Vec<String>>> {
        require(!commits.is_empty(), "no commits to combine")?;
        let unique: Vec<&String> = commits
            .iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let _guard = GIT.lock().expect("git mutex");
        let mut merged = unique[0].clone();
        for next in &unique[1..] {
            if self
                .run(&["merge-base", "--is-ancestor", next, &merged], None)?
                .status
                == 0
            {
                continue;
            }
            if self
                .run(&["merge-base", "--is-ancestor", &merged, next], None)?
                .status
                == 0
            {
                merged = (*next).clone();
                continue;
            }
            let output = self.run(
                &[
                    "merge-tree",
                    "--write-tree",
                    "--name-only",
                    "--no-messages",
                    &merged,
                    next,
                ],
                None,
            )?;
            let text = String::from_utf8_lossy(&output.stdout).into_owned();
            let mut lines = text.lines();
            let tree = lines.next().unwrap_or_default().to_owned();
            if output.status == 1 {
                return Ok(Err(lines
                    .filter(|l| !l.is_empty())
                    .map(str::to_owned)
                    .collect()));
            }
            checked(output, "merge-tree")?;
            merged = self.ok(
                &[
                    "commit-tree",
                    &tree,
                    "-p",
                    &merged,
                    "-p",
                    next,
                    "-m",
                    "antislop: combine dependency results",
                ],
                "commit-tree",
            )?;
        }
        Ok(Ok(merged))
    }
    /// Paths changed between two commits, with Git status letters.
    pub fn changed(&self, base: &str, result: &str) -> Result<Vec<(char, String)>> {
        let out = self.run(
            &["diff", "--name-status", "--no-renames", "-z", base, result],
            None,
        )?;
        let text = String::from_utf8_lossy(&checked_bytes(out, "diff")?).into_owned();
        let mut parts = text.split('\0').filter(|s| !s.is_empty());
        let mut changes = vec![];
        while let (Some(status), Some(path)) = (parts.next(), parts.next()) {
            changes.push((status.chars().next().unwrap_or('M'), path.to_owned()));
        }
        Ok(changes)
    }
    pub fn diff(&self, base: &str, result: &str) -> Result<String> {
        let out = self.run(
            &[
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--no-renames",
                base,
                result,
            ],
            None,
        )?;
        Ok(String::from_utf8_lossy(&checked_bytes(out, "diff")?).into_owned())
    }
    /// File bytes at a commit; `None` if absent.
    pub fn show(&self, commit: &str, path: &str) -> Result<Option<Vec<u8>>> {
        let spec = format!("{commit}:{path}");
        if self.run(&["cat-file", "-e", &spec], None)?.status != 0 {
            return Ok(None);
        }
        Ok(Some(checked_bytes(
            self.run(&["cat-file", "blob", &spec], None)?,
            "cat-file",
        )?))
    }
    pub fn is_ancestor(&self, ancestor: &str, commit: &str) -> Result<bool> {
        Ok(self
            .run(&["merge-base", "--is-ancestor", ancestor, commit], None)?
            .status
            == 0)
    }
    pub fn remove(&self, path: &Path) -> Result<()> {
        let _guard = GIT.lock().expect("git mutex");
        if path.exists() {
            let target = path.to_str().unwrap();
            self.ok(
                &["worktree", "remove", "--force", target],
                "worktree remove",
            )?;
        }
        Ok(())
    }
}

fn checked_bytes(output: Output, what: &str) -> Result<Vec<u8>> {
    require(
        output.status == 0,
        &format!("task view Git {what} failed: {}", output.stderr.trim()),
    )?;
    Ok(output.stdout)
}

/// Three-way merge of one file. Returns the merged bytes and whether conflict
/// markers remain. `None` sides mean the file is absent on that side.
pub fn merge_file(
    scratch: &Path,
    base: Option<&[u8]>,
    ours: Option<&[u8]>,
    theirs: Option<&[u8]>,
    labels: (&str, &str),
) -> Result<(Option<Vec<u8>>, bool)> {
    match (base, ours, theirs) {
        (_, o, t) if o == t => return Ok((o.map(<[u8]>::to_vec), false)),
        (b, o, t) if b == o => return Ok((t.map(<[u8]>::to_vec), false)),
        (b, o, t) if b == t => return Ok((o.map(<[u8]>::to_vec), false)),
        (_, None, _) | (_, _, None) => {
            // One side deleted while the other changed it.
            return Ok((ours.or(theirs).map(<[u8]>::to_vec), true));
        }
        _ => (),
    }
    let dir = scratch.join(format!("merge-{}", crate::id()));
    std::fs::create_dir_all(&dir)?;
    let result = merge_in(&dir, base, ours, theirs, labels);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn merge_in(
    dir: &Path,
    base: Option<&[u8]>,
    ours: Option<&[u8]>,
    theirs: Option<&[u8]>,
    labels: (&str, &str),
) -> Result<(Option<Vec<u8>>, bool)> {
    let write = |name: &str, bytes: Option<&[u8]>| -> Result<PathBuf> {
        let path = dir.join(name);
        std::fs::write(&path, bytes.unwrap_or_default())?;
        Ok(path)
    };
    let ours_path = write("ours", ours)?;
    let base_path = write("base", base)?;
    let theirs_path = write("theirs", theirs)?;
    let output = git(
        dir,
        &[
            "merge-file",
            "-p",
            "--diff3",
            "-L",
            labels.0,
            "-L",
            "base",
            "-L",
            labels.1,
            ours_path.to_str().unwrap(),
            base_path.to_str().unwrap(),
            theirs_path.to_str().unwrap(),
        ],
        &[],
        None,
    )?;
    // Exit status counts conflicts; errors are reported as 128 or above.
    require(
        (0..128).contains(&output.status),
        &format!("git merge-file failed: {}", output.stderr.trim()),
    )?;
    Ok((Some(output.stdout), output.status > 0))
}

fn project_path(root: &Path, path: &str) -> Result<PathBuf> {
    let mut target = root.to_path_buf();
    let components: Vec<_> = Path::new(path).components().collect();
    require(
        !components.is_empty()
            && components
                .iter()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
            && !RUNTIME_STATE
                .iter()
                .any(|state| path == *state || path.starts_with(&format!("{state}/"))),
        "task result path escapes the project",
    )?;
    for component in components {
        target.push(component);
        if let Ok(meta) = target.symlink_metadata() {
            require(
                !meta.file_type().is_symlink(),
                "task result path crosses a symlink",
            )?;
        }
    }
    Ok(target)
}

/// Current bytes of a project file; `None` if absent.
pub fn read_project(root: &Path, path: &str) -> Result<Option<Vec<u8>>> {
    let target = project_path(root, path)?;
    match std::fs::read(&target) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Replace (or delete) one project file only if it still holds `expected`.
/// The new file is renamed into place, never written through existing links.
pub fn land(
    root: &Path,
    path: &str,
    expected: Option<&[u8]>,
    content: Option<&[u8]>,
) -> Result<()> {
    let target = project_path(root, path)?;
    require(
        read_project(root, path)?.as_deref() == expected,
        &format!("{path} changed in the project during integration"),
    )?;
    let Some(content) = content else {
        if expected.is_some() {
            std::fs::remove_file(&target)?;
        }
        return Ok(());
    };
    let parent = target
        .parent()
        .ok_or_else(|| crate::Error::Invalid("task result has no parent".into()))?;
    std::fs::create_dir_all(parent)?;
    let permissions = std::fs::metadata(&target).ok().map(|m| m.permissions());
    let temporary = parent.join(format!(".antislop-{}.tmp", crate::id()));
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(content)?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)?;
        }
        file.sync_all()?;
        require(
            read_project(root, path)?.as_deref() == expected,
            &format!("{path} changed in the project during integration"),
        )?;
        std::fs::rename(&temporary, &target)?;
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Git in the user's repository with the user's identity and configuration.
/// Hooks are disabled: integration commits only reviewed task results.
pub fn project_git(root: &Path, args: &[&str]) -> Result<Output> {
    let mut command = crate::process::command("git")?;
    command.env_clear();
    for name in ["PATH", "HOME", "TMPDIR", "XDG_CONFIG_HOME"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
        ])
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = crate::process::SupervisedChild::spawn(&mut command)?;
    let mut stdout = child.child.stdout.take().expect("git stdout");
    let mut stderr = child.child.stderr.take().expect("git stderr");
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let mut error = Vec::new();
    stderr.read_to_end(&mut error)?;
    let status = child.child.wait()?;
    let stdout = reader
        .join()
        .map_err(|_| crate::Error::Invalid("project Git output reader failed".into()))??;
    Ok(Output {
        status: status.code().unwrap_or(-1),
        stdout,
        stderr: String::from_utf8_lossy(&error).into_owned(),
    })
}

/// Whether `root` is the top level of a Git work tree.
pub fn project_repository(root: &Path) -> Result<bool> {
    let output = project_git(root, &["rev-parse", "--show-toplevel"])?;
    if output.status != 0 {
        return Ok(false);
    }
    let top = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    Ok(top.canonicalize().ok() == root.canonicalize().ok())
}

/// Paths (of `paths`) with local, uncommitted changes relative to HEAD.
pub fn project_dirty(root: &Path, paths: &[String]) -> Result<BTreeSet<String>> {
    if paths.is_empty() {
        return Ok(BTreeSet::new());
    }
    let mut args = vec![
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
        "--no-renames",
        "--",
    ];
    args.extend(paths.iter().map(String::as_str));
    let output = project_git(root, &args)?;
    let text = String::from_utf8_lossy(&checked_bytes(output, "status")?).into_owned();
    Ok(text
        .split('\0')
        .filter(|entry| entry.len() > 3)
        .map(|entry| entry[3..].to_owned())
        .collect())
}
