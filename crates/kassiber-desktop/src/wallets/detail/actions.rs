//! The page's actions, as `ConnectionDetailView` runs them: `onSync` (Refresh / Full rescan /
//! the empty states' Refresh; network egress, only ever from a press), the edit / remove /
//! reveal submits, the import rollback, and opening a UTXO's transaction.

use rune::TextEdit;
use serde_json::{Map, Value, json};

use super::{
    CHANGE_ACK, CLEAR_BACKEND, DELETE_ACK, EditForm, MAX_GAP_LIMIT, REVEAL_ACK, Rollback, s,
};
use crate::app::{App, Reply, ToastTone};
use crate::i18n::{t, tr};
use crate::settings::certtrust::{problem_from_code, problem_from_text};
use crate::wallets::conn::Conn;

fn conn(app: &App) -> Option<Conn> {
    crate::wallets::conn::find(app, &app.wallets.detail.wallet)
}

/// `BARE_XPUB_SCRIPT_TYPES`, modern first.
pub const SCRIPT_TYPES: [&str; 4] = ["p2wpkh", "p2sh-p2wpkh", "p2pkh", "p2tr"];

pub fn script_type_label(v: &str) -> String {
    t(match v {
        "p2wpkh" => "connections:add.descriptor.scriptType.p2wpkh",
        "p2sh-p2wpkh" => "connections:add.descriptor.scriptType.p2shp2wpkh",
        "p2pkh" => "connections:add.descriptor.scriptType.p2pkh",
        _ => "connections:add.descriptor.scriptType.p2tr",
    })
}

