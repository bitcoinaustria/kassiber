//! components/kb/WalletMaterialScannerDialog.tsx: the "Scan <source>" dialog the wallet-export
//! form opens from its Scan button (Auto / Single QR / BBQR modes, a status badge, the square
//! camera preview with its framing overlay and status chip, a camera picker when there are
//! several, BBQR progress, the scanned note, the error box, Cancel / Use scanned text).
//!
//! The camera (`camera`) starts when the dialog opens, which is the user's press on Scan, as
//! the web's does; each decoded QR text goes through the web's own rules (`bbqr::process`):
//! a plain QR is the material in Auto / Single, BBQR frames collect until the set decodes. A
//! scan stops the camera on its last frame, dimmed. Headless runs and platforms without
//! capture show the web's "no camera" state ("Camera not found.").

use rune::{Color, Cx, El, Key, KeyEvent, Shadow, Tx, col, el, row, text};

use super::bbqr::{self, Collector, Progress, ScanMode, ScanResult};
use super::camera::{self, Camera, Device, Event, Inbox};
use crate::app::App;
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant, button, icon_button};
use crate::ui::ic;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Auto,
    Single,
    Bbqr,
}

impl Mode {
    const ALL: [Mode; 3] = [Mode::Auto, Mode::Single, Mode::Bbqr];
    fn key(self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::Single => "single",
            Mode::Bbqr => "bbqr",
        }
    }
    /// `scanModeLabel`.
    fn label(self) -> String {
        t(&format!("connections:scanner.mode.{}", self.key()))
    }
    fn scan(self) -> ScanMode {
        match self {
            Mode::Auto => ScanMode::Auto,
            Mode::Single => ScanMode::Single,
            Mode::Bbqr => ScanMode::Bbqr,
        }
    }
}

