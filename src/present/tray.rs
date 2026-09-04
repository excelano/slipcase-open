//! Concept 12's standing session list on Windows: an icon by the clock.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
//
//! **A thread of its own, because a tray icon is a window and a window needs a
//! pump.** `Shell_NotifyIcon` reports what the person did by posting a message
//! to a window, so something has to call `GetMessage` in a loop forever.
//! `resident::run` cannot: it is pumping the watchers, which concept 8 makes
//! the reason the process exists, and a blocking message loop there would stop
//! them. So the window, the icon and the pump live on one thread and talk to
//! the loop through two mutexes — the lines to show, and what was chosen.
//!
//! **An icon outlives a process that did not get to remove it**, and nothing
//! here can change that. `Shell_NotifyIcon` has no way to say *this window is
//! gone*; the shell finds out when somebody's pointer passes over the icon and
//! the message bounces. So a crash, or a kill, leaves a ghost until then —
//! which is the same shape as concept 8's crash story for sessions, where what
//! is left behind is recoverable rather than tidy. [`Tray::drop`] covers every
//! ending this process is present for, and is the whole of what it can cover.
//!
//! **A real window that is never shown, rather than a message-only one.** A
//! message-only window does not receive broadcast messages, and `TaskbarCreated`
//! — how the shell tells everybody to put their icon back after Explorer
//! restarts — is a broadcast. An icon that vanishes for good the first time
//! Explorer falls over is the failure this avoids.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW,
    GetCursorPos, GetMessageW, LoadIconW, LoadImageW, PostMessageW, PostQuitMessage,
    RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, TrackPopupMenu, TranslateMessage,
    CW_USEDEFAULT, HICON, IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTSIZE, LR_LOADFROMFILE, MF_GRAYED,
    MF_SEPARATOR, MF_STRING, MSG, TPM_BOTTOMALIGN, TPM_RIGHTALIGN, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_APP, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW,
};

use super::{Chosen, Listed, Mood, Standing, Trouble};

/// The message the icon posts to the window.
const CALLBACK: u32 = WM_APP + 1;
/// The loop telling the window that what it is showing has changed.
///
/// A message rather than the loop's thread calling `Shell_NotifyIconW` itself,
/// so that every icon this module creates is created, used and freed on the one
/// thread — which is what makes the cache in [`icon`] a plain thread-local
/// instead of a lock around a handle table.
const REFRESH: u32 = WM_APP + 2;
/// The menu item that ends the instance. Above any trouble's index.
const QUIT: usize = 0xF000;

/// What the icon is showing at the moment.
#[derive(Default)]
struct Shown {
    lines: Vec<Listed>,
    troubles: Vec<Trouble>,
    mood: Mood,
}

/// What the pump thread and the loop share.
///
/// Two mutexes rather than a channel each way, because the shapes differ: what
/// is shown is a value that is replaced, where the choices are events that
/// queue.
struct Shared {
    shown: Mutex<Shown>,
    chosen: Mutex<Vec<Chosen>>,
}

thread_local! {
    /// The pump thread's handle on the shared state, reachable from a window
    /// procedure, which is a C callback and can be handed nothing else.
    static STATE: std::cell::RefCell<Option<std::sync::Arc<Shared>>> =
        const { std::cell::RefCell::new(None) };
}

/// An icon by the clock, for as long as this lives.
pub struct Tray {
    shared: std::sync::Arc<Shared>,
    /// Told to close when the tray is dropped, so the icon goes with it.
    window: Mutex<Option<isize>>,
    ready: Receiver<()>,
}

