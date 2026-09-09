//! The slipcase-open engine: everything the tool does that is not a user
//! interface.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Concept 8 keeps this free of any dependency on how the tool presents
//! itself, so that the session model is one body of code on three platforms
//! rather than three programs sharing a name, and so that every
//! security-relevant decision — validation, policy, the launch path, the
//! write-back — lives in one place and is testable without a desktop.
//!
//! `src/main.rs` is the command line over it, which concept 9 keeps as the
//! floor beneath the notifications and the tray.

// The level `Cargo.toml` explains: `forbid` everywhere Windows is not.
#![cfg_attr(not(windows), forbid(unsafe_code))]

/// What this tool says, in the language the desktop asks for.
///
/// Declared in the engine and not in `main`, because the engine is what
/// narrates: `resident` composes every report and every question and hands
/// them to a `Channel` that only carries them. That is concept 9's boundary
/// holding for language as well — a second presenter would show these
/// sentences without knowing they had been translated.
///
/// **Nothing is translated until `main` says so.** `activate` is called there
/// and nowhere else, so the suite — which this repository runs five times over
/// real directories — sees the English its assertions are written in, whatever
/// the machine is set to.
pub mod i18n {
    pub use potext::fill;

    potext::catalog!();
}

pub mod content;
pub mod endpoint;
pub mod extension;
pub mod extract;
pub mod flow;
pub mod identity;
pub mod ipc;
pub mod outside;
pub mod platform;
pub mod policy;
pub mod present;
pub mod recover;
pub mod resident;
pub mod session;
pub mod table;
pub mod watch;
pub mod writeback;