/// The status badge, kept as what it says so a language switch relabels it.
#[derive(Clone, Debug, Default, PartialEq)]
enum Status {
    #[default]
    Idle,
    Starting,
    /// `scanningStatusFor(mode)`.
    Looking(Mode),
    Scanned,
    Frames(u32, u32),
    Complete(u32, u32),
    /// A message from the scan rules (untranslated in the web too).
    Note(&'static str),
    Failed,
    Unavailable,
}

impl Status {
    fn text(&self) -> String {
        let n = |v: &u32| v.to_string();
        match self {
            Status::Idle => t("connections:scanner.status.cameraIdle"),
            Status::Starting => t("connections:scanner.status.startingCamera"),
            Status::Looking(Mode::Auto) => t("connections:scanner.status.lookingWallet"),
            Status::Looking(Mode::Single) => t("connections:scanner.status.lookingSingle"),
            Status::Looking(Mode::Bbqr) => t("connections:scanner.status.lookingBbqr"),
            Status::Scanned => t("connections:scanner.status.walletScanned"),
            Status::Frames(r, total) => tr(
                "connections:scanner.status.bbqrFrames",
                &[("received", &n(r)), ("total", &n(total))],
            ),
            Status::Complete(r, total) => tr(
                "connections:scanner.status.bbqrComplete",
                &[("received", &n(r)), ("total", &n(total))],
            ),
            Status::Note(m) => (*m).to_owned(),
            Status::Failed => t("connections:scanner.status.scanFailed"),
            Status::Unavailable => t("connections:scanner.status.cameraUnavailable"),
        }
    }
}

#[derive(Default)]
pub struct State {
    pub open: bool,
    pub title: String,
    pub mode: Mode,
    status: Status,
    /// The camera start error, or a scan's.
    pub error: Option<String>,
    starting: bool,
    /// Material decoded from a QR (or a complete BBQR set).
    scanned: Option<String>,
    progress: Option<Progress>,
    collector: Collector,
    /// `lastScannedRef`: the same text twice in a row is one scan.
    last: Option<String>,
    devices: Vec<Device>,
    device: String,
    picker_open: bool,
    surface: Option<rune::NativeSurface>,
    camera: Option<Camera>,
    inbox: Inbox,
}

/// Opens the dialog and starts the camera (the user's press on Scan).
pub fn open(app: &mut App, title: String) {
    app.wallets.add.scanner = State {
        open: true,
        title,
        status: Status::Starting,
        ..State::default()
    };
    start_camera(app);
}

fn start_camera(app: &mut App) {
    let s = &mut app.wallets.add.scanner;
    s.starting = true;
    s.error = None;
    let surface = rune::NativeSurface::new();
    if let Some(w) = crate::native::waker() {
        surface.set_waker(w);
    }
    s.camera = None;
    let device = Some(s.device.clone()).filter(|d| !d.is_empty());
    s.camera = Some(camera::start(device, surface.clone(), s.inbox.clone()));
    s.surface = Some(surface);
    // Headless and capture-less platforms answer at once.
    drain(app);
}

pub fn is_open(app: &App) -> bool {
    app.wallets.add.scanner.open
}

fn close(app: &mut App) {
    // Dropping the camera stops the capture.
    app.wallets.add.scanner = State::default();
}

/// `useScannedMaterial`: hands the material to the form and closes.
fn use_scanned(app: &mut App) {
    let Some(m) = app.wallets.add.scanner.scanned.take() else {
        return;
    };
    close(app);
    crate::wallets::add::on_material_scanned(app, m);
}

/// A mode button: the collection starts over and the status says what it looks for (a scan
/// already in stays, as the web's mode effect leaves `scannedMaterial` alone).
fn set_mode(app: &mut App, mode: Mode) {
    let s = &mut app.wallets.add.scanner;
    s.mode = mode;
    s.collector = Collector::default();
    s.progress = None;
    s.last = None;
    s.status = Status::Looking(mode);
}

/// The camera picker: the scan starts over on the chosen camera.
fn set_device(app: &mut App, id: &str) {
    let s = &mut app.wallets.add.scanner;
    s.picker_open = false;
    if s.device == id {
        return;
    }
    s.device = id.to_owned();
    s.scanned = None;
    start_camera(app);
}

/// Every frame while the dialog is open: what the camera said.
pub fn poll(app: &mut App) -> bool {
    if !is_open(app) {
        return false;
    }
    drain(app)
}

fn drain(app: &mut App) -> bool {
    let events = app.wallets.add.scanner.inbox.drain();
    let changed = !events.is_empty();
    for e in events {
        apply(app, e);
    }
    changed
}

fn apply(app: &mut App, e: Event) {
    let s = &mut app.wallets.add.scanner;
    match e {
        Event::Started(devices) => {
            s.starting = false;
            if s.device.is_empty() {
                s.device = devices.first().map(|d| d.id.clone()).unwrap_or_default();
            }
            s.devices = devices;
            if s.status == Status::Starting {
                s.status = Status::Looking(s.mode);
            }
        }
        Event::Failed(message) => {
            s.starting = false;
            s.error = Some(if message == "Could not start the camera." {
                t("connections:scanner.couldNotStart")
            } else {
                message
            });
            s.status = Status::Unavailable;
        }
        Event::Decoded(raw) => on_scan(app, &raw),
    }
}

/// qr-scanner's result callback through `processWalletMaterialQrScan`.
fn on_scan(app: &mut App, raw: &str) {
    let s = &mut app.wallets.add.scanner;
    if s.scanned.is_some() {
        return;
    }
    let text = raw.trim();
    if text.is_empty() || s.last.as_deref() == Some(text) {
        return;
    }
    s.last = Some(text.to_owned());
    match bbqr::process(text, s.mode.scan(), &s.collector) {
        ScanResult::Single(material) => {
            s.camera = None;
            s.scanned = Some(material);
            s.progress = None;
            s.status = Status::Scanned;
            s.error = None;
            s.collector = Collector::default();
        }
        ScanResult::Progress { state, progress } => {
            s.status = Status::Frames(progress.received, progress.total);
            s.progress = Some(progress);
            s.error = None;
            s.collector = state;
        }
        ScanResult::Complete {
            state,
            progress,
            material,
        } => {
            s.camera = None;
            s.scanned = Some(material);
            s.status = Status::Complete(progress.received, progress.total);
            s.progress = Some(progress);
            s.error = None;
            s.collector = state;
        }
        ScanResult::Ignored(message) => s.status = Status::Note(message),
        ScanResult::Error(message) => {
            s.error = Some(message);
            s.status = Status::Failed;
        }
    }
}

/// Headless scenes: feed decoded QR texts as if the camera had read them.
pub fn scene_feed(app: &mut App, texts: &[&str]) {
    let s = &mut app.wallets.add.scanner;
    s.starting = false;
    s.error = None;
    s.status = Status::Looking(s.mode);
    for text in texts {
        on_scan(app, text);
    }
}

/// Escape closes the camera picker, then the scanner, before anything below it.
pub fn on_key(app: &mut App, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    if is_open(app) && matches!(ev.key, Key::Escape) {
        if app.wallets.add.scanner.picker_open {
            app.wallets.add.scanner.picker_open = false;
        } else {
            close(app);
        }
        return true;
    }
    false
}

/// `scannerPreviewLabel`.
fn preview_label(s: &State) -> String {
    if s.scanned.is_some() {
        t("connections:scanner.preview.ready")
    } else if s.error.is_some() {
        t("connections:scanner.preview.cameraBlocked")
    } else if s.starting {
        t("connections:scanner.preview.startingCamera")
    } else if let Some(p) = &s.progress {
        tr(
            "connections:scanner.preview.bbqrProgress",
            &[
                ("received", &p.received.to_string()),
                ("total", &p.total.to_string()),
            ],
        )
    } else {
        t(match s.mode {
            Mode::Bbqr => "connections:scanner.preview.collectingBbqr",
            Mode::Single => "connections:scanner.preview.scanningQr",
            Mode::Auto => "connections:scanner.preview.scanningWallet",
        })
    }
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) {
    let _ = cx;
    let s = &app.wallets.add.scanner;
    if !s.open {
        return;
    }
    let title = if s.title.is_empty() {
        t("connections:scanner.title")
    } else {
        s.title.clone()
    };
    let width = 640.0f32.min(
        if app.shell.window_w > 0.0 {
            app.shell.window_w
        } else {
            1440.0
        } - tk.r(2.0),
    );
    let inner_w = width - 2.0 * tk.r(1.25) - 2.0;

    // DialogHeader.
    let header = col()
        .w_full()
        .gap(tk.r(0.5))
        .pr(tk.r(1.5))
        .child(
            text(title.clone())
                .font(tk.tlg(600.0).lh(tk.r(1.125)))
                .color(tk.fg)
                .role("heading"),
        )
        .child(
            text(t("connections:scanner.description"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .w_full(),
        );

    // Mode buttons (`size="sm"`, secondary when active) and the status badge (`ml-auto`).
    let mut modes = row().w_full().flex_wrap().gap(tk.r(0.5)).center_y();
    for m in Mode::ALL {
        let active = s.mode == m;
        let label = m.label();
        let b = if active {
            row()
                .key(format!("scanner-mode-{}", m.key()))
                .h(tk.r(2.0))
                .px(tk.r(0.75))
                .center()
                .radius(tk.r(0.375))
                .fill(tk.muted)
                .hover(tk.muted.alpha(0.8))
                .role("button")
                .label(label.clone())
                .checked(true)
                .focusable(true)
                .child(text(label).font(tk.tsm(500.0)).color(tk.fg))
        } else {
            button(
                tk,
                format!("scanner-mode-{}", m.key()),
                Variant::Outline,
                Size::Sm,
                None,
                &label,
                true,
            )
            .checked(false)
        };
        modes = modes.child(b.on_press(move |a: &mut App| set_mode(a, m)));
    }
    let status = s.status.text();
    modes = modes.child(el().grow(1.0)).child(
        row()
            .key("scanner-status")
            .px(tk.r(0.5))
            .py(tk.r(0.125))
            .radius(tk.r(0.75))
            .border(1.0, tk.border)
            .border_layout()
            .role("status")
            .child(text(status).font(tk.txs(500.0)).color(tk.fg)),
    );

    content_and_mount(app, tk, layer, width, inner_w, &title, header, modes, s);
}

#[allow(clippy::too_many_arguments)]
fn content_and_mount(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    width: f32,
    inner_w: f32,
    title: &str,
    header: El,
    modes: El,
    s: &State,
) {
    // The square preview (`aspect-square rounded-lg border bg-black`) with its overlays.
    let side = inner_w;
    let inset = side * 0.14;
    let frame = side - 2.0 * inset;
    let corner = tk.r(2.5);
    let rad = tk.r(0.5);
    let primary = tk.primary;
    let bracket = |x: f32, y: f32, sides: [f32; 4]| {
        el().absolute(x, y)
            .size(corner, corner)
            .radius(rad)
            .border(1.0, Color::TRANSPARENT)
            .border_sides(sides, primary)
    };
    let target = el()
        .absolute(inset, inset)
        .size(frame, frame)
        .radius(rad)
        .border(1.0, Color::WHITE.alpha(0.7))
        // `shadow-[0_0_0_999px_rgba(0,0,0,0.35)]`: dims everything outside the target.
        .box_shadow(Shadow {
            spread: 999.0,
            color: Color::BLACK.alpha(0.35),
            ..Shadow::default()
        })
        .child(bracket(0.0, 0.0, [4.0, 0.0, 0.0, 4.0]))
        .child(bracket(frame - corner, 0.0, [4.0, 4.0, 0.0, 0.0]))
        .child(bracket(0.0, frame - corner, [0.0, 0.0, 4.0, 4.0]))
        .child(bracket(
            frame - corner,
            frame - corner,
            [0.0, 4.0, 4.0, 0.0],
        ))
        .child(
            row()
                .absolute((frame - tk.r(2.5)) / 2.0, (frame - tk.r(2.5)) / 2.0)
                .size(tk.r(2.5), tk.r(2.5))
                .center()
                .child(ic(&I::SCAN_LINE, tk.r(2.5), Color::WHITE.alpha(0.8))),
        );
    let glyph = if s.starting {
        ic(&I::LOADER_CIRCLE, tk.r(0.875), Color::WHITE).looping(rune::LoopSpec::rotation(0.9))
    } else {
        ic(
            if s.error.is_some() {
                &I::VIDEO_OFF
            } else {
                &I::CAMERA
            },
            tk.r(0.875),
            Color::WHITE,
        )
    };
    let chip = row()
        .px(tk.r(0.5))
        .py(tk.r(0.25))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.375))
        .fill(Color::BLACK.alpha(0.6))
        .child(glyph.a11y_hidden())
        .child(
            text(preview_label(s))
                .font(tk.txs(400.0))
                .color(Color::WHITE),
        );
    // `absolute top-3 right-3 left-3 flex gap-2`: the chip, and the camera picker when there
    // are several cameras.
    let mut top_bar = row()
        .absolute(tk.r(0.75), tk.r(0.75))
        .w(side - tk.r(1.5))
        .gap(tk.r(0.5))
        .center_y()
        .child(chip);
    if s.devices.len() > 1 {
        top_bar = top_bar
            .child(el().grow(1.0))
            .child(camera_picker(tk, layer, s));
    }
    // `<video class="object-cover">`, dimmed once a scan is in.
    let mut video = el().absolute(0.0, 0.0).size(side, side).a11y_hidden();
    if let Some(surface) = &s.surface {
        video = video
            .native_surface(surface, rune::SurfaceFit::Fill)
            .opacity(if s.scanned.is_some() { 0.6 } else { 1.0 });
    }
    let preview = col()
        .key("scanner-preview")
        .size(side, side)
        .radius(rad)
        .fill(Color::BLACK)
        .border(1.0, tk.border)
        .clip_rounded(rad)
        .role("img")
        .label(preview_label(s))
        .child(video)
        .child(
            el().absolute(0.0, 0.0)
                .size(side, side)
                .fill(Color::BLACK.alpha(0.2))
                .a11y_hidden(),
        )
        .child(target.a11y_hidden())
        .child(top_bar);

    let mut body = col().w_full().gap(tk.r(1.0)).child(modes).child(preview);
    if let Some(p) = &s.progress {
        // `space-y-2 rounded-md border bg-background p-3 text-xs`: the type, n / total, the bar.
        let pct = if p.total > 0 {
            (p.received as f32 / p.total as f32).min(1.0)
        } else {
            0.0
        };
        body = body.child(
            col()
                .key("scanner-progress")
                .w_full()
                .padding(tk.r(0.75))
                .gap(tk.r(0.5))
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .border_layout()
                .fill(tk.background)
                .role("progressbar")
                .label(tr(
                    "connections:scanner.bbqrType",
                    &[("type", &p.file_type)],
                ))
                .child(
                    row()
                        .w_full()
                        .justify_between()
                        .gap(tk.r(0.75))
                        .child(
                            text(tr(
                                "connections:scanner.bbqrType",
                                &[("type", &p.file_type)],
                            ))
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg),
                        )
                        .child(
                            text(format!("{} / {}", p.received, p.total))
                                .font(tk.txs(500.0).features("'tnum' 1"))
                                .color(tk.fg),
                        ),
                )
                .child(
                    row()
                        .w_full()
                        .h(tk.r(0.375))
                        .radius(tk.r(0.1875))
                        .fill(tk.muted)
                        .clip_rounded(tk.r(0.1875))
                        .child(el().grow(pct.max(0.0001)).h_full().fill(tk.primary))
                        .child(el().grow((1.0 - pct).max(0.0001)).h_full()),
                ),
        );
    }
    if s.scanned.is_some() {
        let em = rune::color::hex("#10b981").expect("hex");
        let ink = rune::color::hex(if tk.dark { "#6ee7b7" } else { "#047857" }).expect("hex");
        body = body.child(
            col()
                .w_full()
                .padding(tk.r(0.75))
                .radius(tk.r(0.375))
                .border(1.0, em.alpha(0.3))
                .fill(em.alpha(0.1))
                .child(
                    text(t("connections:scanner.scannedNote"))
                        .font(tk.tsm(400.0))
                        .color(ink)
                        .wrap()
                        .w_full(),
                ),
        );
    }
    if let Some(e) = &s.error {
        body = body.child(
            col()
                .key("scanner-error")
                .w_full()
                .padding(tk.r(0.75))
                .radius(tk.r(0.375))
                .border(1.0, tk.destructive.alpha(0.3))
                .fill(tk.destructive.alpha(0.1))
                .role("alert")
                .child(
                    text(e.clone())
                        .font(tk.tsm(400.0))
                        .color(tk.destructive)
                        .wrap()
                        .w_full(),
                ),
        );
    }
    let ready = s.scanned.is_some();
    let mut use_btn = button(
        tk,
        "scanner-use",
        Variant::Default,
        Size::Default,
        None,
        &t("connections:scanner.useScannedText"),
        ready,
    );
    if ready {
        use_btn = use_btn.on_press(use_scanned);
    }
    let footer = row()
        .w_full()
        .justify_end()
        .gap(tk.r(0.5))
        .child(
            button(
                tk,
                "scanner-cancel",
                Variant::Outline,
                Size::Default,
                None,
                &t("common:actions.cancel"),
                true,
            )
            .on_press(close),
        )
        .child(use_btn);
    let content = col()
        .w_full()
        .gap(tk.r(1.0))
        .child(header)
        .child(body)
        .child(footer);
    mount(app, tk, layer, width, title, content);
}

