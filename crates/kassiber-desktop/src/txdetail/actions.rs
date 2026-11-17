//! Mutations from the detail panel and their replies. Every call uses the web's kind and
//! argument shape (see the spec sections cited per function).

use rune::{FileDialogOptions, Tx};
use serde_json::{Value, json};

use super::draft::{self, s};
use super::{
    Reuse, Target, close, is_active, open, panel_mut, reload, reload_to, send_for, send_to,
    toast_err, toast_ok,
};
use crate::app::{App, Reply};
use crate::i18n::{t, tc, tr};

fn rec_id(app: &App) -> Option<String> {
    app.detail
        .open
        .as_ref()
        .and_then(|o| o.id().map(str::to_owned))
}

// ---- Save (§3.3, §3.4) -------------------------------------------------------------------------

pub fn has_next(app: &App) -> bool {
    let Some(id) = rec_id(app) else { return false };
    // The Review page: the next row of the list the sheet was opened from.
    if let Some(h) = super::review_host(app) {
        return h
            .ids
            .iter()
            .position(|x| *x == id)
            .is_some_and(|i| i + 1 < h.ids.len());
    }
    let nav = &app.detail.nav;
    !nav.is_empty()
        && nav
            .iter()
            .position(|x| *x == id)
            .is_none_or(|i| i + 1 < nav.len())
}

/// Save: ⌘S (`close` false) or the footer button (`close` true, or "Save & next" with `next`).
pub fn save(app: &mut App, close_after: bool, next: bool) {
    let scope = app.expected_scope();
    let Some(o) = app.detail.open.as_mut() else {
        return;
    };
    let Some(rec) = &o.record else { return };
    if o.saving || o.dirty().count == 0 {
        return;
    }
    let source_tags: Vec<String> = rec
        .get("tags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let tx_id = s(rec, "id").to_owned();
    let args = draft::save_args(&tx_id, &o.draft, &o.base, &source_tags, scope);
    o.saving = true;
    o.save_error = None;
    o.close_after_save = close_after;
    o.next_after_save = next;
    let submitted = o.draft.clone();
    app.detail.saving_ids.push_back((tx_id, submitted));
    app.send("ui.transactions.metadata.update", args, "txd.save");
}

fn save_error_text(e: &str) -> String {
    if e.is_empty() {
        t("transactions:save.couldNotSaveMetadata")
    } else {
        e.to_owned()
    }
}

pub fn on_saved(app: &mut App, r: &Reply) {
    let Some((id, submitted)) = app.detail.saving_ids.pop_front() else {
        return;
    };
    if app.detail.open.as_ref().and_then(|o| o.id()) != Some(id.as_str()) {
        // The user moved to another tab (or closed this one) while it saved: finish the save on
        // the parked panel it came from, without navigating.
        let parked = app
            .detail
            .tabs
            .iter_mut()
            .filter_map(|slot| slot.parked.as_deref_mut())
            .find(|o| o.id() == Some(id.as_str()));
        if let Some(o) = parked {
            o.saving = false;
            match &r.error {
                Some(e) => o.save_error = Some(save_error_text(e)),
                None => {
                    o.base = submitted;
                }
            }
        }
        if r.error.is_none() {
            app.refresh_reads();
        }
        return;
    }
    let Some(o) = app.detail.open.as_mut() else {
        return;
    };
    o.saving = false;
    if let Some(e) = &r.error {
        o.save_error = Some(save_error_text(e));
        return;
    }
    // Only what was submitted is saved; edits made meanwhile stay dirty.
    let saved = submitted;
    o.base = saved;
    let (close_after, next) = (o.close_after_save, o.next_after_save);
    app.refresh_reads();
    // The Review page re-reads its lists; "Save & next" picks the next row from what they return.
    if let Some(h) = super::review_host(app).cloned() {
        if close_after && !next {
            close(app);
        }
        crate::review::after_sheet_save(app, &id, next.then_some(h));
        if !next && !close_after {
            reload(app);
        }
        return;
    }
    if next {
        let nav = app.detail.nav.clone();
        let tab = app
            .detail
            .open
            .as_ref()
            .map(|o| o.tab.code())
            .unwrap_or("details");
        match nav
            .iter()
            .position(|x| *x == id)
            .map(|i| i + 1)
            .or(Some(0))
            .and_then(|i| nav.get(i))
        {
            Some(n) => open(app, &n.clone(), tab),
            None => close(app),
        }
    } else if close_after {
        close(app);
    } else {
        reload(app);
    }
}

pub fn discard(app: &mut App) {
    let Some(o) = app.detail.open.as_mut() else {
        return;
    };
    o.draft = o.base.clone();
    let d = o.draft.clone();
    super::sync_edits(app, &d);
}

// ---- Loans (§4.2, §4.5) ------------------------------------------------------------------------

pub fn loan_mark(app: &mut App, role: Option<&'static str>) {
    let Some(id) = rec_id(app) else { return };
    if let Some(o) = app.detail.open.as_mut() {
        o.busy.loan = true;
    }
    match role {
        Some(r) => send_for(
            app,
            "ui.loans.mark",
            json!({ "txid": id, "as": r }),
            "txd.loan_mark",
            Value::Null,
        ),
        None => send_for(
            app,
            "ui.loans.unmark",
            json!({ "txid": id }),
            "txd.loan_unmark",
            Value::Null,
        ),
    }
}

pub fn loan_link(app: &mut App, target: &str) {
    let Some(id) = rec_id(app) else { return };
    if let Some(o) = app.detail.open.as_mut() {
        o.busy.link = Some(target.to_owned());
    }
    send_for(
        app,
        "ui.loans.link",
        json!({ "txids": [id, target] }),
        "txd.loan_link",
        json!(target),
    );
}

pub fn unpair(app: &mut App, pair_id: &str) {
    // The Review page: the same reviewed plan/apply step as its cards' Unpair.
    if super::review_host(app).is_some() {
        return crate::review::unpair_from_sheet(app, pair_id);
    }
    if let Some(o) = app.detail.open.as_mut() {
        o.busy.unpair = true;
    }
    send_for(
        app,
        "ui.transfers.unpair",
        json!({ "pair_id": pair_id }),
        "txd.unpair",
        json!(pair_id),
    );
}

// ---- Attachments (§5.1) ------------------------------------------------------------------------

pub fn attach_file(app: &mut App, tx: &mut Tx) {
    // The file goes to the transaction shown when the picker opened, whatever is shown when it
    // returns.
    let to = super::active_target(app, Value::Null);
    if to.tx.is_empty() {
        return;
    }
    if let Some(o) = app.detail.open.as_mut() {
        o.busy.attach = true;
        o.attach_error = None;
    }
    let options = FileDialogOptions {
        title: t("transactions:attachments.pickerTitle"),
        ..Default::default()
    };
    tx.open_file(options, move |a: &mut App, result, _tx| match result {
        Ok(Some(path)) => {
            let id = to.tx.clone();
            send_to(
                a,
                to,
                "ui.attachments.add",
                json!({ "transaction": id, "file_path": path.to_string_lossy() }),
                "txd.attach_file",
            );
        }
        Ok(None) => {
            if let Some(o) = panel_mut(a, &to) {
                o.busy.attach = false;
            }
        }
        Err(e) => {
            if let Some(o) = panel_mut(a, &to) {
                o.busy.attach = false;
                let msg = e.to_string();
                o.attach_error = Some(if msg.is_empty() {
                    t("transactions:attachments.pickerError")
                } else {
                    msg
                });
            }
        }
    });
}

/// Validates the links dialog text: (urls, errors).
pub fn parse_links(text: &str) -> (Vec<String>, Vec<String>) {
    let mut urls = vec![];
    let mut errors = vec![];
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let scheme = line
            .split_once("://")
            .map(|(s, rest)| (s.to_ascii_lowercase(), rest));
        match scheme {
            Some((s, rest))
                if matches!(s.as_str(), "http" | "https" | "ipfs")
                    && !rest.is_empty()
                    && !rest.contains(' ') =>
            {
                urls.push(line.to_owned())
            }
            Some((s, rest))
                if !rest.is_empty()
                    && s.chars()
                        .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)) =>
            {
                errors.push(tr(
                    "transactions:attachments.linksDialog.invalidProtocol",
                    &[("url", line)],
                ))
            }
            _ => errors.push(tr(
                "transactions:attachments.linksDialog.invalidUrl",
                &[("url", line)],
            )),
        }
    }
    (urls, errors)
}

