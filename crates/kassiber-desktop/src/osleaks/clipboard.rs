//! Every copy Kassiber makes (addresses, txids, `txid:vout`, payment hashes, amounts, labels,
//! CSV, diagnostics) goes through [`copy`]. A copy is financial data, so each one is kept on
//! this device and out of clipboard histories, and with "Clear clipboard after copy" on it is
//! cleared after [`CLEAR_DELAY`] if the clipboard still holds it (checked by change count).
//!
//! - macOS: Kassiber writes the pasteboard itself (Rune's secret copy sets only the concealed
//!   type): `prepareForNewContentsWithOptions:` with `NSPasteboardContentsCurrentHostOnly`, so
//!   Universal Clipboard / Handoff never sends it to another device, then the text with
//!   `org.nspasteboard.ConcealedType` and `org.nspasteboard.TransientType`, so clipboard managers
//!   that follow nspasteboard.org skip it. A worker thread clears it after the delay when the
//!   change count is still the one the write produced.
//! - Windows: Rune's secret copy, which writes `ExcludeClipboardContentFromMonitorProcessing`
//!   (arboard `exclude_from_monitoring`). Microsoft documents that format as keeping every form
//!   of the item out of clipboard history and cloud clipboard, which covers what
//!   `CanIncludeInClipboardHistory=0` and `CanUploadToCloudClipboard=0` would add.
//! - Linux: Rune's secret copy, which offers `x-kde-passwordManagerHint=secret`. Klipper and the
//!   history tools that read the hint skip it; others may not, and Linux has no cloud clipboard.
//!
//! On Windows and Linux the exclusion only exists on Rune's secret path, which always arms a
//! clear: with clearing off it holds for [`KEEP`] (and Rune clears a held copy when Kassiber
//! quits or, on Windows, when the session locks).
//!
//! Tooling runs (tests, headless renders, scenes, scripts, the click-through) never touch the OS
//! clipboard: they record the plan (never the text) in [`recorded`].

use std::cell::RefCell;
use std::time::Duration;

use rune::Tx;

use crate::app::App;

/// `CLIPBOARD_CLEAR_DELAY_MS` (30 s), the delay the web and the Privacy setting's text name.
pub const CLEAR_DELAY: Duration = crate::settings::effects::CLIPBOARD_CLEAR_DELAY;
/// How long Windows and Linux keep a copy when clearing is off (see the module docs).
pub const KEEP: Duration = Duration::from_secs(24 * 60 * 60);

/// The pasteboard types a macOS copy carries: the text, then the two nspasteboard.org markers.
pub const MAC_TYPES: [&str; 3] = [
    "public.utf8-plain-text",
    "org.nspasteboard.ConcealedType",
    "org.nspasteboard.TransientType",
];
/// The clipboard format Rune's secret copy adds on Windows.
pub const WINDOWS_MARKS: [&str; 1] = ["ExcludeClipboardContentFromMonitorProcessing"];
/// The MIME type Rune's secret copy adds on Linux (X11 and Wayland data-control).
pub const LINUX_MARKS: [&str; 1] = ["x-kde-passwordManagerHint"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Windows,
    Linux,
    Other,
}

impl Platform {
    pub const fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::MacOs
        } else if cfg!(target_os = "windows") {
            Platform::Windows
        } else if cfg!(target_os = "linux") {
            Platform::Linux
        } else {
            Platform::Other
        }
    }
}

/// Who writes the clipboard and what the write is marked with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Writer {
    /// Kassiber's own pasteboard write (macOS).
    Pasteboard {
        types: &'static [&'static str],
        current_host_only: bool,
    },
    /// `Tx::copy_secret` (arboard's exclusion formats).
    RuneSecret { marks: &'static [&'static str] },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub writer: Writer,
    /// Clear after this long if the clipboard still holds the copy.
    pub clear_after: Option<Duration>,
}

/// The write for `platform` with "Clear clipboard after copy" on or off.
pub fn plan(platform: Platform, auto_clear: bool) -> Plan {
    let rune = |marks: &'static [&'static str]| Plan {
        writer: Writer::RuneSecret { marks },
        clear_after: Some(if auto_clear { CLEAR_DELAY } else { KEEP }),
    };
    match platform {
        Platform::MacOs => Plan {
            writer: Writer::Pasteboard {
                types: &MAC_TYPES,
                current_host_only: true,
            },
            clear_after: auto_clear.then_some(CLEAR_DELAY),
        },
        Platform::Windows => rune(&WINDOWS_MARKS),
        Platform::Linux => rune(&LINUX_MARKS),
        Platform::Other => rune(&[]),
    }
}

thread_local! {
    static RECORDED: RefCell<Vec<Plan>> = const { RefCell::new(Vec::new()) };
}

/// The copies a tooling run asked for, in order (the plans only: copied text is never kept).
#[allow(dead_code)]
pub fn recorded() -> Vec<Plan> {
    RECORDED.with(|r| r.borrow().clone())
}

