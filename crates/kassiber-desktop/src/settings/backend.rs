//! The add / edit sync-backend dialog (SyncBackendSettingsModal.tsx): connection type, preset
//! or custom endpoint, Electrum host/port, Silent Payments, privacy posture and ownership,
//! Core Lightning access, TLS and proxy options, LND TLS, authentication, the connection test
//! and its log, Delete / Cancel / Save.
//!
//! "Test connection" (and Save of an untested endpoint, which tests first) contacts the
//! endpoint: it runs only from those presses.

use rune::{Color, El, TextEdit, col, row, text};
use serde_json::{Value, json};

use super::controls::{
    Btn, PressIf, btn, btn_lg, disclosure, emerald, group, input, labelled, muted, opt, select,
    srow, switch,
};
use super::model::{self, Backend, Net, Owner, Posture, Proxy};
use super::{certtrust, send};
use crate::app::{App, Reply, ToastTone};
use crate::i18n::{t, tr};
use crate::theme::Tk;

pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub url: &'static str,
    pub protocol: &'static str,
    pub label: &'static str,
    pub provider: Option<&'static str>,
    pub public: bool,
}

pub struct Type {
    pub id: &'static str,
    pub label: &'static str,
    pub net_kind: Net,
    pub desc_key: &'static str,
    pub subtitle_key: &'static str,
    pub presets: &'static [Preset],
}

const fn p(
    id: &'static str,
    name: &'static str,
    url: &'static str,
    protocol: &'static str,
    label: &'static str,
    provider: Option<&'static str>,
    public: bool,
) -> Preset {
    Preset {
        id,
        name,
        url,
        protocol,
        label,
        provider,
        public,
    }
}

pub static TYPES: [Type; 4] = [
    Type {
        id: "bitcoin",
        label: "Bitcoin",
        net_kind: Net::Btc,
        desc_key: "settings:backendModal.network.bitcoinDesc",
        subtitle_key: "settings:backendModal.network.bitcoinSubtitle",
        presets: &[
            p(
                "electrum",
                "Bitcoin Austria Fulcrum",
                "ssl://index.bitcoin-austria.at:50002",
                "electrum",
                "Electrum / Fulcrum",
                Some("Bitcoin Austria"),
                true,
            ),
            p(
                "mempool",
                "mempool",
                "https://mempool.bitcoin-austria.at/api",
                "esplora",
                "Explorer API",
                Some("mempool.bitcoin-austria.at"),
                true,
            ),
            p(
                "core",
                "Bitcoin Core RPC",
                "http://127.0.0.1:8332",
                "bitcoinrpc",
                "Bitcoin Core RPC",
                None,
                false,
            ),
            p(
                "btcpay",
                "BTCPay Server",
                "https://btcpay.example.com",
                "btcpay",
                "BTCPay",
                None,
                false,
            ),
        ],
    },
    Type {
        id: "coreln",
        label: "Core-LN",
        net_kind: Net::Ln,
        desc_key: "settings:backendModal.network.corelnDesc",
        subtitle_key: "settings:backendModal.network.corelnSubtitle",
        presets: &[p(
            "core-lightning",
            "Core Lightning read-only",
            "cln://local",
            "coreln",
            "Local RPC",
            None,
            true,
        )],
    },
    Type {
        id: "liquid",
        label: "Liquid",
        net_kind: Net::Liquid,
        desc_key: "settings:backendModal.network.liquidDesc",
        subtitle_key: "settings:backendModal.network.liquidSubtitle",
        presets: &[
            p(
                "liquid-bullbitcoin",
                "BullBitcoin Liquid Electrum",
                "ssl://les.bullbitcoin.com:995",
                "electrum",
                "Electrum / Fulcrum",
                Some("BullBitcoin"),
                true,
            ),
            p(
                "liquid-blockstream",
                "Blockstream Liquid Electrum",
                "ssl://blockstream.info:995",
                "electrum",
                "Electrum / Fulcrum",
                Some("Blockstream"),
                true,
            ),
            p(
                "liquid-network",
                "Liquid Network",
                "https://liquid.network/api",
                "liquid-esplora",
                "Explorer API",
                Some("Liquid Network"),
                false,
            ),
        ],
    },
    Type {
        id: "lnd",
        label: "LND",
        net_kind: Net::Ln,
        desc_key: "settings:backendModal.network.lndDesc",
        subtitle_key: "settings:backendModal.network.lndSubtitle",
        presets: &[p(
            "lnd",
            "LND",
            "https://127.0.0.1:8080",
            "lnd",
            "LND REST",
            None,
            true,
        )],
    },
];

fn ty(id: &str) -> &'static Type {
    TYPES.iter().find(|x| x.id == id).unwrap_or(&TYPES[0])
}

fn public_presets(tp: &Type) -> Vec<&'static Preset> {
    tp.presets.iter().filter(|p| p.public).collect()
}

/// `scopedBackendTypes`.
fn scoped(initial: Option<&str>) -> Vec<&'static Type> {
    match initial {
        Some("bitcoin") => vec![&TYPES[0]],
        Some("liquid") => vec![&TYPES[2]],
        Some("coreln") | Some("lnd") => vec![&TYPES[1], &TYPES[3]],
        _ => TYPES.iter().collect(),
    }
}

/// `backendTypeIdForSettingsBackend`.
fn type_for(b: &Backend) -> &'static str {
    let kind = b
        .kind
        .as_deref()
        .unwrap_or("")
        .to_lowercase()
        .replace('-', "");
    if kind == "coreln" {
        return "coreln";
    }
    if kind == "lnd" {
        return "lnd";
    }
    let id = b.id.trim().to_lowercase();
    if (id == "liquid" || id == "liquid-blockstream")
        && (kind.is_empty() || kind == "electrum" || kind == "esplora" || kind == "liquidesplora")
    {
        return "liquid";
    }
    match b.net_kind {
        Net::Liquid => "liquid",
        Net::Btc => "bitcoin",
        _ if kind == "liquidesplora" => "liquid",
        _ => "bitcoin",
    }
}

#[derive(Default)]
pub struct BackendForm {
    pub open: bool,
    pub initial: Option<Backend>,
    pub initial_type: Option<&'static str>,
    pub type_id: &'static str,
    pub source: &'static str,
    pub preset: String,
    pub name: TextEdit,
    pub url: TextEdit,
    pub auth: String,
    pub auth_val: TextEdit,
    pub auth_val2: TextEdit,
    pub e_host: TextEdit,
    pub e_port: TextEdit,
    pub e_ssl: bool,
    pub trust_ssl: bool,
    pub owner: Option<Owner>,
    pub certificate: TextEdit,
    pub sp: bool,
    pub sp_file: TextEdit,
    pub sp_path: TextEdit,
    pub use_proxy: bool,
    pub proxy_host: TextEdit,
    pub proxy_port: TextEdit,
    pub ln_cli: TextEdit,
    pub ln_dir: TextEdit,
    pub rpc_file: TextEdit,
    /// "idle" | "testing" | "ok" | "fail".
    pub test: &'static str,
    pub test_log: String,
    pub saving: bool,
    /// The backend Save is waiting to write once its test passes.
    pub save_after_test: bool,
    pub deleting: Option<String>,
    /// A Core Lightning wallet created once its backend saves.
    pub pending_wallet: Option<Value>,
    /// The certificate trust dialog over the page (edited TLS backends only).
    pub cert: certtrust::Trust,
}

/// The trust dialog's home on this page.
pub static CERT_HOST: certtrust::Host = certtrust::Host {
    key: "set-cert",
    get: |a| &a.settings.backend.cert,
    get_mut: |a| &mut a.settings.backend.cert,
    fetch_tag: "settings.cert_fetch",
    trust_tag: "settings.cert_trust",
    trigger: "set-be-cert-check",
    on_trusted: cert_trusted,
};

/// A confirmed trust: the daemon switched the backend's `certificate` to the pin and turned
/// `insecure` off. Keep the form in step, so a later Save neither drops the pin nor turns
/// verification off again; values the user had not edited stay "unchanged" for the leave prompt.
fn cert_trusted(app: &mut App, p: &certtrust::Presented) {
    let before = super::subpage::snapshot(app);
    let f = &mut app.settings.backend;
    if let Some(path) = &p.certificate {
        f.certificate = te(path);
    }
    f.trust_ssl = false;
    reset_test(f);
    super::subpage::adopt(app, before);
    // `ui.backends.certificate.trust` invalidates the backend lists and status.
    super::refresh_backends(app);
    send(app, "status", Value::Null, "settings.status");
}

