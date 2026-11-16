//! Headless scenes for the detail panel's network and data-source states (`txd-*`). Each opens a
//! demo row and, where the demo book cannot produce the state without network, sets it
//! synthetically: nothing here presses a network control or sends a lookup. Scenes marked
//! SYNTHETIC replace a read's reply or put a fabricated lookup reply into the graph cache.

use std::rc::Rc;

use serde_json::{Value, json};

use super::graph::{self, Entry, Payload};
use crate::app::{App, Reply};

use crate::app::Scene;

/// A demo-book row by fixture name (`parity/fixtures.json`: `partial`, `full`, `graphless`,
/// `liquid`, `multi`, `fanout`, `swap`, …; each entry says what the row exercises).
fn fx(name: &str) -> &'static str {
    crate::fixtures::id(name)
}

fn shell_reads_ready(a: &App) -> bool {
    a.read("ui.network.offline").is_some() && a.read("ui.backends.settings.list").is_some()
}

/// Opens `id` with the sources panel unfolded, the shell's local reads in.
fn open(a: &mut App, id: &str) {
    super::pump_until(a, shell_reads_ready);
    super::open(a, id, "details");
    super::settle(a);
    a.graph.sources_open = true;
}

fn set_read(a: &mut App, kind: &str, data: Value) {
    a.reads.insert(
        kind.into(),
        Reply {
            kind: kind.into(),
            data,
            error: None,
            error_detail: Value::Null,
            elapsed: std::time::Duration::ZERO,
        },
    );
}

/// SYNTHETIC: offline mode off (the preview's isolated preference keeps it on).
fn online(a: &mut App) {
    set_read(
        a,
        "ui.network.offline",
        json!({"offline": false, "environment_blocked": false}),
    );
}

fn local(a: &App, id: &str) -> Option<Rc<Payload>> {
    match a.graph.entry(id, false) {
        Some(Entry::Ready(p)) => Some(p.clone()),
        _ => None,
    }
}

/// SYNTHETIC: the user approved the lookup for `id` and its reply is `entry`.
fn lookup_reply(a: &mut App, id: &str, entry: Entry) {
    graph::scene_lookup(a, id, entry);
}

/// The partial graph with every input amount filled in, as a successful lookup returns it.
fn completed(p: &Payload) -> Payload {
    let mut q = p.clone();
    // Outputs plus a 1 470 sat fee, spread over the inputs.
    let out: f64 = p.outputs.iter().filter_map(|n| n.value_sats).sum();
    let each = ((out + 1_470.0) / p.inputs.len().max(1) as f64).round();
    for n in &mut q.inputs {
        n.value_state = Some("known".into());
        n.value_sats = Some(each);
        n.value_btc = Some(each / 1e8);
    }
    q.support_level = "full".into();
    q.unsupported_reason = None;
    q.warnings.clear();
    q
}

fn with_warning(p: &Payload, code: &str, message: &str) -> Payload {
    let mut q = p.clone();
    q.warnings.push(graph::Warning {
        code: code.into(),
        level: Some("warning".into()),
        message: message.into(),
    });
    q
}

/// SYNTHETIC: the explorer settings of a mainnet book on the built-in servers.
fn mainnet(a: &mut App) {
    if let Some(r) = a.detail.open.as_mut().and_then(|o| o.record.as_mut()) {
        r["network"] = json!("main");
    }
    set_read(
        a,
        "ui.backends.settings.list",
        json!({"summary": {"bootstrap_mode": "public"}, "backends": [
            {"name": "mempool", "display_name": "Bitcoin Austria Explorer API", "kind": "esplora", "chain": "bitcoin", "network": "main", "url": "https://mempool.bitcoin-austria.at/api", "source": "built-in default", "is_default": true, "wallet_refs": []},
            {"name": "fulcrum", "kind": "electrum", "chain": "bitcoin", "network": "main", "url": "ssl://index.bitcoin-austria.at:50002", "source": "built-in default", "is_default": false, "wallet_refs": []}
        ]}),
    );
}