pub fn attach_links(app: &mut App) {
    let (urls, errors) = parse_links(&app.detail.links.text);
    let Some(id) = rec_id(app) else { return };
    let o = app.detail.open.as_mut().unwrap();
    if let Some(e) = errors.first() {
        o.links_error = Some(e.clone());
        return;
    }
    if urls.is_empty() {
        o.links_error = Some(t("transactions:attachments.linksDialog.addAtLeastOne"));
        return;
    }
    o.busy.links = urls.len();
    o.links_error = None;
    // The batch size, for the toast when the panel is gone by the last reply.
    let n = urls.len();
    for u in urls {
        send_for(
            app,
            "ui.attachments.add",
            json!({ "transaction": id, "url": u }),
            "txd.attach_link",
            json!(n),
        );
    }
}

pub fn start_rename(app: &mut App, att_id: &str, current: &str) {
    if let Some(o) = app.detail.open.as_mut() {
        o.rename = Some((att_id.to_owned(), None));
    }
    let mut e = rune::TextEdit::new(current.to_owned());
    e.focused = true;
    app.detail.rename = e;
}

pub fn commit_rename(app: &mut App) {
    let label = app.detail.rename.text.trim().to_owned();
    let Some(o) = app.detail.open.as_mut() else {
        return;
    };
    let Some((att, _)) = o.rename.clone() else {
        return;
    };
    let current = o
        .attachments
        .as_ref()
        .and_then(|l| l.iter().find(|a| attachment_id(a) == att))
        .map(rail_label)
        .unwrap_or_default();
    let error = if label.is_empty() {
        Some(t("transactions:attachments.addLinkText"))
    } else if label.chars().count() > 200 {
        Some(tr(
            "transactions:attachments.linkTextTooLong",
            &[("max", "200")],
        ))
    } else {
        None
    };
    if let Some(e) = error {
        o.rename = Some((att, Some(e)));
        return;
    }
    if label == current {
        o.rename = None;
        return;
    }
    send_for(
        app,
        "ui.attachments.rename",
        json!({ "attachment": att, "label": label }),
        "txd.rename",
        json!(att),
    );
}

pub fn remove_attachment(app: &mut App, att: &str) {
    send_for(
        app,
        "ui.attachments.remove",
        json!({ "attachment": att }),
        "txd.remove",
        json!(att),
    );
}

/// `normalizeExternalBrowserUrl`: an absolute http(s) URL with a host and no credentials.
pub fn external_url(url: &str) -> Result<String, String> {
    let u = url.trim();
    let (scheme, rest) = u
        .split_once("://")
        .ok_or("Only absolute HTTP or HTTPS explorer URLs can be opened.")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Err("Only HTTP or HTTPS explorer URLs can be opened.".into());
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return Err("Explorer URLs with embedded credentials cannot be opened.".into());
    }
    if authority.split(':').next().unwrap_or("").is_empty() {
        return Err("Explorer URLs must include a host.".into());
    }
    if u.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("Only absolute HTTP or HTTPS explorer URLs can be opened.".into());
    }
    Ok(u.to_owned())
}

/// `validated_attachment_file_path`: an absolute path to a file inside the book's managed
/// `attachments` folder (beside its `data` folder).
pub fn attachment_file(
    data_root: &std::path::Path,
    requested: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    if !requested.is_absolute() {
        return Err("Attachment paths must be absolute.".into());
    }
    let state_root = if data_root.file_name().and_then(|n| n.to_str()) == Some("data") {
        data_root.parent().unwrap_or(data_root)
    } else {
        data_root
    };
    let root = std::fs::canonicalize(state_root.join("attachments"))
        .map_err(|e| format!("Attachments folder could not be found: {e}"))?;
    let canonical = std::fs::canonicalize(requested)
        .map_err(|e| format!("Attachment file could not be found: {e}"))?;
    if !canonical.starts_with(&root) {
        return Err("Only managed Kassiber attachment files can be opened.".into());
    }
    let meta = canonical
        .metadata()
        .map_err(|e| format!("Attachment file could not be inspected: {e}"))?;
    if !meta.is_file() {
        return Err("Only attachment files can be opened.".into());
    }
    Ok(canonical)
}

fn opened(ok: bool) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err("Could not open the attachment with the default app.".into())
    }
}

pub fn open_attachment(app: &mut App, att: &str) {
    app.send(
        "ui.attachments.open",
        json!({ "attachment": att }),
        "txd.att_open",
    );
}

/// Link attachments (`attachment_type: "url"`), else files.
pub fn is_url(a: &Value) -> bool {
    s(a, "attachment_type") == "url"
        || (s(a, "attachment_type").is_empty() && draft::opt_s(a, "url").is_some())
}

