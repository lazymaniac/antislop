//! Conditional executable publication. The operator, not flock, excludes other updaters.
use crate::{Error, Result, require};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

mod files;
use files::{Identity, copy_synced, directory_sync, identity, lock, sync_file};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Prepared,
    Publishing,
    Published,
    Restoring,
    Restored,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Journal {
    schema: u32,
    target: PathBuf,
    parent_device: u64,
    parent_inode: u64,
    original: Identity,
    candidate: Identity,
    stage: PathBuf,
    restore_stage: PathBuf,
    replacement: Option<Identity>,
    phase: Phase,
    confirmation: Option<Confirmation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Confirmation {
    action: String,
    challenge: String,
    exclusions: String,
    confirmed_ms: i64,
}

/// Every mutation asks for a newly generated challenge on stdin. No persistent
/// option or old journal entry can authorize a later process after interruption.
pub fn bootstrap(args: &[String]) -> Result<()> {
    let action = args.first().map(String::as_str).unwrap_or("");
    require(
        matches!(
            action,
            "prepare" | "publish" | "resume" | "cancel" | "restore" | "status"
        ),
        "bootstrap requires prepare, publish, resume, cancel, restore or status; see docs/bootstrap.md",
    )?;
    let mut options = std::collections::BTreeMap::new();
    let mut rest = args[1..].iter();
    while let Some(key) = rest.next() {
        require(
            matches!(
                key.as_str(),
                "--journal" | "--target" | "--candidate" | "--exclusions"
            ),
            &format!("unknown bootstrap option: {key}"),
        )?;
        let value = rest
            .next()
            .ok_or_else(|| Error::Invalid(format!("{key} requires a value")))?;
        require(
            options.insert(key.as_str(), value.as_str()).is_none(),
            "duplicate bootstrap option",
        )?;
    }
    let directory = Path::new(options.get("--journal").ok_or_else(|| {
        Error::Invalid("--journal DIR is required (durable directory outside checkouts)".into())
    })?);
    if action == "prepare" {
        require(
            options.len() == 3,
            "prepare requires only --journal, --target and --candidate",
        )?;
        let target = options
            .get("--target")
            .ok_or_else(|| Error::Invalid("--target FILE is required".into()))?;
        let candidate = options
            .get("--candidate")
            .ok_or_else(|| Error::Invalid("--candidate FILE is required".into()))?;
        prepare(directory, Path::new(target), Path::new(candidate))?;
    } else {
        require(
            !options.contains_key("--target") && !options.contains_key("--candidate"),
            "recovery uses the target and candidate in the journal",
        )?;
        let journal = load(directory)?;
        if action == "status" {
            require(options.len() == 1, "status accepts only --journal")?;
            println!("{}", serde_json::to_string_pretty(&journal)?);
        } else {
            let exclusions = options.get("--exclusions").ok_or_else(|| Error::Invalid("--exclusions TEXT must identify stopped updaters and how new writes are prevented".into()))?;
            let _lock = lock(journal.target.parent().unwrap())?;
            let mut journal = load(directory)?;
            let mut input = std::io::stdin().lock();
            let mut output = std::io::stderr().lock();
            confirm_window(&mut journal, action, exclusions, &mut input, &mut output)?;
            save(directory, &journal)?;
            operate(directory, &mut journal, action)?;
        }
    }
    Ok(())
}

fn prepare(directory: &Path, target: &Path, candidate: &Path) -> Result<()> {
    require(
        !fs::symlink_metadata(target)?.file_type().is_symlink(),
        "target must be the verified regular executable path, not a symlink",
    )?;
    let target = target.canonicalize()?;
    let parent = target.parent().unwrap();
    let _lock = lock(parent)?;
    let original = identity(&target)?;
    let source = identity(candidate)?;
    original.check_executable()?;
    source.check_executable()?;
    // A copy with a different owner/group cannot faithfully restore metadata.
    require(
        original.uid == source.uid && original.gid == source.gid,
        "candidate and target owner/group differ",
    )?;
    fs::create_dir(directory)?;
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    directory_sync(directory.parent().unwrap_or(Path::new(".")))?;
    let directory = directory.canonicalize()?;
    require(
        !target.starts_with(&directory),
        "journal must not contain the publication target",
    )?;
    copy_synced(&target, &directory.join("original"), &original)?;
    copy_synced(candidate, &directory.join("candidate"), &source)?;
    directory_sync(&directory)?;
    let id = crate::id();
    let stage = parent.join(format!(".antislop-bootstrap-{id}.stage"));
    copy_synced(&directory.join("candidate"), &stage, &source)?;
    let candidate = identity(&stage)?;
    let metadata = fs::metadata(parent)?;
    let journal = Journal {
        schema: 1,
        target: target.clone(),
        parent_device: metadata.dev(),
        parent_inode: metadata.ino(),
        original,
        candidate,
        stage,
        restore_stage: parent.join(format!(".antislop-bootstrap-{id}.restore")),
        replacement: None,
        phase: Phase::Prepared,
        confirmation: None,
    };
    directory_sync(parent)?;
    save(&directory, &journal)?;
    // Validation is deliberately after durable originals and journal. Failures
    // retain a recoverable prepared operation and never touch the target.
    validate(&journal.stage)?;
    println!("{}", serde_json::to_string_pretty(&journal)?);
    Ok(())
}

fn validate(path: &Path) -> Result<()> {
    #[cfg(test)]
    tests::fail("validation", path)?;
    let output = std::process::Command::new(path)
        .args(["--version", "--json"])
        .output()?;
    require(
        output.status.success(),
        &format!(
            "candidate validation {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ),
    )?;
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    require(
        value["name"] == "antislop"
            && value["protocol"] == crate::lean::daemon::version_info()["protocol"]
            && value["build"].as_str().is_some_and(|s| !s.is_empty())
            && value["capabilities"].as_array().is_some_and(|items| {
                items.iter().any(|v| v == "runtime_preflight")
                    && items.iter().any(|v| v == "runtime_activation_async_v1")
            }),
        "candidate --version --json lacks compatible antislop activation metadata",
    )
}

fn confirm_window(
    journal: &mut Journal,
    action: &str,
    exclusions: &str,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<()> {
    require(!exclusions.trim().is_empty(), "empty updater exclusions")?;
    let challenge = crate::id();
    writeln!(
        output,
        "Target: {}\nAction: {action}\nExcluded updaters / prevention: {exclusions}\nEstablish a NEW window excluding ALL other target and directory updaters (package manager, IDE, installers, other terminals). Keep it through final identity check, rename, directory sync and durable journal settlement. The lock protects cooperating publishers only. File restore does not roll back a live daemon.\nTo attest this window now, type: CONFIRM {challenge}",
        journal.target.display()
    )?;
    output.flush()?;
    let mut response = String::new();
    input.read_line(&mut response)?;
    require(
        response.trim_end_matches(['\r', '\n']) == format!("CONFIRM {challenge}"),
        "publication refused: missing or stale window confirmation; all versions retained",
    )?;
    journal.confirmation = Some(Confirmation {
        action: action.into(),
        challenge,
        exclusions: exclusions.into(),
        confirmed_ms: crate::lean::store::now_ms(),
    });
    Ok(())
}

fn check_parent(journal: &Journal) -> Result<()> {
    let metadata = fs::metadata(journal.target.parent().unwrap())?;
    require(
        metadata.dev() == journal.parent_device && metadata.ino() == journal.parent_inode,
        "target directory identity changed; all versions retained",
    )
}

fn operate(directory: &Path, journal: &mut Journal, action: &str) -> Result<()> {
    check_parent(journal)?;
    let current = identity(&journal.target)?;
    let is_original = current == journal.original;
    let is_candidate = current == journal.candidate;
    let is_restored = journal.replacement.as_ref() == Some(&current);
    require(
        is_original || is_candidate || is_restored,
        "target identity changed; refusing overwrite; original, candidate, stage and journal retained",
    )?;
    match action {
        "resume" if journal.phase == Phase::Restoring => {
            restore(directory, journal, is_original, is_candidate, is_restored)
        }
        "publish" | "resume" => publish(directory, journal, is_original, is_candidate),
        "restore" => restore(directory, journal, is_original, is_candidate, is_restored),
        "cancel" => {
            require(
                journal.phase != Phase::Restoring,
                "restore in progress; use restore to settle it under a new window",
            )?;
            // Cancel settles the observed file without undoing a publication.
            directory_sync(journal.target.parent().unwrap())?;
            journal.phase = Phase::Cancelled;
            save(directory, journal)
        }
        _ => Err(Error::Invalid("unknown bootstrap mutation".into())),
    }
}

fn publish(
    directory: &Path,
    journal: &mut Journal,
    is_original: bool,
    is_candidate: bool,
) -> Result<()> {
    require(
        matches!(
            journal.phase,
            Phase::Prepared | Phase::Publishing | Phase::Published
        ),
        "publication is cancelled or restored; prepare a new operation",
    )?;
    if !is_candidate {
        require(
            is_original && journal.phase != Phase::Published,
            "publication target no longer matches its journal phase",
        )?;
        require(
            identity(&journal.stage)? == journal.candidate,
            "publication stage changed",
        )?;
        validate(&journal.stage)?;
        require(
            identity(&journal.stage)? == journal.candidate,
            "publication stage changed during validation",
        )?;
        journal.phase = Phase::Publishing;
        save(directory, journal)?;
        check_parent(journal)?;
        require(
            identity(&journal.target)? == journal.original,
            "target changed before publication; all versions retained",
        )?;
        #[cfg(test)]
        tests::fail("publication", &journal.target)?;
        fs::rename(&journal.stage, &journal.target)?;
    }
    require(
        identity(&journal.target)? == journal.candidate,
        "published target changed; all versions retained",
    )?;
    directory_sync(journal.target.parent().unwrap())?;
    journal.phase = Phase::Published;
    save(directory, journal)?;
    println!(
        "published {}; activate this final path separately and verify daemon-confirmed applied plus live identity",
        journal.target.display()
    );
    Ok(())
}

fn restore(
    directory: &Path,
    journal: &mut Journal,
    is_original: bool,
    is_candidate: bool,
    is_restored: bool,
) -> Result<()> {
    if !is_original && !is_restored {
        require(is_candidate, "restore target changed")?;
        if journal.replacement.is_none() {
            if !journal.restore_stage.try_exists()? {
                copy_synced(
                    &directory.join("original"),
                    &journal.restore_stage,
                    &journal.original,
                )?;
            }
            require(
                identity(&journal.restore_stage)?.same_contents_and_metadata(&journal.original),
                "incomplete or changed restore stage; all versions retained",
            )?;
            journal.replacement = Some(identity(&journal.restore_stage)?);
        }
        require(
            Some(identity(&journal.restore_stage)?) == journal.replacement,
            "restore stage changed",
        )?;
        // A reused complete copy may have been interrupted before its file sync.
        sync_file(&journal.restore_stage)?;
        directory_sync(journal.target.parent().unwrap())?;
        journal.phase = Phase::Restoring;
        save(directory, journal)?;
        check_parent(journal)?;
        require(
            identity(&journal.target)? == journal.candidate,
            "target changed before restore; all versions retained",
        )?;
        fs::rename(&journal.restore_stage, &journal.target)?;
    }
    let current = identity(&journal.target)?;
    require(
        current == journal.original || Some(&current) == journal.replacement.as_ref(),
        "restored target changed; all versions retained",
    )?;
    directory_sync(journal.target.parent().unwrap())?;
    journal.phase = Phase::Restored;
    save(directory, journal)?;
    println!(
        "restored file {}; live daemon is unchanged",
        journal.target.display()
    );
    Ok(())
}

fn save(directory: &Path, journal: &Journal) -> Result<()> {
    #[cfg(test)]
    if journal.phase == Phase::Published {
        tests::fail("settlement", directory)?;
    }
    // Immutable successive records: interruption cannot destroy the last record.
    let next = records(directory)?.last().map_or(Ok(0), |(n, _)| {
        n.checked_add(1)
            .ok_or_else(|| Error::Invalid("journal sequence overflow".into()))
    })?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(temporary.as_file_mut(), journal)?;
    temporary.as_file_mut().write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(directory.join(format!("{next:020}.json")))
        .map_err(|e| Error::Io(e.error))?;
    directory_sync(directory)
}

fn records(directory: &Path) -> Result<Vec<(u64, PathBuf)>> {
    let mut records = Vec::new();
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let number = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or_else(|| Error::Invalid("invalid bootstrap journal record".into()))?;
            records.push((number, path));
        }
    }
    records.sort_by_key(|(n, _)| *n);
    Ok(records)
}

fn load(directory: &Path) -> Result<Journal> {
    let records = records(directory)?;
    let (_, path) = records.last().ok_or_else(|| {
        Error::Invalid(
            "no durable bootstrap journal; target was not published; preserve preparation files"
                .into(),
        )
    })?;
    let journal: Journal = serde_json::from_reader(File::open(path)?)?;
    require(journal.schema == 1, "unsupported bootstrap journal schema")?;
    let parent = journal
        .target
        .parent()
        .filter(|p| journal.target.is_absolute() && p.is_absolute());
    require(
        parent.is_some()
            && journal.target.file_name().is_some()
            && journal.stage.parent() == parent
            && journal.restore_stage.parent() == parent
            && journal.stage != journal.target
            && journal.restore_stage != journal.target,
        "invalid bootstrap journal paths",
    )?;
    for (index, (number, _)) in records.iter().enumerate() {
        require(
            u64::try_from(index).ok() == Some(*number),
            "bootstrap journal sequence is incomplete",
        )?;
    }
    Ok(journal)
}

#[cfg(test)]
mod tests;
