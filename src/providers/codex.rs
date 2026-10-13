//! Restricted Codex exec JSONL adapter. App-server remains the preferred richer
//! transport, but exec can ignore user configuration in the installed version.
use super::*;
use crate::{Error, id, require};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::Stdio;
use std::sync::{atomic::Ordering, mpsc};
use std::time::Duration;

pub const MAX_FRAME: usize = crate::transport::MAX_FRAME;
#[derive(Debug, PartialEq, Eq)]
pub enum NormalizedEvent {
    Session(String),
    Message(String),
    Completed,
    Failed,
    Unknown,
}
pub fn normalize(value: &Value) -> Result<NormalizedEvent> {
    let kind = value["type"]
        .as_str()
        .ok_or_else(|| Error::Invalid("provider event has no type".into()))?;
    Ok(match kind {
        "thread.started" => NormalizedEvent::Session(
            value["thread_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| Error::Invalid("missing provider session ID".into()))?
                .into(),
        ),
        "item.completed" if value["item"]["type"] == "agent_message" => NormalizedEvent::Message(
            value["item"]["text"]
                .as_str()
                .ok_or_else(|| Error::Invalid("missing agent text".into()))?
                .into(),
        ),
        "turn.completed" => NormalizedEvent::Completed,
        "turn.failed" | "error" => NormalizedEvent::Failed,
        _ => NormalizedEvent::Unknown,
    })
}
/// Byte-limited frames, including EOF without newline; malformed UTF-8 is rejected.
pub fn read_frame(reader: &mut impl BufRead) -> Result<Option<Value>> {
    crate::transport::read(reader)
}
#[derive(Default)]
pub struct CodexProvider {
    pub binary: Option<String>,
    pub native_effort: Option<String>,
}
impl CodexProvider {
    fn binary(&self) -> &str {
        self.binary.as_deref().unwrap_or("codex")
    }
    fn version(&self, cancelled: Option<&AtomicBool>) -> Result<String> {
        let mut command = crate::process::command(self.binary())?;
        command
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = crate::process::SupervisedChild::spawn(&mut command)?;
        let stdout = child.child.stdout.take().expect("version stdout");
        let (sender, receiver) = mpsc::sync_channel(1);
        let reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = stdout
                .take((MAX_FRAME + 1) as u64)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = sender.send(result);
        });
        let mut status = None;
        let mut output = None;
        while status.is_none() || output.is_none() {
            require(
                !cancelled.is_some_and(|flag| flag.load(Ordering::SeqCst)),
                "provider cancellation requested during version probe",
            )?;
            status = status.or(child.child.try_wait()?);
            if output.is_none() {
                match receiver.recv_timeout(Duration::from_millis(25)) {
                    Ok(result) => {
                        let bytes = result?;
                        require(
                            bytes.len() <= MAX_FRAME,
                            "Codex version output exceeds transport limit",
                        )?;
                        output = Some(bytes);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => (),
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return Err(Error::Invalid("Codex version reader failed".into()));
                    }
                }
            } else if status.is_none() {
                std::thread::sleep(Duration::from_millis(25));
            }
        }
        let _ = reader.join();
        require(
            status.expect("probe status").success(),
            "Codex version probe failed",
        )?;
        Ok(String::from_utf8_lossy(&output.expect("probe output"))
            .trim()
            .into())
    }
    pub fn turn(
        &self,
        run: &Run,
        prompt: &str,
        schema: &Value,
        resume: Option<&str>,
        cancelled: Arc<AtomicBool>,
        session: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<(String, String)> {
        let version = self.version(Some(&cancelled))?;
        require(
            version == "codex-cli 0.162.1",
            "unsupported Codex version; rerun M0 before enabling it",
        )?;
        let schema_file = std::env::temp_dir().join(format!("architect-schema-{}.json", id()));
        let mut schema_handle = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&schema_file)?;
        use std::os::unix::fs::PermissionsExt;
        schema_handle.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        serde_json::to_writer(&mut schema_handle, schema)?;
        struct SchemaFile(std::path::PathBuf);
        impl Drop for SchemaFile {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let schema_file = SchemaFile(schema_file);
        let mut command = crate::process::command(self.binary())?;
        command
            .args([
                "exec",
                "--json",
                "--color",
                "never",
                "--ignore-user-config",
                "--ignore-rules",
                "--sandbox",
                "read-only",
                "--output-schema",
            ])
            .arg(&schema_file.0)
            .args([
                "--disable",
                "multi_agent",
                "--disable",
                "multi_agent_v2",
                "--disable",
                "hooks",
                "--disable",
                "plugins",
                "--disable",
                "remote_plugin",
                "--disable",
                "shell_tool",
                "-c",
                "web_search=\"disabled\"",
                "-c",
                "approval_policy=\"never\"",
            ])
            .arg("--cd")
            .arg(&run.spec.workspace);
        if let Some(model) = &run.spec.model {
            command.arg("--model").arg(model);
        }
        if let Some(effort) = &self.native_effort {
            require(matches!(effort.as_str(), "low" | "medium" | "high" | "xhigh"), "unsupported Codex native effort")?;
            command.arg("-c").arg(format!("model_reasoning_effort={effort:?}"));
        }
        if let Some(session_id) = resume {
            require(!session_id.trim().is_empty(), "explicit resume ID required")?;
            command.arg("resume").arg(session_id);
        }
        command
            .arg("-")
            .current_dir(&run.spec.workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = crate::process::SupervisedChild::spawn(&mut command)?;
        let stdout = child.child.stdout.take().expect("stdout pipe");
        let stderr = child.child.stderr.take().expect("stderr pipe");
        // Do not retain raw diagnostics, which can contain account and secret data.
        let diagnostics = std::thread::spawn(move || {
            let _ = std::io::copy(&mut BufReader::new(stderr), &mut std::io::sink());
        });
        let (sender, receiver) = mpsc::sync_channel(16);
        let reader = std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let value = read_frame(&mut reader);
                let terminal = matches!(value, Ok(None) | Err(_));
                if sender.send(value).is_err() || terminal {
                    break;
                }
            }
        });
        let mut stdin = child.child.stdin.take().expect("stdin pipe");
        stdin.write_all(prompt.as_bytes())?;
        drop(stdin);
        let mut session_id = resume.map(String::from);
        let mut message = None;
        let mut completed = false;
        loop {
            require(
                !cancelled.load(Ordering::SeqCst),
                "provider cancellation requested; attempt may have partial effects",
            )?;
            let event = match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(Some(value))) => normalize(&value)?,
                Ok(Ok(None)) => break,
                Ok(Err(error)) => return Err(error),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            match event {
                NormalizedEvent::Session(value) => { session(&value)?; session_id = Some(value); },
                NormalizedEvent::Message(value) => message = Some(value),
                NormalizedEvent::Completed => completed = true,
                NormalizedEvent::Failed => return Err(Error::Invalid("Codex reported a failed turn (auth/network/rate-limit details unavailable; run provider diagnostics locally)".into())),
                NormalizedEvent::Unknown => (),
            }
        }
        let status = child.child.wait()?;
        let _ = reader.join();
        let _ = diagnostics.join();
        require(
            status.success() && completed,
            "Codex exited without a completed turn",
        )?;
        let session_id =
            session_id.ok_or_else(|| Error::Invalid("provider session ID unavailable".into()))?;
        let message =
            message.ok_or_else(|| Error::Invalid("provider returned no artifact".into()))?;
        let _: Value = serde_json::from_str(&message)?;
        Ok((session_id, message))
    }
}
impl Provider for CodexProvider {
    fn probe(&self) -> Result<Capabilities> {
        Ok(Capabilities { provider: "codex".into(), version: self.version(None)?,
            structured_output: true, streaming: true, resume: true, steering: false,
            cancellation: true, approvals: false, native_subagents_disabled: true,
            sandbox: "read-only; user configuration/rules ignored; hooks/plugins/shell/subagents disabled".into(), usage: "unavailable (not yet normalized)".into() })
    }
    fn execute(&self, run: &Run, cancelled: Arc<AtomicBool>, session: &mut dyn FnMut(&str) -> Result<()>) -> Result<(String,String)> {
        super::prompts::execute(self, run, cancelled, session)
    }
    fn verify(&self, run: &Run, artifact: &Artifact, evidence: &Evidence, cancelled: Arc<AtomicBool>, session: &mut dyn FnMut(&str) -> Result<()>) -> Result<Review> {
        super::prompts::verify(self, run, artifact, evidence, cancelled, session)
    }
}
impl super::prompts::JsonTurn for CodexProvider {
 fn json_turn(&self, run: &Run, prompt: &str, schema: &Value, resume: Option<&str>, cancelled: Arc<AtomicBool>, session: &mut dyn FnMut(&str) -> Result<()>) -> Result<(String,String)> {
  self.turn(run, prompt, schema, resume, cancelled, session)
 }
}