pub fn attachment_id(a: &Value) -> String {
    a.get("id")
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| v.to_string())
        })
        .unwrap_or_default()
}

/// `attachmentRecordToItem` label.
pub fn rail_label(a: &Value) -> String {
    draft::opt_s(a, "display_label")
        .or_else(|| draft::opt_s(a, "label"))
        .unwrap_or_else(|| {
            if is_url(a) {
                t("transactions:attachment.linkLabel")
            } else {
                draft::opt_s(a, "original_filename")
                    .unwrap_or_else(|| t("transactions:attachment.fileLabel"))
            }
        })
}

// ---- Evidence reuse (§6.2) ---------------------------------------------------------------------

pub fn open_reuse(app: &mut App, candidates: &[(String, String)]) {
    let first = candidates.first().map(|c| c.0.clone());
    if let Some(o) = app.detail.open.as_mut() {
        o.reuse = Some(Reuse {
            source: first.clone(),
            list: None,
            selected: vec![],
        });
    }
    if let Some(src) = first {
        send_for(
            app,
            "ui.attachments.list",
            json!({ "transaction": src }),
            "txd.reuse_list",
            json!(src),
        );
    }
}

pub fn pick_reuse_source(app: &mut App, src: &str) {
    if let Some(r) = app.detail.open.as_mut().and_then(|o| o.reuse.as_mut()) {
        r.source = Some(src.to_owned());
        r.list = None;
        r.selected.clear();
    }
    send_for(
        app,
        "ui.attachments.list",
        json!({ "transaction": src }),
        "txd.reuse_list",
        json!(src),
    );
}

pub fn copy_evidence(app: &mut App) {
    let Some(id) = rec_id(app) else { return };
    let Some(o) = app.detail.open.as_mut() else {
        return;
    };
    let Some(r) = &o.reuse else { return };
    let (Some(src), sel) = (r.source.clone(), r.selected.clone()) else {
        return;
    };
    if sel.is_empty() {
        return;
    }
    // The dialog hides at once (`busy.reuse`) and reappears with the selection on error.
    o.busy.reuse = true;
    send_for(
        app,
        "ui.attachments.copy",
        json!({ "transaction": id, "source_transaction": src, "attachments": sel }),
        "txd.copy",
        Value::Null,
    );
}

// ---- History (§5.2) ------------------------------------------------------------------------------

pub fn revert(app: &mut App, event: &str, field: Option<&str>, label: &str) {
    let Some(id) = rec_id(app) else { return };
    let reason = match field {
        Some(_) => tr(
            "transactions:history.revertReasonField",
            &[("label", label)],
        ),
        None => t("transactions:history.revertReasonEvent"),
    };
    let mut args = json!({ "transaction": id, "event": event, "reason": reason });
    if let Some(fld) = field {
        args["field"] = json!(fld);
    }
    if let Some(o) = app.detail.open.as_mut() {
        o.revert_confirm = None;
    }
    send_for(
        app,
        "ui.transactions.history.revert",
        args,
        "txd.revert",
        Value::Null,
    );
}

pub fn process_journals(app: &mut App) {
    if let Some(o) = app.detail.open.as_mut() {
        o.busy.process = true;
    }
    app.process_journals();
}

// ---- Commercial review (§5.4) ------------------------------------------------------------------

pub fn commercial_review(app: &mut App, link: &str, state: &str, kind: Option<&str>) {
    let mut args = json!({ "link": link, "state": state });
    if let Some(k) = kind {
        args["commercial_kind"] = json!(k);
    }
    if let Some(o) = app.detail.open.as_mut() {
        o.busy.commercial = true;
        o.commercial_error = None;
    }
    send_for(
        app,
        "ui.btcpay.provenance.review",
        args,
        "txd.commercial_review",
        json!(link),
    );
}

// ---- Acquisition review (§5.6) -----------------------------------------------------------------

pub fn acq_preview(app: &mut App) {
    if let Some(o) = app.detail.open.as_mut() {
        o.busy.acq = true;
        o.acq_error = None;
        o.acq_preview = None;
    }
    send_for(
        app,
        "ui.review.cases",
        json!({ "limit": 1 }),
        "txd.acq_cases",
        Value::Null,
    );
}

/// The second step of the acquisition preview, for the panel that asked (gone: nothing).
fn acq_plan(app: &mut App, to: Target, cases: &Value) {
    let Some(o) = panel_mut(app, &to) else { return };
    o.acq_scope = Some(cases.clone());
    let args = acq_plan_args(o.acq_kind.clone().flatten(), &to.tx, cases);
    send_to(app, to, "ui.review.plan", args, "txd.acq_plan");
}

/// The plan request for classifying `tx` as `kind` (`None`: back to the imported kind).
fn acq_plan_args(kind: Option<String>, tx: &str, cases: &Value) -> Value {
    json!({
        "expected_scope": { "workspace_id": cases["workspace_id"], "profile_id": cases["profile_id"] },
        "expected_input_version": cases["input_version"],
        "operations": [{ "type": "kind_override", "transaction_id": tx, "kind": kind, "reason": "Reviewed acquisition classification" }],
    })
}

pub fn acq_apply(app: &mut App) {
    let Some(o) = app.detail.open.as_mut() else {
        return;
    };
    let (Some(scope), Some(artifact)) = (o.acq_scope.clone(), o.acq_preview.clone()) else {
        return;
    };
    o.busy.acq = true;
    let key = format!(
        "txd-{}-{}",
        s(&artifact, "base_input_version"),
        crate::format::now_secs()
    );
    let args = json!({
        "expected_scope": { "workspace_id": scope["workspace_id"], "profile_id": scope["profile_id"] },
        "artifact": artifact,
        "idempotency_key": key,
    });
    send_for(app, "ui.review.apply", args, "txd.acq_apply", Value::Null);
}

// ---- Split payout (§5.5) -------------------------------------------------------------------------

/// Validation for the split card; returns (out error, payout error).
pub fn split_errors(app: &App, outbound: f64) -> (Option<String>, Option<String>) {
    let check = |v: &str| -> Option<String> {
        let v = v.trim();
        if v.is_empty() {
            return None;
        }
        match v.parse::<f64>() {
            Ok(x) if x > 0.0 && x.is_finite() => None,
            _ => Some(t("transactions:splitPayout.positiveAmount")),
        }
    };
    let mut out = check(&app.detail.split_out.text);
    if out.is_none()
        && let Ok(x) = app.detail.split_out.text.trim().parse::<f64>()
        && x > outbound + 1e-12
    {
        out = Some(tr(
            "transactions:splitPayout.exceedsOutbound",
            &[("total", &outbound.to_string())],
        ));
    }
    (out, check(&app.detail.split_payout.text))
}