/// `--scene txd-open --tx <id> [--tab <tab>] [--width <px>] [--max]`: the parity harness's
/// panel at a given width or maximized (the web's `sidePanelWidth` / Maximize).
fn open_args(a: &mut App) {
    let args = rune::extra_args();
    let id = crate::app::flag(args, "--tx")
        .map(|id| crate::fixtures::expand(&id))
        .unwrap_or_else(|| fx("partial").into());
    let tab = crate::app::flag(args, "--tab").unwrap_or_else(|| "details".into());
    if let Some(w) = crate::app::flag(args, "--width").and_then(|w| w.parse::<f32>().ok()) {
        a.detail.width = w.clamp(super::MIN_WIDTH, super::MAX_WIDTH);
    }
    super::open(a, &id, &tab);
    super::settle(a);
    if args.iter().any(|x| x == "--light") {
        a.prefs.theme = crate::app::ThemeMode::Light;
    }
    if args.iter().any(|x| x == "--max") {
        a.detail.maximized = true;
    }
}

pub const SCENES: &[Scene] = &[
    ("txd-open", open_args),
    // Full local graph: lookup not needed, no explorer for regtest.
    ("txd-src-local", |a| open(a, fx("full"))),
    // Partial graph, online: the lookup is offered, with what it adds and whom it asks.
    ("txd-src-offer", |a| {
        open(a, fx("partial"));
        online(a);
    }),
    ("txd-src-offer-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        open(a, fx("partial"));
        online(a);
    }),
    ("txd-src-offer-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        open(a, fx("partial"));
        online(a);
    }),
    // The same row with offline mode on (the preview's isolated preference): button disabled.
    ("txd-src-offline", |a| open(a, fx("partial"))),
    ("txd-src-offline-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        open(a, fx("partial"));
    }),
    // SYNTHETIC: the approved lookup is in flight; the stored graph stays on screen.
    ("txd-src-running", |a| {
        open(a, fx("partial"));
        online(a);
        lookup_reply(a, fx("partial"), Entry::Loading);
    }),
    // SYNTHETIC: the lookup returned the missing input amount.
    ("txd-src-done", |a| {
        open(a, fx("partial"));
        online(a);
        if let Some(p) = local(a, fx("partial")) {
            lookup_reply(a, fx("partial"), Entry::Ready(Rc::new(completed(&p))));
        }
    }),
    // SYNTHETIC: every configured backend failed (the daemon's warning code and copy).
    ("txd-src-failed", |a| {
        open(a, fx("partial"));
        online(a);
        if let Some(p) = local(a, fx("partial")) {
            let q = with_warning(
                &p,
                "bitcoin_reference_lookup_failed_electrum",
                "Could not fetch public Bitcoin transaction references from a configured Electrum backend.",
            );
            lookup_reply(a, fx("partial"), Entry::Ready(Rc::new(q)));
        }
    }),
    // SYNTHETIC: the daemon refused the request (offline switched on after the click).
    ("txd-src-refused", |a| {
        open(a, fx("partial"));
        online(a);
        lookup_reply(
            a,
            fx("partial"),
            Entry::Failed(
                "network_egress_disabled: Network egress is disabled by offline mode".into(),
            ),
        );
    }),
    // SYNTHETIC: the automatic lookup from the wallet's own backend (`graph::auto`): running,
    // filled in, refused while offline, and its backend unreachable. No notice and no button.
    ("txd-auto-loading", |a| {
        open(a, fx("partial"));
        online(a);
        graph::scene_auto(a, fx("partial"), Entry::Loading);
    }),
    ("txd-auto-filled", |a| {
        open(a, fx("partial"));
        online(a);
        if let Some(p) = local(a, fx("partial")) {
            graph::scene_auto(a, fx("partial"), Entry::Ready(Rc::new(completed(&p))));
        }
    }),
    ("txd-auto-offline", |a| {
        open(a, fx("partial"));
        if let Some(p) = local(a, fx("partial")) {
            let q = with_warning(
                &p,
                "wallet_backend_lookup_offline",
                "Offline mode is on; input amounts were not looked up.",
            );
            graph::scene_auto(a, fx("partial"), Entry::Ready(Rc::new(q)));
        }
    }),
    ("txd-auto-offline-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        open(a, fx("partial"));
        if let Some(p) = local(a, fx("partial")) {
            let q = with_warning(
                &p,
                "wallet_backend_lookup_offline",
                "Offline mode is on; input amounts were not looked up.",
            );
            graph::scene_auto(a, fx("partial"), Entry::Ready(Rc::new(q)));
        }
    }),
    ("txd-auto-failed", |a| {
        open(a, fx("partial"));
        online(a);
        if let Some(p) = local(a, fx("partial")) {
            let q = with_warning(
                &p,
                "bitcoin_reference_lookup_failed_explorer",
                "Could not fetch public Bitcoin transaction references from a configured explorer backend.",
            );
            graph::scene_auto(a, fx("partial"), Entry::Ready(Rc::new(q)));
        }
    }),
    // Graphless deposit with a txid: the lookup could build the whole graph.
    ("txd-src-graphless", |a| {
        open(a, fx("graphless"));
        online(a);
    }),
    // Liquid: a local graph is never replaced; confidential amounts.
    ("txd-src-liquid", |a| open(a, fx("liquid"))),
    // SYNTHETIC: a Lightning payment (the demo book has none): no chain, no txid.
    ("txd-src-lightning", |a| {
        open(a, fx("graphless"));
        if let Some(r) = a.detail.open.as_mut().and_then(|o| o.record.as_mut()) {
            r["paymentMethod"] = json!("Lightning");
            r["chain"] = Value::Null;
            r["explorerId"] = Value::Null;
            r["network"] = Value::Null;
        }
    }),
    // Privacy mode: explorer links hidden.
    ("txd-src-hidden", |a| {
        a.prefs.privacy = crate::app::Privacy::HideAmounts;
        open(a, fx("partial"));
    }),
    // SYNTHETIC: previously fetched public references from the cache.
    ("txd-src-cache", |a| {
        open(a, fx("full"));
        if let Some(p) = local(a, fx("full")) {
            let mut q = (*p).clone();
            q.warnings.push(graph::Warning {
                code: "cached_reference_graph".into(),
                level: Some("info".into()),
                message: String::new(),
            });
            graph::scene_local(a, fx("full"), Entry::Ready(Rc::new(q)));
        }
    }),
    // SYNTHETIC: a mainnet row on the built-in servers: public explorer fallback and targets.
    ("txd-src-mainnet", |a| {
        open(a, fx("partial"));
        online(a);
        mainnet(a);
    }),
    // SYNTHETIC: a mainnet book with only an Electrum backend: the public explorer fallback.
    ("txd-src-fallback", |a| {
        open(a, fx("partial"));
        online(a);
        mainnet(a);
        if let Some(r) = a.reads.get_mut("ui.backends.settings.list")
            && let Some(list) = r.data["backends"].as_array_mut()
        {
            list.retain(|b| b["kind"] != "esplora");
            list[0]["is_default"] = json!(true);
        }
    }),
    // SYNTHETIC: the same with public explorer links turned off in Settings → Privacy.
    ("txd-src-mainnet-off", |a| {
        open(a, fx("partial"));
        online(a);
        mainnet(a);
        // Without an Explorer API backend, so the public fallback is what the switch governs.
        if let Some(r) = a.reads.get_mut("ui.backends.settings.list")
            && let Some(list) = r.data["backends"].as_array_mut()
        {
            list.retain(|b| b["kind"] != "esplora");
        }
        a.settings.prefs.explorer_fallbacks_allowed = false;
    }),
    // The Tax tab's "Explain" dialog for a disposal's calculation (a local read).
    ("txd-explain", |a| {
        super::pump_until(a, shell_reads_ready);
        super::open(a, fx("full"), "tax");
        super::settle(a);
        let reference = a
            .detail
            .open
            .as_ref()
            .and_then(|o| o.events.as_ref())
            .and_then(|e| {
                e["events"].as_array().and_then(|l| {
                    l.iter().find_map(|x| {
                        x.get("explanationReference")
                            .filter(|r| !r.is_null())
                            .cloned()
                    })
                })
            });
        if let Some(r) = reference {
            super::actions::open_explanation(a, r);
            super::pump_until(a, |a| {
                a.detail
                    .open
                    .as_ref()
                    .and_then(|o| o.explain.as_ref())
                    .is_some_and(|(_, l)| !matches!(l, crate::app::Load::Loading))
            });
        }
    }),
    // Transfer and swap rows (xfer): the demo book's paired rows next to its longest wallet name.
    ("xfer-table", |a| xfer_table(a, false)),
    ("xfer-table-de", |a| xfer_table(a, true)),
    // Case 1: an own-wallet transfer inside one transaction (same txid on both legs).
    ("xfer-transfer", |a| {
        xfer_open(a, fx("same_tx_transfer"), false)
    }),
    ("xfer-transfer-de", |a| {
        xfer_open(a, fx("same_tx_transfer"), true)
    }),
    // Case 2: a linked swap across two transactions (Liquid peg-out to Bitcoin).
    ("xfer-swap", |a| xfer_open(a, fx("swap"), false)),
    ("xfer-swap-de", |a| xfer_open(a, fx("swap"), true)),
    // Case 3, SYNTHETIC: a same-asset transfer you linked across two transactions, with a
    // wallet in between that the book doesn't track (the demo book has no such link).
    ("xfer-linked", |a| xfer_linked(a, false)),
    ("xfer-linked-de", |a| xfer_linked(a, true)),
    // An exchange trade without a chain transaction: the Sale / Purchase receipt (Strike rows).
    ("xfer-trade-sell", |a| {
        xfer_trade(a, fx("strike_sell"), false)
    }),
    ("xfer-trade-buy", |a| xfer_trade(a, fx("strike_buy"), false)),
    ("xfer-trade-sell-de", |a| {
        xfer_trade(a, fx("strike_sell"), true)
    }),
    ("xfer-trade-hidden", |a| {
        xfer_trade(a, fx("strike_sell"), false);
        a.prefs.privacy = crate::app::Privacy::HideAmounts;
    }),
    // The route card's ⋯ menu (Unlink) on the owner's swap link.
    ("xfer-swap-menu", |a| {
        xfer_open(a, fx("swap"), false);
        super::graph::scene_route_menu(a);
    }),
    // Scroll test: the panel at 1440 × 900 with the 3D graph, scrolled by scripts/txd-scroll.json.
    ("txd-scroll", |a| open(a, fx("full"))),
    // A toast over the 3D graph (overlays draw above it).
    ("txd-graph-toast", |a| {
        open(a, fx("full"));
        a.graph.sources_open = false;
        a.toast(
            crate::app::ToastTone::Success,
            "BTC price refreshed",
            None,
            Some("market-rate-refresh"),
        );
    }),
    // Graph focus (Expand on the panel's graph): the panel maximized with the graph as its body.
    // `--size 980x700` shows it in the sheet.
    ("txd-graph-focus", |a| focus(a, fx("multi"))),
    ("txd-graph-focus-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        focus(a, fx("multi"));
    }),
    ("txd-graph-focus-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        focus(a, fx("swap"));
    }),
    ("txd-graph-focus-fanout", |a| focus(a, fx("fanout"))),
    // A leg under the pointer: its card over the drawing.
    ("txd-graph-focus-hover", |a| {
        focus(a, fx("full"));
        a.graph.views[1].hover = Some("output:out-0".into());
        a.graph.active = Some("output:out-0".into());
    }),
    ("txd-graph-focus-hidden", |a| {
        a.prefs.privacy = crate::app::Privacy::HideAmounts;
        focus(a, fx("full"));
    }),
    // A graphless import: the empty state in focus (Expand isn't offered there; set directly).
    ("txd-graph-focus-graphless", |a| focus(a, fx("graphless"))),
];

