//! Headless states of the workbench (`--scene provenance-chain-*`). None presses an egress
//! control: they open local previews, menus and disclosures only.

use super::model::Sel;
use crate::app::App;

/// The leftmost transaction of the demo book's overview graph (the parity anchor).
pub const ANCHOR_TXID: &str = "006cf2c07b2cd172a33142e12c6e05afea3bc643b73e92768d17c1bdd9882c45";

pub fn open(a: &mut App, route: &str) {
    a.navigate(route);
    super::prime(a);
}

pub fn tools(a: &mut App) {
    open(a, "/chain-analysis");
    a.provenance.chain.popup = Some("ca-tools".into());
}

pub fn node(a: &mut App) {
    open(a, "/chain-analysis");
    // The anchor when the book still has it, else the first transaction of the overview.
    let anchor = format!("bitcoin:regtest:tx:{ANCHOR_TXID}");
    let id = a.provenance.chain.result.as_ref().and_then(|r| {
        r.nodes
            .iter()
            .find(|n| n.id == anchor)
            .or_else(|| r.nodes.iter().find(|n| n.kind == "transaction"))
            .map(|n| n.id.clone())
    });
    if let Some(id) = id {
        super::select(a, Sel::node(id));
    }
}

pub fn filters(a: &mut App) {
    open(a, "/chain-analysis");
    a.provenance.chain.open.insert("filters".into());
}

pub fn table(a: &mut App) {
    open(a, "/chain-analysis");
    a.provenance.chain.table = true;
}

pub fn tab(a: &mut App, tab: &'static str) {
    open(a, &format!("/chain-analysis?tab={tab}"));
    super::settle(a);
    let key = match tab {
        "coverage" => "cov-cap-summary",
        "entropy" => "ent-run",
        "labels" => "lbl-save",
        _ => "ca-tab",
    };
    a.provenance.chain.reveal = Some(key.into());
}

pub fn saved(a: &mut App) {
    open(a, "/chain-analysis");
    let s = &mut a.provenance.chain;
    s.show_saved = true;
    s.show_acquire = false;
    super::settle(a);
}

pub fn acquire(a: &mut App) {
    open(a, "/chain-analysis");
    super::panels::show_acquire(a);
    super::settle(a);
}

pub fn watch(a: &mut App) {
    open(a, "/chain-analysis");
    let Some(key) = a
        .provenance
        .chain
        .result
        .as_ref()
        .map(|r| format!("result:{}", r.query.key()))
    else {
        return;
    };
    a.provenance.chain.watch.entry(key).or_default().open = true;
    super::load_binding(a);
    super::settle(a);
}

pub fn inbox(a: &mut App) {
    open(a, "/chain-analysis");
    a.provenance.chain.open.insert("inbox".into());
}