impl Tray {
    /// Put an icon in the tray.
    ///
    /// # Errors
    ///
    /// Where the window or the icon cannot be made, which on a session with no
    /// shell is ordinary rather than exceptional.
    pub fn show_up() -> windows::core::Result<Self> {
        let shared = std::sync::Arc::new(Shared {
            shown: Mutex::default(),
            chosen: Mutex::default(),
        });
        let (ready_tx, ready) = channel();
        let (window_tx, window_rx) = channel();
        let theirs = std::sync::Arc::clone(&shared);
        std::thread::Builder::new()
            .name("slipcase-open tray".to_owned())
            .spawn(move || pump(&theirs, &window_tx, &ready_tx))?;

        // The window handle comes back before anything else can use it, and a
        // thread that failed to make one closes the channel instead.
        let window = window_rx.recv().map_err(|_| {
            windows::core::Error::new(
                windows::Win32::Foundation::E_FAIL,
                "the tray thread could not make its window",
            )
        })?;
        Ok(Self {
            shared,
            window: Mutex::new(Some(window)),
            ready,
        })
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        // Ask the pump to end, which is what removes the icon. Dropping without
        // this leaves the icon behind until somebody hovers over it, which is
        // the shell noticing the window has gone rather than being told.
        if let Ok(mut window) = self.window.lock() {
            if let Some(hwnd) = window.take() {
                #[allow(unsafe_code)]
                // SAFETY: posting to a window this type made and has not yet
                // told to close. A handle that is already gone answers false,
                // which is the case where the thread ended first.
                unsafe {
                    let _ =
                        PostMessageW(Some(HWND(hwnd as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
        }
        // Wait briefly for the icon to actually go, so that a run which ends
        // does not leave one behind for the shell to reap later.
        let _ = self
            .ready
            .recv_timeout(std::time::Duration::from_millis(500));
    }
}

impl Standing for Tray {
    fn show(&self, sessions: &[Listed], troubles: &[Trouble], mood: Mood) {
        if let Ok(mut shown) = self.shared.shown.lock() {
            sessions.clone_into(&mut shown.lines);
            troubles.clone_into(&mut shown.troubles);
            shown.mood = mood;
        }
        // The window redraws itself from that, on its own thread. The loop's
        // only job here is to say that there is something new to read.
        if let Ok(window) = self.window.lock() {
            if let Some(hwnd) = *window {
                #[allow(unsafe_code)]
                // SAFETY: posting to a window this type made and has not told
                // to close. A handle that has gone answers false and is ignored,
                // which is the case where the pump ended first.
                unsafe {
                    let _ = PostMessageW(Some(HWND(hwnd as *mut _)), REFRESH, WPARAM(0), LPARAM(0));
                }
            }
        }
    }

    fn taken(&self) -> Vec<Chosen> {
        self.shared
            .chosen
            .lock()
            .map_or_else(|_| Vec::new(), |mut c| std::mem::take(&mut *c))
    }

    fn holding(&self) -> bool {
        // Always, and see `Standing::holding` for why this is a change to
        // concept 8 rather than an exception to it. An icon that vanished when
        // the last document was closed would take every warning with it, and a
        // warning is exactly the thing that arrives after the work is done.
        //
        // There is no terminal case to exclude here. `main::standing` never
        // builds a tray where one exists, because there the command line is
        // already the standing list.
        true
    }
}

/// Re-read what is being shown, and put it on the icon.
///
/// **One call for the colour and the words**, because they are one statement.
/// The colour is the whole of the interface for somebody who never hovers, and
/// the tooltip is the first thing that explains it, so a moment where they
/// disagreed would be the interface contradicting itself.
#[allow(unsafe_code)]
fn redress(hwnd: HWND, shown: &Shown) {
    let mut data = icon_data(hwnd, Some(icon(shown.mood)));
    // What the trouble is, if there is one, because that is what somebody
    // hovering over a coloured icon is asking. Otherwise what the tool is
    // quietly doing, which concept 6.2 has no other place to say: the report
    // that would have said it is `Weight::Routine`, and the default volume
    // drops those for good reason — it is one per save.
    let text = match (shown.troubles.first(), shown.lines.len()) {
        (Some(first), _) => format!("Slipcase Open - {}", first.summary),
        (None, 0) => "Slipcase Open - nothing open".to_owned(),
        (None, 1) => "Slipcase Open - 1 payload open; saves go back into its container".to_owned(),
        (None, n) => {
            format!("Slipcase Open - {n} payloads open; saves go back into their containers")
        }
    };
    for (i, c) in text.encode_utf16().enumerate().take(127) {
        data.szTip[i] = c;
    }
    data.uFlags = NIF_ICON | NIF_TIP;
    // SAFETY: `data` names a window and id this module added an icon for, and
    // is filled in for the duration of the call. The icon it carries is owned
    // by the cache and outlives the call.
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &raw const data);
    }
}

/// The structure both the add and the modify are described by.
fn icon_data(hwnd: HWND, icon: Option<HICON>) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: u32::try_from(std::mem::size_of::<NOTIFYICONDATAW>()).unwrap_or(0),
        hWnd: hwnd,
        uID: 1,
        uCallbackMessage: CALLBACK,
        hIcon: icon.unwrap_or_default(),
        ..Default::default()
    }
}

/// Which drawing each mood wears.
///
/// **The same drawing every time, tinted.** A person recognises the shape and
/// then reads the colour; two drawings would make them read the shape instead,
/// which is slower and says less. `packaging/windows/make-ico.ps1` renders the
/// set from the one piece of artwork and the files are checked in.
const fn artwork(mood: Mood) -> &'static str {
    match mood {
        Mood::Settled => "slipcase-open.ico",
        Mood::Working => "slipcase-open-working.ico",
        Mood::Look => "slipcase-open-yellow.ico",
        Mood::AtRisk => "slipcase-open-orange.ico",
        Mood::Danger => "slipcase-open-red.ico",
    }
}

