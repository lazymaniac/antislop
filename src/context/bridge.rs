//! A per-turn, private Unix bridge. A page cannot leave the daemon-owned reader
//! until its receipt callback has committed. Shutdown never waits on model input.
use super::*;
use std::{
    io::{BufReader, Write},
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

pub struct Commit {
    pub receipt: Receipt,
    reply: mpsc::SyncSender<bool>,
}
impl Commit {
    pub fn acknowledge(self, accepted: bool) {
        let _ = self.reply.send(accepted);
    }
}
pub struct Bridge {
    root: PathBuf,
    socket: PathBuf,
    stopped: Arc<AtomicBool>,
    stream: Arc<Mutex<Option<UnixStream>>>,
    receiver: mpsc::Receiver<Commit>,
    worker: Option<std::thread::JoinHandle<Result<()>>>,
}
struct ConnectionGuard(Arc<Mutex<Option<UnixStream>>>);
impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        if let Some(stream) = self.0.lock().expect("context bridge mutex").take() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}
impl Bridge {
    pub fn start(reader: Reader) -> Result<Self> {
        let root = std::env::temp_dir().join(format!(".ac-{}", &crate::id()[..12]));
        std::fs::DirBuilder::new().mode(0o700).create(&root)?;
        let socket = root.join("s");
        let listener = UnixListener::bind(&socket)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let stream = Arc::new(Mutex::new(None));
        let control = stream.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            let connection = loop {
                if stop.load(Ordering::SeqCst) {
                    return Ok(());
                }
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(e) => return Err(e.into()),
                }
            };
            // macOS may inherit the listener's nonblocking flag on accept.
            // MCP reads wait for the next frame; shutdown owns a separate clone.
            connection.set_nonblocking(false)?;
            *control.lock().expect("context bridge mutex") = Some(connection.try_clone()?);
            let _connection_guard = ConnectionGuard(control.clone());
            // Recheck after publishing the stream, so shutdown cannot miss it.
            if stop.load(Ordering::SeqCst) {
                return Ok(());
            }
            let mut output = connection.try_clone()?;
            stdio::serve(
                &reader,
                &mut BufReader::new(connection),
                &mut output,
                &mut |receipt| {
                    let (reply, ack) = mpsc::sync_channel(1);
                    sender
                        .send(Commit {
                            receipt: receipt.clone(),
                            reply,
                        })
                        .map_err(|_| crate::Error::Invalid("context owner disconnected".into()))?;
                    loop {
                        if stop.load(Ordering::SeqCst) {
                            return Err(crate::Error::Invalid("context owner stopped".into()));
                        }
                        match ack.recv_timeout(Duration::from_millis(10)) {
                            Ok(true) => return Ok(()),
                            Ok(false) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                                return Err(crate::Error::Invalid(
                                    "context receipt was not committed".into(),
                                ));
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                        }
                    }
                },
            )
        });
        Ok(Self {
            root,
            socket,
            stopped,
            stream,
            receiver,
            worker: Some(worker),
        })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn socket(&self) -> &Path {
        &self.socket
    }
    pub fn pending(&self) -> Option<Commit> {
        self.receiver.try_recv().ok()
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(stream) = self.stream.lock().expect("context bridge mutex").as_ref() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let _ = std::fs::remove_file(&self.socket);
        for relative in [
            "native/.agents/mcp_config.json",
            "native/.agents/agents/antislop-stdio.md",
        ] {
            let _ = std::fs::remove_file(self.root.join(relative));
        }
        for relative in ["native/.agents/agents", "native/.agents", "native"] {
            let _ = std::fs::remove_dir(self.root.join(relative));
        }
        let _ = std::fs::remove_dir(&self.root);
    }
}
pub fn connect(socket: &Path) -> Result<()> {
    crate::ipc::private_directory(
        socket
            .parent()
            .ok_or_else(|| crate::Error::Invalid("missing context socket parent".into()))?,
    )?;
    let metadata = socket.symlink_metadata()?;
    require(
        metadata.file_type().is_socket()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.permissions().mode() & 0o077 == 0,
        "unsafe context socket",
    )?;
    let mut stream = UnixStream::connect(socket)?;
    let mut writer = stream.try_clone()?;
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut std::io::stdin().lock(), &mut writer);
        let _ = writer.shutdown(std::net::Shutdown::Write);
    });
    std::io::copy(&mut stream, &mut std::io::stdout().lock())?;
    std::io::stdout().flush()?;
    Ok(())
}
