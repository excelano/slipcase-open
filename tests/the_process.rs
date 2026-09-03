//! The tests that run the program rather than the engine.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! There is one thing only a process can be asked: whether it stays. Concept 8
//! makes the instance live while an open session, a lingering one, or an
//! unanswered question exists, and `main` is where that becomes *return now* or
//! *serve the front door*. Every unit test in the crate can watch `is_idle`
//! answer correctly and none of them can watch `main` act on it.
//!
//! There is a second: what the paths resolve to. `policy` reports the files
//! this machine reads rather than the ones the documentation names, and every
//! one of them comes out of the environment a process was started with — so a
//! test of it is a test of a process or it is a test of nothing.
//!
//! **Nothing here touches the machine it runs on.** The state directory, the
//! front door and the configuration are all pointed at a temporary tree, and
//! the session bus is pointed at nothing — so concept 9's channel falls back to
//! the terminal and the suite does not put notifications on somebody's screen.

// The level `Cargo.toml` explains: `forbid` everywhere Windows is not.
#![cfg_attr(not(windows), forbid(unsafe_code))]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use slipcase_open::{extract, session};

/// Long enough that a process which was going to return has, and short enough
/// to be worth waiting for. The two behaviours this separates are milliseconds
/// and five minutes, so anything in between would do.
const LONG_ENOUGH: Duration = Duration::from_secs(3);

/// A temporary world: state, runtime, and configuration, none of them shared
/// with the machine.
struct Alone {
    dir: tempfile::TempDir,
}