/// What the certificate row says about the form (`certificateTrustAvailable`,
/// `certificateTrustUnsaved`, `testCertificateFailure`, `testCertificateProblem`).
struct CertRow {
    available: bool,
    unsaved: bool,
    failure: Option<certtrust::Problem>,
    problem: Option<certtrust::Problem>,
}

fn cert_row(f: &BackendForm, d: &Derived) -> CertRow {
    let available = f
        .initial
        .as_ref()
        .is_some_and(|b| certtrust::can_trust(b.kind.as_deref(), Some(&b.url)));
    // Trust reaches the saved URL and proxy; with unsaved edits the form's test says nothing
    // about that endpoint.
    let saved_url = f
        .initial
        .as_ref()
        .map(|b| {
            if d.is_electrum {
                let (h, p, ssl) = parse_electrum(&b.url);
                electrum_url(&h, &p, ssl)
            } else {
                b.url.trim().to_owned()
            }
        })
        .unwrap_or_default();
    let saved_proxy = f
        .initial
        .as_ref()
        .and_then(|b| b.proxy.clone().flatten())
        .map(|p| format!("{}:{}", p.host, p.port))
        .unwrap_or_default();
    let unsaved = available
        && (d.effective_url.trim().to_lowercase() != saved_url.to_lowercase()
            || d.proxy_value.clone().unwrap_or_default() != saved_proxy);
    let failure = if f.test == "fail" {
        certtrust::problem_from_text(Some(&f.test_log))
    } else {
        None
    };
    CertRow {
        available,
        unsaved,
        failure,
        problem: if unsaved { None } else { failure },
    }
}

fn te(s: &str) -> TextEdit {
    TextEdit::new(s)
}

fn parse_electrum(raw: &str) -> (String, String, bool) {
    let cand = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("ssl://{raw}")
    };
    match model::Url::parse(&cand) {
        Some(u) => {
            let ssl = u.scheme != "tcp";
            let port = if u.port.is_empty() {
                if ssl { "50002" } else { "50001" }.to_owned()
            } else {
                u.port.clone()
            };
            (u.host, port, ssl)
        }
        None => (String::new(), "50002".into(), true),
    }
}

fn electrum_url(host: &str, port: &str, ssl: bool) -> String {
    let (h, p) = (host.trim(), port.trim());
    if h.is_empty() || p.is_empty() {
        String::new()
    } else {
        format!("{}://{h}:{p}", if ssl { "ssl" } else { "tcp" })
    }
}

fn reset_test(f: &mut BackendForm) {
    f.test = "idle";
    f.test_log.clear();
}

pub fn open_edit(app: &mut App, b: &Backend) {
    let tp = ty(type_for(b));
    let preset = tp.presets.iter().find(|p| p.url == b.url).or_else(|| {
        if b.url.to_lowercase().starts_with("ssl://") || b.url.to_lowercase().starts_with("tcp://")
        {
            tp.presets.iter().find(|p| p.protocol == "electrum")
        } else {
            None
        }
    });
    let (h, port, ssl) = parse_electrum(&b.url);
    app.settings.backend = BackendForm {
        open: true,
        initial: Some(b.clone()),
        initial_type: None,
        type_id: tp.id,
        source: "custom",
        preset: preset
            .map(|p| p.id.to_owned())
            .unwrap_or_else(|| "custom".into()),
        name: te(&b.name),
        url: te(&b.url),
        auth: b.auth.clone(),
        ln_cli: te(b.lightning_cli.as_deref().unwrap_or("")),
        ln_dir: te(b
            .lightning_dir
            .as_deref()
            .filter(|d| *d != model::CLN_SENTINEL_LIGHTNING_DIR)
            .unwrap_or("")),
        rpc_file: te(b
            .rpc_file
            .as_deref()
            .filter(|d| *d != model::CLN_SENTINEL_RPC_FILE)
            .unwrap_or("")),
        e_host: te(&h),
        e_port: te(&port),
        e_ssl: ssl,
        trust_ssl: b.trust_ssl.unwrap_or(false),
        owner: Some(
            b.infrastructure_owner
                .unwrap_or_else(|| model::inferred_owner(&b.url)),
        ),
        certificate: te(b.certificate.as_deref().unwrap_or("")),
        sp: b.silent_payments == Some(true),
        use_proxy: matches!(b.proxy, Some(Some(_))),
        proxy_host: te(b
            .proxy
            .clone()
            .flatten()
            .map(|p| p.host)
            .as_deref()
            .unwrap_or("")),
        proxy_port: te(b
            .proxy
            .clone()
            .flatten()
            .map(|p| p.port)
            .as_deref()
            .unwrap_or("")),
        test: if b.on { "ok" } else { "idle" },
        ..BackendForm::default()
    };
    let trigger = if super::current(app) == super::PRIVACY {
        format!("set-privacy-edit-{}", b.id)
    } else {
        format!("set-edit-{}", b.id)
    };
    super::subpage::opened(app, super::subpage::Kind::Backend, Some(trigger));
}

pub fn open_add(app: &mut App, initial_type: Option<&'static str>) {
    let types = scoped(initial_type);
    let tp = types
        .iter()
        .find(|x| Some(x.id) == initial_type)
        .copied()
        .unwrap_or(types[0]);
    app.settings.backend = BackendForm {
        open: true,
        initial_type,
        test: "idle",
        ..BackendForm::default()
    };
    pick_type(app, tp.id);
    // The book's network binding decides which networks a new backend may serve.
    send(
        app,
        "ui.networks.binding",
        json!({}),
        "settings.backend_scope",
    );
    let layer = match initial_type {
        Some("liquid") => "liquid",
        Some("lnd") | Some("coreln") => "lightning",
        _ => "bitcoin",
    };
    super::subpage::opened(
        app,
        super::subpage::Kind::Backend,
        Some(format!("set-add-{layer}")),
    );
}

/// `onPickType` plus the preset effect for a new backend.
fn pick_type(app: &mut App, id: &'static str) {
    let f = &mut app.settings.backend;
    f.type_id = id;
    f.test_log.clear();
    if f.initial.is_some() {
        f.preset = "custom".into();
        return;
    }
    let tp = ty(id);
    f.source = if tp.net_kind == Net::Ln {
        "custom"
    } else {
        "preset"
    };
    f.preset = public_presets(tp)
        .first()
        .map(|p| p.id.to_owned())
        .unwrap_or_else(|| "custom".into());
    f.e_host = te("index.bitcoin-austria.at");
    f.e_port = te("50002");
    f.e_ssl = true;
    if let Some(p) = public_presets(tp)
        .first()
        .filter(|p| p.protocol == "electrum")
    {
        let (h, port, ssl) = parse_electrum(p.url);
        f.e_host = te(&h);
        f.e_port = te(&port);
        f.e_ssl = ssl;
    }
    apply_preset(app);
}

fn preset(f: &BackendForm) -> Option<&'static Preset> {
    if f.preset == "custom" {
        None
    } else {
        ty(f.type_id).presets.iter().find(|p| p.id == f.preset)
    }
}

/// The preset effect: a new backend takes the preset's endpoint (or a blank custom one).
fn apply_preset(app: &mut App) {
    let f = &mut app.settings.backend;
    if f.initial.is_some() {
        return;
    }
    let tp = ty(f.type_id);
    match preset(f) {
        Some(p) => {
            if f.source == "preset" || tp.net_kind == Net::Ln {
                f.url = te(p.url);
                f.name = te(p.name);
            } else {
                let name = match tp.net_kind {
                    Net::Liquid => t("settings:backendModal.customNameLiquid"),
                    Net::Btc => t("settings:backendModal.customNameBitcoin"),
                    _ => p.name.to_owned(),
                };
                f.name = te(&name);
                f.url = te("");
                if p.protocol == "electrum" {
                    f.e_host = te("");
                    f.e_port = te("50002");
                    f.e_ssl = true;
                }
            }
            f.owner = Some(model::inferred_owner(p.url));
            f.auth = if p.protocol == "lnd" || p.protocol == "btcpay" {
                "apikey".into()
            } else {
                "none".into()
            };
            if f.source == "preset" && p.protocol == "electrum" {
                let (h, port, ssl) = parse_electrum(p.url);
                f.e_host = te(&h);
                f.e_port = te(&port);
                f.e_ssl = ssl;
            }
        }
        None => {
            f.url = te("");
            f.name = te("");
            f.auth = "none".into();
            f.owner = Some(Owner::ThirdParty);
        }
    }
    f.auth_val = te("");
    f.auth_val2 = te("");
    f.ln_cli = te("");
    f.ln_dir = te("");
    f.rpc_file = te("");
    f.trust_ssl = false;
    f.certificate = te("");
    f.sp = false;
    f.sp_file = te("");
    f.sp_path = te("");
    reset_test(f);
}

