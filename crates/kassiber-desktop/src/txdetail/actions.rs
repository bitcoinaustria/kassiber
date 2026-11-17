//! Mutations from the detail panel and their replies. Every call uses the web's kind and
//! argument shape (see the spec sections cited per function).

use rune::{FileDialogOptions, Tx};
use serde_json::{Value, json};

use super::draft::{self, s};
use super::{Reuse, close, open, reload, toast_err, toast_ok};
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
                    o.base = submitted.clone();
                    app.detail.drafts.insert(id.clone(), submitted);
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
    o.base = saved.clone();
    let (close_after, next) = (o.close_after_save, o.next_after_save);
    app.detail.drafts.insert(id.clone(), saved);
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
        Some(r) => app.send(
            "ui.loans.mark",
            json!({ "txid": id, "as": r }),
            "txd.loan_mark",
        ),
        None => app.send("ui.loans.unmark", json!({ "txid": id }), "txd.loan_unmark"),
    };
}

pub fn loan_link(app: &mut App, target: &str) {
    let Some(id) = rec_id(app) else { return };
    if let Some(o) = app.detail.open.as_mut() {
        o.busy.link = Some(target.to_owned());
    }
    app.send(
        "ui.loans.link",
        json!({ "txids": [id, target] }),
        "txd.loan_link",
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
    app.send(
        "ui.transfers.unpair",
        json!({ "pair_id": pair_id }),
        "txd.unpair",
    );
}

// ---- Attachments (§5.1) ------------------------------------------------------------------------

pub fn attach_file(app: &mut App, tx: &mut Tx) {
    if let Some(o) = app.detail.open.as_mut() {
        o.busy.attach = true;
        o.attach_error = None;
    }
    let options = FileDialogOptions {
        title: t("transactions:attachments.pickerTitle"),
        ..Default::default()
    };
    tx.open_file(options, |a: &mut App, result, _tx| match result {
        Ok(Some(path)) => {
            let Some(id) = rec_id(a) else { return };
            a.send(
                "ui.attachments.add",
                json!({ "transaction": id, "file_path": path.to_string_lossy() }),
                "txd.attach_file",
            );
        }
        Ok(None) => {
            if let Some(o) = a.detail.open.as_mut() {
                o.busy.attach = false;
            }
        }
        Err(e) => {
            if let Some(o) = a.detail.open.as_mut() {
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
    for u in urls {
        app.send(
            "ui.attachments.add",
            json!({ "transaction": id, "url": u }),
            "txd.attach_link",
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
    app.send(
        "ui.attachments.rename",
        json!({ "attachment": att, "label": label }),
        "txd.rename",
    );
}

pub fn remove_attachment(app: &mut App, att: &str) {
    app.send(
        "ui.attachments.remove",
        json!({ "attachment": att }),
        "txd.remove",
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
        app.send(
            "ui.attachments.list",
            json!({ "transaction": src }),
            "txd.reuse_list",
        );
    }
}

pub fn pick_reuse_source(app: &mut App, src: &str) {
    if let Some(r) = app.detail.open.as_mut().and_then(|o| o.reuse.as_mut()) {
        r.source = Some(src.to_owned());
        r.list = None;
        r.selected.clear();
    }
    app.send(
        "ui.attachments.list",
        json!({ "transaction": src }),
        "txd.reuse_list",
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
    app.send(
        "ui.attachments.copy",
        json!({ "transaction": id, "source_transaction": src, "attachments": sel }),
        "txd.copy",
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
    app.send("ui.transactions.history.revert", args, "txd.revert");
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
    app.send("ui.btcpay.provenance.review", args, "txd.commercial_review");
}

// ---- Acquisition review (§5.6) -----------------------------------------------------------------

pub fn acq_preview(app: &mut App) {
    if let Some(o) = app.detail.open.as_mut() {
        o.busy.acq = true;
        o.acq_error = None;
        o.acq_preview = None;
    }
    app.send("ui.review.cases", json!({ "limit": 1 }), "txd.acq_cases");
}

fn acq_plan(app: &mut App, cases: &Value) {
    let Some(id) = rec_id(app) else { return };
    let kind = app.detail.open.as_ref().and_then(|o| o.acq_kind.clone());
    let args = json!({
        "expected_scope": { "workspace_id": cases["workspace_id"], "profile_id": cases["profile_id"] },
        "expected_input_version": cases["input_version"],
        "operations": [{ "type": "kind_override", "transaction_id": id, "kind": kind, "reason": "Reviewed acquisition classification" }],
    });
    if let Some(o) = app.detail.open.as_mut() {
        o.acq_scope = Some(cases.clone());
    }
    app.send("ui.review.plan", args, "txd.acq_plan");
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
    app.send("ui.review.apply", args, "txd.acq_apply");
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
            _ => Some("Enter a positive BTC amount".into()),
        }
    };
    let mut out = check(&app.detail.split_out.text);
    if out.is_none()
        && let Ok(x) = app.detail.split_out.text.trim().parse::<f64>()
        && x > outbound + 1e-12
    {
        out = Some(format!("Cannot exceed the outbound total ({outbound})"));
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
    app.send("ui.transfers.payouts.create", args, "txd.split");
}

// ---- Replies -----------------------------------------------------------------------------------------

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    let err = r.error.clone();
    match tag {
        "txd.loan_mark" | "txd.loan_unmark" => {
            if let Some(o) = app.detail.open.as_mut() {
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
            if let Some(o) = app.detail.open.as_mut() {
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
            if let Some(o) = app.detail.open.as_mut() {
                o.busy.unpair = false;
                if err.is_none()
                    && let Some(rec) = o.record.as_mut()
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
            let ok = err.is_none();
            if let Some(o) = app.detail.open.as_mut() {
                o.busy.attach = false;
                match &err {
                    Some(e) => o.attach_error = Some(e.clone()),
                    None => upsert(o, &r.data),
                }
            }
            if ok {
                toast_ok(
                    app,
                    "transactions:notification.filesAttached.title",
                    Some(tc("transactions:notification.filesAttached.body", 1, &[])),
                );
            }
        }
        "txd.attach_link" => {
            let mut done = None;
            if let Some(o) = app.detail.open.as_mut() {
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
                if o.busy.links == 0 && o.links_error.is_none() {
                    o.links_open = false;
                    done = Some(());
                }
            }
            if done.is_some() {
                let n = parse_links(&app.detail.links.text).0.len();
                app.detail.links = rune::TextEdit::new("");
                toast_ok(
                    app,
                    "transactions:notification.linksAttached.title",
                    Some(tc("transactions:notification.linksAttached.body", n, &[])),
                );
            }
        }
        "txd.rename" => {
            let ok = err.is_none();
            if let Some(o) = app.detail.open.as_mut() {
                match &err {
                    Some(e) => {
                        let id = o.rename.as_ref().map(|r| r.0.clone()).unwrap_or_default();
                        o.rename = Some((
                            id,
                            Some(if e.is_empty() {
                                t("transactions:attachments.couldNotRename")
                            } else {
                                e.clone()
                            }),
                        ));
                    }
                    None => {
                        o.rename = None;
                        upsert(o, &r.data);
                    }
                }
            }
            if ok {
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
                let id = attachment_id(&rec);
                if let Some(list) = app
                    .detail
                    .open
                    .as_mut()
                    .and_then(|o| o.attachments.as_mut())
                {
                    list.retain(|a| attachment_id(a) != id);
                }
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
                if let Some(id) = rec_id(app) {
                    super::read_for(
                        app,
                        &id,
                        "ui.attachments.list",
                        json!({ "transaction": id }),
                        "txd.attachments",
                    );
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
            if let Some(re) = app.detail.open.as_mut().and_then(|o| o.reuse.as_mut()) {
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
            if let Some(o) = app.detail.open.as_mut() {
                o.busy.reuse = false;
                match &err {
                    Some(_) => {}
                    None => {
                        o.reuse = None;
                        if let Some(list) = r.data["attachments"].as_array() {
                            for a in list {
                                upsert(o, a);
                            }
                        }
                    }
                }
            }
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
                    if let Some(id) = rec_id(app) {
                        super::read_for(
                            app,
                            &id,
                            "ui.attachments.list",
                            json!({ "transaction": id }),
                            "txd.attachments",
                        );
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
                if let Some(id) = rec_id(app) {
                    app.detail.drafts.remove(&id);
                }
                reload(app);
                app.refresh_reads();
            }
        },
        "txd.commercial_review" => {
            if let Some(o) = app.detail.open.as_mut() {
                o.busy.commercial = false;
                o.commercial_error = err.clone();
            }
            if err.is_none() {
                reload(app);
                app.refresh_reads();
            }
        }
        "txd.acq_cases" => match err {
            Some(e) => acq_fail(app, e),
            None => acq_plan(app, &r.data),
        },
        "txd.acq_plan" => match err {
            Some(e) => acq_fail(app, e),
            None => {
                let artifact = r
                    .data
                    .get("artifact")
                    .cloned()
                    .unwrap_or_else(|| r.data.clone());
                let Some(o) = app.detail.open.as_mut() else {
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
            Some(e) => acq_fail(app, e),
            None => {
                if let Some(o) = app.detail.open.as_mut() {
                    o.busy.acq = false;
                    o.acq_preview = None;
                }
                reload(app);
                app.refresh_reads();
            }
        },
        "txd.split" => {
            let ok = err.is_none();
            if let Some(o) = app.detail.open.as_mut() {
                o.busy.split = false;
                match &err {
                    Some(e) => o.split_error = Some(format!("Could not record split payout: {e}")),
                    None => {
                        o.split_msg =
                            Some("Split payout recorded; reprocessing journals to apply it.".into())
                    }
                }
            }
            if ok {
                app.process_journals();
            }
        }
        _ => {}
    }
}

fn acq_fail(app: &mut App, e: String) {
    if let Some(o) = app.detail.open.as_mut() {
        o.busy.acq = false;
        o.acq_error = Some(e);
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
        assert!(app.detail.drafts.contains_key("a") && !app.detail.drafts.contains_key("b"));
        assert!(app.detail.saving_ids.is_empty());
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

    /// A side read sent for A that lands after switching to B is dropped, and drains its entry.
    #[test]
    fn a_side_read_for_another_transaction_is_dropped() {
        let mut app = crate::app::tests::quiet_app();
        let mut b = super::super::Open::new("b", super::super::Tab::Details, vec![]);
        b.record = Some(json!({ "id": "b" }));
        app.detail.open = Some(Box::new(b));
        for id in ["a", "b"] {
            app.detail
                .read_ids
                .entry("txd.attachments")
                .or_default()
                .push_back(id.into());
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
        assert!(app.detail.read_ids["txd.attachments"].is_empty());
    }

    /// A reload sent for A that lands after switching to B leaves B's record and edits alone.
    #[test]
    fn a_reload_reply_for_another_transaction_is_ignored() {
        let mut app = crate::app::tests::quiet_app();
        let mut b = super::super::Open::new("b", super::super::Tab::Details, vec![]);
        b.record = Some(json!({ "id": "b" }));
        b.draft.note = "unsaved".into();
        app.detail.open = Some(Box::new(b));
        super::super::on_reply(
            &mut app,
            "txd.reload",
            &ok(json!({ "transaction": { "id": "a" } })),
        );
        let o = app.detail.open.as_ref().unwrap();
        assert_eq!(o.id(), Some("b"));
        assert_eq!(o.draft.note, "unsaved");
    }
}

/// "Explain" on a Tax calculation row: opens the dialog and reads the tax engine's calculation
/// (`ui.reports.explain_capital_gain`, a local read; `staleTime: 0`, so every open asks again).
pub fn open_explanation(app: &mut App, reference: Value) {
    let Some(o) = app.detail.open.as_mut() else {
        return;
    };
    o.explain = Some((reference.clone(), crate::app::Load::Loading));
    app.send(
        "ui.reports.explain_capital_gain",
        json!({ "reference": reference }),
        "txd.explain",
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
