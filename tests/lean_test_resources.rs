mod common;

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Command, Stdio};

#[test]
fn cleanup_confirmation_releases_only_its_own_shared_probe() {
    use std::os::fd::AsRawFd;
    let root = common::tempdir();
    let path = root.path().join("core.lock");
    let probe = std::fs::File::create(&path).unwrap();
    let inherited = probe.try_clone().unwrap();
    common::daemon_lock_released_file(&probe).unwrap();
    drop(probe);
    let owner = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    // SAFETY: these are fixture-owned descriptors; the clone retains the same
    // open file description as a child between fork and exec would retain.
    let acquired = unsafe { libc::flock(owner.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    drop(inherited);
    assert_eq!(acquired, 0, "cleanup left its temporary probe locked");
    let contender = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    assert!(common::daemon_lock_released_file(&contender).is_err());
    let still_locked = unsafe { libc::flock(contender.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(still_locked, -1, "failed probe unlocked the actual owner");
}

#[test]
fn parallel_fixtures_have_short_private_sockets_even_with_a_long_project() {
    let threads: Vec<_> = (0..12)
        .map(|_| {
            std::thread::spawn(|| {
                let root = common::tempdir();
                let project = root.path().join("long-worktree-name".repeat(12));
                std::fs::create_dir(&project).unwrap();
                let state = root.path().join("s");
                std::fs::create_dir(&state).unwrap();
                common::assert_test_socket(&state, &project);
                let socket = antislop::lean::daemon::socket(&state);
                let listener = UnixListener::bind(&socket).unwrap();
                let mut client = UnixStream::connect(&socket).unwrap();
                let (mut server, _) = listener.accept().unwrap();
                client.write_all(b"fixture").unwrap();
                let mut bytes = [0; 7];
                server.read_exact(&mut bytes).unwrap();
                assert_eq!(&bytes, b"fixture");
                let path = root.path().to_path_buf();
                drop((client, server, listener));
                drop(root);
                assert!(!path.exists());
                path
            })
        })
        .collect();
    let mut paths: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    paths.sort();
    paths.dedup();
    assert_eq!(paths.len(), 12);
}

#[test]
fn panic_reaps_child_preserves_logs_and_removes_only_owned_directory() {
    let neighbor = common::tempdir();
    std::fs::write(neighbor.path().join("foreign"), "keep").unwrap();
    let started = std::cell::Cell::new(None);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let root = common::tempdir();
        std::fs::write(
            root.path().join("failure.stderr"),
            "raw failure: deliberate panic",
        )
        .unwrap();
        std::fs::write(
            root.path().join("codex.rejections"),
            "fixture completion refused",
        )
        .unwrap();
        std::fs::write(
            root.path().join("source.rs"),
            "fixture source must not be copied",
        )
        .unwrap();
        let child = common::ChildGuard::new(
            Command::new("sh")
                .args(["-c", "exec cat"])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .spawn()
                .unwrap(),
        );
        started.set(Some((root.path().to_path_buf(), child.id())));
        panic!("deliberate lifecycle regression");
    }));
    assert!(result.is_err());
    let (path, pid) = started.take().expect("fixture did not reach child startup");
    assert!(!path.exists());
    // SAFETY: signal 0 only checks existence of the child just reaped by our guard.
    assert_eq!(unsafe { libc::kill(pid as libc::pid_t, 0) }, -1);
    let diagnostic =
        std::path::PathBuf::from(std::env::var_os("ANTISLOP_TEST_DIAGNOSTICS").unwrap())
            .join(path.file_name().unwrap())
            .join("failure.stderr");
    assert_eq!(
        std::fs::read_to_string(diagnostic).unwrap(),
        "raw failure: deliberate panic"
    );
    let diagnostic_root =
        std::path::PathBuf::from(std::env::var_os("ANTISLOP_TEST_DIAGNOSTICS").unwrap())
            .join(path.file_name().unwrap());
    assert_eq!(
        std::fs::read_to_string(diagnostic_root.join("codex.rejections")).unwrap(),
        "fixture completion refused"
    );
    assert!(!diagnostic_root.join("source.rs").exists());
    assert_eq!(
        std::fs::read_to_string(neighbor.path().join("foreign")).unwrap(),
        "keep"
    );
}

#[test]
fn failed_spawn_and_environment_validation_do_not_leak_or_mutate_environment() {
    let before: Vec<_> = std::env::vars_os().collect();
    let path;
    {
        let root = common::tempdir();
        path = root.path().to_path_buf();
        common::runtime_environment(root.path());
        let error = Command::new(root.path().join("missing-cli"))
            .spawn()
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    }
    assert!(!path.exists());
    assert_eq!(before, std::env::vars_os().collect::<Vec<_>>());
}

#[test]
fn panic_after_cli_autostart_stops_the_detached_daemon_before_fixture_cleanup() {
    use antislop::lean::daemon::{Client, ControlRequest, Request};
    let root = common::tempdir();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    common::initialize_git_project(&project);
    let config = root.path().join("config.toml");
    std::fs::write(&config, include_str!("../examples/antislop.toml")).unwrap();
    let state = root.path().join("long-state-name".repeat(15));
    let socket = antislop::lean::daemon::socket(&state);
    assert!(
        !socket.starts_with(root.path()),
        "this regression must exercise fallback"
    );
    assert!(socket.as_os_str().len() < 100);
    let started = std::cell::Cell::new(None);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _daemon = common::DaemonStateGuard(state.clone());
        antislop::lean::daemon::private_directory(&state).unwrap();
        let client = Client::connect(
            &project,
            &config,
            &state,
            std::path::Path::new(env!("CARGO_BIN_EXE_antislop")),
        )
        .unwrap();
        let pid = client
            .call(&Request::Control(ControlRequest::Identity))
            .unwrap()["pid"]
            .as_i64()
            .unwrap();
        started.set(Some(pid));
        panic!("deliberate failure after autostart");
    }));
    assert!(result.is_err());
    assert!(
        Client::at(&state)
            .call(&Request::Control(ControlRequest::Identity))
            .is_err()
    );
    assert!(
        !socket.exists(),
        "the owned fallback socket must be removed"
    );
    let pid = started.take().expect("daemon did not reach readiness") as libc::pid_t;
    // SAFETY: this is the direct daemon child started by Client::connect above.
    // The state guard waited for its lock; reap its process table entry as well.
    let mut status = 0;
    assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
}

