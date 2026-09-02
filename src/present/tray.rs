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
    WM_APP, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_RBUTTONUP, WNDCLASSW,
};

use super::{Chosen, Standing};

/// The message the icon posts to the window.
const CALLBACK: u32 = WM_APP + 1;
/// The menu item that ends the instance. Above any session's index.
const QUIT: usize = 0xF000;

/// What the pump thread and the loop share.
///
/// Two mutexes rather than a channel each way, because the shapes differ: the
/// lines are a value that is replaced, where the choices are events that queue.
struct Shared {
    lines: Mutex<Vec<String>>,
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
    /// Whether this icon is itself a reason for the instance to stay.
    holds: bool,
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
    pub fn show_up(holds: bool) -> windows::core::Result<Self> {
        let shared = std::sync::Arc::new(Shared {
            lines: Mutex::default(),
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
            holds,
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
    fn show(&self, sessions: &[String]) {
        if let Ok(mut lines) = self.shared.lines.lock() {
            if *lines == sessions {
                return;
            }
            sessions.clone_into(&mut lines);
        }
        // The tooltip is the part visible without a click, so it carries the
        // count rather than the list.
        if let Ok(window) = self.window.lock() {
            if let Some(hwnd) = *window {
                retip(hwnd, sessions.len());
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
        // Concept 8's fourth reason to stay, and the amendment this arm needs:
        // somebody who asked for the standing list is owed it until they say
        // otherwise, so an icon does not appear and vanish.
        //
        // Not always, and the exception is the one concept 9 already draws.
        // Where a terminal started this, the terminal *is* the standing list —
        // it is the floor beneath the tray, and `sessions` answers the same
        // question — so the instance ends when the sessions do, the way it
        // always has. An `open` typed at a prompt that never returned would be
        // a worse command than the one it replaced.
        self.holds
    }
}

/// Update the tooltip to say how many sessions there are.
#[allow(unsafe_code)]
fn retip(hwnd: isize, count: usize) {
    let mut data = icon_data(HWND(hwnd as *mut _), None);
    let text = match count {
        0 => "Slipcase Open - nothing open".to_owned(),
        1 => "Slipcase Open - 1 session".to_owned(),
        n => format!("Slipcase Open - {n} sessions"),
    };
    for (i, c) in text.encode_utf16().enumerate().take(127) {
        data.szTip[i] = c;
    }
    data.uFlags = NIF_TIP;
    // SAFETY: `data` names a window and id this module added an icon for, and
    // is filled in for the duration of the call.
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

/// This product's icon, or the one Windows gives anything that has none.
///
/// Loaded from beside the executable rather than compiled in as a resource,
/// because a resource needs `rc.exe` and that is a build step this project does
/// not have. The package carries the file; a run from a checkout does not, and
/// falls back rather than refusing to put up an icon at all.
#[allow(unsafe_code)]
fn icon() -> HICON {
    if let Ok(exe) = std::env::current_exe() {
        let beside = exe.with_file_name("slipcase-open.ico");
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

        let mut data = icon_data(hwnd, Some(icon()));
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
                if u32::try_from(l.0).unwrap_or(0) == WM_RBUTTONUP {
                    offer(hwnd);
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                if w.0 & 0xFFFF == QUIT {
                    STATE.with(|s| {
                        if let Some(shared) = s.borrow().as_ref() {
                            if let Ok(mut chosen) = shared.chosen.lock() {
                                chosen.push(Chosen::Quit);
                            }
                        }
                    });
                }
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
        let lines = STATE.with(|s| {
            s.borrow()
                .as_ref()
                .and_then(|shared| shared.lines.lock().ok().map(|l| l.clone()))
                .unwrap_or_default()
        });
        if lines.is_empty() {
            let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 1, w!("Nothing open"));
        } else {
            // Greyed, because they are what the list says rather than things to
            // press. Acting on one from here wants a verb the engine does not
            // have yet, and a button that does nothing is worse than no button.
            for line in &lines {
                let text: Vec<u16> = line
                    .trim()
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 1, PCWSTR(text.as_ptr()));
            }
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            QUIT,
            w!("Quit, keeping sessions recoverable"),
        );

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