/// Copies `value` the protected way (see the module docs).
pub fn copy(app: &App, tx: &mut Tx, value: impl Into<String>) {
    let value = zeroize::Zeroizing::new(value.into());
    let plan = plan(Platform::current(), app.settings.prefs.clear_clipboard);
    if crate::persist::file::tooling_run() {
        RECORDED.with(|r| r.borrow_mut().push(plan));
        return;
    }
    match plan.writer {
        Writer::Pasteboard { .. } => native::send(value, plan.clear_after),
        Writer::RuneSecret { .. } => {
            let _ = tx.copy_secret(value.to_string(), plan.clear_after.unwrap_or(KEEP));
        }
    }
}

/// Clears the copy Kassiber made last, if the clipboard still holds it (the session locked, or
/// the app quits). Copies with clearing off stay on macOS, as the user chose.
pub fn clear_held(tx: Option<&mut Tx>) {
    if crate::persist::file::tooling_run() {
        return;
    }
    native::clear_held();
    if let Some(tx) = tx
        && !cfg!(target_os = "macos")
    {
        tx.clear_secret_clipboard();
    }
}

/// A copy the macOS worker will clear: the change count its write produced, and when.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Held {
    pub count: isize,
    pub due: std::time::Instant,
}

/// What the worker does with a held copy at `now` when the pasteboard's change count is
/// `current`: (still held, clear now). A changed count means the user copied something else,
/// which is never cleared.
pub fn expire(held: Option<Held>, now: std::time::Instant, current: isize) -> (Option<Held>, bool) {
    match held {
        None => (None, false),
        Some(h) if h.count != current => (None, false),
        Some(h) if now >= h.due => (None, true),
        Some(h) => (Some(h), false),
    }
}

#[cfg(target_os = "macos")]
mod native {
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use objc2::rc::autoreleasepool;
    use objc2::runtime::{AnyObject, Bool};
    use objc2::{class, msg_send};
    use objc2_foundation::NSString;
    use zeroize::Zeroizing;

    use super::{Held, MAC_TYPES, expire};

    /// `NSPasteboardContentsCurrentHostOnly`.
    const CURRENT_HOST_ONLY: usize = 1 << 0;

    struct Write {
        text: Zeroizing<String>,
        clear_after: Option<Duration>,
    }

    /// The copy waiting to be cleared (shared with `clear_held` on the event thread).
    static HELD: Mutex<Option<Held>> = Mutex::new(None);

