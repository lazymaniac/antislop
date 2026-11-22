use super::config_write::cleanup_config_temporaries;
use super::*;
use std::sync::{RwLock, TryLockError};

#[derive(Clone)]
struct Runtime {
    core: Arc<Core>,
    project: PathBuf,
    config: PathBuf,
    socket: PathBuf,
    leases: Arc<super::lease::Leases>,
    shutdown_replies: Arc<RwLock<()>>,
}

pub fn serve(project: &Path, config_path: &Path, state: &Path) -> Result<()> {
    let (runtime, listener) = start(project, config_path, state)?;
    super::activation::confirm(&runtime.core, &runtime.project, &runtime.config)?;
    spawn_idle_cleanup(&runtime, state);
    accept_connections(listener, runtime);
    Ok(())
}

fn start(project: &Path, config_path: &Path, state: &Path) -> Result<(Runtime, UnixListener)> {
    super::super::Store::check_runtime_repository(
        &state.join("core.db"),
        &super::super::paths::project_identity(project)?,
    )?;
    let config_path = config_path.canonicalize()?;
    let config_path = config_path.as_path();
    let loaded = modified(config_path);
    let project = project.canonicalize()?;
    warn_on_config_cleanup(config_path);
    let core = open_core(state, config_path, loaded)?;
    core.store.record_project_identity(
        &project.to_string_lossy(),
        &super::super::paths::project_identity(&project)?,
    )?;
    let config = config_path.canonicalize()?;
    let (socket, listener) = bind_socket(state)?;
    eprintln!("antislop core ready at {}", socket.display());
    Ok((
        Runtime {
            core,
            project,
            config,
            socket,
            leases: Arc::default(),
            shutdown_replies: Arc::default(),
        },
        listener,
    ))
}

fn warn_on_config_cleanup(config_path: &Path) {
    if let Err(error) = cleanup_config_temporaries(config_path) {
        eprintln!("could not clean abandoned configuration writes: {error}");
    }
}

fn open_core(state: &Path, config: &Path, loaded: u64) -> Result<Arc<Core>> {
    let core = Core::open(state, Config::load(config)?)?;
    core.config.set_modified(loaded);
    Ok(core)
}

fn bind_socket(state: &Path) -> Result<(PathBuf, UnixListener)> {
    let socket = super::socket(state);
    if let Some(parent) = socket.parent() {
        private_directory(parent)?;
    }
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket)?;
    Ok((socket, listener))
}

fn spawn_idle_cleanup(runtime: &Runtime, state: &Path) {
    let core = runtime.core.clone();
    let database = state.join("core.db");
    let stopped = state.join("stopped");
    let socket = runtime.socket.clone();
    let shutdown_replies = runtime.shutdown_replies.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(2));
            // A request may have written `stopped` but still owe its reply.
            let _idle = match shutdown_replies.try_write() {
                Ok(idle) => idle,
                Err(TryLockError::WouldBlock) => continue,
                Err(TryLockError::Poisoned(error)) => error.into_inner(),
            };
            if let Err(error) = super::activation::apply_when_idle(&core) {
                eprintln!("runtime activation failed: {error}");
            }
            let stopping = stopped.exists() || !(database.exists() && socket.exists());
            if stopping {
                core.cancel_operations();
                let _ = super::activation::cancel_all(&core);
            }
            if stopping && !core.busy() {
                let _ = core.native.shutdown();
                std::process::exit(0);
            }
        }
    });
}

fn accept_connections(listener: UnixListener, runtime: Runtime) {
    for stream in listener.incoming().flatten() {
        let runtime = runtime.clone();
        std::thread::spawn(move || {
            if serve_connection(stream, &runtime, crate::transport::write) {
                let _ = std::fs::remove_file(&runtime.socket);
                std::process::exit(0);
            }
        });
    }
}

fn serve_connection(
    stream: UnixStream,
    runtime: &Runtime,
    mut respond: impl FnMut(&mut UnixStream, &Response) -> Result<()>,
) -> bool {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let mut reader = BufReader::new(stream);
    loop {
        let (request, client) = match read_request(&mut reader) {
            Ok(Some(request)) => request,
            Ok(None) => return false,
            Err(error) => {
                let response = error_response(error);
                let _ = respond(reader.get_mut(), &response.response);
                return false;
            }
        };
        let _reply = matches!(
            request,
            Request::Control(
                super::ControlRequest::Shutdown | super::ControlRequest::ForceShutdown
            )
        )
        .then(|| {
            runtime
                .shutdown_replies
                .read()
                .unwrap_or_else(|error| error.into_inner())
        });
        let response = handle(runtime, request, client.as_deref());
        let shutdown = response.shutdown;
        let successful_shutdown =
            shutdown && response.response.error.is_none() && !runtime.core.busy();
        if respond(reader.get_mut(), &response.response).is_err() {
            return false;
        }
        if successful_shutdown {
            return true;
        }
    }
}

struct ConnectionResponse {
    response: Response,
    shutdown: bool,
}

