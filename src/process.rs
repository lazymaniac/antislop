//! A separate guardian owns each external process group. Its lifeline closes
//! even when the daemon is killed, so cleanup does not depend on Rust destructors.
use crate::{Error, Result, require};
use std::{
    ffi::OsStr,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        unix::{
            net::UnixStream,
            process::{CommandExt, ExitStatusExt},
        },
    },
    process::{Child, Command, ExitStatus},
    time::Duration,
};

thread_local! {
    static WORKSPACE_LEASE: std::cell::RefCell<Vec<RawFd>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The caller keeps the lease file alive throughout this thread-local scope.
/// Guardians inherit their own reference so daemon death cannot release it early.
pub struct LeaseScope(Vec<RawFd>, std::marker::PhantomData<std::rc::Rc<()>>);
impl LeaseScope {
    pub fn new(fds: &[RawFd]) -> Self {
        Self(
            WORKSPACE_LEASE.with(|lease| lease.replace(fds.to_vec())),
            std::marker::PhantomData,
        )
    }
}
impl Drop for LeaseScope {
    fn drop(&mut self) {
        WORKSPACE_LEASE.with(|lease| {
            lease.replace(std::mem::take(&mut self.0));
        });
    }
}

fn duplicate(fd: RawFd, minimum: RawFd) -> std::io::Result<OwnedFd> {
    // Reserve owned descriptors before pre_exec; no descriptor numbers are fixed.
    let duplicated = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, minimum) };
    if duplicated < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(duplicated) })
}

pub struct SupervisedChild {
    pub child: Child,
    lifeline: Option<UnixStream>,
}

/// Construct the companion guardian command; callers configure stdio/cwd/env.
/// Never fall back to spawning the external command without supervision.
pub fn command(program: impl AsRef<OsStr>) -> Result<Command> {
    let mut command = Command::new(companion()?);
    command.arg("__supervise").arg(program);
    Ok(command)
}
pub fn companion() -> Result<std::path::PathBuf> {
    let current = std::env::current_exe()?;
    let companion = if current.file_name() == Some(OsStr::new("antislop")) {
        current
    } else {
        let directory = current
            .parent()
            .ok_or_else(|| Error::Invalid("supervisor binary unavailable".into()))?;
        let directory = if matches!(
            directory.file_name().and_then(OsStr::to_str),
            Some("deps" | "examples")
        ) {
            directory
                .parent()
                .ok_or_else(|| Error::Invalid("supervisor binary unavailable".into()))?
        } else {
            directory
        };
        directory.join("antislop")
    };
    require(
        companion.is_file(),
        "antislop supervisor binary unavailable; build the companion binary first",
    )?;
    Ok(companion)
}