/// The script types the wallet watches, canonical order (non-empty only for xpub wallets).
pub fn wallet_script_types(app: &App, c: &Conn) -> Vec<String> {
    let have: Vec<String> = super::wallet_detail(app, c)
        .and_then(|w| w.get("script_types"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    SCRIPT_TYPES
        .iter()
        .filter(|v| have.iter().any(|h| h == *v))
        .map(|v| v.to_string())
        .collect()
}

/// `detectWalletMaterial`: (kind, label, hint), the label and hint translated (the setup
/// form's detector).
pub fn detect_material(value: &str) -> (&'static str, String, Option<String>) {
    use crate::wallets::add::libs::wallet_material::{Kind, detect};
    let d = detect(value);
    let kind = match d.kind {
        Kind::Empty => "empty",
        Kind::DescriptorJson => "descriptor-json",
        Kind::Bsms => "bsms",
        Kind::Descriptor => "descriptor",
        Kind::Slip132 => "slip132",
        Kind::BareXpub => "bare-xpub",
        Kind::Unknown => "unknown",
    };
    (kind, d.label, d.hint)
}

/// `editConfigKindForConnection`.
pub fn edit_kind(c: &Conn) -> Option<&'static str> {
    let source = c.sync_source.as_deref().unwrap_or("");
    let mode = c.sync_mode.as_deref().unwrap_or("");
    let fmt = c.source_format.as_deref().unwrap_or("");
    if c.kind == "btcpay" || source == "btcpay" || mode == "btcpay" {
        return Some("btcpay");
    }
    if c.kind == "descriptor" || c.kind == "xpub" {
        return Some("descriptor");
    }
    if mode == "file_import"
        || !fmt.is_empty()
        || matches!(c.kind.as_str(), "river" | "phoenix" | "csv")
    {
        return Some("file-wallet");
    }
    None
}

/// `isWalletLiveBackendSource`.
pub fn live_backend_source(app: &App, c: &Conn) -> bool {
    let mode = super::sync_mode(app, c);
    matches!(
        mode.as_str(),
        "backend_descriptor" | "backend_addresses" | "backend_silent_payment"
    ) || matches!(
        c.kind.as_str(),
        "descriptor" | "xpub" | "address" | "samourai"
    )
}

/// `backendOptionChain`.
fn backend_chain(b: &Value) -> String {
    if let Some(c) = s(b, "chain") {
        return c.trim().to_lowercase();
    }
    let name = s(b, "name").unwrap_or("").trim().to_lowercase();
    if name == "liquid" || name == "liquid-blockstream" {
        "liquid".into()
    } else {
        "bitcoin".into()
    }
}

/// `liveBackendOptions`: the wallet chain's backends, no BTCPay / Lightning.
pub fn live_backends(app: &App, c: &Conn) -> Vec<Value> {
    let chain = super::wallet_chain(app, c);
    app.wallets
        .detail
        .backends
        .iter()
        .filter(|b| {
            let kind = s(b, "kind").unwrap_or("").trim().to_lowercase();
            !matches!(kind.as_str(), "btcpay" | "coreln" | "lnd") && backend_chain(b) == chain
        })
        .cloned()
        .collect()
}

pub fn btcpay_backends(app: &App) -> Vec<Value> {
    app.wallets
        .detail
        .backends
        .iter()
        .filter(|b| s(b, "kind") == Some("btcpay"))
        .cloned()
        .collect()
}

/// `backendOptionLabel`.
pub fn backend_label(b: &Value) -> String {
    let name = s(b, "name").unwrap_or("");
    let label = s(b, "display_name")
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .unwrap_or(name);
    if label == name {
        label.to_owned()
    } else {
        format!("{label} ({name})")
    }
}

/// `canClearLiveBackend`.
pub fn can_clear_backend(app: &App, c: &Conn) -> bool {
    live_backend_source(app, c)
        && super::wallet_chain(app, c) != "liquid"
        && super::wallet_detail(app, c)
            .and_then(|w| w.get("backend"))
            .and_then(|b| s(b, "source"))
            == Some("explicit")
}

/// `effectiveEditBackendKind`.
pub fn effective_backend_kind(app: &App, c: &Conn, e: &EditForm) -> Option<String> {
    let opts = live_backends(app, c);
    let selected = if e.backend == CLEAR_BACKEND {
        opts.iter()
            .find(|b| b.get("is_default").and_then(Value::as_bool) == Some(true))
    } else {
        opts.iter()
            .find(|b| s(b, "name") == Some(e.backend.as_str()))
    };
    match selected {
        Some(b) => s(b, "kind").map(str::to_owned),
        None if e.backend.is_empty() => super::wallet_detail(app, c)
            .and_then(|w| w.get("backend"))
            .and_then(|b| s(b, "kind"))
            .map(str::to_owned),
        None => None,
    }
}

/// `btcpayProvenanceRouteKey`.
pub fn route_key(r: &Value) -> String {
    format!(
        "{}\u{0}{}\u{0}{}",
        s(r, "backend").unwrap_or(""),
        s(r, "store_id").unwrap_or(""),
        s(r, "payment_method_id").unwrap_or("")
    )
}

pub fn provenance_routes(app: &App, c: &Conn) -> Vec<Value> {
    super::wallet_detail(app, c)
        .and_then(|w| w.get("btcpay_provenance"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

pub fn is_deprecated(app: &App, c: &Conn) -> bool {
    super::wallet_detail(app, c)
        .and_then(|w| w.get("deprecated"))
        .and_then(Value::as_bool)
        .unwrap_or(c.deprecated)
}

fn birthday_of(app: &App, c: &Conn) -> String {
    super::wallet_detail(app, c)
        .and_then(|w| s(w, "birthday"))
        .map(|b| b.chars().take(10).collect())
        .unwrap_or_default()
}

// ---- Sync -----------------------------------------------------------------------------------

/// `onSync`: the Refresh button, Full rescan, and the empty states' Refresh. Network egress.
pub fn sync(app: &mut App, force_full: bool) {
    let Some(c) = conn(app) else { return };
    if super::exchange_kind(app, &c).is_some() {
        // Exchange wallets sync from their exchange through the sync card's own request.
        return;
    }
    if super::backend_required(app, &c) {
        app.wallets.detail.sync_error = Some(t("connections:detail.sync.backendRequiredBody"));
        app.wallets.detail.cert_problem = None;
        return;
    }
    if app.wallets.detail.syncing || app.wallets.detail.core_testing || app.busy.syncing {
        return;
    }
    let detail = super::wallet_detail(app, &c).cloned();
    let backend = detail
        .as_ref()
        .and_then(|w| w.get("backend"))
        .cloned()
        .unwrap_or(Value::Null);
    if force_full && s(&backend, "kind") == Some("bitcoinrpc") && s(&backend, "name").is_some() {
        // A rescan through Bitcoin Core first checks the node can cover the wallet's history.
        let mut args = json!({"backend": s(&backend, "name")});
        if let Some(b) = detail.as_ref().and_then(|w| s(w, "birthday")) {
            args["birthday"] = json!(b);
        }
        if app
            .send(
                "ui.backends.bitcoinrpc.test",
                args,
                "wallets.detail.core_test",
            )
            .is_some()
        {
            app.wallets.detail.core_testing = true;
            app.wallets.detail.core_test_wallet = Some(app.wallets.detail.wallet.clone());
        }
        return;
    }
    start_sync(app, &c, force_full);
}

fn start_sync(app: &mut App, c: &Conn, force_full: bool) {
    app.wallets.detail.sync_error = None;
    app.wallets.detail.cert_problem = None;
    let mut args = json!({"wallet": c.label, "force_full": force_full});
    args["expected_scope"] = app.expected_scope();
    let tag = if force_full {
        "wallets.detail.sync_full"
    } else {
        "wallets.detail.sync"
    };
    if app.send("ui.wallets.sync", args, tag).is_some() {
        app.wallets.detail.syncing = true;
        app.wallets.detail.sync_wallet = Some(c.id.clone());
    }
}

/// The wallet's own result (`describeWalletSyncResult`): its sentence, then the journal step's
/// ("Journals updated; 2 transactions in quarantine."), and whether it did not fail.
fn sync_result_message(r: &Value, label: &str) -> (bool, String) {
    let results = r
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let res = results.iter().find(|x| s(x, "wallet") == Some(label));
    let status = res.and_then(|x| s(x, "status")).unwrap_or("error");
    (
        status != "error",
        crate::shell::maint::describe_wallet_sync(res, label, &r["journals"]),
    )
}

// ---- Edit / delete / reveal -----------------------------------------------------------------

fn edit_text(s: &str) -> TextEdit {
    TextEdit::new(s)
}

/// `openEditDialog`.
pub fn open_edit(app: &mut App) {
    let Some(c) = conn(app) else { return };
    let mut label = edit_text(&c.label);
    label.focused = true;
    let form = EditForm {
        label,
        script_types: super::wallet_detail(app, &c)
            .and_then(|w| w.get("script_types"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        gap: edit_text(&c.gap.map(|g| format!("{}", g as i64)).unwrap_or_default()),
        birthday: edit_text(&birthday_of(app, &c)),
        deprecated: is_deprecated(app, &c),
        ..EditForm::default()
    };
    let d = &mut app.wallets.detail;
    d.menu = false;
    d.edit = Some(form);
}

pub fn open_delete(app: &mut App) {
    let d = &mut app.wallets.detail;
    d.menu = false;
    d.delete = Some(super::DeleteForm::default());
}

pub fn open_reveal(app: &mut App) {
    let d = &mut app.wallets.detail;
    d.menu = false;
    d.reveal = Some(super::RevealForm::default());
}

fn edit_error(app: &mut App, msg: String) {
    if let Some(e) = app.wallets.detail.edit.as_mut() {
        e.error = Some(msg);
    }
}

fn auth(app: &App, passphrase: &str, ack_key: &str, ack: &str) -> Value {
    if super::encrypted(app) {
        json!({"passphrase_secret": passphrase})
    } else {
        json!({ ack_key: ack })
    }
}

/// `onEditSubmit`.
pub fn submit_edit(app: &mut App) {
    let Some(c) = conn(app) else { return };
    let enc = super::encrypted(app);
    let Some(e) = app.wallets.detail.edit.as_ref() else {
        return;
    };
    if e.pending {
        return;
    }
    let next_label = e.label.text.trim().to_owned();
    if next_label.is_empty() {
        return edit_error(app, t("connections:detail.edit.errorEnterLabel"));
    }
    if enc && e.passphrase.text.is_empty() {
        return edit_error(app, t("connections:detail.auth.enterPassphrase"));
    }
    if !enc && e.ack.text.trim() != CHANGE_ACK {
        return edit_error(
            app,
            tr(
                "connections:detail.edit.errorPlaintextAck",
                &[("ack", CHANGE_ACK)],
            ),
        );
    }
    let kind = edit_kind(&c);
    let label_changed = next_label != c.label;
    let material = e.material.text.trim().to_owned();
    let gap_text = e.gap.text.trim().to_owned();
    let birthday = e.birthday.text.trim().to_owned();
    let store_id = e.store_id.text.trim().to_owned();
    let payment = e.payment_method.text.trim().to_owned();
    let backend = e.backend.trim().to_owned();
    let source_file = e.source_file.text.trim().to_owned();
    let mut changes = Map::new();
    let mut clear: Vec<&str> = vec![];
    let current_birthday = birthday_of(app, &c);
    let wallet_types = wallet_script_types(app, &c);
    let xpub_wallet = !wallet_types.is_empty();
    if kind == Some("descriptor") && !material.is_empty() {
        let (dk, dlabel, dhint) = detect_material(&material);
        if dk == "unknown" {
            return edit_error(app, dhint.unwrap_or(dlabel));
        }
        if dk == "bare-xpub" && e.script_types.is_empty() {
            return edit_error(app, t("connections:detail.edit.errorSelectScriptType"));
        }
        changes.insert("wallet_material".into(), json!(material));
        if dk == "bare-xpub" {
            changes.insert("script_types".into(), json!(e.script_types));
        }
    } else if kind == Some("descriptor") && xpub_wallet && !same_set(&e.script_types, &wallet_types)
    {
        if e.script_types.is_empty() {
            return edit_error(app, t("connections:detail.edit.errorSelectScriptType"));
        }
        changes.insert("script_types".into(), json!(e.script_types));
    }
    if kind == Some("descriptor") && !gap_text.is_empty() {
        match gap_text.parse::<i64>() {
            Ok(g) if g > 0 => {
                if g > MAX_GAP_LIMIT {
                    return edit_error(
                        app,
                        tr(
                            "connections:detail.edit.errorGapMax",
                            &[("max", &super::count(MAX_GAP_LIMIT as f64))],
                        ),
                    );
                }
                if c.gap.map(|x| x as i64) != Some(g) {
                    changes.insert("gap_limit".into(), json!(g));
                }
            }
            _ => return edit_error(app, t("connections:detail.edit.errorGapPositive")),
        }
    }
    let backend_kind = effective_backend_kind(app, &c, e);
    if (kind == Some("descriptor") || backend_kind.as_deref() == Some("bitcoinrpc"))
        && birthday != current_birthday
    {
        if birthday.is_empty() {
            clear.push("birthday");
        } else {
            changes.insert("birthday".into(), json!(birthday));
        }
    }
    if kind == Some("btcpay") {
        if !store_id.is_empty() {
            changes.insert("store_id".into(), json!(store_id));
        }
        if !payment.is_empty() {
            changes.insert("payment_method_id".into(), json!(payment));
        }
        if !backend.is_empty() {
            changes.insert("backend".into(), json!(backend));
        }
    }
    if live_backend_source(app, &c) && kind != Some("btcpay") {
        let wb = super::wallet_detail(app, &c)
            .and_then(|w| w.get("backend"))
            .cloned()
            .unwrap_or(Value::Null);
        if backend == CLEAR_BACKEND {
            clear.push("backend");
        } else if !backend.is_empty()
            && (Some(backend.as_str()) != s(&wb, "name") || s(&wb, "source") != Some("explicit"))
        {
            changes.insert("backend".into(), json!(backend));
        }
    }
    if kind == Some("file-wallet") && !source_file.is_empty() {
        changes.insert("source_file".into(), json!(source_file));
    }
    let routes = provenance_routes(app, &c);
    if e.clear_provenance && !routes.is_empty() {
        clear.push("btcpay_provenance");
    } else if !e.removed_routes.is_empty() {
        let next: Vec<Value> = routes
            .iter()
            .filter(|r| !e.removed_routes.contains(&route_key(r)))
            .cloned()
            .collect();
        if next.is_empty() {
            clear.push("btcpay_provenance");
        } else if next.len() != routes.len() {
            changes.insert("btcpay_provenance".into(), json!(next));
        }
    }
    if e.deprecated != is_deprecated(app, &c) {
        changes.insert("deprecated".into(), json!(e.deprecated));
    }
    if !label_changed && changes.is_empty() && clear.is_empty() {
        return edit_error(app, t("connections:detail.edit.errorChangeNothing"));
    }
    let mut args = Map::new();
    args.insert("wallet".into(), json!(c.id));
    if label_changed {
        args.insert("label".into(), json!(next_label));
    }
    args.extend(changes);
    if !clear.is_empty() {
        args.insert("clear".into(), json!(clear));
    }
    args.insert(
        "auth_response".into(),
        auth(app, &e.passphrase.text, "plaintext_change_ack", CHANGE_ACK),
    );
    args.insert("expected_scope".into(), app.expected_scope());
    if app
        .send(
            "ui.wallets.update",
            Value::Object(args),
            "wallets.detail.update",
        )
        .is_some()
        && let Some(e) = app.wallets.detail.edit.as_mut()
    {
        e.error = None;
        e.pending = true;
    }
}

fn same_set(a: &[String], b: &[String]) -> bool {
    a.len() == b.len() && b.iter().all(|x| a.contains(x))
}

/// `onDeleteSubmit`.
pub fn submit_delete(app: &mut App) {
    let Some(c) = conn(app) else { return };
    let enc = super::encrypted(app);
    let Some(d) = app.wallets.detail.delete.as_ref() else {
        return;
    };
    if d.pending {
        return;
    }
    let err = if d.confirm.text.trim() != c.label {
        Some(tr(
            "connections:detail.delete.errorConfirmLabel",
            &[("label", &c.label)],
        ))
    } else if enc && d.passphrase.text.is_empty() {
        Some(t("connections:detail.auth.enterPassphrase"))
    } else if !enc && d.ack.text.trim() != DELETE_ACK {
        Some(tr(
            "connections:detail.delete.errorPlaintextAck",
            &[("ack", DELETE_ACK)],
        ))
    } else {
        None
    };
    if let Some(m) = err {
        if let Some(d) = app.wallets.detail.delete.as_mut() {
            d.error = Some(m);
        }
        return;
    }
    let args = json!({
        "wallet": c.id,
        "confirm": "DELETE",
        "confirm_wallet": c.label,
        "cascade": true,
        "auth_response": auth(app, &d.passphrase.text, "plaintext_delete_ack", DELETE_ACK),
        "expected_scope": app.expected_scope(),
    });
    if app
        .send("ui.wallets.delete", args, "wallets.detail.delete")
        .is_some()
        && let Some(d) = app.wallets.detail.delete.as_mut()
    {
        d.error = None;
        d.pending = true;
    }
}

/// `onRevealSubmit`: the daemon returns the stored material, which goes straight to the
/// clipboard (never into state, logs or the screen).
pub fn submit_reveal(app: &mut App) {
    let Some(c) = conn(app) else { return };
    let enc = super::encrypted(app);
    let Some(r) = app.wallets.detail.reveal.as_ref() else {
        return;
    };
    if r.pending || r.copied {
        return;
    }
    let err = if enc && r.passphrase.text.is_empty() {
        Some(t("connections:detail.auth.enterPassphrase"))
    } else if !enc && r.ack.text.trim() != REVEAL_ACK {
        Some(tr(
            "connections:detail.reveal.errorPlaintextAck",
            &[("ack", REVEAL_ACK)],
        ))
    } else {
        None
    };
    if let Some(m) = err {
        if let Some(r) = app.wallets.detail.reveal.as_mut() {
            r.error = Some(m);
        }
        return;
    }
    let args = json!({
        "wallet": c.id,
        "auth_response": auth(app, &r.passphrase.text, "plaintext_reveal_ack", REVEAL_ACK),
        "expected_scope": app.expected_scope(),
    });
    if app
        .send("wallets.reveal_descriptor", args, "wallets.detail.reveal")
        .is_some()
        && let Some(r) = app.wallets.detail.reveal.as_mut()
    {
        r.error = None;
        r.pending = true;
    }
}

// ---- Imports rollback -----------------------------------------------------------------------

/// `openRollback`: the dry-run plan says what the rollback deletes.
pub fn open_rollback(app: &mut App, batch: Value) {
    let id = s(&batch, "id").unwrap_or("").to_owned();
    app.wallets.detail.rollback = Some(Rollback {
        batch,
        plan: None,
        plan_pending: true,
        pending: false,
        error: None,
    });
    app.send(
        "ui.imports.rollback",
        json!({"batch": id, "dry_run": true}),
        "wallets.detail.rollback_plan",
    );
}

pub fn confirm_rollback(app: &mut App) {
    let Some(r) = app.wallets.detail.rollback.as_ref() else {
        return;
    };
    if r.pending || r.plan_pending || r.plan.is_none() {
        return;
    }
    let id = s(&r.batch, "id").unwrap_or("").to_owned();
    let args = json!({"batch": id, "confirm": "DELETE", "expected_scope": app.expected_scope()});
    if app
        .send("ui.imports.rollback", args, "wallets.detail.rollback")
        .is_some()
        && let Some(r) = app.wallets.detail.rollback.as_mut()
    {
        r.pending = true;
        r.error = None;
    }
}

// ---- UTXO → transaction ----------------------------------------------------------------------

/// `openUtxoTransaction`: resolve the reference, then open it in the docked detail.
pub fn open_utxo_transaction(app: &mut App, reference: &str) {
    app.wallets.detail.pending_utxo = Some(reference.to_owned());
    app.send(
        "ui.transactions.resolve",
        json!({"query": reference}),
        "wallets.detail.resolve",
    );
}

// ---- Replies ---------------------------------------------------------------------------------

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if crate::settings::certtrust::on_reply(app, &super::CERT_HOST, tag, r) {
        return;
    }
    match tag {
        "wallets.detail.sync" | "wallets.detail.sync_full" => {
            app.wallets.detail.syncing = false;
            // Its notice names the wallet it was for; its error shows only on that wallet's page.
            let sent_for = app.wallets.detail.sync_wallet.take();
            let here = sent_for
                .as_ref()
                .is_none_or(|w| *w == app.wallets.detail.wallet);
            let label = match &sent_for {
                Some(w) => crate::wallets::conn::find(app, w),
                None => conn(app),
            }
            .map(|c| c.label)
            .unwrap_or_default();
            let force = tag == "wallets.detail.sync_full";
            match &r.error {
                Some(e) => {
                    if here {
                        app.wallets.detail.sync_error = Some(e.clone());
                        app.wallets.detail.cert_problem =
                            problem_from_code(r.error_detail.get("code").and_then(Value::as_str))
                                .or_else(|| problem_from_text(Some(e)));
                    }
                    app.toast(
                        ToastTone::Error,
                        &tr(
                            "chrome:notifications.condition.connectionSyncFailed.title_one",
                            &[("count", "1")],
                        ),
                        Some(label),
                        Some("connection-sync"),
                    );
                }
                None => {
                    let (ok, msg) = sync_result_message(&r.data, &label);
                    if ok {
                        let title = t(if force {
                            "chrome:notifications.result.connectionSynced.rescanTitle"
                        } else {
                            "chrome:notifications.result.connectionSynced.title"
                        });
                        app.toast(
                            ToastTone::Success,
                            &title,
                            Some(label),
                            Some("connection-sync"),
                        );
                        // A journal step that did not finish gets its own notice (`recordJournalStep`).
                        crate::shell::maint::record_journal_step(app, &r.data["journals"]);
                    } else {
                        if here {
                            app.wallets.detail.sync_error = Some(msg);
                            // The wallet's own result: its code, else the daemon's fixed text.
                            let res =
                                r.data
                                    .get("results")
                                    .and_then(Value::as_array)
                                    .and_then(|a| {
                                        a.iter().find(|x| s(x, "wallet") == Some(label.as_str()))
                                    });
                            app.wallets.detail.cert_problem = res.and_then(|x| {
                                problem_from_code(s(x, "code"))
                                    .or_else(|| problem_from_text(s(x, "message")))
                            });
                        }
                        app.toast(
                            ToastTone::Error,
                            &tr(
                                "chrome:notifications.condition.connectionSyncFailed.title_one",
                                &[("count", "1")],
                            ),
                            Some(label),
                            Some("connection-sync"),
                        );
                    }
                }
            }
            super::reload(app);
        }
        "wallets.detail.core_test" => {
            // The rescan was asked for one wallet: after navigating to another, the probe's
            // result starts nothing (an exact action, never carried to another wallet).
            let sent_for = app.wallets.detail.core_test_wallet.take();
            app.wallets.detail.core_testing = false;
            if sent_for.as_deref() != Some(app.wallets.detail.wallet.as_str()) {
                return;
            }
            let Some(c) = conn(app) else { return };
            let birthday = super::wallet_detail(app, &c)
                .and_then(|w| s(w, "birthday"))
                .map(str::to_owned);
            let blocked = match &r.error {
                Some(e) => {
                    if r.error_detail.get("code").and_then(Value::as_str)
                        == Some("bitcoinrpc_pruned_below_birthday")
                    {
                        let retained = r
                            .error_detail
                            .get("details")
                            .and_then(|d| d.get("earliest_retained_at"))
                            .and_then(Value::as_str)
                            .map(|x| x.chars().take(10).collect::<String>())
                            .unwrap_or_else(|| t("connections:detail.sync.retainedDateUnknown"));
                        let b = birthday
                            .as_deref()
                            .map(|x| x.chars().take(10).collect::<String>())
                            .unwrap_or_else(|| t("connections:detail.sync.retainedDateUnknown"));
                        Some(tr(
                            "connections:detail.sync.rescanPrunedBlocked",
                            &[("birthday", &b), ("retained", &retained)],
                        ))
                    } else {
                        Some(e.clone())
                    }
                }
                None if r.data.get("reachable").and_then(Value::as_bool) != Some(true) => Some(
                    r.data
                        .get("error")
                        .and_then(|e| s(e, "message"))
                        .map(str::to_owned)
                        .unwrap_or_else(|| t("connections:detail.sync.rescanCoverageFailed")),
                ),
                None if r.data.get("pruned").and_then(Value::as_bool) == Some(true)
                    && birthday.is_none() =>
                {
                    Some(t("connections:detail.sync.rescanBirthdayRequired"))
                }
                None => None,
            };
            match blocked {
                Some(m) => {
                    app.wallets.detail.sync_error = Some(m);
                    app.wallets.detail.cert_problem = None;
                }
                None => start_sync(app, &c, true),
            }
        }
        "wallets.detail.update" => {
            let Some(c) = conn(app) else { return };
            match &r.error {
                Some(e) => {
                    if let Some(f) = app.wallets.detail.edit.as_mut() {
                        f.pending = false;
                        f.error = Some(if e.is_empty() {
                            t("connections:detail.edit.couldNotChange")
                        } else {
                            e.clone()
                        });
                    }
                }
                None => {
                    let next = app
                        .wallets
                        .detail
                        .edit
                        .as_ref()
                        .map(|e| e.label.text.trim().to_owned())
                        .unwrap_or_default();
                    let body = if !next.is_empty() && next != c.label {
                        tr(
                            "connections:detail.edit.renamed",
                            &[("from", &c.label), ("to", &next)],
                        )
                    } else {
                        tr("connections:detail.edit.updated", &[("label", &c.label)])
                    };
                    app.toast(
                        ToastTone::Success,
                        &t("connections:detail.edit.changedTitle"),
                        Some(body),
                        Some("connection-edit"),
                    );
                    app.wallets.detail.edit = None;
                    super::reload(app);
                }
            }
        }
        "wallets.detail.delete" => match &r.error {
            Some(e) => {
                if let Some(d) = app.wallets.detail.delete.as_mut() {
                    d.pending = false;
                    d.error = Some(if e.is_empty() {
                        t("connections:detail.delete.couldNotRemove")
                    } else {
                        e.clone()
                    });
                }
            }
            None => {
                let label = conn(app).map(|c| c.label).unwrap_or_default();
                app.toast(
                    ToastTone::Success,
                    &t("connections:detail.delete.removedTitle"),
                    Some(tr(
                        "connections:detail.delete.removedBody",
                        &[("label", &label)],
                    )),
                    Some("connection-delete"),
                );
                app.wallets.detail = super::State::default();
                // `navigate({ to: "/connections", replace: true })`.
                app.route = crate::app::Route("/connections".into());
                app.refresh_reads();
            }
        },
        "wallets.detail.reveal" => {
            let label = conn(app).map(|c| c.label).unwrap_or_default();
            let Some(rv) = app.wallets.detail.reveal.as_mut() else {
                return;
            };
            rv.pending = false;
            match &r.error {
                Some(e) => {
                    rv.error = Some(if e.is_empty() {
                        t("connections:detail.reveal.couldNotCopy")
                    } else {
                        e.clone()
                    })
                }
                None => {
                    // `walletDescriptorMaterialFromReveal`.
                    let material = s(&r.data, "wallet_material")
                        .map(|m| m.trim().to_owned())
                        .filter(|m| !m.is_empty())
                        .unwrap_or_else(|| {
                            [s(&r.data, "descriptor"), s(&r.data, "change_descriptor")]
                                .into_iter()
                                .flatten()
                                .map(str::trim)
                                .filter(|x| !x.is_empty())
                                .collect::<Vec<_>>()
                                .join("\n")
                        });
                    if material.is_empty() {
                        rv.error = Some(t("connections:detail.reveal.errorMissingDescriptor"));
                    } else {
                        rv.copied = true;
                        rv.close_at = Some(app.clock + 0.9);
                        PENDING_COPY.with(|p| *p.borrow_mut() = Some(material));
                        app.toast(
                            ToastTone::Success,
                            &t("connections:detail.reveal.copiedTitle"),
                            Some(tr(
                                "connections:detail.reveal.copiedBody",
                                &[("label", &label)],
                            )),
                            Some("descriptor-reveal"),
                        );
                    }
                }
            }
        }
        "wallets.detail.rollback_plan" => {
            // A plan installs only into the dialog of the batch it describes: a cancelled
            // dialog's late plan must not stand in for another batch's deletions. Errors carry
            // no batch; they land only while this dialog still waits for its plan.
            let planned = r.data["batch"]["id"].as_str();
            if let Some(rb) = app.wallets.detail.rollback.as_mut()
                && rb.plan_pending
                && (r.error.is_some() || planned.is_some_and(|p| Some(p) == s(&rb.batch, "id")))
            {
                rb.plan_pending = false;
                match &r.error {
                    Some(e) => rb.error = Some(e.clone()),
                    None => rb.plan = Some(r.data.clone()),
                }
            }
        }
        "wallets.detail.rollback" => match &r.error {
            Some(e) => {
                if let Some(rb) = app.wallets.detail.rollback.as_mut() {
                    rb.pending = false;
                    rb.error = Some(e.clone());
                }
            }
            None => {
                if let Some(rb) = app.wallets.detail.rollback.take() {
                    let n = r
                        .data
                        .get("transactions_deleted")
                        .and_then(Value::as_f64)
                        .or_else(|| super::f(&rb.batch, "rows_present"))
                        .unwrap_or(0.0) as usize;
                    let file = s(&rb.batch, "source_filename")
                        .or_else(|| s(&rb.batch, "source_format"))
                        .unwrap_or("")
                        .to_owned();
                    app.toast(
                        ToastTone::Success,
                        &t("connections:detail.imports.rolledBackTitle"),
                        Some(crate::i18n::tc(
                            "connections:detail.imports.rolledBackBody",
                            n,
                            &[("file", &file)],
                        )),
                        Some("import-rollback"),
                    );
                }
                super::reload(app);
            }
        },
        "wallets.detail.resolve" => {
            let pending = app.wallets.detail.pending_utxo.take();
            if pending.is_none() {
                return;
            }
            match &r.error {
                Some(e) => app.toast(
                    ToastTone::Warning,
                    &t("connections:detail.utxoTransaction.notOpenedTitle"),
                    Some(if e.is_empty() {
                        t("connections:detail.utxoTransaction.couldNotResolve")
                    } else {
                        e.clone()
                    }),
                    Some("utxo-transaction-open"),
                ),
                None => match r
                    .data
                    .get("transaction")
                    .and_then(|t| s(t, "id"))
                    .map(str::to_owned)
                {
                    Some(id) => crate::txdetail::open(app, &id, "details"),
                    None => app.toast(
                        ToastTone::Warning,
                        &t("connections:detail.utxoTransaction.notFoundTitle"),
                        Some(t("connections:detail.utxoTransaction.notFoundBody")),
                        Some("utxo-transaction-open"),
                    ),
                },
            }
        }
        _ => {}
    }
}

thread_local! {
    /// Revealed wallet material on its way to the clipboard: the reply handler has no `Tx`,
    /// so the next press-free frame (`take_pending_copy` in the view's poll) hands it over.
    static PENDING_COPY: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Drops revealed material still waiting for the clipboard (the book state starts over: lock,
/// book switch).
pub fn forget_pending_copy() {
    PENDING_COPY.with(|p| p.borrow_mut().take());
}

/// Hands revealed material to the clipboard once (called from `poll`, which has a `Tx`).
pub fn take_pending_copy(app: &App, tx: &mut rune::Tx) {
    if let Some(m) = PENDING_COPY.with(|p| p.borrow_mut().take()) {
        crate::osleaks::clipboard::copy(app, tx, m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Full rescan's Bitcoin Core probe that answers after navigating to another wallet starts
    /// nothing there, even when it reports full coverage.
    #[test]
    fn a_rescan_probe_for_another_wallet_starts_nothing() {
        let mut app = crate::app::tests::quiet_app();
        app.wallets.detail.wallet = "b".into();
        app.wallets.detail.core_test_wallet = Some("a".into());
        let reply = Reply {
            kind: "ui.backends.bitcoinrpc.test".into(),
            data: serde_json::json!({"reachable": true, "pruned": false}),
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        on_reply(&mut app, "wallets.detail.core_test", &reply);
        assert!(!app.wallets.detail.syncing && app.wallets.detail.sync_error.is_none());
        assert!(app.wallets.detail.core_test_wallet.is_none());
    }

    /// Wallet A's sync answers after the page moved to wallet B: B's page stays busy until it
    /// does (no second sync starts meanwhile), and the failure names A without showing as B's.
    #[test]
    fn a_sync_for_another_wallet_answers_as_that_wallets() {
        let mut app = crate::app::tests::quiet_app();
        let snapshot = Reply {
            kind: "ui.overview.snapshot".into(),
            data: serde_json::json!({ "connections": [
                { "id": "a", "label": "Alpha" },
                { "id": "b", "label": "Beta" },
            ] }),
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        app.reads.insert("ui.overview.snapshot".into(), snapshot);
        app.wallets.detail.wallet = "a".into();
        app.wallets.detail.syncing = true;
        app.wallets.detail.sync_wallet = Some("a".into());
        let b = Conn {
            id: "b".into(),
            label: "Beta".into(),
            ..Default::default()
        };
        super::super::load(&mut app, &b);
        assert!(super::super::sync_running(&app), "A's sync still runs");
        let reply = Reply {
            kind: "ui.wallets.sync".into(),
            data: Value::Null,
            error: Some("backend unreachable".into()),
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        on_reply(&mut app, "wallets.detail.sync", &reply);
        let d = &app.wallets.detail;
        assert!(!d.syncing && d.sync_wallet.is_none());
        assert!(d.sync_error.is_none(), "not B's error");
        let toast = app.toasts.last().expect("a notice");
        assert_eq!(toast.body.as_deref(), Some("Alpha"));
    }

    /// A rollback plan for import A that arrives after the dialog moved to import B is not shown
    /// as B's plan (confirm would delete B on A's numbers); B's own plan is.
    #[test]
    fn a_rollback_plan_installs_only_for_its_batch() {
        let mut app = crate::app::tests::quiet_app();
        app.wallets.detail.rollback = Some(super::super::Rollback {
            batch: serde_json::json!({"id": "b", "source_name": "b.csv"}),
            plan: None,
            plan_pending: true,
            pending: false,
            error: None,
        });
        let plan = |id: &str| Reply {
            kind: "ui.imports.rollback".into(),
            data: serde_json::json!({"batch": {"id": id}, "transactions_to_delete": 3}),
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        on_reply(&mut app, "wallets.detail.rollback_plan", &plan("a"));
        let rb = app.wallets.detail.rollback.as_ref().unwrap();
        assert!(rb.plan.is_none() && rb.plan_pending);
        on_reply(&mut app, "wallets.detail.rollback_plan", &plan("b"));
        let rb = app.wallets.detail.rollback.as_ref().unwrap();
        assert!(rb.plan.is_some() && !rb.plan_pending);
    }
}

#[cfg(test)]
mod material_tests {
    /// The edit dialog's detection uses the translated setup detector: kinds unchanged, the
    /// label and hint from the i18n bundles.
    #[test]
    fn detection_is_the_setup_forms() {
        let (k, label, hint) = super::detect_material("xpub6C...");
        let d = crate::wallets::add::libs::wallet_material::detect("xpub6C...");
        assert_eq!(k, "bare-xpub");
        assert_eq!((label, hint), (d.label, d.hint));
        assert_eq!(super::detect_material("nonsense").0, "unknown");
    }
}
