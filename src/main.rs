use architect::{
    Result,
    ipc::{self, Operation, Request},
    runtime::Command,
};
use std::{io::Read, path::PathBuf};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let verb = args.get(1).map(String::as_str).unwrap_or("help");
    if verb == "__supervise" {
        return architect::process::supervise(&args[2..]);
    }
    let state = args
        .get(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".architect"));
    if verb == "daemon" {
        return ipc::serve(&state);
    }
    if verb == "help" || verb == "--help" {
        println!(
            "Architect local prototype\n\narchitect daemon STATE_DIR\narchitect request STATE_DIR < request.json\narchitect create STATE_DIR SPEC.json\narchitect snapshot STATE_DIR RUN_ID\narchitect list STATE_DIR\narchitect export STATE_DIR RUN_ID\narchitect events STATE_DIR CURSOR\n\nAll mutations require explicit idempotency keys via request.\nCreate generates a unique key. Stage approvals use scope_hashes from snapshot."
        );
        return Ok(());
    }
    if verb == "export" {
        let document = args.get(4).map(String::as_str).unwrap_or("run");
        let text = ipc::read_document(&state, arg(&args, 3)?, document)?;
        println!("{text}");
        return Ok(());
    }
    let request = match verb {
        "request" => {
            let mut input = String::new();
            std::io::stdin()
                .take(4 * 1024 * 1024 + 1)
                .read_to_string(&mut input)?;
            architect::require(
                input.len() <= 4 * 1024 * 1024,
                "request exceeds transport limit",
            )?;
            architect::transport::decode(input.as_bytes())?
        }
        "create" => Request {
            version: ipc::PROTOCOL_VERSION,
            request_id: architect::id(),
            idempotency_key: Some(architect::id()),
            operation: Operation::Mutate {
                command: Box::new(Command::Create {
                    spec: architect::transport::decode(&std::fs::read(arg(&args, 3)?)?)?,
                }),
            },
        },
        "snapshot" => Request {
            version: ipc::PROTOCOL_VERSION,
            request_id: architect::id(),
            idempotency_key: None,
            operation: Operation::Snapshot {
                run_id: arg(&args, 3)?.into(),
            },
        },
        "list" => Request {
            version: ipc::PROTOCOL_VERSION,
            request_id: architect::id(),
            idempotency_key: None,
            operation: args.get(3).map_or(Operation::List, |after| Operation::ListPage { after: after.clone() }),
        },
        "events" => Request {
            version: ipc::PROTOCOL_VERSION,
            request_id: architect::id(),
            idempotency_key: None,
            operation: Operation::Events {
                after: arg(&args, 3)?
                    .parse()
                    .map_err(|_| architect::Error::Invalid("invalid event cursor".into()))?,
            },
        },
        _ => {
            return Err(architect::Error::Invalid(
                "unknown command; use --help".into(),
            ));
        }
    };
    let response = ipc::call(&state, &request)?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    if let Some(error) = response.error {
        return Err(architect::Error::Invalid(error));
    }
    Ok(())
}
fn arg(args: &[String], index: usize) -> Result<&str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| architect::Error::Invalid("missing command argument".into()))
}
