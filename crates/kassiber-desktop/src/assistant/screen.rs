//! The typed screen context each turn carries (assistantScreenContext.ts): a positive allowlist
//! of routes, capability packs, a safe entity id, bounded filters, and the counts the page shows
//! with the book state they were counted on. Never descriptors, file content or secrets.

use serde_json::{Map, Value, json};

use crate::app::App;

const TRANSACTION_TABS: [&str; 5] = ["details", "classify", "pricing", "tax", "linked"];
const TRANSACTION_QUICK: [&str; 5] = [
    "external_flow",
    "review_queue",
    "no_explorer_id",
    "missing_price",
    "failed_import",
];
const EXIT_DESTINATIONS: [&str; 2] = ["eu_eea", "third_country"];

/// `SAFE_ENTITY_ID`: `^[A-Za-z0-9][A-Za-z0-9._:@+-]{0,255}$`.
pub fn safe_entity_id(v: Option<&str>) -> Option<String> {
    let t = v?.trim();
    let mut chars = t.chars();
    let first = chars.next()?;
    let ok = first.is_ascii_alphanumeric()
        && t.len() <= 256
        && chars.all(|c| c.is_ascii_alphanumeric() || "._:@+-".contains(c));
    ok.then(|| t.to_owned())
}

/// `%XX` decoding of a query value (`+` as space), as URLSearchParams does.
fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
                {
                    Ok(v) => {
                        out.push(v);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn params(search: &str) -> Vec<(String, String)> {
    search
        .trim_start_matches('?')
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (decode(k), decode(v))
        })
        .collect()
}

fn get<'a>(ps: &'a [(String, String)], k: &str) -> Option<&'a str> {
    ps.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str())
}

fn first_safe(ps: &[(String, String)], keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| safe_entity_id(get(ps, k)))
}

fn ctx(route: &str, caps: &[&str]) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("route".into(), json!(route));
    m.insert("capabilities".into(), json!(caps));
    m
}

fn entity(m: &mut Map<String, Value>, ty: &str, id: Option<String>) {
    if let Some(id) = id {
        m.insert("entity_type".into(), json!(ty));
        m.insert("entity_id".into(), json!(id));
    }
}

fn filters(m: &mut Map<String, Value>, entries: Vec<(&str, Option<Value>)>) {
    let f: Map<String, Value> = entries
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| (k.to_owned(), v)))
        .collect();
    if !f.is_empty() {
        m.insert("filters".into(), Value::Object(f));
    }
}