/// What the form currently describes.
struct Derived {
    tp: &'static Type,
    preset: Option<&'static Preset>,
    editing: bool,
    is_cln: bool,
    is_electrum: bool,
    is_lnd: bool,
    is_btcpay: bool,
    show_auth: bool,
    effective_url: String,
    kind: String,
    explorer_api: bool,
    is_rpc: bool,
    http_tls: bool,
    proxy_capable: bool,
    proxy_value: Option<String>,
    onion: bool,
    chain: String,
    network: String,
    blocked: bool,
}

fn derive(app: &App) -> Derived {
    let f = &app.settings.backend;
    let tp = ty(f.type_id);
    let preset = preset(f);
    let editing = f.initial.is_some();
    let init_kind = f.initial.as_ref().and_then(|b| b.kind.clone());
    let is_cln =
        preset.is_some_and(|p| p.protocol == "coreln") || init_kind.as_deref() == Some("coreln");
    let is_electrum = preset.is_some_and(|p| p.protocol == "electrum");
    let is_lnd = preset.is_some_and(|p| p.protocol == "lnd") || init_kind.as_deref() == Some("lnd");
    let is_btcpay =
        preset.is_some_and(|p| p.protocol == "btcpay") || init_kind.as_deref() == Some("btcpay");
    // A stored Core backend on a non-preset URL edits as `custom`: its kind, not the preset,
    // decides that it has credentials, or saving any field would clear them.
    let is_core = preset.is_some_and(|p| p.protocol == "bitcoinrpc")
        || init_kind.as_deref() == Some("bitcoinrpc");
    let show_auth = is_core || is_lnd || is_btcpay;
    let effective_url = if is_electrum {
        electrum_url(&f.e_host.text, &f.e_port.text, f.e_ssl)
    } else if is_cln {
        let u = f.url.text.trim();
        if u.is_empty() || u.to_lowercase().starts_with("cln://commando") {
            "cln://local".into()
        } else {
            u.to_owned()
        }
    } else {
        f.url.text.trim().to_owned()
    };
    let kind = preset
        .map(|p| p.protocol.to_owned())
        .or(init_kind)
        .unwrap_or_else(|| match tp.net_kind {
            Net::Liquid => "electrum".into(),
            Net::Ln => "lnd".into(),
            _ => "esplora".into(),
        });
    let explorer_api = kind == "esplora" || kind == "liquid-esplora";
    let is_rpc = kind == "bitcoinrpc";
    let public_endpoint = public_presets(tp)
        .iter()
        .any(|p| p.url.trim().eq_ignore_ascii_case(effective_url.trim()));
    let http_tls = f.source == "custom"
        && !public_endpoint
        && effective_url.to_lowercase().starts_with("https://")
        && (explorer_api || is_rpc);
    let proxy_capable = tp.net_kind != Net::Ln
        && [
            "bitcoinrpc",
            "btcpay",
            "electrum",
            "esplora",
            "liquid-esplora",
            "mempool",
        ]
        .contains(&kind.as_str());
    let proxy_value = (proxy_capable
        && f.use_proxy
        && !f.proxy_host.text.trim().is_empty()
        && !f.proxy_port.text.trim().is_empty())
    .then(|| format!("{}:{}", f.proxy_host.text.trim(), f.proxy_port.text.trim()));
    let onion = proxy_capable && model::is_onion(&effective_url);
    let chain = f
        .initial
        .as_ref()
        .and_then(|b| b.chain.clone())
        .unwrap_or_else(|| {
            if tp.net_kind == Net::Liquid {
                "liquid"
            } else {
                "bitcoin"
            }
            .into()
        });
    let scope = app.read("ui.networks.binding");
    let domain = scope.and_then(|s| s["domains"].as_array()).and_then(|d| {
        d.iter()
            .find(|x| x["chain"].as_str() == Some(chain.as_str()))
            .cloned()
    });
    let network = f
        .initial
        .as_ref()
        .and_then(|b| b.network.clone())
        .or_else(|| {
            domain
                .as_ref()
                .and_then(|d| d["network"].as_str().map(str::to_owned))
        })
        .unwrap_or_else(|| {
            if tp.net_kind == Net::Liquid {
                "liquidv1"
            } else {
                "main"
            }
            .into()
        });
    let scope_loading = app.settings.is_busy("settings.backend_scope") && scope.is_none();
    let bound = scope.is_some_and(|s| s["state"].as_str() == Some("bound"));
    let blocked = !editing
        && (scope_loading
            || scope.is_none()
            || (bound
                && (domain.is_none()
                    || (f.source == "preset" && network != "main" && network != "liquidv1"))));
    Derived {
        tp,
        preset,
        editing,
        is_cln,
        is_electrum,
        is_lnd,
        is_btcpay,
        show_auth,
        effective_url,
        kind,
        explorer_api,
        is_rpc,
        http_tls,
        proxy_capable,
        proxy_value,
        onion,
        chain,
        network,
        blocked,
    }
}

/// The onion effect: an onion endpoint turns the proxy on (Tor's default port) and plain TCP
/// / `http://` for the connection. Run every frame while the dialog is open.
pub fn fixup(app: &mut App) -> bool {
    if !app.settings.backend.open {
        return false;
    }
    let d = derive(app);
    if !d.onion {
        return false;
    }
    let f = &mut app.settings.backend;
    let mut changed = false;
    if !f.use_proxy {
        f.use_proxy = true;
        changed = true;
    }
    if f.proxy_host.text.trim().is_empty() {
        f.proxy_host = te("127.0.0.1");
        changed = true;
    }
    if f.proxy_port.text.trim().is_empty() {
        f.proxy_port = te("9050");
        changed = true;
    }
    if d.is_electrum && f.e_ssl && (f.e_port.text == "50002" || f.e_port.text.trim().is_empty()) {
        f.e_ssl = false;
        f.e_port = te("50001");
        changed = true;
    }
    if !d.is_electrum {
        let u = f.url.text.trim();
        if !u.is_empty() && !u.contains("://") && model::is_onion(u) {
            let mut e = te(&format!("http://{u}"));
            e.focused = f.url.focused;
            f.url = e;
            changed = true;
        }
    }
    if changed {
        reset_test(f);
    }
    changed
}

fn can_add(app: &App, d: &Derived) -> bool {
    let f = &app.settings.backend;
    let cln_ok = !d.is_cln || {
        // `coreLightningBackendModeValid`: local RPC through lightning-cli, a lightning dir or
        // an rpc file (an existing one's stored dir / file counts).
        let had_dir = f
            .initial
            .as_ref()
            .is_some_and(|b| b.lightning_dir.as_deref() == Some(model::CLN_SENTINEL_LIGHTNING_DIR));
        let had_rpc = f
            .initial
            .as_ref()
            .is_some_and(|b| b.rpc_file.as_deref() == Some(model::CLN_SENTINEL_RPC_FILE));
        !f.ln_cli.text.trim().is_empty()
            || !f.ln_dir.text.trim().is_empty()
            || !f.rpc_file.text.trim().is_empty()
            || had_dir
            || had_rpc
    };
    !d.blocked && !f.name.text.trim().is_empty() && !d.effective_url.is_empty() && cln_ok
}

/// `testConnection` (egress: only from Test connection or Save).
fn test_connection(app: &mut App) {
    let d = derive(app);
    if d.blocked || d.effective_url.is_empty() {
        app.settings.backend.saving = false;
        app.settings.backend.save_after_test = false;
        return;
    }
    let f = &mut app.settings.backend;
    if d.is_cln {
        f.test = "ok";
        f.test_log = t("settings:backendModal.clnTestLog");
        if f.save_after_test {
            save_now(app);
        }
        return;
    }
    f.test = "testing";
    let (kind, args, tag) = test_request(app, &d);
    let key = format!("{kind}{args}");
    if send(app, kind, args, tag) {
        app.settings.backend_tests.push_back(key);
    }
}