/// The camera `<select>` (`h-8 rounded-md border-white/20 bg-black/70 text-xs text-white`), its
/// options the cameras' names (`Camera n` when a camera has none).
fn camera_picker(tk: &Tk, layer: &mut Vec<El>, s: &State) -> El {
    let label_of = |i: usize, d: &Device| {
        if d.label.is_empty() {
            tr(
                "connections:scanner.cameraFallback",
                &[("index", &(i + 1).to_string())],
            )
        } else {
            d.label.clone()
        }
    };
    let shown = s
        .devices
        .iter()
        .enumerate()
        .find(|(_, d)| d.id == s.device)
        .map(|(i, d)| label_of(i, d))
        .unwrap_or_default();
    let trigger = row()
        .key("scanner-camera")
        .h(tk.r(2.0))
        .min_w(0.0)
        .max_w(tk.r(14.0))
        .px(tk.r(0.5))
        .gap(tk.r(0.375))
        .center_y()
        .radius(tk.r(0.375))
        .border(1.0, Color::WHITE.alpha(0.2))
        .fill(Color::BLACK.alpha(0.7))
        .role("combobox")
        .label(shown.clone())
        .expanded(s.picker_open)
        .focusable(true)
        .on_press(|a: &mut App| {
            let s = &mut a.wallets.add.scanner;
            s.picker_open = !s.picker_open;
        })
        .child(
            text(shown)
                .font(tk.txs(400.0))
                .color(Color::WHITE)
                .truncate()
                .min_w(0.0),
        )
        .child(ic(&I::CHEVRON_DOWN, tk.r(0.875), Color::WHITE.alpha(0.7)).a11y_hidden());
    if s.picker_open {
        let opts = s
            .devices
            .iter()
            .enumerate()
            .map(|(i, d)| (d.id.clone(), label_of(i, d), true, None))
            .collect();
        super::btcpay::form::select_menu(
            tk,
            layer,
            "scanner-camera",
            260.0,
            opts,
            &s.device,
            |a: &mut App| a.wallets.add.scanner.picker_open = false,
            |a: &mut App, v: &str| set_device(a, v),
        );
    }
    trigger
}