/// Opens `id` in the docked panel and enters graph focus, as the Expand button does.
fn focus(a: &mut App, id: &str) {
    super::pump_until(a, shell_reads_ready);
    super::open(a, id, "details");
    super::settle(a);
    graph::scene_expand(a);
}

fn xfer_table(a: &mut App, de: bool) {
    if de {
        a.prefs.lang = crate::i18n::Lang::De;
    }
    // The rows the table scenes filter to: swaps, transfers and sends of several wallets.
    let rows = [
        "swap",
        "same_tx_transfer",
        "liquid",
        "cold_send",
        "merchant_send",
        "multi",
    ]
    .map(fx)
    .join(",");
    a.navigate(&format!("/transactions?period=all&txids={rows}"));
    crate::transactions::prime(a, true);
    a.txs.expanded = true;
}

fn xfer_open(a: &mut App, id: &str, de: bool) {
    if de {
        a.prefs.lang = crate::i18n::Lang::De;
    }
    // On the Transactions page, which wires Unlink as it wires the Linked tab's Unpair.
    a.navigate("/transactions?period=all");
    crate::transactions::prime(a, true);
    open(a, id);
    a.graph.sources_open = false;
    // `--panel-w <pt>` docks the panel that wide; `--panel-max` maximizes it.
    let args = rune::extra_args();
    if let Some(w) = crate::app::flag(args, "--panel-w").and_then(|w| w.parse::<f32>().ok()) {
        a.detail.width = w;
    }
    if args.iter().any(|x| x == "--panel-max") {
        a.detail.maximized = true;
    }
}

