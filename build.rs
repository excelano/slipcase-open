//! Embed the Windows application manifest, and do nothing else ever.
//!
//! This is the crate's only build script, and it exists for one line of XML:
//! the DPI declaration in `packaging/windows/slipcase-open.manifest`, which the
//! certification kit asked for on 2026-09-06 and which this product needs for
//! its own sake — a message box and a tray icon are the whole of what a person
//! sees here, and an unaware process has both bitmap-stretched.
//!
//! **Nothing is compiled that was not compiled before.** The usual way to embed
//! a manifest is `rc.exe` or `windres`, and this project has no resource
//! compiler and wants none; the tray's icons are loaded from files beside the
//! binary for the same reason. This prints two linker arguments and the linker
//! that was already linking the binary does the embedding.
//!
//! **A second use for this file is a decision, not a precedent.** The one
//! opened here is narrow on purpose: a build should need a Rust toolchain and
//! nothing else.
//!
//! Author: David M. Anderson
//! Built with AI assistance (Claude, Anthropic)

#![forbid(unsafe_code)]

use std::path::Path;

fn main() {
    // Beside the rest of this platform's files rather than at the crate root,
    // which is the same rule everything else under `packaging/` follows.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("packaging")
        .join("windows")
        .join("slipcase-open.manifest");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", manifest.display());

    // **Read from the environment and not from `cfg!`.** A build script is
    // compiled for the host, so `cfg!(windows)` in here answers about the
    // machine doing the building rather than the machine being built for — and
    // this repository does cross-check the Windows target from Linux, which
    // would take the wrong branch.
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os != "windows" || env != "msvc" {
        return;
    }

    // `/MANIFEST:EMBED` is MSVC's, which is why the guard above tests the
    // environment as well as the operating system: a `windows-gnu` target links
    // with something that would not understand it.
    //
    // Named rather than blanket, so that this applies to the binary that has a
    // dialog to draw and would silently follow along to any other the package
    // grew later.
    for arg in [
        "/MANIFEST:EMBED",
        &format!("/MANIFESTINPUT:{}", manifest.display()),
    ] {
        println!("cargo:rustc-link-arg-bin=slipcase-open={arg}");
    }
}
