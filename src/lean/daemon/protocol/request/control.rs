#[derive(Debug, Clone, PartialEq)]
pub enum ControlRequest {
    /// Which binary the daemon runs and whether it is executing work.
    Identity,
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
    TrustStatus,
    Trust {
        trusted: bool,
    },
}
