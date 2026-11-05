#[derive(Debug, Clone, PartialEq)]
pub enum ControlRequest {
    /// Which binary the daemon runs and whether it is executing work.
    Identity,
    /// Trusted-client operational information, never an execution grant.
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