/// The test request for the form's current values: (kind, args, reply tag).
fn test_request(app: &App, d: &Derived) -> (&'static str, Value, &'static str) {
    let f = &app.settings.backend;
    if d.is_electrum {
        let mut args = json!({"url": d.effective_url, "trust_self_signed": f.e_ssl && f.trust_ssl});
        if f.e_ssl && !f.trust_ssl && !f.certificate.text.trim().is_empty() {
            args["certificate"] = json!(f.certificate.text.trim());
        }
        if let Some(p) = &d.proxy_value {
            args["proxy"] = json!(p);
        }
        (
            "ui.backends.electrum.test",
            args,
            "settings.backend_test_electrum",
        )
    } else if d.is_rpc {
        let mut config = json!({"insecure": d.http_tls && f.trust_ssl});
        if d.http_tls && !f.trust_ssl {
            config["certificate"] = json!(f.certificate.text.trim());
        }
        if f.auth == "basic" && !f.auth_val.text.trim().is_empty() {
            config["username"] = json!(f.auth_val.text.trim());
        }
        if f.auth == "basic" && !f.auth_val2.text.trim().is_empty() {
            config["password"] = json!(f.auth_val2.text.trim());
        }
        let mut args = json!({"url": d.effective_url, "network": d.network, "config": config});
        if let Some(b) = &f.initial {
            args["backend"] = json!(b.id);
        }
        if let Some(p) = &d.proxy_value {
            args["proxy"] = json!(p);
        }
        (
            "ui.backends.bitcoinrpc.test",
            args,
            "settings.backend_test_rpc",
        )
    } else {
        let mut args = json!({"url": d.effective_url, "insecure": d.http_tls && f.trust_ssl});
        if d.http_tls && !f.trust_ssl && !f.certificate.text.trim().is_empty() {
            args["certificate"] = json!(f.certificate.text.trim());
        }
        if let Some(p) = &d.proxy_value {
            args["proxy"] = json!(p);
        }
        ("ui.backends.http.test", args, "settings.backend_test_http")
    }
}

/// Whether a test reply belongs to the form's current values (else the form was edited, or
/// another form opened, since it was sent: its result says nothing about these values).
fn test_is_current(app: &mut App) -> bool {
    let asked = app.settings.backend_tests.pop_front();
    let d = derive(app);
    let (kind, args, _) = test_request(app, &d);
    asked.as_deref() == Some(format!("{kind}{args}").as_str())
}

/// `save`: tests first unless the last test of these values passed.
fn save(app: &mut App) {
    let d = derive(app);
    if !can_add(app, &d) {
        return;
    }
    let f = &mut app.settings.backend;
    f.saving = true;
    if f.test == "ok" || d.is_cln {
        save_now(app);
    } else {
        f.save_after_test = true;
        test_connection(app);
    }
}

fn save_now(app: &mut App) {
    let d = derive(app);
    let f = &mut app.settings.backend;
    f.save_after_test = false;
    let secret = f.auth_val.text.trim().to_owned();
    let pw = f.auth_val2.text.trim().to_owned();
    let initial = f.initial.clone();
    let preserve_redacted = initial
        .as_ref()
        .and_then(|b| b.proxy.clone().flatten())
        .is_some_and(|p| {
            p.redacted && f.proxy_host.text.trim() == p.host && f.proxy_port.text.trim() == p.port
        });
    let tls_fields = (d.is_electrum && f.e_ssl) || d.http_tls || d.is_lnd;
    let cert_used =
        ((d.is_electrum && f.e_ssl && !f.trust_ssl) || (d.http_tls && !f.trust_ssl) || d.is_lnd)
            && !f.certificate.text.trim().is_empty();
    let btc = d.tp.net_kind == Net::Btc;
    let auth = if d.show_auth {
        f.auth.clone()
    } else {
        "none".into()
    };
    let backend = Backend {
        id: initial
            .as_ref()
            .map(|b| b.id.clone())
            .unwrap_or_else(|| f.name.text.trim().to_owned()),
        name: f.name.text.trim().to_owned(),
        url: d.effective_url.clone(),
        net_kind: d.tp.net_kind,
        kind: Some(d.kind.clone()),
        chain: Some(d.chain.clone()),
        network: Some(d.network.clone()),
        health: t(if initial.is_some() {
            "settings:backendModal.healthJustChecked"
        } else {
            "settings:backendModal.healthJustAdded"
        }),
        on: true,
        auth: auth.clone(),
        auth_header: (d.show_auth && auth == "bearer" && !secret.is_empty())
            .then(|| format!("Bearer {secret}")),
        token: (d.show_auth && auth == "apikey" && !secret.is_empty()).then(|| secret.clone()),
        username: (d.show_auth && auth == "basic" && !secret.is_empty()).then(|| secret.clone()),
        password: (d.show_auth && auth == "basic" && !pw.is_empty()).then(|| pw.clone()),
        lightning_cli: (d.is_cln && !f.ln_cli.text.trim().is_empty())
            .then(|| f.ln_cli.text.trim().to_owned()),
        lightning_dir: (d.is_cln && !f.ln_dir.text.trim().is_empty())
            .then(|| f.ln_dir.text.trim().to_owned()),
        rpc_file: (d.is_cln && !f.rpc_file.text.trim().is_empty())
            .then(|| f.rpc_file.text.trim().to_owned()),
        trust_ssl: tls_fields.then_some(f.trust_ssl),
        infrastructure_owner: if d.tp.net_kind == Net::Ln {
            None
        } else {
            f.owner
        },
        certificate: cert_used.then(|| f.certificate.text.trim().to_owned()),
        silent_payments: btc.then_some(f.sp),
        sp_scan_file: (btc && f.sp && !f.sp_file.text.trim().is_empty())
            .then(|| f.sp_file.text.trim().to_owned()),
        sp_scan_path: (btc && f.sp && !f.sp_path.text.trim().is_empty())
            .then(|| f.sp_path.text.trim().to_owned()),
        proxy: if d.proxy_capable {
            Some(d.proxy_value.as_ref().map(|_| Proxy {
                host: f.proxy_host.text.trim().to_owned(),
                port: f.proxy_port.text.trim().to_owned(),
                redacted: preserve_redacted,
            }))
        } else {
            None
        },
        ..Backend::default()
    };
    let mut payload = model::backend_payload(&backend);
    match &initial {
        Some(b) => {
            payload["name"] = json!(b.id);
            send(app, "ui.backends.update", payload, "settings.backend_save");
        }
        None => {
            app.settings.backend.pending_wallet = (backend.kind.as_deref() == Some("coreln")).then(|| json!({"label": backend.name, "kind": "coreln", "backend": backend.name, "chain": backend.chain, "network": backend.network}));
            send(app, "ui.backends.create", payload, "settings.backend_save");
        }
    }
}

fn delete(app: &mut App) {
    let Some(b) = app.settings.backend.initial.clone() else {
        return;
    };
    if !b.wallet_refs.is_empty() {
        // Wallet labels are user data.
        app.toast(
            ToastTone::Error,
            &t("settings:deleteBackend.blockedTitle"),
            Some(tr(
                "settings:deleteBackend.walletWarning",
                &[("wallets", &b.wallet_refs.join("\n- "))],
            )),
            None,
        );
        return;
    }
    super::dialogs::confirm(
        app,
        tr("settings:deleteBackend.confirm", &[("name", &b.name)]),
        super::dialogs::Confirmed::DeleteBackend(b.id.clone()),
    );
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if matches!(
        tag,
        "settings.backend_test_electrum"
            | "settings.backend_test_http"
            | "settings.backend_test_rpc"
    ) && !test_is_current(app)
    {
        // A test of other values: no result for these, and nothing is saved from it.
        let f = &mut app.settings.backend;
        if f.test == "testing" {
            f.test = "idle";
        }
        if f.save_after_test {
            f.save_after_test = false;
            f.saving = false;
        }
        return;
    }
    let f = &mut app.settings.backend;
    match tag {
        "settings.backend_test_electrum" | "settings.backend_test_http" => {
            let ok = r.error.is_none() && r.data["ok"].as_bool() == Some(true);
            f.test = if ok { "ok" } else { "fail" };
            f.test_log = match &r.error {
                Some(e) => e.clone(),
                None => r.data["logs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n"),
            };
            after_test(app, ok);
        }
        "settings.backend_test_rpc" => {
            let ok = r.error.is_none() && r.data["reachable"].as_bool() == Some(true);
            f.test = if ok { "ok" } else { "fail" };
            f.test_log = match &r.error {
                Some(e) => e.clone(),
                None if ok => r.data["status"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| t("settings:backendModal.healthJustChecked")),
                None => r.data["error"]["message"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| t("settings:backendModal.httpTestFailed")),
            };
            after_test(app, ok);
        }
        "settings.backend_save" => match &r.error {
            Some(e) => {
                f.saving = false;
                f.test = "fail";
                f.test_log = e.clone();
            }
            None => {
                if let Some(w) = f.pending_wallet.take() {
                    send(app, "ui.wallets.create", w, "settings.backend_wallet");
                }
                let f = &mut app.settings.backend;
                f.saving = false;
                f.open = false;
                super::refresh_backends(app);
            }
        },
        "settings.cert_fetch" | "settings.cert_trust" => {
            certtrust::on_reply(app, &CERT_HOST, tag, r);
        }
        "settings.backend_delete" => {
            app.settings.backend.deleting = None;
            match &r.error {
                Some(e) => app.toast(
                    ToastTone::Error,
                    &t("settings:deleteBackend.failedTitle"),
                    Some(e.clone()),
                    None,
                ),
                None => {
                    app.settings.backend.open = false;
                    super::refresh_backends(app);
                }
            }
        }
        _ => {}
    }
}

