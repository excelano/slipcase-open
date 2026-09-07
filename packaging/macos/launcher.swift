// The bundle's executable: receive the document Finder sends, hand it to
// `slipcase-open` beside this file, and leave.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
// macOS does not deliver a double-clicked document as an argument. It launches
// the bundle with no arguments and sends an Apple Event, and something has to
// be listening or Finder reports that the application cannot open files of
// that kind. `slipcase-desktop` listens from inside its window loop, in the one
// module of that crate that writes `unsafe`. This tool has no window loop and
// no reason to take on AppKit bindings for one event, so the listening is done
// here, in the platform's own language, and the Rust crate keeps
// `forbid(unsafe_code)` on this platform.
//
// **This process is a moment, not the instance.** It runs `slipcase-open open
// PATH`, which becomes the resident instance where none is running and hands
// the container over where one is, and then it terminates. That is deliberate
// and not a shortcut: Launch Services sends a second double-click to the
// running application if there is one, and the Rust process cannot receive it.
// With this process gone, every double-click launches a fresh copy of it, and
// the front door in `endpoint.rs` does what it does on every platform.
//
// The child is not killed when this process ends. It is started with
// `posix_spawn` semantics and simply loses its parent, which is the same shape
// as the first `open` typed at a shell that has since closed.

import AppKit

final class Delegate: NSObject, NSApplicationDelegate {
    private var handed = 0

    /// The Rust binary, installed beside this executable by `build-app.sh`.
    private var tool: URL {
        Bundle.main.executableURL!
            .deletingLastPathComponent()
            .appendingPathComponent("slipcase-open")
    }

    func application(_ app: NSApplication, open urls: [URL]) {
        for url in urls where url.isFileURL {
            hand(url.path)
        }
        app.terminate(nil)
    }

    /// Launched with nothing to open: the bundle clicked in Applications, or
    /// Spotlight. There is no standing list on this platform for that to raise
    /// — concept 9 keeps the command line as the floor here — so say where the
    /// floor is and go. A document launch delivers `open` before this fires
    /// and has already terminated by now, which is what `handed` records.
    func applicationDidFinishLaunching(_ note: Notification) {
        if handed > 0 {
            return
        }
        let alert = NSAlert()
        alert.messageText = "Slipcase Open works from a double-click."
        alert.informativeText = "Open a .slpc container and its payload opens in its own application. Edits are written back when you save. From a terminal: slipcase-open sessions"
        alert.addButton(withTitle: "OK")
        NSApp.activate(ignoringOtherApps: true)
        alert.runModal()
        NSApp.terminate(nil)
    }

    private func hand(_ path: String) {
        let child = Process()
        child.executableURL = tool
        child.arguments = ["open", path]
        // The instance's own narration goes to its channel, and on this
        // platform the channel is the error stream (concept 9's floor). Nobody
        // is reading this process's streams, and leaving them attached would
        // hold a pipe open to a launcher that has gone.
        child.standardInput = FileHandle.nullDevice
        child.standardOutput = FileHandle.nullDevice
        child.standardError = FileHandle.nullDevice
        do {
            try child.run()
            handed += 1
        } catch {
            let alert = NSAlert()
            alert.messageText = "Slipcase Open could not start."
            alert.informativeText = "\(tool.path): \(error.localizedDescription)"
            alert.alertStyle = .critical
            alert.runModal()
        }
    }
}

let app = NSApplication.shared
let delegate = Delegate()
app.delegate = delegate
app.run()