/// SYNTHETIC: a Cold Storage 2024 Rotation receipt the owner linked to a send from Merchant 2022
/// Rotation seven months earlier, through a wallet the book doesn't track. Both rows are real
/// demo rows; only the graph read's `swapRoute` is replaced, by exactly what the daemon returned
/// after `kassiber transfers pair --kind manual --policy carrying-value` on a copy of the book:
/// the link carries what arrived and leaves the rest of the send unlinked.
fn xfer_linked(a: &mut App, de: bool) {
    xfer_open(a, fx("linked_in"), de);
    let Some(p) = local(a, fx("linked_in")) else {
        return;
    };
    let mut q = (*p).clone();
    let route = json!({
        "id": "synthetic-linked-pair", "kind": "manual", "routeKind": "transfer", "policy": "carrying-value",
        "pairSource": "manual", "createdAt": "2026-11-14T09:12:00Z", "currentLeg": "in",
        "outAmountBtc": 0.09909123, "outFullAmountBtc": 0.09910158,
        "out": {"id": fx("linked_out"), "externalId": crate::fixtures::field("linked_out", "txid"),
                "txid": crate::fixtures::field("linked_out", "txid"), "asset": "BTC", "network": "Bitcoin",
                "amountBtc": 0.09909123, "feeBtc": 0.00003311, "occurredAt": "2023-10-29T07:25:00Z", "kind": "withdrawal", "role": "spend",
                "wallet": {"label": "Regtest Merchant 2022 Rotation", "kind": "address"}},
        "in": {"id": fx("linked_in"), "externalId": crate::fixtures::field("linked_in", "txid"),
               "txid": crate::fixtures::field("linked_in", "txid"), "asset": "BTC", "network": "Bitcoin",
               "amountBtc": 0.09909123, "feeBtc": 0.0, "occurredAt": "2024-05-24T10:59:00Z", "kind": "deposit", "role": "receive",
               "wallet": {"label": "Regtest Cold Storage 2024 Rotation", "kind": "descriptor"}}
    });
    q.swap_route = serde_json::from_value(route).ok();
    super::graph::scene_local(a, fx("linked_in"), Entry::Ready(Rc::new(q)));
}