/// `DialogContent` (ui/dialog.tsx): the glass backdrop and the `kb-glass-dialog` surface
/// (`sm:max-w-[640px]`, `rounded-[1rem]`, `p-5 gap-4`) at `top: max(1rem, 8vh)`, its X button,
/// scrolling inside past the window height.
fn mount(app: &App, tk: &Tk, layer: &mut Vec<El>, width: f32, label: &str, content: El) {
    let wash = if tk.dark {
        Color::BLACK.alpha(0.22)
    } else {
        tk.fg.alpha(0.06)
    };
    layer.push(
        crate::shell::window_backdrop("scanner-scrim", wash)
            .on_press(close)
            .a11y_hidden(),
    );
    let win_h = if app.shell.window_h > 0.0 {
        app.shell.window_h
    } else {
        900.0
    };
    let top = crate::shell::dialog_top(tk, win_h);
    let (border, shadows) = if tk.dark {
        (
            Color::WHITE.alpha(0.08),
            vec![
                Shadow {
                    y: 1.0,
                    color: Color::WHITE.alpha(0.04),
                    inset: true,
                    ..Default::default()
                },
                Shadow {
                    y: 24.0,
                    blur: 72.0,
                    spread: -20.0,
                    color: Color::BLACK.alpha(0.9),
                    ..Default::default()
                },
            ],
        )
    } else {
        (
            tk.fg.alpha(0.1),
            vec![Shadow {
                y: 24.0,
                blur: 64.0,
                spread: -24.0,
                color: Color::BLACK.alpha(0.65),
                ..Default::default()
            }],
        )
    };
    let close_btn = icon_button(
        tk,
        "scanner-close",
        Variant::Ghost,
        tk.r(1.75),
        &I::X,
        tk.r(1.0),
        t("common:actions.close"),
    )
    .pin(None, Some(tk.r(0.75)), Some(tk.r(0.75)), None)
    .on_press(close);
    let max_h = win_h - top - tk.r(1.0);
    let body = col()
        .key("scanner-scroll")
        .w_full()
        .max_h(max_h - 2.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(col().w_full().padding(tk.r(1.25)).child(content));
    let mut surface = col()
        .key("scanner-dialog")
        .role("dialog")
        .label(label.to_owned())
        .w(width)
        .radius(tk.r(1.0))
        .fill(tk.background.alpha(0.8))
        .backdrop_blur(if tk.dark { 16.0 } else { 12.0 })
        .transition(crate::shell::dialog_enter())
        .border(1.0, border)
        .border_layout()
        .clip_rounded(tk.r(1.0))
        .modal(true, close)
        .child(body)
        .child(close_btn);
    for sh in shadows {
        surface = surface.box_shadow(sh);
    }
    let win_w = if app.shell.window_w > 0.0 {
        app.shell.window_w
    } else {
        1440.0
    };
    layer.push(crate::shell::centered(surface, win_w, width, top));
}