impl Alone {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["state", "run", "config"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        Self { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn sessions(&self) -> PathBuf {
        self.path().join("state/slipcase-open/sessions")
    }

    /// The program, aimed at this world and at no bus.
    fn run(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_slipcase-open"));
        command
            .args(args)
            .env("HOME", self.path())
            .env("XDG_STATE_HOME", self.path().join("state"))
            .env("XDG_RUNTIME_DIR", self.path().join("run"))
            .env("XDG_CONFIG_HOME", self.path().join("config"))
            // Windows reads none of the three above. `session::default_root`
            // takes `%LOCALAPPDATA%`, so without this the program under test
            // works in the real one: the world would not be alone, and the
            // suite would leave sessions in the developer's own state
            // directory. Found by running this suite on Windows for the first
            // time, where `sessions` answered from a directory no test wrote.
            .env("LOCALAPPDATA", self.path().join("state"))
            // No session bus, so `Desktop::connect` fails and the terminal
            // takes over. A test that reached the developer's own notification
            // service would put its fixtures on their screen.
            .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent/no-bus")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    fn container(&self, name: &str, payload: &[u8]) -> PathBuf {
        let doc: slpc::toml_edit::DocumentMut =
            format!("slipcase_version = \"1.0\"\n\n[payload]\nfile = \"{name}\"\n")
                .parse()
                .unwrap();
        let path = self.path().join(format!("{name}.slpc"));
        slpc::pack_reader(name, payload, doc, std::fs::File::create(&path).unwrap()).unwrap();
        path
    }

    /// What a process that died mid-session leaves behind: a session on disk,
    /// an edit that never reached the container, and nobody holding it.
    ///
    /// Its container has not moved, so concept 6.3 as amended puts the edit
    /// back rather than asking about it.
    fn a_crashed_session(&self, container: &Path, name: &str, edit: &[u8]) {
        let mut left = session::create(&self.sessions(), container, name).unwrap();
        extract::extract(&mut slpc::Container::open(container).unwrap(), &mut left).unwrap();
        std::fs::write(left.payload_path(), edit).unwrap();
    }

    /// The same, and then somebody else repacks the container behind its back,
    /// which is the only shape that still raises concept 6.3's question.
    fn a_diverged_session(&self, container: &Path, name: &str, edit: &[u8]) {
        self.a_crashed_session(container, name, edit);
        self.container(name, b"what somebody else put there in the meantime");
    }
}

/// Whether it has finished within `within`.
fn finished(child: &mut Child, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

#[test]
fn a_refusal_that_raised_a_question_stays_to_be_answered() {
    // Concept 8: an open on a container with a session left behind refuses and
    // asks what to do about the leftover. The refusal is not the end of the
    // invocation, because the question's buttons have nothing to reach if the
    // process goes — and it would go without withdrawing them.
    let world = Alone::new();
    let c = world.container("report.txt", b"first");
    world.a_diverged_session(&c, "report.txt", b"an edit that never landed");

    let mut child = world.run(&["open", c.to_str().unwrap()]).spawn().unwrap();
    let left_early = finished(&mut child, LONG_ENOUGH);
    let _ = child.kill();
    let done = child.wait_with_output().unwrap();

    assert!(
        !left_early,
        "it returned on the refusal, leaving the question with nobody behind it: {}",
        String::from_utf8_lossy(&done.stderr)
    );
    let said = String::from_utf8_lossy(&done.stderr);
    assert!(said.contains("left behind"), "{said}");
    // And the question itself was put, which on this channel is the two
    // commands that answer it.
    assert!(said.contains("--write-back"), "{said}");
}

#[test]
fn a_refusal_that_raised_nothing_returns() {
    // The other half, and the reason the test above is not satisfied by a
    // program that never exits. A payload the built-in set does not permit is
    // refused with nothing held, so there is no front door worth keeping.
    let world = Alone::new();
    let c = world.container("inner.zip", b"not a document");

    let mut child = world.run(&["open", c.to_str().unwrap()]).spawn().unwrap();
    assert!(
        finished(&mut child, Duration::from_secs(30)),
        "a refusal holding nothing should return at once"
    );
    let done = child.wait_with_output().unwrap();
    assert!(!done.status.success());
    let said = String::from_utf8_lossy(&done.stderr);
    assert!(said.contains("not in the allowed set"), "{said}");
    assert!(
        session::scan(&world.sessions()).map_or(true, |s| s.is_empty()),
        "a refused open should leave no session"
    );
}

#[test]
fn asking_what_is_open_with_nobody_running_reads_the_state_directory() {
    let world = Alone::new();
    let done = world.run(&["sessions"]).output().unwrap();
    assert!(done.status.success());
    assert!(String::from_utf8_lossy(&done.stdout).contains("No sessions."));

    let c = world.container("report.txt", b"first");
    world.a_crashed_session(&c, "report.txt", b"edited");
    let done = world.run(&["sessions"]).output().unwrap();
    let said = String::from_utf8_lossy(&done.stdout);
    assert!(said.contains("report.txt"), "{said}");
    assert!(said.contains("--write-back"), "{said}");
}

#[test]
fn an_edit_left_by_a_crash_goes_back_when_the_container_is_opened() {
    // Concept 6.3 as amended, through the whole program rather than the engine:
    // the session below is what a crash leaves, and opening the container it
    // belongs to used to stop and ask. It now puts the edit back and carries
    // on, and the container on disk is the assertion.
    let world = Alone::new();
    let c = world.container("report.txt", b"first");
    world.a_crashed_session(&c, "report.txt", b"the edit that never landed");

    let mut child = world.run(&["open", c.to_str().unwrap()]).spawn().unwrap();
    let returned = finished(&mut child, LONG_ENOUGH);
    let _ = child.kill();
    let done = child.wait_with_output().unwrap();
    let said = String::from_utf8_lossy(&done.stderr);

    assert!(
        !said.contains("left behind"),
        "the person was asked about their own save: {said}"
    );
    // It stays, because it opened something: concept 8 keeps the instance for
    // the session it just started, and there is no terminal-driven exit here.
    assert!(!returned || said.contains("is open"), "{said}");

    let mut held = slpc::Container::open(&c).unwrap();
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut held.payload().unwrap(), &mut bytes).unwrap();
    assert_eq!(
        bytes, b"the edit that never landed",
        "the edit did not reach the container"
    );
}

// Unix, because every path it names is one: `/etc/slipcase/open.toml`, the XDG
// configuration file, and the socket under `$XDG_RUNTIME_DIR`. Concept 10 gives
// Windows a registry subtree instead of the first two and concept 8 gives it a
// named pipe instead of the third, so the counterpart is a different test rather
// than this one widened. It arrives with the registry source in PLAN.md Phase 4;
// until then `Files::for_this_platform` reads nothing there and this would be
// asserting Linux's answers against Windows.
#[cfg(unix)]
#[test]
fn the_settings_verb_names_the_files_this_world_would_read() {
    // What the verb is for. The paths come out of `XDG_CONFIG_HOME` and
    // `XDG_STATE_HOME`, so the answer is a property of the environment and not
    // of the build, and this world's spellings are what should come back.
    //
    // Nothing is asserted about the resolved lists. `/etc/slipcase/open.toml`
    // is the one layer no environment variable moves — deliberately, since a
    // machine policy that could be redirected is not one — so a machine with a
    // real policy applied would fail an assertion about what is permitted, and
    // that assertion would be measuring the machine rather than the code.
    let world = Alone::new();
    let done = world.run(&["policy"]).output().unwrap();
    let said = String::from_utf8_lossy(&done.stdout);
    assert!(done.status.success(), "{said}");

    assert!(said.contains("/etc/slipcase/open.toml"), "{said}");
    assert!(
        said.contains(
            world
                .path()
                .join("config/slipcase-open/policy.toml")
                .to_str()
                .unwrap()
        ),
        "{said}"
    );
    assert!(said.contains(world.sessions().to_str().unwrap()), "{said}");
    assert!(
        said.contains(
            world
                .path()
                .join("run/slipcase-open/front-door")
                .to_str()
                .unwrap()
        ),
        "{said}"
    );
    // The user's own file is not there, and saying so is the point: somebody
    // asking where their settings live is usually asking where to put them.
    assert!(said.contains("not there"), "{said}");
}

// Unix, for the reason above: the file it breaks on is the XDG one, and Windows
// has no file layer to break until Phase 4 gives it one.
#[cfg(unix)]
#[test]
fn a_settings_file_that_will_not_parse_is_named_before_it_is_refused() {
    // The reason the locations are printed before anything is resolved. A
    // person runs this verb *because* something is wrong, and a resolution that
    // refuses to guess past a broken layer would otherwise print nothing at all
    // — leaving them with a complaint about a file and no list of the files
    // there are.
    let world = Alone::new();
    let config = world.path().join("config/slipcase-open");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("policy.toml"), "denied = [\"exe\"\n").unwrap();

    let done = world.run(&["policy"]).output().unwrap();
    let printed = String::from_utf8_lossy(&done.stdout);
    let complained = String::from_utf8_lossy(&done.stderr);

    assert!(!done.status.success(), "{printed}{complained}");
    assert!(
        printed.contains(config.join("policy.toml").to_str().unwrap()),
        "the broken file was not named in the listing: {printed}"
    );
    assert!(printed.contains("cannot be read"), "{printed}");
    // And the refusal says what is wrong with it, which the listing does not.
    assert!(complained.contains("unclosed array"), "{complained}");
}

#[test]
fn a_lone_path_is_an_open() {
    // Concept 4's double-click. On Windows a packaged handler is launched with
    // the container path and nothing else, and a manifest has nowhere to put a
    // verb in front of it the way the desktop entry does, so the verb has to be
    // implied. What is asserted is the implication rather than the platform,
    // which is why this runs everywhere.
    //
    // The payload is one the built-in set refuses, so both invocations stop
    // before the launcher and no application is handed anything. Comparing the
    // two against each other rather than against a fixed string is the point:
    // it says they are the same command, whatever that command says.
    let world = Alone::new();
    let c = world.container("inner.zip", b"not a document");

    let bare = world.run(&[c.to_str().unwrap()]).output().unwrap();
    let spelled = world.run(&["open", c.to_str().unwrap()]).output().unwrap();

    let said = String::from_utf8_lossy(&bare.stderr).to_string();
    assert!(!bare.status.success(), "{said}");
    assert!(said.contains("not in the allowed set"), "{said}");
    assert_eq!(bare.status.code(), spelled.status.code());
    assert_eq!(said, String::from_utf8_lossy(&spelled.stderr));
}
