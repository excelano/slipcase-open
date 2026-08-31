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

/// A failure whoever needed to see it has already seen.
///
/// Concept 9 puts the instance's narration on its channel, and on a machine
/// with no notification service that channel is this program's own error
/// stream. A refusal reported there and then returned to the top would appear
/// in it twice. This carries the exit code and no second sentence.
#[derive(Debug)]
struct AlreadySaid;

impl std::fmt::Display for AlreadySaid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("reported through the channel")
    }
}

impl std::error::Error for AlreadySaid {}

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
            if e.downcast_ref::<AlreadySaid>().is_none() {
                eprintln!("slipcase-open: {e}");
            }
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
    // Concept 9. A double-click has no terminal, so the lines handed back here
    // go nowhere and the instance has to speak for this invocation. Asked of
    // the error stream because that is where this program's own messages go; a
    // run whose output is piped into something still has somewhere to show a
    // refusal.
    let voice = if std::io::stderr().is_terminal() {
        Voice::Client
    } else {
        Voice::Instance
    };
    let request = Request::Open { container, voice };

    if let Some(response) = hand_over(door, &request)? {
        return say(&response);
    }

    // Nobody is listening, so become the instance. Losing the race to bind
    // means somebody else became it between the connect and the bind, and the
    // answer is to hand over to them rather than to fail.
    let listener = match endpoint::bind(door) {
        Ok(listener) => listener,
        Err(why) => {
            // Where nobody answers either, the bind error is the only account
            // of what went wrong, and it is worth more than a sentence saying
            // that neither worked. A socket path over the platform's length
            // limit fails both halves and reads as a mystery without it.
            return match hand_over(door, &request)? {
                Some(response) => say(&response),
                None => Err(format!(
                    "could not reach the instance, and {} could not be bound: {why}",
                    door.display()
                )
                .into()),
            };
        }
    };

    let channel = channel();
    let source = policy::files::Files::for_this_platform();
    // Resolved once and held for the life of the instance. Concept 10's warning
    // about caching is about what may be opened, where a value held across a
    // policy push is a bypass; `flow::open` still resolves the lists itself on
    // every launch. How loud to be gates no decision.
    let volume = policy::resolve(&source)
        .map(|e| e.notify)
        .unwrap_or_default();
    let outside = Outside::new(&source, &Host, channel.as_ref()).saying(volume);
    report_policy(&outside);

    // Before the first session, and with nothing live yet to protect. Concept
    // 6.3: an unchanged leftover means nothing was lost, so it goes quietly.
    let _ = resident::sweep(root, &[]);

    let mut instance = Resident::new(root);
    let response = instance.handle(request, &outside);
    if instance.is_idle() {
        // Nothing was started and nothing is being held, so there is no reason
        // to keep the front door.
        return match &response {
            Response::Ok(_) => say(&response),
            Response::Err(_) if voice == Voice::Client => say(&response),
            Response::Err(_) => Err(AlreadySaid.into()),
        };
    }

    // Something is being held, and a refusal can be what is holding it.
    // Concept 8 has an open on a container with a session left behind refuse
    // *and* raise the recovery question, so returning on the refusal would end
    // the process that the question's buttons have to reach — and end it
    // without withdrawing them, which is the one thing `stand_down` exists to
    // prevent. The refusal is reported and the loop runs anyway.
    match &response {
        Response::Ok(_) => say(&response)?,
        // Said once, on the same rule the instance narrates by: where the voice
        // is the instance's, it has already gone through the channel — which on
        // a machine with no notification service is this same error stream.
        Response::Err(why) if voice == Voice::Client => eprintln!("slipcase-open: {why}"),
        Response::Err(_) => {}
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
        outside.report(&report);
    }
    for entry in &effective.uncomparable_entries {
        outside.report(&Report::ordinary(format!(
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
