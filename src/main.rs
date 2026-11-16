use antislop::Result;

const HELP: &str = "\
antislop — a team of CLI agents working on your project

usage:
  antislop [chat] [options] [message]
  antislop stop [--state DIR] [--force]
                              stop the daemon; --force cancels running work
  antislop activate --binary FILE [--conversation ID --task ID] [--config FILE] [--state DIR]
                              activate a checked runtime when current work finishes
  antislop activate --status [--request-id ID] [--state DIR]
                              show the durable activation result
  antislop activate --cancel --request-id ID [--state DIR]
                              cancel validation or a queued activation
  antislop init [--force]     write antislop.toml for the agent CLIs found on PATH
  antislop doctor [--config FILE] [--state DIR]
                              check the configuration and that every profile's CLI launches
  antislop setup agy          register antislop's MCP bridge in Agy's own MCP list (once)

With no message in a terminal it opens the TUI; with a message, or when
input is not a terminal, it prints the conversation line by line.

options:
  --config FILE        select a configuration file explicitly
  --state DIR          explicit runtime directory (default: global project state)
  --conversation ID    continue this conversation
  --new                start a new conversation
  --version [--json]   print the version and runtime capabilities

The user config directory follows ANTISLOP_CONFIG_DIR, then XDG_CONFIG_HOME/antislop,
then ~/.config/antislop. Runtime follows ANTISLOP_STATE_ROOT, then
XDG_STATE_HOME/antislop, then ~/.local/state/antislop. Linked worktrees share state.
";

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") if args.get(1).is_some_and(|arg| arg == "--json") => {
            println!("{}", antislop::lean::daemon::version_info());
            Ok(())
        }
        Some("--version") => {
            // The build identity tells two installs of one version apart
            // and matches what a running daemon reports.
            let build = std::env::current_exe()
                .ok()
                .and_then(|path| antislop::lean::daemon::build_identity(&path).ok())
                .map(|identity| format!(" (build {})", &identity[..12.min(identity.len())]))
                .unwrap_or_default();
            println!("antislop {}{build}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("help" | "--help" | "-h") => {
            print!("{HELP}");
            Ok(())
        }
        Some("__core") => {
            antislop::require(args.len() == 4, "__core requires PROJECT CONFIG STATE")?;
            antislop::lean::daemon::serve(
                std::path::Path::new(&args[1]),
                std::path::Path::new(&args[2]),
                std::path::Path::new(&args[3]),
            )
        }
        Some("__runtime_preflight") => {
            antislop::require(
                args.len() == 4,
                "__runtime_preflight requires PROJECT CONFIG STATE",
            )?;
            let checked = antislop::lean::daemon::runtime_preflight(
                std::path::Path::new(&args[1]),
                std::path::Path::new(&args[2]),
                std::path::Path::new(&args[3]),
            )?;
            println!("{checked}");
            Ok(())
        }
        // Started by Agy as its stdio MCP server; see `antislop setup agy`.
        Some("mcp-bridge") => antislop::lean::mcp::bridge(),
        Some("setup") => antislop::lean::cli::setup(&args[1..]),
        Some("init") => antislop::lean::cli::init(&args[1..]),
        Some("doctor") => antislop::lean::cli::doctor(&args[1..]),
        Some("stop") => antislop::lean::cli::stop(&args[1..]),
        Some("activate") => antislop::lean::cli::activate(&args[1..]),
        Some("chat") => antislop::lean::cli::chat(&args[1..]),
        Some(command) if command.starts_with("__") => Err(antislop::Error::Invalid(format!(
            "unknown internal command: {command}"
        ))),
        _ => antislop::lean::cli::chat(&args),
    }
}
