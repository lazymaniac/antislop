use super::Shared;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub(super) fn spawn_listener(
    server: tiny_http::Server,
    worker: Arc<Shared>,
    halt: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        while !halt.load(std::sync::atomic::Ordering::SeqCst) {
            match server.recv_timeout(Duration::from_millis(100)) {
                Ok(Some(request)) => {
                    let shared = worker.clone();
                    std::thread::spawn(move || {
                        let _ = super::super::transport::serve(&shared, request);
                    });
                }
                Ok(None) => {}
                Err(_) => break,
            }
        }
    });
}
