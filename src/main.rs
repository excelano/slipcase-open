//! The command line over the engine.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 9 makes this the floor: always present, the way the session list
//! stays reachable on a desktop that has no tray, and the harness the engine is
//! driven by before the notifications and the tray exist.
//!
//! **Every verb is a client of the front door first.** Concept 8: where an
//! instance is running the invocation hands over and exits; where none is, an
//! `open` becomes the instance and the rest do their work against the state
//! directory, which is where the answer lives when nobody is holding anything.
//!
//! **The instance runs in the foreground and holds the terminal.** Detaching
//! means `fork`, and this crate forbids `unsafe`; nothing in concept 8 asks for
//! a background process, and from Phase 3 the tool is started from a desktop
//! entry rather than a shell, where there is no terminal to hold. Interrupting
//! it rather than closing its sessions leaves them recoverable, which is the
//! crash path working as designed.

use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

use slipcase_open::endpoint;
use slipcase_open::ipc::{self, Request, Response, Voice};
use slipcase_open::outside::Outside;
use slipcase_open::platform::Host;
use slipcase_open::policy;
use slipcase_open::present::{self, Channel, Report};
use slipcase_open::resident::{self, Resident};
use slipcase_open::{recover, session};

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
    /// Open a container's payload and watch for edits.
    Open(Open),
    /// List the sessions on this machine and what became of each.
    Sessions,
    /// Close an open session, writing back what it has.
    Close(Close),
    /// Act on a session left behind by one that did not close.
    Recover(Recover),
}

#[derive(Args)]
struct Open {
    /// The container to open.
    container: PathBuf,
}