thread_local! {
    /// The icons this thread has loaded, kept for the life of the process.
    ///
    /// A cache because the alternative is a `LoadImageW` per mood change and a
    /// `DestroyIcon` to match, and a mood changes on every save. Five handles
    /// held until exit is the cheaper and the simpler of the two, and the pump
    /// thread is the only one that touches them.
    static ICONS: std::cell::RefCell<std::collections::HashMap<&'static str, HICON>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// This product's icon in the state asked for, or the one Windows gives
/// anything that has none.
///
/// Loaded from beside the executable rather than compiled in as a resource,
/// because a resource needs `rc.exe` and that is a build step this project does
/// not have. The package carries the files; a run from a checkout does not, and
/// falls back rather than refusing to put up an icon at all — so a developer
/// build shows one colour for every mood instead of none.
#[allow(unsafe_code)]
fn icon(mood: Mood) -> HICON {
    let name = artwork(mood);
    ICONS.with(|cache| {
        if let Some(had) = cache.borrow().get(name) {
            return *had;
        }
        let loaded = load(name);
        cache.borrow_mut().insert(name, loaded);
        loaded
    })
}

/// One `.ico` from beside the executable.
#[allow(unsafe_code)]
fn load(name: &str) -> HICON {
    if let Ok(exe) = std::env::current_exe() {
        let beside = exe.with_file_name(name);
        if beside.exists() {
            use std::os::windows::ffi::OsStrExt as _;
            let wide: Vec<u16> = beside
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            // SAFETY: `wide` is null-terminated and outlives the call, and the
            // handle is owned by this process for its lifetime.
            let loaded = unsafe {
                LoadImageW(
                    None,
                    PCWSTR(wide.as_ptr()),
                    IMAGE_ICON,
                    0,
                    0,
                    LR_LOADFROMFILE | LR_DEFAULTSIZE,
                )
            };
            if let Ok(handle) = loaded {
                return HICON(handle.0);
            }
        }
    }
    // SAFETY: a stock icon, which needs no module and is never freed.
    unsafe { LoadIconW(None, IDI_APPLICATION) }.unwrap_or_default()
}

/// The window, the icon, and the loop that serves them.
#[allow(unsafe_code)]
fn pump(shared: &std::sync::Arc<Shared>, window: &Sender<isize>, ready: &Sender<()>) {
    STATE.with(|s| *s.borrow_mut() = Some(std::sync::Arc::clone(shared)));

    // SAFETY: the documented sequence for a window with a class of its own.
    // Every handle is this thread's and is released before it returns.
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return;
        };
        let class = w!("slipcase-open-tray");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        // A second instance in one process would register the same class
        // twice, which fails; the window is what matters and it is made either
        // way.
        let _ = RegisterClassW(&raw const wc);
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("slipcase-open"),
            WINDOW_STYLE(0),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        ) else {
            return;
        };

        let mut data = icon_data(hwnd, Some(icon(Mood::Settled)));
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        for (i, c) in "Slipcase Open".encode_utf16().enumerate().take(127) {
            data.szTip[i] = c;
        }
        if !Shell_NotifyIconW(NIM_ADD, &raw const data).as_bool() {
            return;
        }
        // Only once there is an icon: a handle sent before this would let the
        // loop believe it had a tray it does not have.
        if window.send(hwnd.0 as isize).is_err() {
            let _ = Shell_NotifyIconW(NIM_DELETE, &raw const data);
            return;
        }

        // The shell's way of saying it has restarted and lost every icon.
        let restarted = RegisterWindowMessageW(w!("TaskbarCreated"));

        let mut msg = MSG::default();
        while GetMessageW(&raw mut msg, None, 0, 0).as_bool() {
            if msg.message == restarted && restarted != 0 {
                let _ = Shell_NotifyIconW(NIM_ADD, &raw const data);
            }
            let _ = TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
        let _ = Shell_NotifyIconW(NIM_DELETE, &raw const data);
    }
    let _ = ready.send(());
}

