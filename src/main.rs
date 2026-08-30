//! The command line over the engine.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 9 makes this the floor: always present, the way the session list
//! stays reachable on a desktop with no tray, and the harness the engine is
//! driven by before any of the rest exists.

use std::io::Read;
use std::process::ExitCode;

/// Fill as much of `head` as the payload has, treating a short payload as the
/// short answer it is rather than as a failure. SPEC 2.3 permits a payload of
/// zero length.
fn read_head(payload: &mut impl Read, head: &mut [u8]) -> usize {
    let mut at = 0;
    while at < head.len() {
        match payload.read(&mut head[at..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => at += n,
        }
    }
    at
}

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: slipcase-open <container.slpc>");
        return ExitCode::FAILURE;
    };

    let mut container = match slpc::Container::open(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}: {e}", std::path::Path::new(&path).display());
            return ExitCode::FAILURE;
        }
    };

    // Escaped rather than applied. SPEC 3 requires it of anything displaying a
    // payload name, and a refusal message is a display path like any other.
    // Owned, because reading the payload below needs the container mutably.
    let name = slpc::display_name(container.payload_name()).to_string();
    let key = slipcase_open::extension::policy_key(container.payload_name());
    match &key {
        Some(k) => println!("{name}: policy key {k}"),
        None => println!("{name}: no usable extension"),
    }

    // Read from the container rather than from an extracted copy, so the
    // warning is available before anything is written to disk. Concept 5.1
    // reports and never refuses, so this changes nothing about what happens
    // next.
    let mut head = [0u8; slipcase_open::content::HEAD];
    let read = match container.payload() {
        Ok(mut p) => read_head(&mut p, &mut head),
        Err(e) => {
            eprintln!("{name}: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(what) = slipcase_open::content::misrepresents(&head[..read], key.as_deref()) {
        println!("{name}: warning — the payload is {}", what.describes());
    }

    ExitCode::SUCCESS
}
