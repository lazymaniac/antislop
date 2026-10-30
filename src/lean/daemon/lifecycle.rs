use super::config_write::cleanup_config_temporaries;
use super::*;

#[derive(Clone)]
struct Runtime {
    core: Arc<Core>,
    project: PathBuf,
    config: PathBuf,
    socket: PathBuf,
}

pub fn serve(project: &Path, config_path: &Path, state: &Path) -> Result<()> {
    let (runtime, listener) = start(project, config_path, state)?;
    spawn_idle_cleanup(&runtime, state);
    accept_connections(listener, runtime);
    Ok(())
}

fn start(project: &Path, config_path: &Path, state: &Path) -> Result<(Runtime, UnixListener)> {
    let loaded = modified(config_path);
    let project = trusted_project(project)?;
    warn_on_config_cleanup(config_path);
    let core = open_core(state, config_path, loaded)?;
    let config = config_path.canonicalize()?;
    let (socket, listener) = bind_socket(state)?;
    eprintln!("antislop core ready at {}", socket.display());
    Ok((
        Runtime {
            core,
            project,
            config,
            socket,
        },
        listener,
    ))
}

fn trusted_project(project: &Path) -> Result<PathBuf> {
    let project = project.canonicalize()?;
    let registry = super::super::trust::Registry::from_env()?;
    require(
        registry.is_trusted(&project),
        &format!(
            "{} is not a trusted directory: run antislop there in a terminal first, or pass --trust",
            project.display()
        ),
    )?;
    Ok(project)
}

fn warn_on_config_cleanup(config_path: &Path) {
    if let Err(error) = cleanup_config_temporaries(config_path) {
        eprintln!("could not clean abandoned configuration writes: {error}");
    }
}

fn open_core(state: &Path, config: &Path, loaded: u64) -> Result<Arc<Core>> {
    let core = Core::open(state, Config::load(config)?)?;
    core.config_modified.store(loaded, Ordering::SeqCst);
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
    let socket = runtime.socket.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(2));
            if !(database.exists() && socket.exists()) && !core.busy() {
                let _ = core.native.shutdown();
                std::process::exit(0);
            }
        }
    });
}

fn accept_connections(listener: UnixListener, runtime: Runtime) {
    for stream in listener.incoming().flatten() {
        let runtime = runtime.clone();
        std::thread::spawn(move || serve_connection(stream, runtime));
    }
}

fn serve_connection(mut stream: UnixStream, runtime: Runtime) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let response = match read_request(&stream) {
        Ok(Some(request)) => handle(&runtime, request),
        Ok(None) => error_response(Error::Invalid("empty request".into())),
        Err(error) => error_response(error),
    };
    let shutdown = response.shutdown;
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let _ = crate::transport::write(&mut stream, &response.response);
    if shutdown && response.response.error.is_none() {
        let _ = std::fs::remove_file(&runtime.socket);
        std::process::exit(0);
    }
}

struct ConnectionResponse {
    response: Response,
    shutdown: bool,
}

fn read_request(stream: &UnixStream) -> Result<Option<Request>> {
    let envelope = crate::transport::read::<RequestEnvelope>(&mut BufReader::new(stream))?;
    envelope
        .map(|wire| {
            require(
                wire.protocol == PROTOCOL_VERSION,
                "incompatible daemon protocol; restart client and daemon together",
            )?;
            Ok(wire.request)
        })
        .transpose()
}

fn handle(runtime: &Runtime, request: Request) -> ConnectionResponse {
    let shutdown = matches!(request, Request::Shutdown | Request::ForceShutdown);
    let response = match super::handle(&runtime.core, &runtime.project, &runtime.config, request) {
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
