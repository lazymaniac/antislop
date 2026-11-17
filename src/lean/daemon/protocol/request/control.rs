#[derive(Debug, Clone, PartialEq)]
pub enum ControlRequest {
    /// Which binary the daemon runs and whether it is executing work.
    Identity,
    ActivateRuntime {
        request_id: Option<String>,
        executable: String,
        path: Option<String>,
        conversation: Option<String>,
        task: Option<String>,
    },
    RuntimeActivationStatus,
    RuntimeActivationOperation {
        request_id: String,
    },
    CancelRuntimeActivation {
        request_id: String,
    },
    /// Trusted-client operational information, never an execution grant.
    ClassifyRuntimeFailure {
        conversation: String,
        message: u64,
        reason: String,
    },
    RuntimeNotice {
        conversation: String,
        text: String,
    },
    GetConfig,
    SetConfig {
        toml: String,
        expected_revision: String,
    },
    /// Stops an idle daemon so a frontend from another build can replace it.
    Shutdown,
    ForceShutdown,
    Conversations,
    Start,
    StartIn {
        project: String,
    },
}