fn after_test(app: &mut App, ok: bool) {
    let f = &mut app.settings.backend;
    if f.save_after_test {
        if ok {
            save_now(app);
        } else {
            f.save_after_test = false;
            f.saving = false;
        }
    }
}

/// A form field that resets the last test when edited.
#[allow(clippy::too_many_arguments)]
fn field(
    tk: &Tk,
    edit: &TextEdit,
    key: &str,
    label: &str,
    placeholder: &str,
    secure: bool,
    enabled: bool,
    mono: bool,
    on_edit: fn(&mut App) -> &mut TextEdit,
) -> El {
    let k = key.to_owned();
    let _ = k;
    labelled(
        tk,
        label,
        input(
            tk,
            edit,
            key,
            label,
            placeholder,
            secure,
            enabled,
            tk.r(2.25),
            mono,
            on_edit,
        ),
    )
}

fn posture_color(tk: &Tk, p: Posture) -> Color {
    match p {
        Posture::OnDevice | Posture::SelfHosted => emerald(tk),
        Posture::Shielded => tk.fg,
        Posture::Remote => super::controls::amber(tk),
    }
}

/// `NetworkMark`: the network's logo in a `size-9` bordered tile (white `brandLogoFrame` for
/// Bitcoin and Liquid, the brand's dark tile for Core-LN and LND), every logo's artwork at the
/// bitcoin coin's `size-6`. The form has no `Cx`, so logos rasterise at 2× (mips keep 1× crisp).
fn mark(app: &App, tk: &Tk, tp: &Type) -> El {
    use crate::logos::{self, Logo};
    let hx = |s: &str| rune::color::hex(s).unwrap_or(Color::WHITE);
    let (logo, fill, border) = match tp.id {
        "coreln" => (
            Logo::svg(logos::CORE_LIGHTNING),
            hx("#494120"),
            hx("#525252"),
        ),
        "lnd" => (
            Logo::raster(logos::LIGHTNING_LABS_PNG),
            hx("#0a0a0a"),
            hx("#404040"),
        ),
        "liquid" => (
            Logo::svg(logos::LIQUID),
            Color::WHITE,
            hx(if tk.dark { "#404040" } else { "#e5e5e5" }),
        ),
        _ => (
            Logo::svg(logos::BITCOIN),
            Color::WHITE,
            hx(if tk.dark { "#404040" } else { "#e5e5e5" }),
        ),
    };
    let side = tk.r(2.25);
    row()
        .size(side, side)
        .center()
        .radius(tk.r(0.375))
        .border(1.0, border)
        .border_layout()
        .fill(fill)
        .a11y_hidden()
        .child(logos::fitted_at(
            app,
            2.0,
            logo,
            tk.r(logos::role::NETWORK_MARK),
            side - 2.0,
        ))
}

/// The title / description key prefix for the form's mode and scope.
pub fn copy_key(app: &App) -> &'static str {
    let f = &app.settings.backend;
    let editing = f.initial.is_some();
    let types = if editing {
        TYPES.iter().collect()
    } else {
        scoped(f.initial_type)
    };
    if editing {
        "settings:backendModal.edit"
    } else if types.len() == 1 && f.type_id == "bitcoin" {
        "settings:backendModal.addBitcoin"
    } else if types.len() == 1 && f.type_id == "liquid" {
        "settings:backendModal.addLiquid"
    } else if types.iter().all(|x| x.net_kind == Net::Ln) {
        "settings:backendModal.addLightning"
    } else {
        "settings:backendModal.addGeneric"
    }
}