impl SupervisedChild {
    pub fn spawn(command: &mut Command) -> Result<Self> {
        let (parent, guardian) = UnixStream::pair()?;
        let lease_fds = WORKSPACE_LEASE.with(|lease| lease.borrow().clone());
        require(lease_fds.len() <= 1024, "too many workspace leases")?;
        let source = duplicate(guardian.as_raw_fd(), 3)?;
        let fd = source.as_raw_fd();
        let lease_sources = lease_fds
            .iter()
            .map(|fd| duplicate(*fd, 3))
            .collect::<std::io::Result<Vec<_>>>()?;
        let lease_fds: Vec<_> = lease_sources.iter().map(AsRawFd::as_raw_fd).collect();
        command.env("ARCHITECT_SUPERVISOR_LIFELINE", fd.to_string());
        command.env(
            "ARCHITECT_SUPERVISOR_LEASE_FDS",
            lease_fds
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(","),
        );
        // SAFETY: fcntl is async-signal-safe; no allocation in pre_exec. The
        // owned source FDs cannot collide with std::process's private error pipe.
        unsafe {
            command.pre_exec(move || {
                if libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                for lease_fd in &lease_fds {
                    if libc::fcntl(*lease_fd, libc::F_SETFD, 0) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let child = command.spawn()?;
        drop(guardian);
        drop(source);
        drop(lease_sources);
        let mut supervised = Self {
            child,
            lifeline: Some(parent),
        };
        let mut ready = [0];
        supervised
            .lifeline
            .as_mut()
            .expect("lifeline")
            .read_exact(&mut ready)?;
        require(ready[0] == 1, "external process supervisor failed to start")?;
        Ok(supervised)
    }

    pub fn cancel(&mut self) -> std::io::Result<ExitStatus> {
        self.lifeline.take();
        self.child.wait()
    }
}

impl Drop for SupervisedChild {
    fn drop(&mut self) {
        // Closing the private lifeline asks the guardian to kill and reap the
        // entire group before it exits; never kill the guardian prematurely.
        let _ = self.cancel();
    }
}

/// Internal binary entry point. Its inherited lifeline must be a connected socket.
pub fn supervise(args: &[String]) -> Result<()> {
    require(!args.is_empty(), "missing supervised command")?;
    let fd: RawFd = std::env::var("ARCHITECT_SUPERVISOR_LIFELINE")
        .map_err(|_| Error::Invalid("missing guardian lifeline".into()))?
        .parse()
        .map_err(|_| Error::Invalid("invalid guardian lifeline".into()))?;
    require(fd >= 3, "invalid guardian lifeline descriptor")?;
    let mut socket_type: libc::c_int = 0;
    let mut size = std::mem::size_of_val(&socket_type) as libc::socklen_t;
    // SAFETY: valid pointers and lengths; reject direct invocations without a socket.
    require(
        unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_TYPE,
                (&mut socket_type as *mut libc::c_int).cast(),
                &mut size,
            )
        } == 0
            && socket_type == libc::SOCK_STREAM,
        "supervisor requires its private lifeline",
    )?;
    // SAFETY: the inherited descriptor belongs exclusively to this guardian.
    let mut lifeline = unsafe { UnixStream::from_raw_fd(fd) };
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let leases = std::env::var("ARCHITECT_SUPERVISOR_LEASE_FDS")
        .map_err(|_| Error::Invalid("missing guardian lease descriptors".into()))?;
    let lease_fds = if leases.is_empty() {
        Vec::new()
    } else {
        leases
            .split(',')
            .map(str::parse::<RawFd>)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| Error::Invalid("invalid guardian lease descriptors".into()))?
    };
    require(lease_fds.len() <= 1024, "too many guardian leases")?;
    // Keep every ancestor/root workspace lease in the guardian, never the tool.
    for lease_fd in lease_fds {
        require(
            lease_fd >= 3 && lease_fd != fd,
            "invalid guardian lease descriptor",
        )?;
        if unsafe { libc::fcntl(lease_fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    lifeline.set_nonblocking(true)?;
    let mut child = Command::new(&args[0])
        .args(&args[1..])
        .env_remove("ARCHITECT_SUPERVISOR_LIFELINE")
        .env_remove("ARCHITECT_SUPERVISOR_LEASE_FDS")
        .process_group(0)
        .spawn()?;
    let pid = child.id() as libc::pid_t;
    // Failed handshake also means the parent has disappeared: still clean up.
    let ready = lifeline.write_all(&[1]).is_ok();
    loop {
        let mut byte = [0];
        let disconnected = !ready
            || match lifeline.read(&mut byte) {
                Ok(0) => true,
                Ok(_) => false,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => false,
                Err(_) => true,
            };
        let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
        // Keep an exited leader unreaped until killpg: its PID cannot be reused.
        // SAFETY: info points to initialized storage and pid is our direct child.
        let waited = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                info.as_mut_ptr(),
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if waited < 0 {
            // Retain ownership until cleanup also on an unexpected wait error.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
            let _ = child.wait();
            return Err(std::io::Error::last_os_error().into());
        }
        // SAFETY: waitid filled the zeroed siginfo structure.
        let exited = unsafe { info.assume_init().si_pid() } != 0;
        if disconnected || exited {
            // The still-owned child reserves the process-group identifier.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
            let status = child.wait()?;
            // Cleanup is complete before reproducing the leader's actual termination.
            // Converting a signal to exit 1 could falsely satisfy a failing baseline.
            if let Some(signal) = status.signal() {
                // SAFETY: restore the default disposition and unblock this observed signal
                // in the guardian only; no children or shared parent state remain affected.
                unsafe {
                    libc::signal(signal, libc::SIG_DFL);
                    let mut mask = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
                    libc::sigemptyset(mask.as_mut_ptr());
                    libc::sigaddset(mask.as_mut_ptr(), signal);
                    libc::sigprocmask(libc::SIG_UNBLOCK, mask.as_ptr(), std::ptr::null_mut());
                    libc::raise(signal);
                }
                // A failed re-raise must remain abnormal, never an expected exit 1.
                std::process::abort();
            }
            std::process::exit(status.code().unwrap_or(1));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