#[derive(Args)]
struct Close {
    /// The session, as `sessions` lists it.
    id: String,
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

type Fallible = Result<(), Box<dyn std::error::Error>>;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let outcome = || -> Fallible {
        let root = session::default_root()?;
        let door = endpoint::path()?;
        match cli.verb {
            Verb::Open(a) => open(&root, &door, &a),
            Verb::Sessions => sessions(&root, &door),
            Verb::Close(a) => close(&door, &a),
            Verb::Recover(a) => recover_one(&root, &door, &a),
        }
    }();
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("slipcase-open: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Hand a request to the instance, if there is one.
///
/// `Ok(None)` means nobody is listening, which is not a failure: it is the
/// common case for the first invocation.
fn hand_over(door: &Path, request: &Request) -> Result<Option<Response>, ipc::Error> {
    match endpoint::connect(door) {
        Err(_) => Ok(None),
        Ok(mut stream) => ipc::ask(&mut stream, request).map(Some),
    }
}

fn say(response: &Response) -> Fallible {
    match response {
        Response::Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            Ok(())
        }
        Response::Err(why) => Err(why.clone().into()),
    }
}

fn open(root: &Path, door: &Path, a: &Open) -> Fallible {
    // Resolved here, so that what goes over the wire and what the instance
    // matches against its table are the same file rather than two spellings.
    let container = std::fs::canonicalize(&a.container)
        .map_err(|e| format!("{}: {e}", a.container.display()))?;
    let request = Request::Open {
        container,
        // Concept 9. A double-click has no terminal, so the lines handed back
        // here go nowhere and the instance has to speak for this invocation.
        // Asked of the error stream because that is where this program's own
        // messages go; a run whose output is piped into something still has
        // somewhere to show a refusal.
        voice: if std::io::stderr().is_terminal() {
            Voice::Client
        } else {
            Voice::Instance
        },
    };

    if let Some(response) = hand_over(door, &request)? {
        return say(&response);
    }

    // Nobody is listening, so become the instance. Losing the race to bind
    // means somebody else became it between the connect and the bind, and the
    // answer is to hand over to them rather than to fail.
    let Ok(listener) = endpoint::bind(door) else {
        return match hand_over(door, &request)? {
            Some(response) => say(&response),
            None => Err("could not reach or become the instance".into()),
        };
    };

    let channel = channel();
    let source = policy::files::Files::for_this_platform();
    let outside = Outside::new(&source, &Host, channel.as_ref());
    report_policy(&outside);

    // Before the first session, and with nothing live yet to protect. Concept
    // 6.3: an unchanged leftover means nothing was lost, so it goes quietly.
    let _ = resident::sweep(root, &[]);

    let mut instance = Resident::new(root);
    say(&instance.handle(request, &outside))?;
    if instance.is_idle() {
        // The open was refused, so there is nothing to hold and no reason to
        // keep the front door.
        return Ok(());
    }

    if std::io::stderr().is_terminal() {
        // Only where somebody is looking at it. As a notification this would be
        // the tool announcing that it had started, which is what the document
        // opening already said.
        eprintln!("Watching. Interrupt to leave the sessions recoverable, or:");
        eprintln!("  slipcase-open close <session>");
    }

    resident::run(listener, &mut instance, &outside)?;
    instance.stand_down(&outside);
    Ok(())
}

/// Concept 9's channel, or the floor beneath it.
///
/// The fallback is silent on purpose. A machine with no session bus is one
/// where the command line is the interface, and announcing that the
/// notifications are unavailable would be the first thing this tool said on
/// every run in an SSH session.
fn channel() -> Box<dyn Channel> {
    #[cfg(target_os = "linux")]
    if let Ok(desktop) = present::freedesktop::Desktop::connect() {
        return Box::new(desktop);
    }
    Box::new(present::terminal::Terminal)
}

fn sessions(root: &Path, door: &Path) -> Fallible {
    // The instance knows which are open; the state directory knows what was
    // left. Ask whoever can answer.
    if let Some(response) = hand_over(door, &Request::List)? {
        return say(&response);
    }
    let found = session::scan(root)?;
    if found.is_empty() {
        println!("No sessions.");
        return Ok(());
    }
    for s in &found {
        let state = recover::state(s);
        println!(
            "{}  {}  {}",
            id_of(s),
            slpc::display_name(&s.record().payload),
            state
        );
        println!("    from {}", slpc::display_path(&s.record().container));
        if state.needs_a_person() {
            println!(
                "    slipcase-open recover {} --write-back|--discard",
                id_of(s)
            );
        }
    }
    Ok(())
}

fn close(door: &Path, a: &Close) -> Fallible {
    match hand_over(door, &Request::Close(a.id.clone()))? {
        Some(response) => say(&response),
        None => Err("no instance is running, so nothing is open to close".into()),
    }
}

fn recover_one(root: &Path, door: &Path, a: &Recover) -> Fallible {
    // A session the instance is holding is not recovery's to touch: its watcher
    // is live and its payload may be mid-save. `close` is the verb for that,
    // and it writes back on the way out.
    if let Some(Response::Ok(lines)) = hand_over(door, &Request::List)? {
        if lines
            .iter()
            .any(|l| l.starts_with(&a.id) && l.contains("open,"))
        {
            return Err(format!(
                "{} is open, not left behind. Use `slipcase-open close {}`.",
                a.id, a.id
            )
            .into());
        }
    }

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

/// Concept 10 asks the interface to say when settings are administered, both to
/// set expectations and to keep somebody from reporting that the application
/// randomly refuses to open files.
///
/// Nothing is said where policy cannot be read: the open below resolves it
/// again — it has to, because §10 puts the decision in the launch path and not
/// in whatever ran before it — and refuses with that reason. Two copies of one
/// message is noise.
fn report_policy(outside: &Outside<'_>) {
    let Ok(effective) = policy::resolve(outside.policy) else {
        return;
    };
    if effective.managed {
        let mut report = Report::ordinary("Settings on this machine are administered.");
        if effective.configuration_suppressed {
            report = report.and("Your own configuration is not being consulted.");
        }
        outside.channel.report(&report);
    }
    for entry in &effective.uncomparable_entries {
        outside.channel.report(&Report::ordinary(format!(
            "Ignored: `{entry}` in a policy list cannot match any payload."
        )));
    }
}

/// The name `sessions` prints and the other verbs take back.
fn id_of(s: &session::Session) -> String {
    s.dir()
        .file_name()
        .map_or_else(|| "?".to_string(), |n| n.to_string_lossy().into_owned())
}