/// `assistantScreenContextFor(pathname, search)`, in the daemon's snake_case wire shape.
pub fn context_for(pathname: &str, search: &str) -> Value {
    let path = if !pathname.starts_with('/') || pathname.contains('\\') {
        "/overview".to_owned()
    } else {
        pathname.trim_end_matches('/').to_owned()
    };
    let path = if path.is_empty() {
        "/overview".to_owned()
    } else {
        path
    };
    let ps = params(search);
    let m = match path.as_str() {
        "/transactions" => {
            // No entity: the transaction open in the detail panel travels as the tray's item
            // (`tray::build`), which the user can see and remove.
            let mut m = ctx("/transactions", &["transactions"]);
            let tab = get(&ps, "tab")
                .filter(|t| TRANSACTION_TABS.contains(t))
                .map(|t| json!(t));
            let quick = get(&ps, "quick")
                .filter(|q| TRANSACTION_QUICK.contains(q))
                .map(|q| json!(q));
            filters(&mut m, vec![("tab", tab), ("quick", quick)]);
            m
        }
        "/activity" => ctx("/activity", &["transactions", "operations"]),
        "/reports" => {
            let mut m = ctx("/reports", &["reports"]);
            let year = get(&ps, "year")
                .and_then(|y| y.parse::<i64>().ok())
                .filter(|y| (2009..=2100).contains(y))
                .map(|y| json!(y));
            filters(&mut m, vec![("year", year)]);
            m
        }
        "/chain-analysis" => ctx("/chain-analysis", &["privacy", "transactions", "transfers"]),
        "/privacy-mirror" => ctx("/privacy-mirror", &["privacy"]),
        "/egress" => ctx("/egress", &["privacy"]),
        "/exit-tax" => {
            let mut m = ctx("/exit-tax", &["reports"]);
            m.insert("entity_type".into(), json!("report"));
            m.insert("entity_id".into(), json!("exit-tax"));
            let dest = get(&ps, "destination")
                .filter(|d| EXIT_DESTINATIONS.contains(d))
                .map(|d| json!(d));
            let date = get(&ps, "departure_date")
                .filter(|d| {
                    d.len() == 10
                        && d.chars().enumerate().all(|(i, c)| {
                            if i == 4 || i == 7 {
                                c == '-'
                            } else {
                                c.is_ascii_digit()
                            }
                        })
                })
                .map(|d| json!(d));
            filters(
                &mut m,
                vec![("destination", dest), ("departure_date", date)],
            );
            m
        }
        "/source-of-funds" => {
            let mut m = ctx("/source-of-funds", &["source_funds", "transactions"]);
            match first_safe(&ps, &["case", "case_id"]) {
                Some(c) => entity(&mut m, "source_funds_case", Some(c)),
                None => entity(
                    &mut m,
                    "transaction",
                    first_safe(&ps, &["tx", "transaction"]),
                ),
            }
            m
        }
        "/journals" => {
            let mut m = ctx("/journals", &["transactions", "reports", "operations"]);
            entity(
                &mut m,
                "transaction",
                first_safe(&ps, &["tx", "transaction"]),
            );
            m
        }
        "/review" => {
            let mut m = ctx("/review", &["review"]);
            entity(
                &mut m,
                "transaction",
                first_safe(&ps, &["tx", "transaction"]),
            );
            m
        }
        "/swaps" | "/transfers" => {
            // `parseTransfersCustodySearch`: section (default "pairs"), tab, link.
            let section =
                get(&ps, "section").filter(|s| ["pairs", "swaps", "advanced"].contains(s));
            let tab = get(&ps, "tab");
            if section == Some("advanced") && tab.unwrap_or("gaps") == "gaps" {
                let mut m = ctx("/swaps", &["transfers", "transactions", "wallets"]);
                entity(&mut m, "custody_gap", first_safe(&ps, &["gap", "gap_id"]));
                m.insert("filters".into(), json!({"tab": "gaps"}));
                m
            } else {
                let mut m = ctx("/swaps", &["transfers", "transactions"]);
                let link = get(&ps, "link")
                    .filter(|l| *l != "new")
                    .and_then(|l| safe_entity_id(Some(l)));
                entity(&mut m, "transaction", link);
                filters(
                    &mut m,
                    vec![
                        ("section", section.map(|s| json!(s))),
                        (
                            "tab",
                            if section == Some("advanced") {
                                tab.map(|t| json!(t))
                            } else {
                                None
                            },
                        ),
                    ],
                );
                m
            }
        }
        "/custody-gaps" => {
            let mut m = ctx("/custody-gaps", &["transfers", "transactions", "wallets"]);
            entity(&mut m, "custody_gap", first_safe(&ps, &["gap", "gap_id"]));
            m
        }
        "/reconcile" => ctx("/reconcile", &["wallets", "transactions"]),
        "/connections" => ctx("/connections", &["wallets", "operations"]),
        p if p.starts_with("/connections/") && !p["/connections/".len()..].contains('/') => {
            let mut m = ctx("/connections", &["wallets", "operations"]);
            entity(
                &mut m,
                "connection",
                safe_entity_id(Some(&decode(&p["/connections/".len()..]))),
            );
            m
        }
        "/imports" | "/Imports" => ctx("/imports", &["wallets", "merchant", "transactions"]),
        p if p == "/books"
            || p == "/profiles"
            || (p.starts_with("/books/")
                && p.ends_with("/birds-eye")
                && p.matches('/').count() == 3) =>
        {
            ctx("/books", &["wallets", "reports", "operations"])
        }
        "/logs" | "/diagnostics" => ctx("/logs", &["operations"]),
        "/settings" => ctx("/settings", &["wallets", "privacy", "operations"]),
        _ => ctx("/overview", &["transactions", "reports", "operations"]),
    };
    Value::Object(m)
}

/// The context of a pinned or open transaction (the tray's item): its typed entity on the route
/// whose capability pack reads it, Review for a held one. None for an unsafe id.
pub fn item_context(tx_id: &str, quarantine: bool) -> Option<Value> {
    let id = safe_entity_id(Some(tx_id))?;
    let mut m = if quarantine {
        ctx("/review", &["review"])
    } else {
        ctx("/transactions", &["transactions"])
    };
    entity(&mut m, "transaction", Some(id));
    Some(Value::Object(m))
}

/// A custody gap's context: its typed entity and the custody capability packs.
pub fn gap_context(gap_id: &str) -> Option<Value> {
    let id = safe_entity_id(Some(gap_id))?;
    let mut m = ctx("/transfers", &["transfers", "transactions", "wallets"]);
    entity(&mut m, "custody_gap", Some(id));
    Some(Value::Object(m))
}

/// The page on screen now: the route with its own query, or Settings while its dialog is open.
pub fn current_location(app: &App) -> (String, String) {
    if app.shell.settings_open {
        return ("/settings".into(), String::new());
    }
    let path = app.route.path().to_owned();
    let search = app
        .route
        .0
        .split_once('?')
        .map(|(_, q)| q.split('#').next().unwrap_or("").to_owned())
        .unwrap_or_default();
    (path, search)
}

