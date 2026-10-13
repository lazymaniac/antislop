pub mod codex;
pub mod claude;
pub mod gemini;
pub mod fake;
mod prompts;
pub mod shared_cli;
use crate::{Result, domain::*};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, atomic::AtomicBool};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind { Authentication, RateLimit, Network, Unsupported, Policy, InvalidSession, Protocol, Cancelled, Process }

#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize)]
#[error("{provider}: {kind:?}: {message}")]
pub struct ProviderFailure {
    pub provider: String,
    pub kind: FailureKind,
    pub message: String,
    pub session_id: Option<String>,
    pub stopped: bool,
    pub partial_effects: bool,
}
pub fn failure(provider: &str, kind: FailureKind, message: &str) -> crate::Error {
    ProviderFailure { provider:provider.into(),kind,message:message.into(),session_id:None,stopped:true,partial_effects:false }.into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag="state",rename_all="snake_case")]
pub enum UsageReport {
    Unavailable,
    Reported { input_tokens: Option<u64>, output_tokens: Option<u64>, cached_tokens: Option<u64>, estimated_usd: Option<f64>, cumulative: bool },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag="type",rename_all="snake_case")]
pub enum ProviderEvent {
    DeliveryPending { turn_id:String, session_id:String },
    DeliverySent { turn_id:String },
    SessionObserved { session_id:String },
    CancellationRequested { turn_id:String },
    Stopped { turn_id:String },
    Completed { turn_id:String },
}
pub struct TurnRequest<'a> {
    pub turn_id: &'a str,
    pub run: &'a Run,
    pub prompt: &'a str,
    pub schema: &'a Value,
    /// An exact durable provider ID, never "latest" or a display index.
    pub resume: Option<&'a str>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnReceipt {
    pub turn_id:String,
    pub provider:String,
    pub session_id:String,
    pub delivered:bool,
    pub completed:bool,
    pub stopped:bool,
    pub output:String,
    pub usage:UsageReport,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all="snake_case")]
pub enum Support { Supported, Unsupported, Unverified }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeReport {
    pub capabilities:Capabilities,
    pub history:Support,
    pub exact_id_resume:Support,
    pub mid_turn_steering:Support,
    pub approvals:Support,
    pub os_sandbox:Support,
    pub tool_free:Support,
    pub authentication:Support,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub provider: String,
    pub version: String,
    pub structured_output: bool,
    pub streaming: bool,
    pub resume: bool,
    pub steering: bool,
    pub cancellation: bool,
    pub approvals: bool,
    pub native_subagents_disabled: bool,
    pub sandbox: String,
    pub usage: String,
}
pub trait Provider: Send + Sync {
    fn probe(&self) -> Result<Capabilities>;
    fn probe_report(&self) -> Result<ProbeReport> {
        Ok(ProbeReport { capabilities:self.probe()?,history:Support::Unverified,exact_id_resume:Support::Unverified,
            mid_turn_steering:Support::Unsupported,approvals:Support::Unsupported,os_sandbox:Support::Unverified,
            tool_free:Support::Unverified,authentication:Support::Unverified })
    }
    fn turn_receipt(&self, _request: &TurnRequest<'_>, _cancelled: Arc<AtomicBool>, _events: &mut dyn FnMut(&ProviderEvent)->Result<()>) -> Result<TurnReceipt> {
        Err(failure("provider",FailureKind::Unsupported,"receipt transport unavailable for this provider"))
    }
    fn execute(
        &self,
        run: &Run,
        cancelled: Arc<AtomicBool>,
        session: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<(String, String)>;
    fn verify(
        &self,
        run: &Run,
        artifact: &Artifact,
        evidence: &Evidence,
        cancelled: Arc<AtomicBool>,
        session: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<Review>;
}
/// One provider instance per worker; selection is explicit in the persisted run.
pub fn build(name: &str) -> Result<Box<dyn Provider>> {
    match name {
        "fake" => Ok(Box::new(fake::FakeProvider)),
        "codex" => Ok(Box::new(codex::CodexProvider::default())),
        "claude" => Ok(Box::new(claude::ClaudeProvider::default())),
        "gemini" => Ok(Box::new(gemini::GeminiProvider::default())),
        _ => Err(crate::Error::Invalid("unsupported provider".into())),
    }
}
