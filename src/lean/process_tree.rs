//! Ends an agent CLI together with every process it started. A CLI may run a
//! command in its own process group, and a command it abandons is re-parented
//! to init, so neither the CLI's group nor parent links alone reach them. Each
//! CLI therefore carries a marker in its environment that its descendants
//! inherit; macOS hides the environment of its own platform binaries, so the
//! parent links still count.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// The environment variable that tags an agent CLI and its descendants.
pub(crate) const MARKER: &str = "ANTISLOP_AGENT";

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A marker unique to one CLI of the daemon owning `state`.
pub(crate) fn marker(state: &Path) -> String {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{}#{}-{n}", state.display(), std::process::id())
}

/// Kills `root`'s descendants and every process tagged with `marker`;
/// `root` itself is left to its owner.
pub(crate) fn kill_descendants(root: Option<i32>, marker: &str) {
    let table = processes();
    let mut doomed = root.map_or_else(Vec::new, |pid| descendants(&table, pid));
    doomed.extend(tagged(&table, &format!("{MARKER}={marker} ")));
    kill(&doomed);
}

/// After a crash: kills what an earlier daemon owning `state` left running.
pub(crate) fn kill_leftovers(state: &Path) {
    // Markers name the canonical directory the daemon runs from.
    let state = state.canonicalize().unwrap_or_else(|_| state.to_path_buf());
    let table = processes();
    kill(&tagged(&table, &format!("{MARKER}={}#", state.display())));
}

struct Process {
    pid: i32,
    ppid: i32,
    /// Command line and, where the system shows it, environment.
    line: String,
}

fn processes() -> Vec<Process> {
    let Ok(output) = Command::new("ps").args(listing()).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse)
        .collect()
}

#[cfg(target_os = "macos")]
fn listing() -> &'static [&'static str] {
    &["-A", "-E", "-ww", "-o", "pid=,ppid=,command="]
}

#[cfg(not(target_os = "macos"))]
fn listing() -> &'static [&'static str] {
    &["axeww", "-o", "pid=,ppid=,command="]
}

fn parse(line: &str) -> Option<Process> {
    let mut fields = line.trim_start().splitn(2, ' ');
    let pid = fields.next()?.parse().ok()?;
    let rest = fields.next()?.trim_start();
    let (ppid, command) = rest.split_once(' ').unwrap_or((rest, ""));
    Some(Process {
        pid,
        ppid: ppid.parse().ok()?,
        line: format!("{command} "),
    })
}

fn descendants(table: &[Process], root: i32) -> Vec<i32> {
    let mut found = vec![root];
    let mut next = 0;
    while next < found.len() {
        let parent = found[next];
        found.extend(table.iter().filter(|p| p.ppid == parent).map(|p| p.pid));
        next += 1;
    }
    found.split_off(1)
}

fn tagged(table: &[Process], needle: &str) -> Vec<i32> {
    table
        .iter()
        .filter(|p| p.line.contains(needle))
        .map(|p| p.pid)
        .collect()
}

/// SIGKILLs each process, and the group it leads, never this daemon's own.
fn kill(pids: &[i32]) {
    let own = std::process::id() as i32;
    // SAFETY: getpgid only reads the process table.
    let own_group = unsafe { libc::getpgid(0) };
    for &pid in pids.iter().filter(|&&pid| pid > 1 && pid != own) {
        // SAFETY: signals a process this daemon's CLI started, and the group
        // only when that process leads it.
        unsafe {
            let group = libc::getpgid(pid);
            if group == pid && group != own_group {
                libc::kill(-pid, libc::SIGKILL);
            }
            libc::kill(pid, libc::SIGKILL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_line_gives_pid_parent_and_the_rest() {
        let p = parse("  412   1 python3 - ANTISLOP_AGENT=/s#9-0 HOME=/h").unwrap();
        assert_eq!((p.pid, p.ppid), (412, 1));
        assert!(p.line.contains("ANTISLOP_AGENT=/s#9-0 "));
    }

    #[test]
    fn descendants_follow_parent_links_through_every_generation() {
        let table: Vec<Process> = [(10, 1), (11, 10), (12, 11), (13, 1)]
            .into_iter()
            .map(|(pid, ppid)| Process {
                pid,
                ppid,
                line: String::new(),
            })
            .collect();
        assert_eq!(descendants(&table, 10), vec![11, 12]);
    }

    #[test]
    fn a_marker_matches_only_its_own_cli() {
        let table = vec![
            parse("20 1 python3 - ANTISLOP_AGENT=/s#9-1 X=1").unwrap(),
            parse("21 1 python3 - ANTISLOP_AGENT=/s#9-12 X=1").unwrap(),
        ];
        assert_eq!(tagged(&table, &format!("{MARKER}=/s#9-1 ")), vec![20]);
        assert_eq!(tagged(&table, &format!("{MARKER}=/s#")), vec![20, 21]);
    }

    #[test]
    fn an_abandoned_command_in_its_own_group_dies_with_its_cli() {
        let dir = tempfile::tempdir().unwrap();
        let marker = marker(dir.path());
        let pid_file = dir.path().join("pid");
        // The grandchild leads its own group and outlives the shell that
        // started it, as a command a CLI gave up on does.
        let mut shell = Command::new("python3")
            .args([
                "-c",
                "import os,subprocess,sys\n\
                 p=subprocess.Popen([sys.executable,'-c','import time;time.sleep(60)'],\
                 start_new_session=True)\nopen(sys.argv[1],'w').write(str(p.pid))",
            ])
            .arg(&pid_file)
            .env(MARKER, &marker)
            .spawn()
            .unwrap();
        shell.wait().unwrap();
        let orphan: i32 = std::fs::read_to_string(&pid_file).unwrap().parse().unwrap();
        // SAFETY: signal 0 only checks that the process exists.
        assert_eq!(unsafe { libc::kill(orphan, 0) }, 0);
        kill_descendants(None, &marker);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        // SAFETY: as above.
        while unsafe { libc::kill(orphan, 0) } == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_ne!(unsafe { libc::kill(orphan, 0) }, 0, "the orphan survived");
    }
}