    fn held() -> std::sync::MutexGuard<'static, Option<Held>> {
        HELD.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn worker() -> &'static mpsc::Sender<Write> {
        static WORKER: OnceLock<mpsc::Sender<Write>> = OnceLock::new();
        WORKER.get_or_init(|| {
            let (send, recv) = mpsc::channel::<Write>();
            let _ = std::thread::Builder::new()
                .name("kassiber-clipboard".into())
                .spawn(move || run(recv));
            send
        })
    }

    fn run(recv: mpsc::Receiver<Write>) {
        loop {
            let due = held().map(|h| h.due);
            let next = match due {
                Some(at) => recv.recv_timeout(at.saturating_duration_since(Instant::now())),
                None => recv.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match next {
                Ok(w) => {
                    let count = unsafe { write_board(general(), &w.text, &MAC_TYPES, true) };
                    *held() = match (count, w.clear_after) {
                        (Ok(count), Some(after)) => Some(Held {
                            count,
                            due: Instant::now() + after,
                        }),
                        _ => None,
                    };
                    // `w.text` is wiped as it drops here.
                }
                Err(RecvTimeoutError::Timeout) => tick(),
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    fn tick() {
        let mut h = held();
        let pb = general();
        let (keep, clear) = expire(*h, Instant::now(), unsafe { change_count(pb) });
        *h = keep;
        if clear {
            unsafe { clear_contents(pb) };
        }
    }

    pub fn send(text: Zeroizing<String>, clear_after: Option<Duration>) {
        let _ = worker().send(Write { text, clear_after });
    }

    /// Clears the held copy now if it is still on the pasteboard.
    pub fn clear_held() {
        let mut h = held();
        let Some(copy) = h.take() else { return };
        let pb = general();
        if unsafe { change_count(pb) } == copy.count {
            unsafe { clear_contents(pb) };
        }
    }

    fn general() -> *mut AnyObject {
        unsafe { msg_send![class!(NSPasteboard), generalPasteboard] }
    }

    unsafe fn change_count(pb: *mut AnyObject) -> isize {
        unsafe { msg_send![pb, changeCount] }
    }

    unsafe fn clear_contents(pb: *mut AnyObject) {
        let _: isize = unsafe { msg_send![pb, clearContents] };
    }

    /// Replaces `pb`'s contents: `types[0]` carries `text`, the other types are empty markers.
    /// Returns the change count the write left.
    pub(super) unsafe fn write_board(
        pb: *mut AnyObject,
        text: &str,
        types: &[&str],
        current_host_only: bool,
    ) -> Result<isize, ()> {
        autoreleasepool(|_| unsafe {
            let options: usize = if current_host_only {
                CURRENT_HOST_ONLY
            } else {
                0
            };
            let _: isize = msg_send![pb, prepareForNewContentsWithOptions: options];
            for (i, ty) in types.iter().enumerate() {
                let value = NSString::from_str(if i == 0 { text } else { "" });
                let ty = NSString::from_str(ty);
                let ok: Bool = msg_send![pb, setString: &*value, forType: &*ty];
                if !ok.as_bool() {
                    return Err(());
                }
            }
            Ok(change_count(pb))
        })
    }

    #[cfg(test)]
    mod tests {
        use objc2::rc::Retained;
        use objc2_foundation::NSArray;

        use super::*;

        /// The write lands on a private, uniquely named pasteboard (never the general one): the
        /// text and both markers are there, and the change count it reports is the board's.
        #[test]
        fn private_pasteboard_gets_text_and_markers() {
            autoreleasepool(|_| unsafe {
                let pb: Retained<AnyObject> =
                    msg_send![class!(NSPasteboard), pasteboardWithUniqueName];
                let raw = Retained::as_ptr(&pb) as *mut AnyObject;
                let count = write_board(raw, "bc1qtest", &MAC_TYPES, true).expect("write");
                assert_eq!(count, change_count(raw));
                let types: Retained<NSArray<NSString>> = msg_send![&*pb, types];
                let names: Vec<String> = types.iter().map(|t| t.to_string()).collect();
                for ty in MAC_TYPES {
                    assert!(names.iter().any(|n| n == ty), "{ty} missing from {names:?}");
                }
                let text: Option<Retained<NSString>> =
                    msg_send![&*pb, stringForType: &*NSString::from_str(MAC_TYPES[0])];
                assert_eq!(text.map(|t| t.to_string()).as_deref(), Some("bc1qtest"));
                clear_contents(raw);
                assert_ne!(change_count(raw), count);
                let _: () = msg_send![&*pb, releaseGlobally];
            });
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    pub fn send(_: zeroize::Zeroizing<String>, _: Option<std::time::Duration>) {}
    pub fn clear_held() {}
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[test]
    fn each_platform_marks_copies_for_its_clipboard_tools() {
        let mac = plan(Platform::MacOs, true);
        assert_eq!(
            mac.writer,
            Writer::Pasteboard {
                types: &MAC_TYPES,
                current_host_only: true
            }
        );
        assert_eq!(mac.clear_after, Some(CLEAR_DELAY));
        assert!(
            MAC_TYPES.contains(&"org.nspasteboard.ConcealedType")
                && MAC_TYPES.contains(&"org.nspasteboard.TransientType")
        );
        // Clearing off: still concealed and kept on this Mac, never cleared.
        assert_eq!(
            plan(Platform::MacOs, false),
            Plan {
                writer: mac.writer,
                clear_after: None
            }
        );
        for (p, marks) in [
            (Platform::Windows, &WINDOWS_MARKS[..]),
            (Platform::Linux, &LINUX_MARKS[..]),
        ] {
            assert_eq!(
                plan(p, true),
                Plan {
                    writer: Writer::RuneSecret { marks },
                    clear_after: Some(CLEAR_DELAY)
                }
            );
            assert_eq!(plan(p, false).clear_after, Some(KEEP));
        }
        assert_eq!(CLEAR_DELAY, Duration::from_secs(30));
    }

    #[test]
    fn a_held_copy_clears_only_while_it_is_still_on_the_clipboard() {
        let t0 = Instant::now();
        let h = Held {
            count: 7,
            due: t0 + CLEAR_DELAY,
        };
        assert_eq!(expire(Some(h), t0, 7), (Some(h), false));
        assert_eq!(expire(Some(h), t0 + CLEAR_DELAY, 7), (None, true));
        // The user copied something else meanwhile: theirs stays.
        assert_eq!(expire(Some(h), t0 + CLEAR_DELAY, 8), (None, false));
        assert_eq!(expire(Some(h), t0, 8), (None, false));
        assert_eq!(expire(None, t0, 7), (None, false));
    }

    #[test]
    fn tooling_runs_record_the_plan_and_never_the_text() {
        let app = crate::app::App::new();
        let mut tx = Tx::default();
        let before = recorded().len();
        copy(&app, &mut tx, "bc1qsecretaddress");
        let all = recorded();
        assert_eq!(all.len(), before + 1);
        assert_eq!(
            all[before],
            plan(Platform::current(), app.settings.prefs.clear_clipboard)
        );
        assert!(!format!("{all:?}").contains("bc1q"));
    }
}