/// What the window does with what the shell tells it.
#[allow(unsafe_code)]
unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: every call below is on the thread that made this window, with
    // handles that thread owns.
    unsafe {
        match msg {
            CALLBACK => {
                // Either button opens the menu. A left click on a tray icon
                // conventionally opens the thing, and this product has no
                // window to open — so the menu is the whole of what there is
                // to show, and hiding it behind the right button would leave a
                // left click doing nothing at all.
                let what = u32::try_from(l.0).unwrap_or(0);
                if what == WM_RBUTTONUP || what == WM_LBUTTONUP {
                    offer(hwnd);
                }
                LRESULT(0)
            }
            REFRESH => {
                STATE.with(|s| {
                    let Some(shared) = s.borrow().as_ref().map(std::sync::Arc::clone) else {
                        return;
                    };
                    // Bound rather than locked in the `if let` itself: a
                    // scrutinee temporary in a block's last statement outlives
                    // the block, and `shared` is what it borrows from.
                    let shown = shared.shown.lock();
                    if let Ok(shown) = shown {
                        redress(hwnd, &shown);
                    }
                });
                LRESULT(0)
            }
            WM_COMMAND => {
                let picked = w.0 & 0xFFFF;
                STATE.with(|s| {
                    let Some(shared) = s.borrow().as_ref().map(std::sync::Arc::clone) else {
                        return;
                    };
                    let what = if picked == QUIT {
                        Some(Chosen::Quit)
                    } else {
                        // Menu ids are one-based positions in the troubles the
                        // menu was built from, which is what they were when it
                        // opened. One that cleared itself while the menu was up
                        // is a position that is no longer there, and nothing is
                        // the right answer for it.
                        shared
                            .shown
                            .lock()
                            .ok()
                            .and_then(|shown| shown.troubles.get(picked.wrapping_sub(1)).cloned())
                            .map(|trouble| Chosen::Dismiss(trouble.id))
                    };
                    if let Some(what) = what {
                        if let Ok(mut chosen) = shared.chosen.lock() {
                            chosen.push(what);
                        }
                    }
                });
                LRESULT(0)
            }
            WM_CLOSE | WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

/// The menu: what is open, and the way out.
#[allow(unsafe_code)]
unsafe fn offer(hwnd: HWND) {
    // SAFETY: a menu made, shown and destroyed inside this call, on the thread
    // that owns the window it is shown for.
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        let shown = STATE.with(|s| {
            s.borrow()
                .as_ref()
                .and_then(|shared| {
                    shared.shown.lock().ok().map(|shown| Shown {
                        lines: shown.lines.clone(),
                        troubles: shown.troubles.clone(),
                        mood: shown.mood,
                    })
                })
                .unwrap_or_default()
        });

        // **What is wrong comes first, and it is the only part that does
        // anything.** The icon has already said that something is; this says
        // what, about a file the person recognises, and clicking it is them
        // saying they have read it. That is the whole of what this menu asks of
        // anybody — and it is the answer to the thing that made the first
        // version of it worthless, which was three greyed lines offering
        // nothing.
        if !shown.troubles.is_empty() {
            let _ = AppendMenuW(
                menu,
                MF_STRING | MF_GRAYED,
                0,
                w!("Needs a look - click one to clear it"),
            );
            for (at, trouble) in shown.troubles.iter().enumerate() {
                let text: Vec<u16> = trouble
                    .summary
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                let _ = AppendMenuW(menu, MF_STRING, at + 1, PCWSTR(text.as_ptr()));
            }
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        }

        // Concept 6.3 sweeps what needs nobody and says nothing about it.
        // Putting it in a standing surface is furniture with a name on it.
        let lines: Vec<&Listed> = shown
            .lines
            .iter()
            .filter(|l| l.live || l.needs_a_person)
            .collect();
        if lines.is_empty() {
            if shown.troubles.is_empty() {
                let _ = AppendMenuW(
                    menu,
                    MF_STRING | MF_GRAYED,
                    0,
                    w!("Nothing open - double-click a .slpc container"),
                );
            }
        } else {
            for listed in lines {
                // **In files, not in sessions.** A person is asking whether
                // their work is safe, and `6a986220-0  report.txt  open, 0
                // write-back(s)` answers a different question in a different
                // vocabulary — an identifier they never chose, a state name
                // from the engine, and a count of an operation they have never
                // heard of. The command line keeps all of that, because there
                // the id is what the next verb takes. Here it is noise.
                //
                // Statements, and deliberately so. There is nothing to do to a
                // document that is open and saving: it is open, in the
                // application it belongs in, and the person is already working
                // in it. A menu item here would be a step invented to have
                // something to offer.
                let label = match (listed.needs_a_person, listed.write_backs) {
                    (true, _) => format!("{}  -  left behind, needs a decision", listed.payload),
                    (false, Some(0u64)) => {
                        format!("{}  -  open, nothing saved yet", listed.payload)
                    }
                    (false, Some(1u64)) => format!("{}  -  saved once", listed.payload),
                    (false, Some(n)) => format!("{}  -  saved {n} times", listed.payload),
                    (false, None) => listed.payload.clone(),
                };
                let text: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
                let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, PCWSTR(text.as_ptr()));
            }
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        // **Just "Quit".** The longer label said *keeping sessions
        // recoverable*, which is the engine's vocabulary and reassurance about
        // a thing the person was never worried about — they have not been told
        // a session exists and should not have to learn. It also read as a
        // warning, which is the opposite of what it describes: nothing is lost
        // either way, and a menu item that explains itself is one somebody has
        // to stop and read.
        let _ = AppendMenuW(menu, MF_STRING, QUIT, w!("Quit"));

        let mut at = POINT::default();
        let _ = GetCursorPos(&raw mut at);
        // Documented, and the menu misbehaves without it: an unfocused window's
        // popup does not close when the person clicks away from it.
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(
            menu,
            TPM_RIGHTALIGN | TPM_BOTTOMALIGN,
            at.x,
            at.y,
            None,
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu);
    }
}
