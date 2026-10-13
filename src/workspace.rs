//! Worktrees isolate file ownership, not permissions. External commands remain
//! explicitly authorized local execution, never a substitute for an OS sandbox.
use crate::{Result, hash, require};
use std::{
    collections::BTreeMap,
    path::{Component, Path},
    process::Command,
};

pub fn read_input_bytes(root: &Path, name: &str) -> Result<Vec<u8>> {
    let root = root.canonicalize()?;
    let relative = Path::new(name);
    require(!relative.as_os_str().is_empty() && relative.components().all(|c| matches!(c, Component::Normal(_))),
        "input path must be a normal relative path")?;
    let mut path = root.clone();
    for component in relative.components() {
        path.push(component);
        require(!std::fs::symlink_metadata(&path)?.file_type().is_symlink(), "symlink inputs are not supported")?;
    }
    require(path.canonicalize()?.starts_with(&root), "input escaped workspace")?;
    Ok(std::fs::read(path)?)
}
pub fn read_inputs(root: &Path, paths: &[String]) -> Result<BTreeMap<String, String>> {
    paths.iter().map(|name| {
        let bytes = read_input_bytes(root, name)?;
        let text = String::from_utf8(bytes).map_err(|_| crate::Error::Invalid("pinned input is not UTF-8".into()))?;
        Ok((name.clone(), text))
    }).collect()
}
pub fn worktree(repository: &Path, destination: &Path) -> Result<()> {
    require(!destination.exists(), "worktree destination exists")?;
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["worktree", "add", "--detach"])
        .arg(destination)
        .arg("HEAD")
        .output()?;
    require(output.status.success(), "git worktree creation failed")
}
/// Compare-and-write refuses a changed input rather than overwriting user work.
pub fn replace(root: &Path, relative: &str, expected_hash: &str, content: &str) -> Result<()> {
    let inputs = read_inputs(root, &[relative.into()])?;
    require(
        hash(&inputs[relative]) == expected_hash,
        "workspace input changed; replan required",
    )?;
    // A single runtime writer owns this worktree. Concurrent external writers are
    // detected before write, but arbitrary hostile filesystem races need sandboxing.
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let target = root.join(relative);
    let permissions = std::fs::metadata(&target)?.permissions();
    let temporary = target.with_file_name(format!(".architect-{}.tmp", crate::id()));
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(content.as_bytes())?;
        file.set_permissions(permissions)?;
        file.sync_all()?;
        // Rename replaces this directory entry instead of mutating shared hard links.
        std::fs::rename(&temporary, &target)?;
        if let Some(parent) = target.parent() {
            std::fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result?;
    Ok(())
}

pub fn check(
    root: &Path,
    argv: &[String],
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<(bool, String)> {
    use std::io::Read;
    use std::process::Stdio;
    use std::sync::atomic::Ordering;
    require(!argv.is_empty(), "empty check command")?;
    let mut command = crate::process::command(&argv[0])?;
    command
        .args(&argv[1..])
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = crate::process::SupervisedChild::spawn(&mut command)?;
    let stdout = child.child.stdout.take().expect("check stdout");
    let stderr = child.child.stderr.take().expect("check stderr");
    fn drain(mut source: impl Read) -> std::io::Result<(Vec<u8>, bool)> {
        let mut output = Vec::new();
        let mut chunk = [0; 8192];
        let mut truncated = false;
        loop {
            let count = source.read(&mut chunk)?;
            if count == 0 {
                break;
            }
            let keep = count.min((4 * 1024 * 1024usize).saturating_sub(output.len()));
            output.extend_from_slice(&chunk[..keep]);
            truncated |= keep < count;
        }
        Ok((output, truncated))
    }
    let out_reader = std::thread::spawn(move || drain(stdout));
    let err_reader = std::thread::spawn(move || drain(stderr));
    let status = loop {
        if cancelled.load(Ordering::SeqCst) {
            child.cancel()?;
            return Err(crate::Error::Invalid(
                "check cancelled; effects require reconciliation".into(),
            ));
        }
        if let Some(status) = child.child.try_wait()? {
            break status;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    };
    let (stdout, out_truncated) = out_reader
        .join()
        .map_err(|_| crate::Error::Invalid("stdout reader failed".into()))??;
    let (stderr, err_truncated) = err_reader
        .join()
        .map_err(|_| crate::Error::Invalid("stderr reader failed".into()))??;
    let output = serde_json::json!({"argv":argv,"exit_code":status.code(),"stdout":String::from_utf8_lossy(&stdout),
        "stderr":String::from_utf8_lossy(&stderr),"truncated":out_truncated || err_truncated}).to_string();
    Ok((status.success() && !out_truncated && !err_truncated, output))
}