/// `withScreenSnapshot`: the Transactions page publishes its dashboard counts with the book state
/// they were counted on; other pages publish nothing.
fn with_snapshot(app: &App, mut c: Value) -> Value {
    if c["route"] != "/transactions" {
        return c;
    }
    let Some(d) = app.read("ui.transactions.dashboard") else {
        return c;
    };
    let scope = &d["metricsScope"];
    let summary = &d["summary"];
    let count = |v: &Value| v.as_u64();
    let metrics: Map<String, Value> = [
        ("review_queue_review", count(&summary["review"])),
        ("review_queue_pending", count(&summary["pending"])),
        ("review_queue_failed", count(&summary["failed"])),
        ("incoming_count", count(&summary["incoming"]["count"])),
        ("outgoing_count", count(&summary["outgoing"]["count"])),
    ]
    .into_iter()
    .filter_map(|(k, v)| v.map(|v| (k.to_owned(), json!(v))))
    .collect();
    if let Some(period) = d["period"].as_str().filter(|p| {
        !p.is_empty()
            && p.len() <= 16
            && p.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    }) {
        let mut f = c["filters"].as_object().cloned().unwrap_or_default();
        f.insert("period".into(), json!(period));
        c["filters"] = Value::Object(f);
    }
    let valid_scope = safe_entity_id(scope["workspaceId"].as_str()).as_deref()
        == scope["workspaceId"].as_str()
        && scope["workspaceId"].is_string()
        && safe_entity_id(scope["profileId"].as_str()).as_deref() == scope["profileId"].as_str()
        && scope["profileId"].is_string()
        && scope["inputVersion"].as_u64().is_some()
        && (scope["wallet"].is_null()
            || scope["wallet"]
                .as_str()
                .is_some_and(|w| !w.trim().is_empty() && w.len() <= 256))
        && scope["pairsGrouped"].is_boolean();
    if !metrics.is_empty() && valid_scope {
        c["metrics"] = Value::Object(metrics);
        c["metrics_scope"] = json!({
            "workspace_id": scope["workspaceId"],
            "profile_id": scope["profileId"],
            "input_version": scope["inputVersion"],
            "wallet": scope["wallet"],
            "pairs_grouped": scope["pairsGrouped"],
        });
    }
    c
}

/// `currentPageScreenContext`: the page on screen, or `None` on the full assistant page.
pub fn current_page(app: &App) -> Option<Value> {
    let (path, search) = current_location(app);
    if path == "/assistant" {
        return None;
    }
    Some(with_snapshot(app, context_for(&path, &search)))
}

/// `currentAssistantScreenContext(fallback)`: the page on screen, or on the assistant page the
/// last page the user was on (`AppShell`'s remembered context).
pub fn current(app: &App) -> Value {
    current_page(app).unwrap_or_else(|| {
        app.assistant
            .last_screen
            .clone()
            .unwrap_or_else(|| context_for("/overview", ""))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_ids_are_narrow() {
        assert_eq!(safe_entity_id(Some(" abc:1 ")).as_deref(), Some("abc:1"));
        assert_eq!(safe_entity_id(Some("/etc/passwd")), None);
        assert_eq!(safe_entity_id(Some("https://x")), None);
        assert_eq!(safe_entity_id(Some("")), None);
    }

    #[test]
    fn routes_map_to_their_packs() {
        // The route's `tx` is not the page's: the open transaction is the tray's item.
        let t = context_for("/transactions", "?tx=abc&tab=pricing&quick=nope");
        assert_eq!(
            t,
            json!({"route": "/transactions", "capabilities": ["transactions"], "filters": {"tab": "pricing"}})
        );
        assert_eq!(
            item_context("abc", true),
            Some(
                json!({"route": "/review", "capabilities": ["review"], "entity_type": "transaction", "entity_id": "abc"})
            )
        );
        assert_eq!(item_context("../x", false), None);
        assert_eq!(context_for("/reports", "year=1999")["filters"], Value::Null);
        assert_eq!(
            context_for("/reports", "year=2024")["filters"],
            json!({"year": 2024})
        );
        assert_eq!(context_for("/nowhere", "")["route"], "/overview");
        assert_eq!(
            context_for("/connections/w%201", "")["entity_id"],
            Value::Null
        );
        assert_eq!(context_for("/connections/w1", "")["entity_id"], "w1");
        assert_eq!(
            context_for("/transfers", "section=advanced")["filters"],
            json!({"tab": "gaps"})
        );
        assert_eq!(context_for("/books/x/birds-eye", "")["route"], "/books");
    }
}