fn read_request(reader: &mut BufReader<UnixStream>) -> Result<Option<(Request, Option<String>)>> {
    let envelope = crate::transport::read::<RequestEnvelope>(reader)?;
    envelope
        .map(|wire| {
            require(
                wire.protocol == PROTOCOL_VERSION,
                "incompatible daemon protocol; restart client and daemon together",
            )?;
            Ok((wire.request, wire.client))
        })
        .transpose()
}

fn handle(runtime: &Runtime, request: Request, client: Option<&str>) -> ConnectionResponse {
    let shutdown = matches!(
        request,
        Request::Control(crate::lean::daemon::ControlRequest::Shutdown)
            | Request::Control(crate::lean::daemon::ControlRequest::ForceShutdown)
    );
    let frontend = client.map(|client| (client, runtime.leases.as_ref()));
    let response = match super::handle(
        &runtime.core,
        &runtime.project,
        &runtime.config,
        request,
        frontend,
    ) {
        Ok(data) => Response {
            data: Some(data),
            error: None,
        },
        Err(error) => Response {
            data: None,
            error: Some(error.to_string()),
        },
    };
    ConnectionResponse { response, shutdown }
}

fn error_response(error: Error) -> ConnectionResponse {
    ConnectionResponse {
        response: Response {
            data: None,
            error: Some(error.to_string()),
        },
        shutdown: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lean::daemon::{Client, Request, SessionRequest};

    fn runtime() -> (tempfile::TempDir, Runtime, UnixListener) {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let init = std::process::Command::new("git")
            .args(["init", "--quiet", "--initial-branch=main"])
            .current_dir(&project)
            .status()
            .unwrap();
        assert!(init.success());
        let commit = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                "initialize",
            ])
            .current_dir(&project)
            .status()
            .unwrap();
        assert!(commit.success());
        let project = project.canonicalize().unwrap();
        let state = temp.path().join("state");
        std::fs::create_dir(&state).unwrap();
        std::fs::set_permissions(&state, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .unwrap();
        let config = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/antislop.toml");
        let (runtime, listener) = start(&project, &config, &state).unwrap();
        (temp, runtime, listener)
    }

    #[test]
    fn protocol_version_is_checked_after_decoding_the_envelope() {
        for (protocol, expected) in [
            (6, None),
            (
                5,
                Some("incompatible daemon protocol; restart client and daemon together"),
            ),
            (
                7,
                Some("incompatible daemon protocol; restart client and daemon together"),
            ),
        ] {
            let (mut client, server) = UnixStream::pair().unwrap();
            crate::transport::write(
                &mut client,
                &json!({
                    "protocol": protocol, "request": {"op":"identity"}
                }),
            )
            .unwrap();
            let result = read_request(&mut BufReader::new(server));
            if let Some(error) = expected {
                assert_eq!(result.unwrap_err().to_string(), error);
            } else {
                assert_eq!(
                    result.unwrap(),
                    Some((
                        Request::Control(super::super::ControlRequest::Identity),
                        None
                    ))
                );
            }
        }
    }

    #[test]
    fn daemon_serves_sequential_session_requests_on_one_connection() {
        let (temp, runtime, listener) = runtime();
        let project = runtime.project.clone();
        let state = temp.path().join("state");
        let conversation = runtime
            .core
            .store
            .create_conversation(project.to_str().unwrap())
            .unwrap()
            .id;
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve_connection(stream, &runtime, crate::transport::write)
        });
        let mut connection = Client::at(&state).open_reusable_connection().unwrap();
        for _ in 0..2 {
            let documents = connection
                .call(&Request::Session(SessionRequest::ResultDocuments {
                    conversation: conversation.clone(),
                }))
                .unwrap();
            assert_eq!(documents, serde_json::json!([]));
        }
        drop(connection);
        assert!(!server.join().unwrap());
    }

    #[test]
    fn idle_cleanup_waits_until_the_shutdown_reply_is_written() {
        let (temp, runtime, _listener) = runtime();
        let (mut client, stream) = UnixStream::pair().unwrap();
        let (ready, awaiting_reply) = std::sync::mpsc::channel();
        let (release, resume) = std::sync::mpsc::channel();
        let server_runtime = runtime.clone();
        let server = std::thread::spawn(move || {
            serve_connection(stream, &server_runtime, |stream, response| {
                ready.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(120)).unwrap();
                crate::transport::write(stream, response)
            })
        });
        crate::transport::write(
            &mut client,
            &json!({"protocol":PROTOCOL_VERSION,"request":{"op":"shutdown"},"client":null}),
        )
        .unwrap();
        awaiting_reply
            .recv_timeout(Duration::from_secs(120))
            .unwrap();
        assert!(temp.path().join("state/stopped").exists());
        assert!(!runtime.core.busy());
        assert!(
            matches!(
                runtime.shutdown_replies.try_write(),
                Err(TryLockError::WouldBlock)
            ),
            "cleanup cannot exit between dispatching shutdown and writing its reply"
        );
        release.send(()).unwrap();
        let response: Response = crate::transport::read(&mut BufReader::new(client))
            .unwrap()
            .unwrap();
        assert!(response.error.is_none(), "{response:?}");
        assert!(server.join().unwrap());
        assert!(runtime.shutdown_replies.try_write().is_ok());
    }
}
