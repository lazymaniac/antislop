use architect::Result;

const HELP: &str = "\
antislop — a team of CLI agents working on your project

usage:
  antislop [chat] [options] [message]
  antislop export [options]   the conversation's history as JSON lines,
                              secrets redacted, on stdout
  antislop eval SUITE --cli CLI [--model M] [--effort E] [--arm A] [--jobs N]
                              calibrate verifiers on seeded cases (evals/)
                              against a plain one-agent review

With no message in a terminal it opens the TUI; with a message, or when
input is not a terminal, it prints the conversation line by line.

options:
  --project DIR        project to work on (default: current directory)
  --config FILE        configuration (default: PROJECT/architect.toml)
  --state DIR          private state directory (default: PROJECT/.antislop)
  --conversation ID    continue this conversation
  --new                start a new conversation
  --version            print the version
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
        Some("--version") => {
            println!("antislop {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("help" | "--help" | "-h") => {
            print!("{HELP}");
            Ok(())
        }
        Some("__core") => {
            architect::require(args.len() == 4, "__core requires PROJECT CONFIG STATE")?;
            architect::lean::daemon::serve(
                std::path::Path::new(&args[1]),
                std::path::Path::new(&args[2]),
                std::path::Path::new(&args[3]),
            )
        }
        Some("chat") => architect::lean::cli::chat(&args[1..]),
        Some("export") => architect::lean::export::export(&args[1..]),
        Some("eval") => architect::lean::eval::eval(&args[1..]),
        _ => architect::lean::cli::chat(&args),
    }
}