/// The Strike rows as `ui.transactions.resolve` returns them in the demo book's "Exchange
/// Reconciliation" book (id and times come from the fixtures), and the sell's graph read there
/// (graphless, no chain transaction).
const XFER_SELL_ROW: &str = r#"{"id": null, "externalId": "strike:demo-sell", "explorerId": null, "date": "2025-11-09 15:59", "type": "Sell", "kind": "sell", "kindOverride": null, "asset": "BTC", "chain": "bitcoin", "network": "regtest", "paymentMethod": "Exchange", "account": "Strike", "counter": "Strike", "amountSat": -100000, "feeSat": 0, "eur": -149.0, "rate": 149000.0, "fiatCurrency": "EUR", "pricingSourceKind": "exchange_execution", "pricingQuality": "exact", "pricingExternalRef": "demo-sell", "pricingProvider": "Strike", "pricingPair": "BTC-EUR", "pricingTimestamp": "2025-11-09T15:59:00Z", "pricingMethod": "strike_csv", "queueStatus": "pending", "tag": "Sell", "note": "", "excluded": false, "conf": 0, "internal": false, "tags": [], "occurredAt": "2025-11-09T15:59:00Z"}"#;
const XFER_BUY_ROW: &str = r#"{"id": null, "externalId": "strike:demo-buy", "explorerId": null, "date": "2025-11-09 15:38", "type": "Buy", "kind": "buy", "kindOverride": null, "asset": "BTC", "chain": "bitcoin", "network": "regtest", "paymentMethod": "Exchange", "account": "Strike", "counter": "Strike", "amountSat": 1000000, "feeSat": 0, "eur": 1001.0, "rate": 100100.0, "fiatCurrency": "EUR", "pricingSourceKind": "exchange_execution", "pricingQuality": "exact", "pricingProvider": "Strike", "pricingPair": "BTC-EUR", "pricingMethod": "strike_csv", "queueStatus": "pending", "tag": "Buy", "note": "", "excluded": false, "conf": 0, "internal": false, "tags": [], "occurredAt": "2025-11-09T15:38:00Z"}"#;

