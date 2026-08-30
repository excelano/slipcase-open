//! The command line over the engine.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 9 makes this the floor: always present, the way the session list
//! stays reachable on a desktop with no tray, and the harness the engine is
//! driven by before any of the rest exists.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: slipcase-open <container.slpc>");
        return ExitCode::FAILURE;
    };

    let container = match slpc::Container::open(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}: {e}", std::path::Path::new(&path).display());
            return ExitCode::FAILURE;
        }
    };

    // Escaped rather than applied. SPEC 3 requires it of anything displaying a
    // payload name, and a refusal message is a display path like any other.
    let name = slpc::display_name(container.payload_name());
    match slipcase_open::extension::policy_key(container.payload_name()) {
        Some(key) => println!("{name}: policy key {key}"),
        None => println!("{name}: no usable extension"),
    }
    ExitCode::SUCCESS
}
