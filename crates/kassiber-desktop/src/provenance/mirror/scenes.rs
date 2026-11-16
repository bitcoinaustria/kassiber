//! Headless states of the Privacy Mirror (registered as `provenance-mirror-*` in
//! `provenance::SCENES`). Every scene reads the disposable book's real report first; the
//! states the book cannot produce (loading, failures, an empty book) are set on top of it.

use serde_json::{Value, json};

use crate::app::{App, ThemeMode};
use crate::i18n::Lang;

/// Navigates to the page and waits for the report.
pub fn open(a: &mut App) {
    a.navigate(super::ROUTE);
    super::prime(a);
}

/// The page with disclosures `keys` open.
pub fn with_open(a: &mut App, keys: &[&str]) {
    open(a);
    for k in keys {
        a.provenance.mirror.open.insert((*k).to_owned());
    }
}

pub fn light(a: &mut App) {
    a.prefs.theme = ThemeMode::Light;
    open(a);
}

pub fn german(a: &mut App) {
    a.prefs.lang = Lang::De;
    open(a);
}

fn findings(a: &App) -> Vec<Value> {
    a.provenance
        .mirror
        .payload
        .as_ref()
        .and_then(|p| p.get("findings"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// The first repeated group of personal findings, open.
pub fn attention(a: &mut App) {
    open(a);
    let personal: Vec<Value> = findings(a).into_iter().filter(super::personal).collect();
    if let Some(g) = super::group(&personal).into_iter().find(|g| g.len() > 1) {
        let id = g[0]["id"].as_str().unwrap_or("").to_owned();
        a.provenance.mirror.open.insert(format!("group-{id}"));
    }
}

/// Surrounding activity open, down to the first finding with a basis ("Why this is a finding").
pub fn basis(a: &mut App) {
    open(a);
    let context: Vec<Value> = findings(a)
        .into_iter()
        .filter(|f| !super::personal(f))
        .collect();
    let has_basis = |f: &Value| {
        ["assumptions", "limitations"]
            .iter()
            .any(|k| f[*k].as_array().is_some_and(|v| !v.is_empty()))
    };
    let s = &mut a.provenance.mirror;
    s.open.insert("context".into());
    for g in super::group(&context) {
        if let Some(f) = g.iter().find(|f| has_basis(f)) {
            if g.len() > 1 {
                s.open
                    .insert(format!("group-{}", g[0]["id"].as_str().unwrap_or("")));
            }
            s.open
                .insert(format!("basis-{}", f["id"].as_str().unwrap_or("")));
            return;
        }
    }
}

/// The first read in flight (`ScreenSkeleton`).
pub fn loading(a: &mut App) {
    open(a);
    let s = &mut a.provenance.mirror;
    s.payload = None;
    s.loading = true;
}

/// The first read failed after its retries (`ScreenNotice` with the daemon's message).
pub fn error(a: &mut App) {
    open(a);
    let s = &mut a.provenance.mirror;
    s.payload = None;
    s.loading = false;
    s.error = Some("Database is busy; try again in a moment.".into());
}

/// A refetch in flight over the shown report (the refresh icon spins).
pub fn refreshing(a: &mut App) {
    open(a);
    a.provenance.mirror.loading = true;
}

/// A refetch failed over the shown report, and the assistant hand-off failed.
pub fn refresh_error(a: &mut App) {
    open(a);
    let s = &mut a.provenance.mirror;
    s.error = Some("Database is busy; try again in a moment.".into());
    s.action_error =
        Some("The chain analysis snapshot changed; run the investigation again.".into());
}

/// A book with no local transactions: the daemon's report for it (no findings, nothing examined).
pub fn empty(a: &mut App) {
    open(a);
    reshape(a, |p| {
        p["summary"] = json!({"status": "unavailable", "finding_count": 0, "attention_count": 0, "owned_output_count": 0, "analyzed_transaction_count": 0, "local_transaction_count": 0, "domain_count": 0});
        p["findings"] = json!([]);
        let c = &mut p["coverage"];
        for k in [
            "examined_transactions",
            "available_transactions",
            "missing_nodes",
            "stale_nodes",
            "conflicting_nodes",
        ] {
            c[k] = json!(0);
        }
        c["status"] = json!("unavailable");
        c["truncated"] = json!(false);
        c["stopped_reasons"] = json!([]);
        p["entropy"] = json!({"evaluated": 0, "eligible": 0, "omitted": 0, "results": []});
    });
}

/// Owned outputs examined completely and nothing found (the headline gets its caveat body).
pub fn nothing_found(a: &mut App) {
    open(a);
    reshape(a, |p| {
        p["summary"]["status"] = json!("no_observed_exposure");
        p["summary"]["finding_count"] = json!(0);
        p["summary"]["attention_count"] = json!(0);
        p["findings"] = json!([]);
        let c = &mut p["coverage"];
        c["status"] = json!("complete");
        c["missing_nodes"] = json!(0);
        c["truncated"] = json!(false);
    });
}

fn reshape(a: &mut App, f: impl FnOnce(&mut Value)) {
    if let Some(p) = a.provenance.mirror.payload.as_mut() {
        f(p);
    }
}

/// Investigate on the first finding: the workbench opens on that finding's own query.
pub fn drill(a: &mut App) {
    open(a);
    let Some(first) = findings(a).into_iter().next() else {
        return;
    };
    let tab = super::finding_tab(first["category"].as_str().unwrap_or(""));
    super::investigate(
        a,
        &first["investigation"],
        crate::provenance::chain::model::Workspace::Graph,
        tab,
    );
    crate::provenance::chain::prime(a);
}
