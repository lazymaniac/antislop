use super::super::config_write::{
    ConfigUpdateReply, ConfigUpdateStage, ConfigUpdateStatus, ConfigUpdateWarning,
    PreparedConfigWrite,
};
use super::*;

pub(super) fn route(context: &Context<'_>, request: Request) -> Result<Route> {
    route_pipeline(context, request, &[set_config_route, get_config_route])
}

fn set_config_route(context: &Context<'_>, request: Request) -> Result<Route> {
    match request {
        Request::SetConfig {
            toml,
            expected_revision,
        } => Ok(Route::Handled(set_config(
            context,
            toml,
            expected_revision,
        )?)),
        request => Ok(Route::Unmatched(request)),
    }
}

fn set_config(context: &Context<'_>, toml: String, expected_revision: String) -> Result<Value> {
    let parsed = Config::from_toml(&toml)?;
    let core = context.core;
    let config = context.config;
    let _guard = core
        .config_update
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    validate_config_revision(config, &expected_revision)?;
    let prepared = PreparedConfigWrite::prepare(config, &toml)?;
    let warnings = publish_config(core, config, parsed, &prepared)?;
    config_reply(toml, config, warnings)
}

fn validate_config_revision(config: &Path, expected_revision: &str) -> Result<()> {
    require(
        crate::hash(std::fs::read(config)?) == expected_revision,
        "configuration changed: reload before saving; draft preserved",
    )
}

fn publish_config(
    core: &Arc<Core>,
    config: &Path,
    parsed: Config,
    prepared: &PreparedConfigWrite,
) -> Result<Vec<ConfigUpdateWarning>> {
    let mut applied = false;
    let mut warnings = Vec::new();
    let result = core.store.publish_config_with_event(config, || {
        prepared.publish()?;
        core.replace_config(parsed);
        core.config_modified
            .store(modified(config), Ordering::SeqCst);
        applied = true;
        Ok(())
    });
    record_publish_warning(result, applied, &mut warnings)?;
    record_sync_warning(prepared, &mut warnings);
    Ok(warnings)
}

fn record_publish_warning(
    result: Result<()>,
    applied: bool,
    warnings: &mut Vec<ConfigUpdateWarning>,
) -> Result<()> {
    if let Err(error) = result {
        if !applied {
            return Err(error);
        }
        warnings.push(ConfigUpdateWarning {
            stage: ConfigUpdateStage::EventCommit,
            message: error.to_string(),
        });
    }
    Ok(())
}

fn record_sync_warning(prepared: &PreparedConfigWrite, warnings: &mut Vec<ConfigUpdateWarning>) {
    if let Err(error) = prepared.sync_directory() {
        warnings.push(ConfigUpdateWarning {
            stage: ConfigUpdateStage::DirectorySync,
            message: error.to_string(),
        });
    }
}

fn config_reply(toml: String, config: &Path, warnings: Vec<ConfigUpdateWarning>) -> Result<Value> {
    let status = if warnings.is_empty() {
        ConfigUpdateStatus::Applied
    } else {
        ConfigUpdateStatus::AppliedWithWarnings
    };
    Ok(serde_json::to_value(ConfigUpdateReply {
        toml,
        path: config.to_path_buf(),
        status,
        warnings,
    })?)
}

fn get_config_route(context: &Context<'_>, request: Request) -> Result<Route> {
    match request {
        Request::GetConfig => Ok(Route::Handled(get_config(context)?)),
        request => Ok(Route::Unmatched(request)),
    }
}

fn get_config(context: &Context<'_>) -> Result<Value> {
    let _guard = context
        .core
        .config_update
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let toml = std::fs::read_to_string(context.config)?;
    Ok(json!({
        "revision": crate::hash(&toml),
        "toml": toml,
        "path": context.config,
    }))
}
