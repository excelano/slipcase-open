//! The command line over the engine.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 9 makes this the floor: always present, the way the session list
//! stays reachable on a desktop that has no tray, and the harness the engine is
//! driven by before the notifications and the tray exist.
//!
//! **`open` runs the session in the foreground for now.** PLAN.md Phase 2 moves
//! it into a resident single instance behind an IPC front door, which is what
//! concept 8 requires and what lets one process hold several sessions. The loop
//! itself does not change when it moves; only who runs it does.
//!
//! Interrupting this rather than closing it leaves a recoverable session, which
//! is the crash path working as designed rather than a gap: the payload and the
//! record are on disk, and the next `sessions` finds them.

use std::io::{BufRead as _, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use clap::{Args, Parser, Subcommand};

use slipcase_open::platform::Host;
use slipcase_open::policy::{Layer, Origin, Source};
use slipcase_open::{flow, recover, session};

/// Open the payload of a slipcase container in its own application, and write
/// edits back into the container.
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    verb: Verb,
}

#[derive(Subcommand)]
enum Verb {
    /// Open a container's payload and watch for edits until told to stop.
    Open(Open),
    /// List the sessions on this machine and what became of each.
    Sessions,
    /// Act on a session left behind by one that did not close.
    Recover(Recover),
}

#[derive(Args)]
struct Open {
    /// The container to open.
    container: PathBuf,
}

#[derive(Args)]
struct Recover {
    /// The session, as `sessions` lists it.
    id: String,
    /// Put the payload back into its container.
    #[arg(long, conflicts_with = "discard")]
    write_back: bool,
    /// Throw the payload away and remove the session.
    #[arg(long)]
    discard: bool,
}

/// The stand-in for the platform policy sources, which arrive with the platform
/// arms in PLAN.md Phases 3 and 4. Saying nothing at every layer is what makes
/// `policy::resolve` fall through to the set this build ships, which is the
/// right behaviour for a machine with no policy applied and so is not a lie in
/// the meantime.
struct Unconfigured;

impl Source for Unconfigured {
    fn layer(&self, _origin: Origin) -> Option<Layer> {
        None
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let root = match session::default_root() {
        Ok(r) => r,
        Err(e) => return fail(&e),
    };
    let outcome = match cli.verb {
        Verb::Open(a) => open(&root, &a),
        Verb::Sessions => sessions(&root),
        Verb::Recover(a) => recover_one(&root, &a),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}

fn fail(e: &dyn std::fmt::Display) -> ExitCode {
    eprintln!("slipcase-open: {e}");
    ExitCode::FAILURE
}

fn open(root: &std::path::Path, a: &Open) -> Result<(), Box<dyn std::error::Error>> {
    let mut opened = flow::open(root, &a.container, &Unconfigured, &Host)?;

    let name = slpc::display_name(&opened.session().record().payload);
    println!("{name} is open.");
    if opened.mark != slpc::provenance::Mark::Silent {
        println!("  It came from somewhere else, and the copy says so.");
    }
    if let Some(what) = opened.misrepresented {
        println!("  Warning: the payload is {}.", what.describes());
    }
    println!("  Session {}", session_id(opened.session()));
    println!("Press Enter to close the session.");

    let stop = watch_for_enter();
    let mut wrote_back = 0u64;
    while !stop.load(Ordering::Relaxed) {
        match opened.wait_and_pump(Duration::from_millis(250)) {
            Ok(true) => {
                wrote_back += 1;
                println!("  Written back ({wrote_back}).");
            }
            Ok(false) => {}
            // Not fatal. Concept 6.2 puts the close at the user's hand, and a
            // failed save is a reason to tell them rather than to give up on
            // the container — the next save may well succeed, and the session
            // is still recoverable either way.
            Err(e) => eprintln!("  Could not write back: {e}"),
        }
    }

    // Concept 6.2: asking is the only available answer to Save As. No event
    // fires when somebody saves to a different location, so a session that saw
    // nothing may still have an edit that belongs in the container. Asking is
    // not detection and the question says so.
    //
    // The answer goes through the same comparison every other write-back does,
    // so saying yes when the payload already matches the container reports that
    // rather than rebuilding it to no purpose.
    if !opened.saw_a_change() && ask("The payload was not seen to change. Write it back anyway?")? {
        match opened.save_if_changed() {
            Ok(true) => println!("  Written back."),
            Ok(false) => println!("  The payload matches the container; nothing to write back."),
            Err(e) => eprintln!("  Could not write back: {e}"),
        }
    }

    match opened.close()? {
        flow::Closed::Cleared => println!("Session closed."),
        flow::Closed::LeftForRecovery => println!(
            "Session closed, and the application still has the payload open.\n\
             It has been left for recovery: run `slipcase-open sessions`."
        ),
    }
    Ok(())
}

fn sessions(root: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let found = session::scan(root)?;
    if found.is_empty() {
        println!("No sessions.");
        return Ok(());
    }
    for s in &found {
        let state = recover::state(s);
        println!(
            "{}  {}  {}",
            session_id(s),
            slpc::display_name(&s.record().payload),
            state
        );
        println!("    from {}", slpc::display_path(&s.record().container));
        if state.needs_a_person() {
            println!(
                "    slipcase-open recover {} --write-back|--discard",
                session_id(s)
            );
        }
    }
    Ok(())
}

fn recover_one(root: &std::path::Path, a: &Recover) -> Result<(), Box<dyn std::error::Error>> {
    let mut s = session::find(root, &a.id)?;
    if a.discard {
        s.remove()?;
        println!("Discarded.");
        return Ok(());
    }
    if !a.write_back {
        // Concept 6.3: recovery reports and never acts. Naming a session
        // without saying what to do with it is a question, so this answers it
        // rather than choosing.
        println!("{}: {}", a.id, recover::state(&s));
        println!("Pass --write-back or --discard to act on it.");
        return Ok(());
    }
    slipcase_open::writeback::write_back(&mut s)?;
    println!(
        "Written back to {}.",
        slpc::display_path(&s.record().container)
    );
    s.remove()?;
    Ok(())
}

/// The name `sessions` prints and `recover` takes back.
fn session_id(s: &session::Session) -> String {
    s.dir()
        .file_name()
        .map_or_else(|| "?".to_string(), |n| n.to_string_lossy().into_owned())
}

/// A thread that waits for a line, so the loop can keep pumping the watch
/// meanwhile. Reading stdin on the loop's own thread would block it for as long
/// as the person takes to decide, which is the whole session.
fn watch_for_enter() -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    std::thread::spawn(move || {
        let mut line = String::new();
        // End of input closes the session too, so this behaves under a pipe as
        // well as under a terminal.
        let _ = std::io::stdin().lock().read_line(&mut line);
        flag.store(true, Ordering::Relaxed);
    });
    stop
}

fn ask(question: &str) -> std::io::Result<bool> {
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "Yes"))
}
