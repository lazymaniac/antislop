use super::*;
use crate::lean::daemon::ControlRequest;

pub fn activate(args: &[String]) -> Result<()> {
    let mut binary = None;
    let mut task = None;
    let mut status = false;
    let mut rest = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--binary" => binary = Some(PathBuf::from(option_value(arg, &mut args)?)),
            "--task" => task = Some(option_value(arg, &mut args)?),
            "--status" => status = true,
            _ => rest.push(arg.clone()),
        }
    }
    crate::require(
        !status || (binary.is_none() && task.is_none()),
        "--status cannot request an activation",
    )?;
    let parsed = ParsedArgs::parse(&rest)?;
    crate::require(
        parsed.words.is_empty() && !parsed.new,
        "unexpected activation arguments",
    )?;
    let project = std::env::current_dir()?;
    let state = match parsed.state {
        Some(state) => project.join(state),
        None => crate::lean::paths::project_state(&project)?,
    };
    if status {
        println!(
            "{}",
            serde_json::to_string_pretty(&crate::lean::daemon::activation::status(&state)?)?
        );
        return Ok(());
    }
    let client = Client::at(&state);
    if let Some(config) = parsed.config {
        let identity = client.call(&Request::Control(ControlRequest::Identity))?;
        crate::require(
            identity["config"]
                == project
                    .join(config)
                    .canonicalize()?
                    .to_string_lossy()
                    .as_ref(),
            "activation configuration differs from the running daemon",
        )?;
    }
    let request = {
        let binary = binary.ok_or_else(|| {
            crate::Error::Invalid("activate requires --binary FILE or --status".into())
        })?;
        ControlRequest::ActivateRuntime {
            executable: binary.canonicalize()?.to_string_lossy().into_owned(),
            path: std::env::var("PATH").ok(),
            conversation: parsed.conversation,
            task,
        }
    };
    // Never start, replace or stop a daemon merely to query/request its handoff.
    let result = client.call(&Request::Control(request))?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
