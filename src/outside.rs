//! The three things the engine works against that are not its own state.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! Where policy is read from, what opens a payload, and how the person is
//! spoken to. They travel together because they are chosen together — once, by
//! `main`, from what the machine turns out to be — and because the engine
//! passes all three down the same path: concept 10 puts the policy decision
//! immediately before the launch, and concept 5.1's warning is said about the
//! same payload in the same breath.
//!
//! Held as trait objects rather than as type parameters. The alternative
//! threads three generics through every signature from the front door down to
//! the session, so that a program which picks its implementations at startup can
//! pretend it knew them at compile time.

use crate::platform::Launcher;
use crate::policy;
use crate::present::Channel;

/// What the engine has been given to work with.
#[derive(Clone, Copy)]
pub struct Outside<'a> {
    /// Concept 10's layers, in whatever form this platform keeps them.
    pub policy: &'a dyn policy::Source,
    /// Concept 5 step 7, which hands the payload to the desktop.
    pub launcher: &'a dyn Launcher,
    /// Concept 9, which narrates and asks.
    pub channel: &'a dyn Channel,
}

impl<'a> Outside<'a> {
    /// Gather the three.
    #[must_use]
    pub fn new(
        policy: &'a dyn policy::Source,
        launcher: &'a dyn Launcher,
        channel: &'a dyn Channel,
    ) -> Self {
        Self {
            policy,
            launcher,
            channel,
        }
    }
}