/// A Strike row with the fixture's id and time (the stored replies keep everything else).
fn strike_row(id: &str) -> Value {
    let name = if id == fx("strike_sell") {
        "strike_sell"
    } else {
        "strike_buy"
    };
    let mut row: Value = serde_json::from_str(if name == "strike_sell" {
        XFER_SELL_ROW
    } else {
        XFER_BUY_ROW
    })
    .unwrap_or(Value::Null);
    let at = crate::fixtures::field(name, "occurred_at");
    row["id"] = json!(id);
    row["occurredAt"] = json!(at);
    row["date"] = json!(at.get(..16).unwrap_or(at).replace('T', " "));
    if row.get("pricingTimestamp").is_some() {
        row["pricingTimestamp"] = json!(at);
    }
    row
}

/// Opens a Strike trade. With the "Exchange Reconciliation" book active (run with a data root
/// where it is) the row and its graph read are real. Otherwise, SYNTHETIC: the row and the
/// graphless read are loaded from the replies above, as switching books would write the
/// disposable book's active profile.
fn xfer_trade(a: &mut App, id: &str, de: bool) {
    xfer_open(a, id, de);
    if a.detail.open.as_ref().is_some_and(|o| o.record.is_some()) {
        return;
    }
    let row = strike_row(id);
    super::load_record(a, row.clone(), false);
    super::settle(a);
    let graph = json!({
        "transaction": {"id": id, "chain": "bitcoin", "network": "regtest", "externalId": row["externalId"], "txid": row["externalId"],
                        "direction": if id == fx("strike_sell") { "outbound" } else { "inbound" }, "asset": "BTC",
                        "wallet": {"label": "Strike", "kind": "strike"}},
        "supportLevel": "graphless", "unsupportedReason": "graphless_import",
        "warnings": [{"code": "graphless_import", "level": "info", "message": "This source record does not contain Bitcoin input/output references for a flow diagram."}],
        "inputs": [], "outputs": [], "annotations": [], "swapRoute": null,
        "context": {"workspace": "Regtest Demo", "profile": "Exchange Reconciliation"}
    });
    if let Ok(p) = Payload::parse(&graph) {
        super::graph::scene_local(a, id, Entry::Ready(Rc::new(p)));
    }
}