/// The add / edit backend form as a Settings sub-page (was a dialog over the modal).
pub fn page(app: &App, tk: &Tk, overlays: &mut Vec<El>) -> super::subpage::Parts {
    let f = &app.settings.backend;
    let d = derive(app);
    let types = if d.editing {
        TYPES.iter().collect()
    } else {
        scoped(f.initial_type)
    };
    let copy_key = copy_key(app);
    let mut body = col().w_full().gap(tk.r(1.25));

    // Connection type cards.
    if !d.editing && types.len() > 1 {
        let mut grid = row().w_full().gap(tk.r(0.5)).flex_wrap();
        for tp in &types {
            let active = tp.id == f.type_id;
            let id = tp.id;
            let border = if active {
                if tk.dark {
                    Color::WHITE.alpha(0.45)
                } else {
                    tk.fg.alpha(0.5)
                }
            } else if tk.dark {
                Color::WHITE.alpha(0.2)
            } else {
                tk.border
            };
            let fill = if active {
                if tk.dark {
                    Color::WHITE.alpha(0.10)
                } else {
                    tk.muted
                }
            } else if tk.dark {
                Color::WHITE.alpha(0.04)
            } else {
                tk.background
            };
            grid = grid.child(
                row()
                    .key(format!("set-be-type-{id}"))
                    .w(tk.r(19.0))
                    .min_h(72.0)
                    .padding(tk.r(0.75))
                    .gap(tk.r(0.75))
                    .center_y()
                    .radius(tk.r(0.375))
                    .border(1.0, border)
                    .fill(fill)
                    .hover(if tk.dark {
                        Color::WHITE.alpha(0.08)
                    } else {
                        tk.muted
                    })
                    .role("radio")
                    .checked(active)
                    .label(tp.label)
                    .focusable(true)
                    .on_press(move |a: &mut App| pick_type(a, id))
                    .child(mark(app, tk, tp))
                    .child(
                        col()
                            .min_w(0.0)
                            .gap(tk.r(0.125))
                            .child(text(tp.label).font(tk.tsm(500.0)).color(tk.fg))
                            .child(
                                text(t(tp.subtitle_key))
                                    .font(tk.txs(400.0))
                                    .color(tk.muted_fg)
                                    .wrap(),
                            ),
                    ),
            );
        }
        body = body.child(
            col()
                .w_full()
                .gap(tk.r(0.75))
                .child(
                    col()
                        .gap(tk.r(0.25))
                        .child(
                            text(t(&format!("{copy_key}.selectorLabel")))
                                .font(tk.tsm(500.0))
                                .color(tk.fg),
                        )
                        .child(muted(tk, t(d.tp.desc_key))),
                )
                .child(grid),
        );
    }

    // Source / preset / endpoint type.
    let show_source = !d.editing && d.tp.net_kind != Net::Ln;
    let pubs = public_presets(d.tp);
    let show_preset = !d.editing && !pubs.is_empty() && f.source == "preset";
    let show_custom_proto = !d.editing && d.tp.presets.len() > 1 && f.source == "custom";
    if show_source || show_preset || show_custom_proto {
        let mut g = group();
        if show_source {
            let mut dsc = col().w_full();
            if d.blocked && app.read("ui.networks.binding").is_some() {
                dsc = dsc.child(
                    text(tr(
                        "settings:bookNetwork.customBackend",
                        &[("network", &d.network)],
                    ))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .wrap()
                    .role("status"),
                );
            }
            dsc = dsc.child(muted(tk, t("settings:backendModal.backendSourceHint")));
            let sel = select(
                app,
                tk,
                overlays,
                "set-be-source",
                &t("settings:backendModal.backendSourceLabel"),
                f.source,
                vec![
                    opt("preset", t("settings:backendModal.presetTab")),
                    opt("custom", t("settings:backendModal.customTab")),
                ],
                true,
                None,
                |a, v| {
                    a.settings.backend.source = if v == "custom" { "custom" } else { "preset" };
                    apply_preset(a);
                },
            );
            g = g.row(srow(
                app,
                tk,
                t("settings:backendModal.backendSourceLabel"),
                Some(dsc),
                vec![sel],
            ));
        }
        if show_preset {
            let mut dsc = col()
                .w_full()
                .child(muted(tk, t("settings:backendModal.presetEndpointHint")));
            if let Some(p) = d.preset {
                dsc = dsc.child(muted(tk, p.label));
            }
            let sel = select(
                app,
                tk,
                overlays,
                "set-be-preset",
                &t("settings:backendModal.presetEndpointLabel"),
                &f.preset,
                pubs.iter()
                    .map(|p| super::controls::Opt {
                        value: p.id.into(),
                        label: p.provider.unwrap_or(p.name).into(),
                        title: Some(p.label.into()),
                        disabled: false,
                    })
                    .collect(),
                true,
                Some(&t("settings:backendModal.presetPlaceholder")),
                |a, v| {
                    a.settings.backend.preset = v.to_owned();
                    apply_preset(a);
                },
            );
            g = g.row(srow(
                app,
                tk,
                t("settings:backendModal.presetEndpointLabel"),
                Some(dsc),
                vec![sel],
            ));
        }
        if show_custom_proto {
            let sel = select(
                app,
                tk,
                overlays,
                "set-be-proto",
                &t("settings:backendModal.endpointTypeLabel"),
                &f.preset,
                d.tp.presets.iter().map(|p| opt(p.id, p.label)).collect(),
                true,
                None,
                |a, v| {
                    a.settings.backend.preset = v.to_owned();
                    apply_preset(a);
                },
            );
            g = g.row(srow(
                app,
                tk,
                t("settings:backendModal.endpointTypeLabel"),
                Some(muted(tk, t("settings:backendModal.endpointTypeHint"))),
                vec![sel],
            ));
        }
        body = body.child(g.build(app, tk));
    }

    // Connection: name, then the endpoint; the stable id as a muted detail.
    let locked = f.source == "preset" && !d.editing;
    let mut fields = col()
        .w_full()
        .px(tk.r(1.0))
        .py(tk.r(1.0))
        .gap(tk.r(0.75))
        .child(field(
            tk,
            &f.name,
            "set-be-name",
            &t("settings:backendModal.displayNameLabel"),
            &t("settings:backendModal.displayNamePlaceholder"),
            false,
            true,
            false,
            |a| &mut a.settings.backend.name,
        ));
    let endpoint = if d.is_electrum {
        let host_ph = t(if d.tp.net_kind == Net::Liquid {
            "settings:backendModal.hostPlaceholderLiquid"
        } else {
            "settings:backendModal.hostPlaceholderBitcoin"
        });
        row()
            .w_full()
            .min_w(0.0)
            .gap(tk.r(0.75))
            .child(
                field(
                    tk,
                    &f.e_host,
                    "set-be-host",
                    &t("settings:backendModal.hostLabel"),
                    &host_ph,
                    false,
                    !locked,
                    false,
                    |a| &mut a.settings.backend.e_host,
                )
                .grow(1.0)
                .min_w(0.0),
            )
            .child(
                field(
                    tk,
                    &f.e_port,
                    "set-be-port",
                    &t("settings:backendModal.portLabel"),
                    if f.e_ssl { "50002" } else { "50001" },
                    false,
                    !locked,
                    false,
                    |a| &mut a.settings.backend.e_port,
                )
                .w(120.0),
            )
    } else {
        col().w_full().min_w(0.0).child(field(
            tk,
            &f.url,
            "set-be-url",
            &t("settings:backendModal.endpointUrlLabel"),
            "https://...",
            false,
            !locked,
            false,
            |a| &mut a.settings.backend.url,
        ))
    };
    fields = fields.child(endpoint);
    if d.tp.net_kind != Net::Ln && d.explorer_api {
        fields = fields.child(muted(tk, t("settings:backendModal.explorerApiNote")));
    }
    if d.editing {
        fields = fields.child(
            text(tr(
                "settings:backendModal.displayNameStableHint",
                &[(
                    "id",
                    &f.initial.as_ref().map(|b| b.id.clone()).unwrap_or_default(),
                )],
            ))
            .font(tk.txs(400.0).mono())
            .color(tk.muted_fg)
            .wrap_anywhere(),
        );
    }
    body = body.child(
        group()
            .title(t("settings:backendModal.testLabel"))
            .row(fields)
            .build(app, tk),
    );

    // Silent Payments (a toggle that reveals its scanner fields), then who runs the server; the
    // privacy classification it yields sits in the side rail.
    let owner = if d.tp.net_kind == Net::Ln {
        None
    } else {
        f.owner
    };
    let trust = model::trust_for(
        &d.effective_url,
        d.proxy_value.is_some(),
        Some(&d.kind),
        owner,
    );
    if d.tp.net_kind == Net::Btc {
        let mut g = group()
            .title(t("settings:networkLayer.silentPayments"))
            .row(srow(
                app,
                tk,
                t("settings:backendModal.silentPaymentsLabel"),
                Some(muted(tk, t("settings:backendModal.silentPaymentsHint"))),
                vec![switch(
                    tk,
                    "set-be-sp",
                    f.sp,
                    &t("settings:backendModal.silentPaymentsLabel"),
                    true,
                    |a, v| {
                        a.settings.backend.sp = v;
                        reset_test(&mut a.settings.backend);
                    },
                )],
            ));
        if f.sp {
            g = g.row(
                col()
                    .w_full()
                    .px(tk.r(1.0))
                    .py(tk.r(0.75))
                    .gap(tk.r(0.75))
                    .child(muted(
                        tk,
                        t("settings:backendModal.silentPaymentsScannerConfigHint"),
                    ))
                    .child(field(
                        tk,
                        &f.sp_file,
                        "set-be-sp-file",
                        &t("settings:backendModal.silentPaymentsScanFileLabel"),
                        &t("settings:backendModal.silentPaymentsScanFilePlaceholder"),
                        false,
                        true,
                        false,
                        |a| &mut a.settings.backend.sp_file,
                    ))
                    .child(field(
                        tk,
                        &f.sp_path,
                        "set-be-sp-path",
                        &t("settings:backendModal.silentPaymentsScanPathLabel"),
                        &t("settings:backendModal.silentPaymentsScanPathPlaceholder"),
                        false,
                        true,
                        false,
                        |a| &mut a.settings.backend.sp_path,
                    )),
            );
        }
        body = body.child(g.build(app, tk));
    }
    if d.tp.net_kind != Net::Ln {
        let owner_sel = select(
            app,
            tk,
            overlays,
            "set-be-owner",
            &t("settings:backendModal.infraOwnerLabel"),
            f.owner.unwrap_or(Owner::ThirdParty).wire(),
            vec![
                opt("self", t("settings:backendModal.infraOwnerMine")),
                opt(
                    "third_party",
                    t("settings:backendModal.infraOwnerThirdParty"),
                ),
            ],
            true,
            None,
            |a, v| a.settings.backend.owner = Owner::parse(v),
        );
        body = body.child(
            group()
                .title(t("settings:nav.section.privacy.label"))
                .row(srow(
                    app,
                    tk,
                    t("settings:backendModal.infraOwnerLabel"),
                    Some(muted(tk, t("settings:backendModal.infraOwnerHint"))),
                    vec![owner_sel],
                ))
                .build(app, tk),
        );
    }

    // Core Lightning local access.
    if d.is_cln {
        let mut c = col().w_full().gap(tk.r(0.75)).child(
            col()
                .gap(tk.r(0.25))
                .child(
                    text(t("settings:backendModal.corelnAccessLabel"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                )
                .child(muted(tk, t("settings:backendModal.corelnAccessHint"))),
        );
        if f.initial.as_ref().is_some_and(model::is_coreln_commando) {
            let amber = rune::color::hex("#f59e0b").unwrap();
            c = c.child(
                col()
                    .w_full()
                    .px(tk.r(0.75))
                    .py(tk.r(0.5))
                    .radius(tk.r(0.375))
                    .border(1.0, amber.alpha(0.5))
                    .fill(amber.alpha(0.1))
                    .role("alert")
                    .child(
                        text(t("settings:backendModal.corelnCommandoUnsupported"))
                            .font(tk.tsm(500.0))
                            .color(super::controls::amber_soft(tk))
                            .wrap(),
                    )
                    .child(
                        text(t("settings:backendModal.corelnCommandoHint"))
                            .font(tk.tsm(400.0))
                            .color(super::controls::amber_soft(tk))
                            .wrap(),
                    )
                    .child(
                        text(t("settings:backendModal.corelnCommandoConvert"))
                            .font(tk.tsm(400.0))
                            .color(super::controls::amber_soft(tk))
                            .wrap(),
                    ),
            );
        }
        c = c
            .child(
                row()
                    .w_full()
                    .gap(tk.r(0.75))
                    .child(
                        field(
                            tk,
                            &f.ln_cli,
                            "set-be-lncli",
                            &t("settings:backendModal.lightningCliLabel"),
                            "lightning-cli",
                            false,
                            true,
                            false,
                            |a| &mut a.settings.backend.ln_cli,
                        )
                        .grow(1.0),
                    )
                    .child(
                        field(
                            tk,
                            &f.ln_dir,
                            "set-be-lndir",
                            &t("settings:backendModal.lightningDirLabel"),
                            "~/.lightning",
                            false,
                            true,
                            false,
                            |a| &mut a.settings.backend.ln_dir,
                        )
                        .grow(1.0),
                    ),
            )
            .child(field(
                tk,
                &f.rpc_file,
                "set-be-rpcfile",
                &t("settings:backendModal.rpcFileLabel"),
                "lightning-rpc",
                false,
                true,
                false,
                |a| &mut a.settings.backend.rpc_file,
            ))
            .child(muted(tk, t("settings:backendModal.rpcFileHint")));
        body = body.child(c);
    }

    // Advanced: TLS and proxy.
    let show_adv = (d.is_electrum && f.source == "custom") || d.http_tls || d.proxy_capable;
    if show_adv {
        let mut rows: Vec<El> = vec![];
        if d.is_electrum && f.source == "custom" {
            rows.push(srow(
                app,
                tk,
                t("settings:backendModal.useSslLabel"),
                Some(muted(tk, t("settings:backendModal.useSslHint"))),
                vec![switch(
                    tk,
                    "set-be-ssl",
                    f.e_ssl,
                    &t("settings:backendModal.useSslLabel"),
                    true,
                    |a, v| {
                        let f = &mut a.settings.backend;
                        f.e_ssl = v;
                        if !v {
                            f.trust_ssl = false;
                            f.certificate = te("");
                        }
                        if f.e_port.text == "50002" || f.e_port.text == "50001" {
                            f.e_port = te(if v { "50002" } else { "50001" });
                        }
                        reset_test(f);
                    },
                )],
            ));
            rows.push(srow(
                app,
                tk,
                t("settings:backendModal.trustSelfSignedLabel"),
                Some(muted(tk, t("settings:backendModal.trustSelfSignedHint"))),
                vec![switch(
                    tk,
                    "set-be-trust",
                    f.trust_ssl,
                    &t("settings:backendModal.trustSelfSignedLabel"),
                    f.e_ssl,
                    |a, v| {
                        a.settings.backend.trust_ssl = v;
                        reset_test(&mut a.settings.backend);
                    },
                )],
            ));
            let mut cert = col()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.75))
                .gap(tk.r(0.5))
                .child(field(
                    tk,
                    &f.certificate,
                    "set-be-cert",
                    &t("settings:backendModal.certificateLabel"),
                    &t("settings:backendModal.certificatePlaceholder"),
                    false,
                    f.e_ssl && !f.trust_ssl,
                    false,
                    |a| &mut a.settings.backend.certificate,
                ));
            if f.e_ssl && f.trust_ssl {
                cert = cert.child(muted(tk, t("settings:backendModal.certificateIgnored")));
            }
            rows.push(cert);
        }
        if d.http_tls {
            rows.push(srow(
                app,
                tk,
                t("settings:backendModal.trustSelfSignedLabel"),
                Some(muted(tk, t("settings:backendModal.trustSelfSignedHint"))),
                vec![switch(
                    tk,
                    "set-be-http-trust",
                    f.trust_ssl,
                    &t("settings:backendModal.trustSelfSignedLabel"),
                    true,
                    |a, v| {
                        a.settings.backend.trust_ssl = v;
                        reset_test(&mut a.settings.backend);
                    },
                )],
            ));
            let mut cert = col()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.75))
                .gap(tk.r(0.5))
                .child(field(
                    tk,
                    &f.certificate,
                    "set-be-http-cert",
                    &t("settings:backendModal.certificateLabel"),
                    &t("settings:backendModal.certificatePlaceholder"),
                    false,
                    !f.trust_ssl,
                    false,
                    |a| &mut a.settings.backend.certificate,
                ));
            if f.trust_ssl {
                cert = cert.child(muted(tk, t("settings:backendModal.certificateIgnored")));
            }
            rows.push(cert);
        }
        if d.proxy_capable {
            if d.onion {
                rows.push(
                    col()
                        .w_full()
                        .px(tk.r(1.0))
                        .py(tk.r(0.75))
                        .child(
                            text(t("settings:backendModal.onionDetectedTitle"))
                                .font(tk.tsm(500.0))
                                .color(tk.fg),
                        )
                        .child(muted(tk, t("settings:backendModal.onionDetectedBody"))),
                );
            }
            rows.push(srow(
                app,
                tk,
                t("settings:backendModal.useProxyLabel"),
                Some(muted(tk, t("settings:backendModal.useProxyHint"))),
                vec![switch(
                    tk,
                    "set-be-proxy",
                    f.use_proxy,
                    &t("settings:backendModal.useProxyLabel"),
                    true,
                    |a, v| {
                        a.settings.backend.use_proxy = v;
                        reset_test(&mut a.settings.backend);
                    },
                )],
            ));
            if f.use_proxy {
                rows.push(
                    row()
                        .w_full()
                        .px(tk.r(1.0))
                        .py(tk.r(0.75))
                        .gap(tk.r(0.75))
                        .child(
                            field(
                                tk,
                                &f.proxy_host,
                                "set-be-proxy-host",
                                &t("settings:backendModal.proxyHostLabel"),
                                "127.0.0.1",
                                false,
                                true,
                                false,
                                |a| &mut a.settings.backend.proxy_host,
                            )
                            .grow(1.0),
                        )
                        .child(
                            field(
                                tk,
                                &f.proxy_port,
                                "set-be-proxy-port",
                                &t("settings:backendModal.proxyPortLabel"),
                                "9050",
                                false,
                                true,
                                false,
                                |a| &mut a.settings.backend.proxy_port,
                            )
                            .grow(1.0),
                        ),
                );
            }
        }
        let default_open = f.trust_ssl || !f.certificate.text.is_empty() || f.use_proxy || d.onion;
        let title = col()
            .min_w(0.0)
            .child(
                text(t("settings:backendModal.advancedLabel"))
                    .font(tk.tsm(500.0))
                    .color(tk.fg),
            )
            .child(
                text(t("settings:backendModal.advancedHint"))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg),
            );
        body = body.child(disclosure(
            app,
            tk,
            "set-be-advanced",
            title,
            default_open,
            rows,
            None,
        ));
    }

    // LND TLS.
    if d.is_lnd {
        let mut cert = col()
            .w_full()
            .px(tk.r(1.0))
            .py(tk.r(0.75))
            .gap(tk.r(0.5))
            .child(field(
                tk,
                &f.certificate,
                "set-be-lnd-cert",
                &t("settings:backendModal.tlsCertLabel"),
                &t("settings:backendModal.tlsCertPlaceholder"),
                false,
                !f.trust_ssl,
                false,
                |a| &mut a.settings.backend.certificate,
            ));
        cert = cert.min_w(0.0);
        body = body.child(
            group()
                .row(srow(
                    app,
                    tk,
                    t("settings:backendModal.trustTlsLabel"),
                    Some(muted(tk, t("settings:backendModal.trustTlsHint"))),
                    vec![switch(
                        tk,
                        "set-be-lnd-trust",
                        f.trust_ssl,
                        &t("settings:backendModal.trustTlsLabel"),
                        true,
                        |a, v| {
                            a.settings.backend.trust_ssl = v;
                            reset_test(&mut a.settings.backend);
                        },
                    )],
                ))
                .row(cert)
                .build(app, tk),
        );
    }

    // Authentication (secrets masked; never logged).
    if d.show_auth {
        let auth_sel = select(
            app,
            tk,
            overlays,
            "set-be-auth",
            &t("settings:backendModal.rpcAuthLabel"),
            &f.auth,
            ["none", "apikey", "basic", "bearer"]
                .iter()
                .map(|m| opt(*m, t(&format!("settings:backendModal.authMode.{m}"))))
                .collect(),
            true,
            None,
            |a, v| a.settings.backend.auth = v.to_owned(),
        );
        let mut g = group().row(srow(
            app,
            tk,
            t("settings:backendModal.rpcAuthLabel"),
            None,
            vec![auth_sel],
        ));
        let keep = d.editing && f.initial.as_ref().is_some_and(|b| b.auth == "apikey");
        match f.auth.as_str() {
            "apikey" => {
                let label = t(if d.is_lnd {
                    "settings:backendModal.macaroonLabel"
                } else if d.is_btcpay {
                    "settings:backendModal.btcpayKeyLabel"
                } else {
                    "settings:backendModal.apiKeyLabel"
                });
                let ph = if keep {
                    t("settings:backendModal.apiKeyKeepPlaceholder")
                } else if d.is_lnd {
                    "0201036c6e64...".into()
                } else if d.is_btcpay {
                    t("settings:backendModal.btcpayKeyPlaceholder")
                } else {
                    "sk_live_...".into()
                };
                let mut c = col()
                    .w_full()
                    .px(tk.r(1.0))
                    .py(tk.r(0.75))
                    .gap(tk.r(0.375))
                    .child(field(
                        tk,
                        &f.auth_val,
                        "set-be-apikey",
                        &label,
                        &ph,
                        true,
                        true,
                        false,
                        |a| &mut a.settings.backend.auth_val,
                    ));
                if keep {
                    c = c.child(muted(tk, t("settings:backendModal.apiKeyKeepHint")));
                }
                g = g.row(c);
            }
            "bearer" => {
                g = g.row(col().w_full().px(tk.r(1.0)).py(tk.r(0.75)).child(field(
                    tk,
                    &f.auth_val,
                    "set-be-bearer",
                    &t("settings:backendModal.bearerLabel"),
                    "eyJ...",
                    true,
                    true,
                    false,
                    |a| &mut a.settings.backend.auth_val,
                )));
            }
            "basic" => {
                g = g.row(
                    row()
                        .w_full()
                        .px(tk.r(1.0))
                        .py(tk.r(0.75))
                        .gap(tk.r(0.75))
                        .child(
                            field(
                                tk,
                                &f.auth_val,
                                "set-be-user",
                                &t("settings:backendModal.usernameLabel"),
                                "",
                                false,
                                true,
                                false,
                                |a| &mut a.settings.backend.auth_val,
                            )
                            .grow(1.0),
                        )
                        .child(
                            field(
                                tk,
                                &f.auth_val2,
                                "set-be-pass",
                                &t("settings:backendModal.passwordLabel"),
                                "",
                                true,
                                true,
                                false,
                                |a| &mut a.settings.backend.auth_val2,
                            )
                            .grow(1.0),
                        ),
                );
            }
            _ => {}
        }
        body = body.child(g.build(app, tk));
    }

    // Connection test and its log.
    let testing = f.test == "testing";
    let cert = cert_row(f, &d);
    let test_desc = match f.test {
        "ok" => text(t("settings:backendModal.connected"))
            .font(tk.tsm(400.0))
            .color(emerald(tk)),
        // A certificate the server presents but Kassiber does not trust reached the server.
        "fail" => text(t(match cert.failure {
            Some(certtrust::Problem::Changed) => "settings:backendModal.certificateChanged",
            Some(certtrust::Problem::Untrusted) => "settings:backendModal.certificateUntrusted",
            None => "settings:backendModal.couldNotReach",
        }))
        .font(tk.tsm(400.0))
        .color(tk.destructive)
        .wrap(),
        _ => muted(tk, t("settings:backendModal.testHint")),
    };
    let can_test = !d.blocked && !d.effective_url.is_empty() && !testing && !f.saving;
    let test_btn = btn(
        tk,
        "set-be-test",
        &t(if testing {
            "settings:backendModal.testing"
        } else {
            "settings:backendModal.testConnection"
        }),
        Btn::Outline,
        can_test,
    )
    .press_if(can_test, test_connection);
    // Side rail: the status card (test result, its log once there is one, the certificate),
    // the privacy classification, and Delete.
    let mut status = vec![test_desc, row().w_full().child(test_btn)];
    if !f.test_log.trim().is_empty() {
        status.push(
            col()
                .w_full()
                .padding(tk.r(0.75))
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .fill(tk.muted.alpha(0.3))
                .role("textbox")
                .label(t("settings:backendModal.testLogAria"))
                .read_only(true)
                .child(
                    text(f.test_log.clone())
                        .font(tk.txs(400.0).mono().lh(tk.r(1.25)))
                        .color(tk.fg)
                        .wrap_anywhere(),
                ),
        );
    }
    if cert.available {
        // Offered only for an edited TLS backend; nothing is fetched until the dialog's Fetch.
        let hint = t(if cert.unsaved {
            "settings:certificateTrust.saveFirst"
        } else {
            match cert.problem {
                Some(certtrust::Problem::Changed) => "settings:certificateTrust.testChanged",
                Some(certtrust::Problem::Untrusted) => "settings:certificateTrust.testUntrusted",
                None => "settings:certificateTrust.rowHint",
            }
        });
        let enabled = !f.saving && !cert.unsaved;
        let problem = cert.problem;
        let check = btn(
            tk,
            CERT_HOST.trigger,
            &t("settings:certificateTrust.checkButton"),
            if problem.is_some() {
                Btn::Primary
            } else {
                Btn::Outline
            },
            enabled,
        )
        .press_if(enabled, move |a: &mut App| {
            let Some(b) = a.settings.backend.initial.clone() else {
                return;
            };
            certtrust::open(a, &CERT_HOST, &b.id, &b.name, problem);
        });
        status.push(
            col()
                .w_full()
                .gap(tk.r(0.375))
                .pt(tk.r(0.5))
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .border_layout()
                .child(
                    text(t("settings:certificateTrust.rowLabel"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                )
                .child(muted(tk, hint))
                .child(row().w_full().pt(tk.r(0.25)).child(check)),
        );
    }
    let mut side = vec![super::subpage::side_card(
        tk,
        "set-be-status",
        Some(t("common:field.status")),
        status,
    )];
    if d.tp.net_kind != Net::Ln {
        side.push(super::subpage::side_card(
            tk,
            "set-be-privacy",
            None,
            vec![
                text(trust.label.clone())
                    .font(tk.tsm(600.0))
                    .color(posture_color(tk, trust.posture)),
                muted(tk, trust.note.clone()),
            ],
        ));
    }
    if let Some(b) = &f.initial {
        let del_label = tr("settings:backendModal.deleteBackend", &[("name", &b.name)]);
        let del = btn(
            tk,
            "set-be-delete",
            &t("common:actions.delete"),
            Btn::Outline,
            f.deleting.is_none(),
        )
        .label(del_label.clone())
        .help(del_label)
        .press_if(f.deleting.is_none(), delete);
        side.push(row().w_full().child(del));
    }
    certtrust::mount(app, tk, overlays, &CERT_HOST);

    // Footer bar: Delete left; Cancel and Save right.
    let ok = can_add(app, &d) && !f.saving && !testing;
    let save_label = t(if f.saving {
        "settings:backendModal.connecting"
    } else if f.test == "ok" {
        if d.editing {
            "settings:backendModal.saveBackend"
        } else {
            "settings:backendModal.addSyncBackend"
        }
    } else {
        "settings:backendModal.connectAndSave"
    });
    // Editing: the backend's own name as the title, its type and network as the subtitle.
    let (title, description) = match &f.initial {
        Some(b) => (
            b.name.clone(),
            format!(
                "{} · {}",
                d.preset
                    .map(|p| p.label.to_owned())
                    .unwrap_or_else(|| d.kind.clone()),
                d.tp.label
            ),
        ),
        None => (
            t(&format!("{copy_key}.title")),
            t(&format!("{copy_key}.description")),
        ),
    };
    super::subpage::Parts {
        title,
        description: Some(description),
        head_extra: None,
        body,
        footer_left: None,
        aside: None,
        side,
        actions: vec![
            super::subpage::cancel_button(tk, "set-be-cancel", true),
            btn_lg(tk, "set-be-save", &save_label, Btn::Primary, ok).press_if(ok, save),
        ],
        scroll_key: "set-be-scroll".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Core backend on a non-preset URL edits as `custom`; its credentials stay part of the
    /// form, so saving a name change doesn't clear them.
    #[test]
    fn a_custom_url_core_backend_keeps_its_credentials_in_the_form() {
        let mut app = crate::app::tests::quiet_app();
        let b = crate::settings::model::Backend {
            id: "core".into(),
            name: "My node".into(),
            url: "http://10.0.0.5:8332".into(),
            kind: Some("bitcoinrpc".into()),
            auth: "basic".into(),
            username: Some("rpcuser".into()),
            ..Default::default()
        };
        open_edit(&mut app, &b);
        assert!(derive(&app).show_auth);
    }
}

#[cfg(test)]
mod stale_test_tests {
    use super::*;

    /// "Connect and save" tested endpoint A; the form changed before A answered: A's success
    /// is no result for the current values and saves nothing.
    #[test]
    fn an_earlier_values_test_saves_nothing() {
        let mut app = crate::app::tests::quiet_app();
        app.settings
            .backend_tests
            .push_back("ui.backends.http.test{\"url\":\"A\"}".into());
        let f = &mut app.settings.backend;
        f.test = "testing";
        f.save_after_test = true;
        f.saving = true;
        let ok = Reply {
            kind: "ui.backends.http.test".into(),
            data: json!({"ok": true}),
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        on_reply(&mut app, "settings.backend_test_http", &ok);
        let f = &app.settings.backend;
        assert_eq!(f.test, "idle");
        assert!(!f.save_after_test && !f.saving, "nothing saved");
    }
}
