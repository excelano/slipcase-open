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

pub mod content;
pub mod extension;
pub mod extract;
pub mod flow;
pub mod platform;
pub mod policy;
pub mod recover;
pub mod session;
pub mod watch;
pub mod writeback;
