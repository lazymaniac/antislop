//! Sanitized clipboard output. The TUI calls this only between frame draws.

use super::sanitize::sanitize;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::io::{self, Write};

#[derive(Debug, PartialEq, Eq)]
pub enum CopyMethod {
    Osc52,
    Fallback { osc52_error: String },
}

pub fn clipboard_sequence(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", STANDARD.encode(sanitize(text)))
}

/// A successful write is the strongest acknowledgement OSC 52 provides.
/// Terminals that silently ignore OSC 52 cannot be detected this way.
fn copy_with_fallback(
    writer: &mut impl Write,
    text: &str,
    fallback: impl FnOnce(&str) -> io::Result<()>,
) -> io::Result<CopyMethod> {
    let clean = sanitize(text);
    match writer
        .write_all(clipboard_sequence(&clean).as_bytes())
        .and_then(|()| writer.flush())
    {
        Ok(()) => Ok(CopyMethod::Osc52),
        Err(primary) => {
            fallback(&clean).map_err(|secondary| {
                io::Error::other(format!(
                    "OSC 52 failed: {primary}; clipboard fallback failed: {secondary}"
                ))
            })?;
            Ok(CopyMethod::Fallback {
                osc52_error: primary.to_string(),
            })
        }
    }
}

pub fn copy_text(writer: &mut impl Write, text: &str) -> io::Result<CopyMethod> {
    copy_with_fallback(writer, text, native_copy)
}

#[cfg(not(target_os = "macos"))]
fn native_copy(_: &str) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "pbcopy is available only on macOS",
    ))
}

#[cfg(target_os = "macos")]
fn native_copy(text: &str) -> io::Result<()> {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let input = child.stdin.take();
    let text = text.to_owned();
    let writer = std::thread::spawn(move || {
        input
            .ok_or_else(|| io::Error::other("pbcopy stdin unavailable"))?
            .write_all(text.as_bytes())
    });
    let stderr = child.stderr.take();
    // Drain errors while waiting, retaining a bounded diagnostic.
    let errors = std::thread::spawn(move || {
        use std::io::Read;
        let mut diagnostic = Vec::new();
        if let Some(mut stderr) = stderr {
            let mut chunk = [0; 1024];
            loop {
                let count = stderr.read(&mut chunk)?;
                if count == 0 {
                    break;
                }
                let keep = count.min(4096 - diagnostic.len());
                diagnostic.extend_from_slice(&chunk[..keep]);
            }
        }
        Ok::<_, io::Error>(String::from_utf8_lossy(&diagnostic).into_owned())
    });
    let deadline = Instant::now() + Duration::from_secs(1);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            Ok(None) => break Err(io::Error::new(io::ErrorKind::TimedOut, "pbcopy timed out")),
            Err(error) => break Err(error),
        }
    };
    if status.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let written = writer
        .join()
        .map_err(|_| io::Error::other("pbcopy stdin writer panicked"))?;
    let diagnostic = errors
        .join()
        .map_err(|_| io::Error::other("pbcopy stderr reader panicked"))??;
    let status = status?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "pbcopy: {status}: {}",
            sanitize(&diagnostic)
        )));
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    struct BrokenWriter {
        fail_flush: bool,
    }
    impl Write for BrokenWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.fail_flush {
                Ok(bytes.len())
            } else {
                Err(io::Error::other("write failure"))
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("flush failure"))
        }
    }

    #[test]
    fn success_never_runs_fallback() {
        let mut output = Vec::new();
        let method =
            copy_with_fallback(&mut output, "foo", |_| panic!("fallback after success")).unwrap();
        assert_eq!(method, CopyMethod::Osc52);
        assert_eq!(output, b"\x1b]52;c;Zm9v\x07");
    }

    #[test]
    fn write_and_flush_errors_use_sanitized_fallback_and_keep_primary_error() {
        for fail_flush in [false, true] {
            let method = copy_with_fallback(
                &mut BrokenWriter { fail_flush },
                "\x1b[31mfoo\x1b[0m",
                |text| {
                    assert_eq!(text, "foo");
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(
                method,
                CopyMethod::Fallback {
                    osc52_error: if fail_flush {
                        "flush failure"
                    } else {
                        "write failure"
                    }
                    .into()
                }
            );
        }
    }

    #[test]
    fn both_failures_reach_the_caller() {
        let error = copy_with_fallback(&mut BrokenWriter { fail_flush: false }, "foo", |_| {
            Err(io::Error::other("pbcopy unavailable"))
        })
        .unwrap_err();
        assert!(error.to_string().contains("write failure"));
        assert!(error.to_string().contains("pbcopy unavailable"));
    }
    #[test]
    fn osc52_padding_and_sanitization() {
        for (text, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("\x1b]52;c;evil\x07foo", "Zm9v"),
        ] {
            assert_eq!(clipboard_sequence(text), format!("\x1b]52;c;{encoded}\x07"));
        }
    }
}