pub fn split_submit(app: &mut App) {
    let Some(id) = rec_id(app) else { return };
    let ui = &app.detail;
    let mut args = json!({
        "tx_out": id,
        "out_amount": ui.split_out.text.trim(),
        "payout_amount": ui.split_payout.text.trim(),
    });
    let o = ui.open.as_ref().unwrap();
    args["payout_asset"] = json!(o.split_asset);
    args["policy"] = json!(o.split_policy);
    if !ui.split_counterparty.text.trim().is_empty() {
        args["counterparty"] = json!(ui.split_counterparty.text.trim());
    }
    if !ui.split_notes.text.trim().is_empty() {
        args["notes"] = json!(ui.split_notes.text.trim());
    }
    if let Some(o) = app.detail.open.as_mut() {
        o.busy.split = true;
        o.split_error = None;
    }
    send_for(
        app,
        "ui.transfers.payouts.create",
        args,
        "txd.split",
        Value::Null,
    );
}

// ---- Replies -----------------------------------------------------------------------------------------

/// Replies of the panel's mutations. `to` is the panel each was sent for: its UI state updates
/// there, shown or parked, and nowhere when that panel is gone; toasts and book refreshes still
/// follow the write.
pub fn on_reply(app: &mut App, tag: &str, to: Option<Target>, r: &Reply) {
    let to = to.unwrap_or_else(Target::nowhere);
    let err = r.error.clone();
    match tag {
        "txd.loan_mark" | "txd.loan_unmark" => {
            if let Some(o) = panel_mut(app, &to) {
                o.busy.loan = false;
            }
            match err {
                Some(e) => toast_err(
                    app,
                    "transactions:notification.loanMarkFailed.title",
                    Some(if e.is_empty() {
                        t("transactions:notification.loanMarkFailed.body")
                    } else {
                        e
                    }),
                ),
                None if tag == "txd.loan_mark" => toast_ok(
                    app,
                    "transactions:notification.loanMarked.title",
                    Some(t("transactions:notification.loanMarked.body")),
                ),
                None => toast_ok(
                    app,
                    "transactions:notification.loanUnmarked.title",
                    Some(t("transactions:notification.loanUnmarked.body")),
                ),
            }
            app.send("ui.loans.list", Value::Null, "txd.loans");
            app.refresh_reads();
        }
        "txd.loan_link" => {
            if let Some(o) = panel_mut(app, &to)
                && o.busy.link.as_deref() == to.key.as_str()
            {
                o.busy.link = None;
            }
            match err {
                Some(e) => toast_err(
                    app,
                    "transactions:notification.loanLinkFailed.title",
                    Some(if e.is_empty() {
                        t("transactions:notification.loanLinkFailed.body")
                    } else {
                        e
                    }),
                ),
                None => toast_ok(
                    app,
                    "transactions:notification.loanLinked.title",
                    Some(t("transactions:notification.loanLinked.body")),
                ),
            }
            app.send("ui.loans.list", Value::Null, "txd.loans");
        }
        "txd.unpair" => {
            if let Some(o) = panel_mut(app, &to) {
                o.busy.unpair = false;
                // Only the pair that was removed leaves the record.
                if err.is_none()
                    && let Some(rec) = o.record.as_mut()
                    && s(&rec["pair"], "id") == to.key.as_str().unwrap_or_default()
                    && let Some(m) = rec.as_object_mut()
                {
                    m.remove("pair");
                }
            }
            match err {
                Some(e) => app.toast(crate::app::ToastTone::Error, &e, None, Some("txd-err")),
                None => {
                    toast_ok(
                        app,
                        "transactions:notification.pairRemoved.title",
                        Some(t("transactions:notification.pairRemoved.body")),
                    );
                    app.refresh_reads();
                }
            }
        }
        "txd.attach_file" => {
            if let Some(o) = panel_mut(app, &to) {
                o.busy.attach = false;
                match &err {
                    Some(e) => o.attach_error = Some(e.clone()),
                    None => upsert(o, &r.data),
                }
            } else if let Some(e) = &err {
                app.toast(crate::app::ToastTone::Error, e, None, Some("txd-err"));
            }
            if err.is_none() {
                toast_ok(
                    app,
                    "transactions:notification.filesAttached.title",
                    Some(tc("transactions:notification.filesAttached.body", 1, &[])),
                );
            }
        }
        "txd.attach_link" => {
            let active = is_active(app, &to);
            // The batch's last reply: the dialog closes (or the toast follows a gone panel).
            let done = match panel_mut(app, &to) {
                Some(o) => {
                    o.busy.links = o.busy.links.saturating_sub(1);
                    match &err {
                        Some(e) => {
                            o.links_error = Some(if e.is_empty() {
                                t("transactions:attachments.linksDialog.couldNotAdd")
                            } else {
                                e.clone()
                            })
                        }
                        None => upsert(o, &r.data),
                    }
                    let done = o.busy.links == 0 && o.links_error.is_none();
                    if done {
                        o.links_open = false;
                    }
                    done
                }
                None => {
                    if let Some(e) = &err {
                        app.toast(crate::app::ToastTone::Error, e, None, Some("txd-err"));
                    }
                    let more = app
                        .detail
                        .targets
                        .get("txd.attach_link")
                        .and_then(|q| q.front())
                        .is_some_and(|n| n.panel == to.panel);
                    err.is_none() && !more
                }
            };
            if done {
                // The links field belongs to the shown panel's dialog.
                if active {
                    app.detail.links = rune::TextEdit::new("");
                }
                let n = to.key.as_u64().unwrap_or(1) as usize;
                toast_ok(
                    app,
                    "transactions:notification.linksAttached.title",
                    Some(tc("transactions:notification.linksAttached.body", n, &[])),
                );
            }
        }
        "txd.rename" => {
            let att = to.key.as_str().unwrap_or_default().to_owned();
            if let Some(o) = panel_mut(app, &to) {
                let editing = o.rename.as_ref().is_some_and(|(id, _)| *id == att);
                match &err {
                    Some(e) if editing => {
                        o.rename = Some((
                            att.clone(),
                            Some(if e.is_empty() {
                                t("transactions:attachments.couldNotRename")
                            } else {
                                e.clone()
                            }),
                        ));
                    }
                    Some(_) => {}
                    None => {
                        if editing {
                            o.rename = None;
                        }
                        upsert(o, &r.data);
                    }
                }
            } else if let Some(e) = &err {
                app.toast(crate::app::ToastTone::Error, e, None, Some("txd-err"));
            }
            if err.is_none() {
                toast_ok(
                    app,
                    "transactions:notification.linkTextUpdated.title",
                    Some(t("transactions:notification.linkTextUpdated.body")),
                );
            }
        }
        "txd.remove" => match err {
            Some(e) => app.toast(crate::app::ToastTone::Error, &e, None, Some("txd-err")),
            None => {
                let rec = attachment_of(&r.data);
                let is_url = is_url(&rec);
                let id = match attachment_id(&rec) {
                    id if id.is_empty() => to.key.as_str().unwrap_or_default().to_owned(),
                    id => id,
                };
                let shown = panel_mut(app, &to).map(|o| {
                    if let Some(list) = o.attachments.as_mut() {
                        list.retain(|a| attachment_id(a) != id);
                    }
                });
                let body = if is_url {
                    "transactions:notification.attachmentRemoved.linkBody"
                } else {
                    "transactions:notification.attachmentRemoved.fileBody"
                };
                toast_ok(
                    app,
                    "transactions:notification.attachmentRemoved.title",
                    Some(t(body)),
                );
                if shown.is_some() {
                    refresh_attachments(app, to);
                }
            }
        },
        "txd.att_open" => match err {
            Some(e) => app.toast(crate::app::ToastTone::Error, &e, None, Some("txd-err")),
            None => {
                // `onOpenAttachment`: a link through `openExternalUrl`, a file through
                // `open_attachment_file`; each refuses what it may not open.
                let result = match (
                    s(&r.data, "target_type"),
                    draft::opt_s(&r.data, "url"),
                    draft::opt_s(&r.data, "path"),
                ) {
                    ("url", Some(url), _) => {
                        external_url(&url).and_then(|u| opened(super::open_external(&u)))
                    }
                    ("file", _, Some(path)) => app
                        .data_root()
                        .ok_or_else(|| "Attachments folder could not be found.".to_owned())
                        .and_then(|root| attachment_file(&root, std::path::Path::new(&path)))
                        .and_then(|p| opened(super::open_external(&p.to_string_lossy()))),
                    _ => Ok(()),
                };
                if let Err(e) = result {
                    app.toast(crate::app::ToastTone::Error, &e, None, Some("txd-err"));
                }
            }
        },
        "txd.reuse_list" => {
            // Only the dialog still showing the source that was read.
            if let Some(re) = panel_mut(app, &to).and_then(|o| o.reuse.as_mut())
                && re.source.as_deref() == to.key.as_str()
            {
                re.list = Some(
                    r.data["attachments"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default(),
                );
            }
        }
        "txd.copy" => {
            let copied = r.data["copied"].as_u64().unwrap_or(0) as usize;
            let shown = panel_mut(app, &to).map(|o| {
                o.busy.reuse = false;
                if err.is_none() {
                    o.reuse = None;
                    if let Some(list) = r.data["attachments"].as_array() {
                        for a in list {
                            upsert(o, a);
                        }
                    }
                }
            });
            match err {
                Some(e) => app.toast(crate::app::ToastTone::Error, &e, None, Some("txd-err")),
                None => {
                    toast_ok(
                        app,
                        "transactions:notification.evidenceReused.title",
                        Some(tc(
                            "transactions:notification.evidenceReused.body",
                            copied,
                            &[],
                        )),
                    );
                    if shown.is_some() {
                        refresh_attachments(app, to);
                    }
                }
            }
        }
        "txd.revert" => match err {
            Some(e) => app.toast(crate::app::ToastTone::Error, &e, None, Some("txd-err")),
            None => {
                toast_ok(
                    app,
                    "transactions:notification.editReverted.title",
                    Some(t("transactions:notification.editReverted.body")),
                );
                // The reverted value is the new baseline.
                if panel_mut(app, &to).is_some() {
                    reload_to(app, to);
                }
                app.refresh_reads();
            }
        },
        "txd.commercial_review" => {
            let shown = panel_mut(app, &to).map(|o| {
                o.busy.commercial = false;
                o.commercial_error = err.clone();
            });
            match err {
                None => {
                    if shown.is_some() {
                        reload_to(app, to);
                    }
                    app.refresh_reads();
                }
                Some(e) if shown.is_none() => {
                    app.toast(crate::app::ToastTone::Error, &e, None, Some("txd-err"))
                }
                Some(_) => {}
            }
        }
        "txd.acq_cases" => match err {
            Some(e) => acq_fail(app, &to, e),
            None => acq_plan(app, to, &r.data),
        },
        "txd.acq_plan" => match err {
            Some(e) => acq_fail(app, &to, e),
            None => {
                let artifact = r
                    .data
                    .get("artifact")
                    .cloned()
                    .unwrap_or_else(|| r.data.clone());
                let Some(o) = panel_mut(app, &to) else {
                    return;
                };
                let scope = o.acq_scope.clone().unwrap_or(Value::Null);
                let fits = artifact["workspace_id"] == scope["workspace_id"]
                    && artifact["profile_id"] == scope["profile_id"]
                    && artifact["base_input_version"] == scope["input_version"];
                o.busy.acq = false;
                if fits {
                    o.acq_preview = Some(artifact);
                } else {
                    o.acq_error = Some(t("transactions:tax.acquisition.invalidPreview"));
                }
            }
        },
        "txd.acq_apply" => match err {
            Some(e) => acq_fail(app, &to, e),
            None => {
                if let Some(o) = panel_mut(app, &to) {
                    o.busy.acq = false;
                    o.acq_preview = None;
                    reload_to(app, to);
                }
                app.refresh_reads();
            }
        },
        "txd.split" => {
            let shown = panel_mut(app, &to).map(|o| {
                o.busy.split = false;
                match &err {
                    Some(e) => {
                        o.split_error =
                            Some(tr("transactions:splitPayout.recordFailed", &[("error", e)]))
                    }
                    None => o.split_msg = Some(t("transactions:splitPayout.recorded")),
                }
            });
            match err {
                None => app.process_journals(),
                Some(e) if shown.is_none() => {
                    app.toast(crate::app::ToastTone::Error, &e, None, Some("txd-err"))
                }
                Some(_) => {}
            }
        }
        _ => {}
    }
}

/// Re-reads the attachment list of the panel `to` names.
fn refresh_attachments(app: &mut App, to: Target) {
    let id = to.tx.clone();
    send_to(
        app,
        to,
        "ui.attachments.list",
        json!({ "transaction": id }),
        "txd.attachments",
    );
}

fn acq_fail(app: &mut App, to: &Target, e: String) {
    match panel_mut(app, to) {
        Some(o) => {
            o.busy.acq = false;
            o.acq_error = Some(e);
        }
        None => app.toast(crate::app::ToastTone::Error, &e, None, Some("txd-err")),
    }
}

fn attachment_of(data: &Value) -> Value {
    data.get("attachment")
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or_else(|| data.clone())
}

/// New records first; replace by id.
fn upsert(o: &mut super::Open, data: &Value) {
    let rec = attachment_of(data);
    if rec.get("id").is_none() {
        return;
    }
    let id = attachment_id(&rec);
    let list = o.attachments.get_or_insert_with(Vec::new);
    if let Some(slot) = list.iter_mut().find(|a| attachment_id(a) == id) {
        *slot = rec;
    } else {
        list.insert(0, rec);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_validate_like_the_web() {
        let (ok, bad) =
            parse_links("https://a.example/x\n\nftp://b.example\nnot a url\nipfs://cid");
        assert_eq!(ok, vec!["https://a.example/x", "ipfs://cid"]);
        assert_eq!(bad.len(), 2);
    }

    /// Saving transaction A, then switching to tab B before the reply: the reply completes A's
    /// parked panel and leaves B's edits alone.
    #[test]
    fn a_save_reply_finds_its_tab_after_a_switch() {
        let mut app = crate::app::tests::quiet_app();
        let panel = |id: &str, saving: bool| {
            let mut o = super::super::Open::new(id, super::super::Tab::Details, vec![]);
            o.record = Some(json!({ "id": id }));
            o.draft.note = format!("edited {id}");
            o.saving = saving;
            Box::new(o)
        };
        app.detail.tabs = vec![
            super::super::TabSlot {
                parked: Some(panel("a", true)),
                pinned: true,
                label: "A".into(),
            },
            super::super::TabSlot {
                parked: None,
                pinned: true,
                label: "B".into(),
            },
        ];
        app.detail.active_tab = 1;
        app.detail.open = Some(panel("b", false));
        let submitted = app.detail.tabs[0].parked.as_ref().unwrap().draft.clone();
        app.detail.saving_ids.push_back(("a".into(), submitted));
        // An edit made after the save was sent stays unsaved.
        app.detail.tabs[0].parked.as_mut().unwrap().draft.note = "edited again".into();
        let reply = Reply {
            kind: "ui.transactions.metadata.update".into(),
            data: json!({}),
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        on_saved(&mut app, &reply);
        let a = app.detail.tabs[0].parked.as_ref().unwrap();
        assert!(!a.saving, "A finished saving");
        assert_eq!(
            a.base.note, "edited a",
            "the submitted draft is the baseline"
        );
        assert!(a.dirty().count > 0, "the later edit stays dirty");
        let b = app.detail.open.as_ref().unwrap();
        assert!(b.dirty().count > 0, "B keeps its unsaved edits");
        assert!(app.detail.saving_ids.is_empty());
    }

    /// A transaction saved here and later excluded elsewhere (the Transactions table, the
    /// assistant): the next read is the baseline, so a later save cannot include it again.
    #[test]
    fn a_fresh_read_outranks_an_earlier_save() {
        let mut app = crate::app::tests::quiet_app();
        let mut a = shown("a");
        a.draft.note = "saved here".into();
        app.detail
            .saving_ids
            .push_back(("a".into(), a.draft.clone()));
        app.detail.open = Some(a);
        on_saved(&mut app, &ok(json!({})));
        assert_eq!(app.detail.open.as_ref().unwrap().base.note, "saved here");
        // Excluded through the table; the panel re-reads.
        super::super::reload(&mut app);
        super::super::on_reply(
            &mut app,
            "txd.reload",
            &ok(json!({ "transaction": { "id": "a", "note": "saved here", "excluded": true } })),
        );
        let o = app.detail.open.as_ref().unwrap();
        assert!(
            o.base.excluded && o.draft.excluded,
            "the read's exclusion stands"
        );
        assert_eq!(o.dirty().count, 0);
    }

    /// A save normalizes its value (`70000,5` comes back `70000.5`) while the user already edits
    /// the note: the read takes the price, the note edit stays.
    #[test]
    fn a_normalizing_read_keeps_unrelated_edits() {
        let mut app = crate::app::tests::quiet_app();
        let mut a = shown("a");
        a.draft.manual_price = "70000,5".into();
        app.detail
            .saving_ids
            .push_back(("a".into(), a.draft.clone()));
        app.detail.open = Some(a);
        on_saved(&mut app, &ok(json!({})));
        app.detail.open.as_mut().unwrap().draft.note = "typing".into();
        app.detail.note = rune::TextEdit::new("typing");
        super::super::reload(&mut app);
        super::super::on_reply(
            &mut app,
            "txd.reload",
            &ok(json!({ "transaction": { "id": "a", "rate": 70000.5 } })),
        );
        let o = app.detail.open.as_ref().unwrap();
        assert_eq!(o.draft.note, "typing", "the note edit stays");
        assert_eq!(o.draft.manual_price, "70000.5", "the price takes the read");
        assert_eq!(o.base.manual_price, "70000.5");
        assert_eq!(app.detail.note.text, "typing");
    }

    fn ok(data: Value) -> Reply {
        Reply {
            kind: "ui.transactions.resolve".into(),
            data,
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        }
    }

    /// Saving A and closing the last tab: the reply still drains A's entry, so B's later save
    /// reply finds B, not A.
    #[test]
    fn a_save_reply_drains_after_the_last_tab_closes() {
        let mut app = crate::app::tests::quiet_app();
        let draft = super::super::Open::new("a", super::super::Tab::Details, vec![]).draft;
        app.detail.saving_ids.push_back(("a".into(), draft));
        app.detail.open = None;
        super::super::on_reply(&mut app, "txd.save", &ok(json!({})));
        assert!(app.detail.saving_ids.is_empty());
    }

    fn shown(id: &str) -> Box<super::super::Open> {
        let mut o = super::super::Open::new(id, super::super::Tab::Details, vec![]);
        o.record = Some(json!({ "id": id }));
        Box::new(o)
    }

    /// Tab A parked, tab B shown.
    fn two_tabs(app: &mut App, a: Box<super::super::Open>, b: Box<super::super::Open>) {
        app.detail.tabs = vec![
            super::super::TabSlot {
                parked: Some(a),
                pinned: true,
                label: "A".into(),
            },
            super::super::TabSlot {
                parked: None,
                pinned: true,
                label: "B".into(),
            },
        ];
        app.detail.active_tab = 1;
        app.detail.open = Some(b);
    }

    fn fail(e: &str) -> Reply {
        Reply {
            error: Some(e.into()),
            ..ok(Value::Null)
        }
    }

    /// A side read sent for A that lands after switching to B is dropped, and drains its entry.
    #[test]
    fn a_side_read_for_another_transaction_is_dropped() {
        let mut app = crate::app::tests::quiet_app();
        // A's panel was replaced (a coin-trail step, say): its read has nowhere to land.
        let gone = shown("a").target(Value::Null);
        app.detail.open = Some(shown("b"));
        let b = super::super::active_target(&app, Value::Null);
        for to in [gone, b] {
            app.detail
                .targets
                .entry("txd.attachments")
                .or_default()
                .push_back(to);
        }
        let list = |n: &str| ok(json!({ "attachments": [{ "id": n }] }));
        super::super::on_reply(&mut app, "txd.attachments", &list("for-a"));
        assert!(app.detail.open.as_ref().unwrap().attachments.is_none());
        super::super::on_reply(&mut app, "txd.attachments", &list("for-b"));
        let got = app
            .detail
            .open
            .as_ref()
            .unwrap()
            .attachments
            .clone()
            .unwrap();
        assert_eq!(got[0]["id"], "for-b");
        assert!(app.detail.targets["txd.attachments"].is_empty());
    }

    /// Attaching a link on A, then switching to B before the reply: the new attachment lands on
    /// A's parked panel, never on B (whose Remove could then delete it by id).
    #[test]
    fn an_attachment_reply_after_a_tab_switch_lands_on_its_own_tab() {
        let mut app = crate::app::tests::quiet_app();
        let mut a = shown("a");
        a.busy.links = 1;
        a.links_open = true;
        let to = a.target(json!(1));
        two_tabs(&mut app, a, shown("b"));
        app.detail.links = rune::TextEdit::new("typed on B");
        app.detail
            .targets
            .entry("txd.attach_link")
            .or_default()
            .push_back(to);
        super::super::on_reply(
            &mut app,
            "txd.attach_link",
            &ok(json!({ "attachment": { "id": "att-1", "attachment_type": "url" } })),
        );
        let b = app.detail.open.as_ref().unwrap();
        assert!(b.attachments.is_none(), "B shows nothing of A's");
        assert_eq!(app.detail.links.text, "typed on B", "B's dialog text stays");
        let a = app.detail.tabs[0].parked.as_ref().unwrap();
        assert_eq!(a.attachments.as_ref().unwrap()[0]["id"], "att-1");
        assert_eq!(a.busy.links, 0);
        assert!(!a.links_open, "A's dialog closed");
        assert!(app.detail.targets["txd.attach_link"].is_empty());
    }

    /// A removal on A that lands after switching to B leaves B's list alone and re-reads A's.
    #[test]
    fn a_removal_reply_after_a_tab_switch_updates_its_own_tab() {
        let mut app = crate::app::tests::quiet_app();
        let same = json!([{ "id": "att-1" }, { "id": "att-2" }]);
        let mut a = shown("a");
        a.attachments = same.as_array().cloned();
        let mut b = shown("b");
        b.attachments = same.as_array().cloned();
        let to = a.target(json!("att-1"));
        two_tabs(&mut app, a, b);
        app.detail
            .targets
            .entry("txd.remove")
            .or_default()
            .push_back(to.clone());
        super::super::on_reply(
            &mut app,
            "txd.remove",
            &ok(json!({ "attachment": { "id": "att-1" } })),
        );
        assert_eq!(
            app.detail
                .open
                .as_ref()
                .unwrap()
                .attachments
                .as_ref()
                .unwrap()
                .len(),
            2
        );
        let a = app.detail.tabs[0].parked.as_ref().unwrap();
        assert_eq!(a.attachments.as_ref().unwrap().len(), 1);
        // The follow-up list read is A's.
        assert_eq!(app.detail.targets["txd.attachments"].front(), Some(&to));
    }

    /// A opened with a `:` query (which the record need not echo) and B opened before A's
    /// resolve reply: the reply resolves A's panel, never B's record or draft.
    #[test]
    fn a_resolve_reply_for_another_tabs_colon_query_leaves_the_shown_tab() {
        let mut app = crate::app::tests::quiet_app();
        let a = Box::new(super::super::Open::new(
            "wallet:abc:0",
            super::super::Tab::Details,
            vec![],
        ));
        let to = a.target(Value::Null);
        assert!(to.tx.is_empty(), "sent while A resolves");
        let mut b = shown("b");
        b.draft.note = "unsaved".into();
        two_tabs(&mut app, a, b);
        app.detail
            .targets
            .entry("txd.resolve")
            .or_default()
            .push_back(to);
        super::super::on_reply(
            &mut app,
            "txd.resolve",
            &ok(json!({ "transaction": { "id": "a-row" } })),
        );
        let b = app.detail.open.as_ref().unwrap();
        assert_eq!(b.id(), Some("b"));
        assert_eq!(b.draft.note, "unsaved");
        let a = app.detail.tabs[0].parked.as_ref().unwrap();
        assert_eq!(
            a.id(),
            Some("a-row"),
            "the colon query resolves its own tab"
        );
        assert!(!a.loading);
        // A's side reads follow, for A.
        assert_eq!(
            app.detail.targets["txd.history"]
                .front()
                .map(|t| t.tx.as_str()),
            Some("a-row")
        );
    }

    /// A resolve sent by a panel that was replaced since (Retry, a coin-trail step) is dropped.
    #[test]
    fn a_resolve_reply_for_a_replaced_panel_is_dropped() {
        let mut app = crate::app::tests::quiet_app();
        let old = super::super::Open::new("wallet:abc:0", super::super::Tab::Details, vec![]);
        let to = old.target(Value::Null);
        app.detail.open = Some(Box::new(super::super::Open::new(
            "wallet:abc:0",
            super::super::Tab::Details,
            vec![],
        )));
        app.detail
            .targets
            .entry("txd.resolve")
            .or_default()
            .push_back(to);
        super::super::on_reply(
            &mut app,
            "txd.resolve",
            &ok(json!({ "transaction": { "id": "stale" } })),
        );
        let o = app.detail.open.as_ref().unwrap();
        assert!(
            o.record.is_none() && o.loading,
            "the new panel waits for its own reply"
        );
    }

    /// An acquisition preview started on A continues for A after switching to B: the plan step
    /// names A's transaction and its failure lands on A.
    #[test]
    fn an_acquisition_preview_follows_its_own_tab() {
        let mut app = crate::app::tests::quiet_app();
        let mut a = shown("a");
        a.busy.acq = true;
        a.acq_kind = Some(Some("gift_in".into()));
        let to = a.target(Value::Null);
        two_tabs(&mut app, a, shown("b"));
        app.detail
            .targets
            .entry("txd.acq_cases")
            .or_default()
            .push_back(to.clone());
        let cases = json!({ "workspace_id": "w", "profile_id": "p", "input_version": "v1" });
        super::super::on_reply(&mut app, "txd.acq_cases", &ok(cases.clone()));
        assert!(app.detail.open.as_ref().unwrap().acq_scope.is_none());
        assert_eq!(
            app.detail.tabs[0].parked.as_ref().unwrap().acq_scope,
            Some(cases)
        );
        assert_eq!(app.detail.targets["txd.acq_plan"].front(), Some(&to));
        super::super::on_reply(&mut app, "txd.acq_plan", &fail("refused"));
        let a = app.detail.tabs[0].parked.as_ref().unwrap();
        assert!(!a.busy.acq);
        assert_eq!(a.acq_error.as_deref(), Some("refused"));
        assert!(app.detail.open.as_ref().unwrap().acq_error.is_none());
    }

    /// Resetting an override to the imported classification asks for `kind: null`.
    #[test]
    fn an_explicit_reset_previews_a_null_kind() {
        let mut a = shown("a");
        a.record = Some(json!({ "id": "a", "kindOverride": "income" }));
        a.acq_kind = Some(None);
        let cases = json!({ "workspace_id": "w", "profile_id": "p", "input_version": "v1" });
        let args = acq_plan_args(a.acq_kind.clone().flatten(), "a", &cases);
        let op = &args["operations"][0];
        assert_eq!(op["type"], "kind_override");
        assert!(op["kind"].is_null(), "{op}");
        assert_eq!(args["expected_input_version"], "v1");
    }

    /// An explanation read for A's row that lands after switching to B stays off B's dialog,
    /// and one for a reference the dialog no longer shows is dropped.
    #[test]
    fn an_explanation_lands_only_on_the_dialog_that_asked() {
        let mut app = crate::app::tests::quiet_app();
        let mut a = shown("a");
        a.explain = Some((json!("ref-a"), crate::app::Load::Loading));
        let to = a.target(json!("ref-a"));
        let mut b = shown("b");
        b.explain = Some((json!("ref-b"), crate::app::Load::Loading));
        let b_old = b.target(json!("ref-b-old"));
        two_tabs(&mut app, a, b);
        let q = app.detail.targets.entry("txd.explain").or_default();
        q.push_back(to);
        q.push_back(b_old);
        super::super::on_reply(&mut app, "txd.explain", &ok(json!({ "for": "a" })));
        super::super::on_reply(&mut app, "txd.explain", &ok(json!({ "for": "old" })));
        let a = app.detail.tabs[0].parked.as_ref().unwrap();
        assert!(matches!(&a.explain, Some((_, crate::app::Load::Ready(v))) if v["for"] == "a"));
        let b = app.detail.open.as_ref().unwrap();
        assert!(matches!(&b.explain, Some((_, crate::app::Load::Loading))));
    }

    /// A reload sent for A that lands after switching to B refreshes A's parked panel and
    /// leaves B's record and edits alone.
    #[test]
    fn a_reload_reply_for_another_transaction_is_ignored() {
        let mut app = crate::app::tests::quiet_app();
        let a = shown("a");
        let to = a.target(Value::Null);
        let mut b = shown("b");
        b.draft.note = "unsaved".into();
        two_tabs(&mut app, a, b);
        app.detail
            .targets
            .entry("txd.reload")
            .or_default()
            .push_back(to);
        super::super::on_reply(
            &mut app,
            "txd.reload",
            &ok(json!({ "transaction": { "id": "a", "fresh": true } })),
        );
        let o = app.detail.open.as_ref().unwrap();
        assert_eq!(o.id(), Some("b"));
        assert_eq!(o.draft.note, "unsaved");
        let a = app.detail.tabs[0].parked.as_ref().unwrap();
        assert_eq!(a.record.as_ref().unwrap()["fresh"], true);
    }
}

/// "Explain" on a Tax calculation row: opens the dialog and reads the tax engine's calculation
/// (`ui.reports.explain_capital_gain`, a local read; `staleTime: 0`, so every open asks again).
pub fn open_explanation(app: &mut App, reference: Value) {
    let Some(o) = app.detail.open.as_mut() else {
        return;
    };
    o.explain = Some((reference.clone(), crate::app::Load::Loading));
    send_for(
        app,
        "ui.reports.explain_capital_gain",
        json!({ "reference": reference }),
        "txd.explain",
        reference,
    );
}

#[cfg(test)]
mod open_checks {
    use super::{attachment_file, external_url};

    #[test]
    fn only_plain_http_links_open() {
        assert_eq!(
            external_url(" https://mempool.space/tx/ab ").as_deref(),
            Ok("https://mempool.space/tx/ab")
        );
        assert!(external_url("HTTP://example.org").is_ok());
        assert!(external_url("file:///etc/passwd").is_err());
        assert!(external_url("javascript:alert(1)").is_err());
        assert!(external_url("https://user:pw@example.org/").is_err());
        assert!(external_url("https:///nohost").is_err());
        assert!(external_url("/Applications/Calculator.app").is_err());
    }

    #[test]
    fn attachments_open_only_inside_the_managed_folder() {
        let dir = std::env::temp_dir().join(format!("kb-att-{}", std::process::id()));
        let data = dir.join("data");
        let att = dir.join("attachments").join("ab");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&att).unwrap();
        std::fs::write(att.join("receipt.pdf"), b"%PDF").unwrap();
        std::fs::write(dir.join("outside.pdf"), b"%PDF").unwrap();
        assert!(attachment_file(&data, &att.join("receipt.pdf")).is_ok());
        assert_eq!(
            attachment_file(&data, &dir.join("outside.pdf")).unwrap_err(),
            "Only managed Kassiber attachment files can be opened."
        );
        assert_eq!(
            attachment_file(&data, &att).unwrap_err(),
            "Only attachment files can be opened."
        );
        assert!(
            attachment_file(&data, std::path::Path::new("attachments/ab/receipt.pdf")).is_err()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
