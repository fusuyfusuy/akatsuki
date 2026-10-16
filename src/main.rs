use clap::Parser;

use akatsuki::cli::{run_cli, Cli};

/// Exit 1 on failure. Under `--json` the error is also emitted on stdout as
/// `{"error": "<message>"}` so machine consumers always receive parseable output.
fn main() -> std::process::ExitCode {
    let json = std::env::args().any(|a| a == "--json");
    let cli = Cli::parse();
    match run_cli(cli) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            if json {
                println!("{}", serde_json::json!({ "error": format!("{e:#}") }));
            }
            eprintln!("Error: {e:?}");
            std::process::ExitCode::FAILURE
        }
    }
}
