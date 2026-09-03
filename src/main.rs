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

// The level `Cargo.toml` explains: `forbid` everywhere Windows is not.
#![cfg_attr(not(windows), forbid(unsafe_code))]
// No console of its own, and this is behaviour rather than appearance.
//
// A console subsystem binary always has a terminal, so `is_terminal` below was
// answering yes to a double-click and the voice was the client's — which is the
// branch where the instance says nothing through concept 9's channel, because a
// person is taken to be reading the lines it hands back. Measured on 2026-09-01
// against the packaged build: the narration appeared in a console window that
// stayed open for the life of the session, and no toast was ever raised.
//
// So the subsystem is what makes the channel fire on a double-click, and the
// console window it also removes is the smaller half of it. `attach_console`
// gives the terminal back to an invocation that came from one.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, CommandFactory as _, Parser, Subcommand};

use slipcase_open::endpoint;
use slipcase_open::ipc::{self, Request, Response, Voice};
use slipcase_open::outside::Outside;
use slipcase_open::platform::Host;
use slipcase_open::policy;
use slipcase_open::present::{self, Channel, Report};
use slipcase_open::resident::{self, Resident};
use slipcase_open::{recover, session};

/// Open the payload of a Slipcase container in its own application, and write
/// edits back into the container.
///
/// Two lines rather than the table, and under `--help` rather than `-h`.
/// Concept 9 keeps the command line the floor beneath everything else, and the
/// thing read most often is the list of verbs: every line spent on paths here
/// is a line between somebody and the verb they came for. `policy` is where the
/// paths belong, because it prints the ones this machine resolves rather than
/// the ones this build documents.
///
/// Wrapped by hand. `clap` reflows help text only with its `wrap_help`
/// feature, which pulls in a terminal-size crate and is not enabled here, so an
/// unbroken line would run off the side of the screen rather than off the side
/// of nothing.
#[derive(Parser)]
#[command(
    version,
    about,
    long_about = None,
    // So that a lone container path is not read as a malformed subcommand.
    args_conflicts_with_subcommands = true,
    after_long_help = "Settings are read from /etc/slipcase/open.toml and from\n\
$XDG_CONFIG_HOME/slipcase-open/policy.toml, neither of which has to exist.\n\
Run `slipcase-open policy` for the paths on this machine, or see\n\
slipcase-open(1)."
)]
struct Cli {
    #[command(subcommand)]
    verb: Option<Verb>,
    /// A container to open, for an invocation that names no verb.
    ///
    /// **The association is why this exists.** Concept 4 wants a double-click,
    /// and on Windows a packaged handler is launched with the file path and
    /// nothing else — there is no place in a manifest to put a verb in front of
    /// it, the way `Exec=slipcase-open open %f` does in the desktop entry. So a
    /// lone path means `open`, which is also what every other document handler
    /// does and is no worse a command line for it.
    container: Option<PathBuf>,
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
    /// Where settings are read from, and what they add up to.
    Policy,
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
    #[cfg(windows)]
    attach_console();
    let cli = Cli::parse();
    // A verb, or a bare path meaning `open`, or neither — and neither is what
    // clap used to refuse for us, so it is refused here in the same shape.
    let verb = match (cli.verb, cli.container) {
        (Some(verb), _) => Some(verb),
        (None, Some(container)) => Some(Verb::Open(Open { container })),
        // Neither a verb nor a container. On Windows that is the Start tile
        // being clicked, which concept 12 answers with the standing list; a
        // packaged application whose tile does nothing is the first thing a
        // person sees of it. Everywhere else it is a command missing its verb.
        (None, None) => {
            if cfg!(windows) {
                None
            } else {
                let _ = Cli::command().print_help();
                return ExitCode::from(2);
            }
        }
    };
    let outcome = || -> Fallible {
        let root = session::default_root()?;
        let door = endpoint::path()?;
        match verb {
            Some(Verb::Open(a)) => open(&root, &door, &a),
            Some(Verb::Sessions) => sessions(&root, &door),
            Some(Verb::Close(a)) => close(&door, &a),
            Some(Verb::Recover(a)) => recover_one(&root, &door, &a),
            Some(Verb::Policy) => settings(&root, &door),
            None => stand_by(&root, &door),
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
    // Before the request goes anywhere. This process is the one the shell just
    // activated, so it is the one holding the right to put a window in front,
    // and the instance about to do the launching is not. `platform::shell` says
    // what that costs when it is left here.
    #[cfg(windows)]
    slipcase_open::platform::hand_the_foreground_on();
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

    let standing = standing();
    resident::run(listener, &mut instance, &outside, standing.as_ref())?;
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
    #[cfg(windows)]
    if let Ok(toast) = present::toast::Toast::connect() {
        return Box::new(toast);
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

/// Concept 10's layers, named rather than described.
///
/// **The paths are resolved and not documented.** Every one of them comes out
/// of the environment — `XDG_CONFIG_HOME` and `XDG_STATE_HOME` here,
/// `%LOCALAPPDATA%` and a registry subtree elsewhere — so the file somebody
/// should edit and the file this build's documentation names are two
/// questions, and only the running program can answer the first. `git config
/// --show-origin` and `npm config ls -l` exist for the same reason.
///
/// **Read here rather than asked of the instance.** There is nothing for the
/// front door to add: concept 10 forbids holding what may be opened, so a
/// running instance re-reads these files on every launch as well. The one
/// value it does keep is how loud to be, and that is what it read when it
/// started rather than what is in the file now — which is a reason to print
/// the file's answer here rather than the instance's.
///
/// **The locations are printed before anything is resolved.** A file with a
/// typo in it still has to be *named* by the verb somebody ran to find out
/// where their settings are, and a resolution that refuses to guess past a
/// broken layer would otherwise print nothing at all. The refusal follows the
/// listing and still fails the run.
fn settings(root: &Path, door: &Path) -> Fallible {
    let source = policy::files::Files::for_this_platform();
    let resolved = policy::resolve(&source);

    let mut layers = source.locations().peekable();
    if layers.peek().is_none() {
        // Every platform but Linux, until PLAN.md Phases 4 and 5. Said out
        // loud, because a blank heading reads as a program that could not find
        // its own configuration.
        println!("No settings files on this platform yet. The built-in set is what decides.");
    } else {
        println!("Where settings are read, in order of authority:");
        println!();
        for (origin, path) in layers {
            println!(
                "  {:<14}  {} ({})",
                origin,
                slpc::display_path(path),
                layer_state(&source, origin, resolved.as_ref().ok())
            );
        }
        println!();
    }

    // Named by the error itself, which carries the path. Returned rather than
    // reported, because a policy that cannot be established is not a policy
    // that permits and the exit code has to say so.
    let effective = resolved?;

    println!("What they add up to:");
    println!();
    let allowed: Vec<&str> = effective.allowed().collect();
    let denied: Vec<&str> = effective.denied().collect();
    labelled("allowed", &allowed);
    labelled("denied", &denied);
    println!(
        "  {:<14}  {}",
        "notify",
        // The spelling the file itself uses, so that what this prints can be
        // typed back into it.
        match effective.notify {
            policy::Notify::Everything => "everything",
            policy::Notify::Important => "important",
        }
    );
    println!(
        "  {:<14}  {}",
        "write-back",
        if effective.confirm_each_write_back {
            "confirmed each time"
        } else {
            "as the payload is saved"
        }
    );
    println!();

    // The same sentences `report_policy` says on the way into an open, because
    // somebody who saw one there and came here to find out more should meet
    // the claim they arrived with rather than a paraphrase of it.
    if effective.managed {
        println!("Settings on this machine are administered.");
        if effective.configuration_suppressed {
            println!("Your own configuration is not being consulted.");
        }
        println!();
    }
    for entry in &effective.uncomparable_entries {
        println!("Ignored: `{entry}` in a policy list cannot match any payload.");
    }
    if !effective.uncomparable_entries.is_empty() {
        println!();
    }

    // Not policy, and here anyway. Somebody looking for where this tool keeps
    // its files has one question and not two, and the answer to the half that
    // is not configuration is a directory nothing else prints.
    println!("Where the tool keeps its own state:");
    println!();
    println!("  {:<14}  {}", "sessions", slpc::display_path(root));
    println!("  {:<14}  {}", "front door", slpc::display_path(door));
    let (speaks, refused) = how_it_speaks();
    println!("  {:<14}  {speaks}", "notifications");
    if let Some(why) = refused {
        // Named here and nowhere else. Concept 9's fallback is silent on
        // purpose — announcing it on every run would be the first thing this
        // tool said in an SSH session — but *where settings are read from* is
        // exactly the verb somebody reaches for when they are asking why
        // nothing appeared, and a silence with no account of itself is the
        // thing that cannot be debugged.
        println!("  {:<14}  {why}", "");
    }
    Ok(())
}

/// Concept 12's standing session list, where the platform has somewhere to put
/// one.
///
/// The same shape as [`channel`]: try the platform's, and fall back to the one
/// that does nothing. A session with no shell has no tray, and that is ordinary.
/// **Where a terminal started this there is no icon, and that is concept 9's
/// own rule rather than a special case.** The command line is the floor beneath
/// the tray: a terminal *is* a standing list, `sessions` answers the same
/// question, and the person is looking at the output already. It is also what
/// keeps concept 8's exit rule intact for that invocation — an `open` typed at
/// a prompt that never returned would be a worse command than the one it
/// replaced.
///
/// Everything else — a double-click, the Start tile, the context menu — has no
/// terminal and gets an icon that stays until it is asked to leave.
fn standing() -> Box<dyn present::Standing> {
    if std::io::stderr().is_terminal() {
        return Box::new(present::Nowhere);
    }
    #[cfg(windows)]
    if let Ok(tray) = present::tray::Tray::show_up() {
        return Box::new(tray);
    }
    Box::new(present::Nowhere)
}

/// Stand by with the standing list and nothing open.
///
/// **The Start tile**, which is what a packaged product does when somebody
/// clicks the thing they installed and has no container in mind. It puts up the
/// icon and waits; every other invocation reaches the same place by opening
/// something first.
///
/// The earlier version of this comment claimed this was the *only* invocation
/// that outlived its work, on the reasoning that otherwise every container
/// opened in a morning would leave an icon behind. That reasoning was wrong,
/// and the front door is why: the second double-click hands over to the
/// instance the first one started, so a morning's containers are one icon and
/// not ten. There was never a pile to prevent.
///
/// Where an instance is already running there is already an icon, so this hands
/// over and says nothing rather than raising a second one.
///
/// # Errors
///
/// Where the front door cannot be bound or served.
fn stand_by(root: &Path, door: &Path) -> Fallible {
    #[cfg(windows)]
    {
        if hand_over(door, &Request::Ping)?.is_some() {
            return Ok(());
        }
        let Ok(listener) = endpoint::bind(door) else {
            // Somebody bound it between the two calls, which means there is an
            // instance and therefore a tray. Nothing to say.
            return Ok(());
        };
        let channel = channel();
        let source = policy::files::Files::for_this_platform();
        let volume = policy::resolve(&source)
            .map(|e| e.notify)
            .unwrap_or_default();
        let outside = Outside::new(&source, &Host, channel.as_ref()).saying(volume);
        let _ = resident::sweep(root, &[]);
        let mut instance = Resident::new(root);
        let standing = standing();
        resident::run(listener, &mut instance, &outside, standing.as_ref())?;
        instance.stand_down(&outside);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (root, door);
        Err("no standing list on this platform".into())
    }
}

/// Join the console of whoever started this, where there is one.
///
/// The windows subsystem means no console is made for this process, which is
/// what a double-click should get. An invocation from a terminal wants the
/// opposite, and `ATTACH_PARENT_PROCESS` is the difference between the two
/// without either having to be declared: a shell has a console to join and
/// Explorer does not.
///
/// Handles this process was given are left alone. A redirect — `slipcase-open
/// sessions > list.txt` — arrives as inherited handles, and joining a console
/// over the top of them would send the output somewhere the caller did not ask
/// for.
#[cfg(windows)]
#[allow(unsafe_code)]
fn attach_console() {
    use windows::Win32::System::Console::{
        AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE,
    };

    // SAFETY: both are plain queries against this process's own handle table.
    unsafe {
        if GetStdHandle(STD_OUTPUT_HANDLE).is_ok_and(|h| !h.is_invalid()) {
            return;
        }
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// How this machine will be spoken to, and what refused a richer channel.
///
/// Asked by opening the channel and closing it again, which is the only honest
/// way to answer: the platform's own refusal is the answer, and anything else
/// would be this code guessing at what it would find.
fn how_it_speaks() -> (String, Option<String>) {
    #[cfg(target_os = "linux")]
    {
        match present::freedesktop::Desktop::connect() {
            Ok(_) => ("desktop notifications".to_owned(), None),
            Err(why) => ("the terminal".to_owned(), Some(why.to_string())),
        }
    }
    #[cfg(windows)]
    {
        match present::toast::Toast::connect() {
            Ok(_) => ("toast notifications".to_owned(), None),
            Err(why) => ("the terminal".to_owned(), Some(why.to_string())),
        }
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        ("the terminal".to_owned(), None)
    }
}

/// What reading this layer right now turns out to say.
///
/// A read rather than a look at the filesystem: a file that is there and holds
/// no key is a layer that exists and has no opinion, and telling somebody it is
/// *present* without telling them it sets nothing is how the shipped
/// `/etc/slipcase/open.toml` gets mistaken for a policy nobody wrote.
fn layer_state(
    source: &policy::files::Files,
    origin: policy::Origin,
    effective: Option<&policy::Effective>,
) -> String {
    // Asked before the file is read, because a suppressed layer is not
    // consulted at all and saying what is in it would describe a file that
    // played no part in anything.
    if origin == policy::Origin::Configuration
        && effective.is_some_and(|e| e.configuration_suppressed)
    {
        return "not consulted; policy has suppressed it".to_string();
    }
    match policy::Source::layer(source, origin) {
        Ok(None) => "not there".to_string(),
        Ok(Some(layer)) if layer.says_nothing() => "there, and sets nothing".to_string(),
        Ok(Some(_)) => "in force".to_string(),
        // No detail, because the resolution below refuses with it and names the
        // same file. Two accounts of one typo is noise.
        Err(_) => "cannot be read".to_string(),
    }
}

/// A label and a list, wrapped under itself rather than off the side.
///
/// The permitted set is thirty entries before anybody configures anything,
/// which is one line nobody reads and three that anybody can.
fn labelled(label: &str, items: &[&str]) {
    const INDENT: usize = 18;
    const WIDTH: usize = 78;

    if items.is_empty() {
        println!("  {label:<14}  nothing");
        return;
    }
    let mut line = String::new();
    let mut first = true;
    for item in items {
        let piece = if line.is_empty() {
            (*item).to_string()
        } else {
            format!(", {item}")
        };
        if !line.is_empty() && INDENT + line.len() + piece.len() > WIDTH {
            println!("{}{line},", head(label, first));
            first = false;
            line = (*item).to_string();
        } else {
            line.push_str(&piece);
        }
    }
    println!("{}{line}", head(label, first));
}

/// The label on the first line of a wrapped list, and the space it occupied on
/// every line after it.
fn head(label: &str, first: bool) -> String {
    if first {
        format!("  {label:<14}  ")
    } else {
        " ".repeat(18)
    }
}

/// The name `sessions` prints and the other verbs take back.
fn id_of(s: &session::Session) -> String {
    s.dir()
        .file_name()
        .map_or_else(|| "?".to_string(), |n| n.to_string_lossy().into_owned())
}