#[test]
fn missing_endpoint_with_live_daemon_retains_state_and_reports_lock() {
    use antislop::lean::daemon::{Client, ControlRequest, Request};
    let root = common::tempdir();
    let path = root.path().to_path_buf();
    let project = path.join("project");
    std::fs::create_dir(&project).unwrap();
    common::initialize_git_project(&project);
    let config = path.join("config.toml");
    std::fs::write(&config, include_str!("../examples/antislop.toml")).unwrap();
    let state = path.join("s");
    antislop::lean::daemon::private_directory(&state).unwrap();
    let client = Client::connect(
        &project,
        &config,
        &state,
        std::path::Path::new(env!("CARGO_BIN_EXE_antislop")),
    )
    .unwrap();
    let pid = client
        .call(&Request::Control(ControlRequest::Identity))
        .unwrap()["pid"]
        .as_i64()
        .unwrap() as libc::pid_t;
    let socket = antislop::lean::daemon::socket(&state);
    let hidden = state.join("hidden.sock");
    std::fs::rename(&socket, &hidden).unwrap();
    assert!(!Client::stop(&state, true).unwrap());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _root = root;
        let _daemon = common::DaemonStateGuard(state.clone());
    }));
    // Restore our endpoint before assertions so even a failed regression stops its daemon.
    std::fs::rename(&hidden, &socket).unwrap();
    common::stop_daemon(&client);
    let mut status = 0;
    // SAFETY: reap the direct daemon child started above.
    assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
    assert!(result.is_err());
    assert!(state.join("core.lock").exists(), "active state was deleted");
    let diagnostic =
        std::path::PathBuf::from(std::env::var_os("ANTISLOP_TEST_DIAGNOSTICS").unwrap())
            .join(path.file_name().unwrap())
            .join("retain-resources.log");
    let message = std::fs::read_to_string(diagnostic).unwrap();
    assert!(message.contains("daemon lock not released"), "{message}");
    // Release only this regression's retained data after confirmed shutdown.
    std::fs::remove_dir_all(&path).unwrap();
    let marker = std::path::PathBuf::from(std::env::var_os("ANTISLOP_TEST_ROOT").unwrap())
        .join("retain-resources.log");
    if std::fs::read_to_string(&marker).unwrap() == message {
        std::fs::remove_file(marker).unwrap();
    }
}
