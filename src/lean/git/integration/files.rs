use super::super::*;
use std::collections::HashMap;

/// Blob ids of `files` at `rev`; files absent there are absent here.
pub(super) fn blobs(dir: &Path, rev: &str, files: &[String]) -> Result<HashMap<String, String>> {
    let mut args = vec!["--literal-pathspecs", "ls-tree", "-r", "-z", rev, "--"];
    args.extend(files.iter().map(String::as_str));
    Ok(ok(dir, &args)?
        .split('\0')
        .filter_map(|entry| {
            let (meta, path) = entry.split_once('\t')?;
            Some((path.to_owned(), meta.split(' ').nth(2)?.to_owned()))
        })
        .collect())
}

/// Whether every one of `files` in the working tree is exactly as at `rev`.
pub(super) fn unchanged_since(project: &Path, rev: &str, files: &[String]) -> Result<bool> {
    let expected = blobs(project, rev, files)?;
    let present: Vec<&str> = files
        .iter()
        .filter(|file| project.join(file).symlink_metadata().is_ok())
        .map(String::as_str)
        .collect();
    if present.len() != expected.len() || present.iter().any(|file| !expected.contains_key(*file)) {
        return Ok(false);
    }
    if present.is_empty() {
        return Ok(true);
    }
    let mut args = vec!["hash-object", "--"];
    args.extend(&present);
    let hashes = ok(project, &args)?;
    Ok(present
        .iter()
        .zip(hashes.lines())
        .all(|(file, hash)| expected[*file] == hash))
}

/// Writes `files` as they are at `rev` into the working tree only.
pub(super) fn write_files(project: &Path, rev: &str, files: &[String]) -> Result<()> {
    let present = blobs(project, rev, files)?;
    for file in files.iter().filter(|file| !present.contains_key(*file)) {
        match std::fs::remove_file(project.join(file)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
    }
    if !present.is_empty() {
        let source = format!("--source={rev}");
        let mut args = vec![
            "--literal-pathspecs",
            "restore",
            &source,
            "--worktree",
            "--",
        ];
        args.extend(present.keys().map(String::as_str));
        ok(project, &args)?;
    }
    Ok(())
}
