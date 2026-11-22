//! "The assistant sees: …": what the next question shares, built from typed state so the tray
//! the composer shows and the `screen_context` the turn sends are one value.
//!
//! In order: the item (the pinned one, or else the transaction open in the detail panel), the
//! Transactions chart selection as bounded filters, and the page. Every part but the page is
//! removable. Chip labels name things in plain words and stay on this device; only the typed
//! entity, the route's capability packs and bounded filters travel, and nothing travels before
//! the user sends. Only a pinned item's review packet is read for the model (`Focus::read`).

use serde_json::{Map, Value, json};

use super::{Focus, screen, transaction_focus};
use crate::app::App;
use crate::i18n::tr;
use crate::transactions::model::ChartSel;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// "Ask about this" pinned it; removing it unpins.
    Pinned,
    /// Open in the detail panel; removing it keeps it out until another transaction opens.
    Open,
    /// The Transactions chart selection; removing it keeps it out until the selection changes.
    Chart,
    /// The page on screen; always shared.
    Page,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chip {
    pub part: Part,
    pub label: String,
}

/// One turn's shared context: the chips, the `screen_context` they describe, and the item
/// (with the book it was captured in) whose review packet the daemon reads.
pub struct Shared {
    pub chips: Vec<Chip>,
    pub screen: Value,
    pub item: Option<Focus>,
}

/// What a question sent now would share. A page's request waiting for a model shares its own
/// page alone, as `chat::send_prompt` sends it.
pub fn build(app: &App) -> Shared {
    if let Some(p) = &app.assistant.parked {
        let page = p.context.clone().unwrap_or_else(|| screen::current(app));
        return compose(page, None, None, None);
    }
    let item = match app.assistant.focus.clone() {
        Some(f) => Some((Part::Pinned, f)),
        None => open_item(app).map(|f| (Part::Open, f)),
    };
    compose(
        screen::current(app),
        item,
        chart(app),
        app.assistant.hidden_open.as_deref(),
    )
}

/// The id of the transaction open in the detail panel, once its record is read.
pub fn open_id(app: &App) -> Option<&str> {
    app.detail
        .open
        .as_ref()
        .and_then(|o| o.id())
        .filter(|id| !id.is_empty())
}

/// The open transaction, unless the user removed it; its book is the open book now (none while
/// the book's ids are unknown).
fn open_item(app: &App) -> Option<Focus> {
    use crate::txdetail::draft::opt_s;
    let id = open_id(app)?;
    if app.assistant.hidden_open.as_deref() == Some(id) {
        return None;
    }
    let r = app.detail.open.as_ref()?.record.as_ref()?;
    let held = opt_s(r, "quarantineReason").is_some();
    let book = super::chat::book_now(app)?;
    let mut item = transaction_focus(id, opt_s(r, "date").as_deref(), held, book)?;
    item.read = false;
    Some(item)
}

/// The chart selection on the Transactions page, unless the user removed it.
fn chart(app: &App) -> Option<&ChartSel> {
    if app.shell.settings_open || app.route.path() != "/transactions" {
        return None;
    }
    let c = app.txs.filters.chart.as_ref()?;
    (app.assistant.hidden_chart.as_ref() != Some(c)).then_some(c)
}

/// The chart selection as filters: the flows shown, the segment and the bucket's period key
/// (`2024`, `2024-Q1`, `2024-03`), never its localized label.
fn chart_filters(c: &ChartSel) -> Vec<(&'static str, Value)> {
    let mut f = vec![(
        "chart_flows",
        json!(if c.external { "external" } else { "all" }),
    )];
    if let Some(s) = c.segment {
        f.push(("chart_segment", json!(s.param())));
    }
    if let Some(k) = c
        .bucket_key
        .as_deref()
        .filter(|k| k.len() <= 16 && screen::safe_entity_id(Some(k)).as_deref() == Some(*k))
    {
        f.push(("chart_bucket", json!(k)));
    }
    f
}

/// The page's context with the item's entity and capability packs and the chart's filters
/// added, and the chips that name each part. `hidden`: a transaction the user removed from the
/// tray; a page whose own address names it (`/source-of-funds?tx=`, `/review?tx=`) does not
/// send it either. A transaction the page names that is still shared is said on the page chip.
fn compose(
    mut screen: Value,
    item: Option<(Part, Focus)>,
    chart: Option<&ChartSel>,
    hidden: Option<&str>,
) -> Shared {
    let mut chips = Vec::new();
    let item = item.filter(|(_, f)| {
        f.context["entity_type"].is_string() && f.context["entity_id"].is_string()
    });
    if item.is_none()
        && screen["entity_type"] == "transaction"
        && hidden.is_some_and(|h| screen["entity_id"] == h)
        && let Some(m) = screen.as_object_mut()
    {
        m.remove("entity_type");
        m.remove("entity_id");
    }
    let page_item = item.is_none() && screen["entity_type"] == "transaction";
    if let Some((part, f)) = &item {
        screen["entity_type"] = f.context["entity_type"].clone();
        screen["entity_id"] = f.context["entity_id"].clone();
        let mut caps = screen["capabilities"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for c in f.context["capabilities"].as_array().into_iter().flatten() {
            if !caps.contains(c) {
                caps.push(c.clone());
            }
        }
        screen["capabilities"] = Value::Array(caps);
        let label = match part {
            Part::Open => tr("assistant:panel.contextOpen", &[("label", &f.label)]),
            _ => f.label.clone(),
        };
        chips.push(Chip { part: *part, label });
    }
    if let Some(c) = chart.filter(|_| screen["route"] == "/transactions") {
        let mut f: Map<String, Value> = screen["filters"].as_object().cloned().unwrap_or_default();
        f.extend(chart_filters(c).into_iter().map(|(k, v)| (k.to_owned(), v)));
        screen["filters"] = Value::Object(f);
        chips.push(Chip {
            part: Part::Chart,
            label: c.label(),
        });
    }
    let route = screen["route"].as_str().unwrap_or("/overview");
    chips.push(Chip {
        part: Part::Page,
        label: tr(
            if page_item {
                "assistant:panel.contextPageItem"
            } else {
                "assistant:panel.contextPage"
            },
            &[("page", crate::shell::page_title(route))],
        ),
    });
    Shared {
        chips,
        screen,
        item: item.map(|(_, f)| f),
    }
}

/// The chip's remove button: unpin, or keep the open transaction or chart selection out.
pub fn remove(app: &mut App, part: Part) {
    match part {
        Part::Pinned => {
            // The same transaction open in the detail panel would otherwise take its place at
            // once: one removal stops sharing it.
            let pinned = app
                .assistant
                .focus
                .take()
                .and_then(|f| f.context["entity_id"].as_str().map(str::to_owned));
            if pinned.is_some() && pinned.as_deref() == open_id(app) {
                app.assistant.hidden_open = pinned;
            }
        }
        Part::Open => app.assistant.hidden_open = open_id(app).map(str::to_owned),
        Part::Chart => app.assistant.hidden_chart = app.txs.filters.chart.clone(),
        Part::Page => {}
    }
}

/// Forget a removal once what it removed is gone: another transaction opened, the selection
/// changed. True when anything changed.
pub fn poll(app: &mut App) -> bool {
    let mut changed = false;
    if app.assistant.hidden_open.is_some() && app.assistant.hidden_open.as_deref() != open_id(app) {
        app.assistant.hidden_open = None;
        changed = true;
    }
    if app.assistant.hidden_chart.is_some()
        && app.assistant.hidden_chart.as_ref() != app.txs.filters.chart.as_ref()
    {
        app.assistant.hidden_chart = None;
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transactions::model::Segment;

    fn book() -> super::super::chat::Book {
        ("w".to_owned(), "p".to_owned())
    }

    fn pin(id: &str, held: bool) -> Focus {
        transaction_focus(id, Some("2025-03-05 10:00"), held, book()).expect("safe id")
    }

    fn sel() -> ChartSel {
        ChartSel {
            bucket_key: Some("2025-Q1".into()),
            bucket_label: "Q1 2025".into(),
            segment: Some(Segment::Outgoing),
            external: true,
        }
    }

    /// The tray names exactly what the context carries: an item chip iff the entity is the
    /// item's (and it is read), a chart chip iff chart filters travel, and the page last.
    fn assert_tray_is_json(s: &Shared) {
        let parts: Vec<Part> = s.chips.iter().map(|c| c.part).collect();
        let has_item = parts.contains(&Part::Pinned) || parts.contains(&Part::Open);
        assert_eq!(has_item, s.item.is_some());
        match &s.item {
            Some(f) => assert_eq!(s.screen["entity_id"], f.context["entity_id"]),
            // A transaction the page itself names travels only when the page chip says so.
            None => assert_eq!(
                s.screen["entity_type"] == "transaction",
                s.chips.last().unwrap().label
                    == tr(
                        "assistant:panel.contextPageItem",
                        &[(
                            "page",
                            crate::shell::page_title(s.screen["route"].as_str().unwrap())
                        )]
                    )
            ),
        }
        let chart = s.screen["filters"]["chart_flows"].is_string();
        assert_eq!(parts.contains(&Part::Chart), chart);
        assert_eq!(parts.last(), Some(&Part::Page));
        let page = crate::shell::page_title(s.screen["route"].as_str().unwrap());
        assert!(s.chips.last().unwrap().label.contains(page));
        for c in &s.chips {
            assert!(!c.label.contains("tx-"), "no raw id in a chip: {}", c.label);
        }
    }

    #[test]
    fn the_page_alone() {
        let s = compose(
            screen::context_for("/reports", "year=2024"),
            None,
            None,
            None,
        );
        assert_tray_is_json(&s);
        assert_eq!(s.chips.len(), 1);
        assert_eq!(
            s.screen,
            json!({"route": "/reports", "capabilities": ["reports"], "filters": {"year": 2024}})
        );
    }

    #[test]
    fn an_open_transaction_on_another_page() {
        let s = compose(
            screen::context_for("/reports", ""),
            Some((Part::Open, pin("tx-1", false))),
            None,
            None,
        );
        assert_tray_is_json(&s);
        assert_eq!(s.chips[0].part, Part::Open);
        assert!(s.chips[0].label.contains("2025-03-05"));
        assert_eq!(
            s.screen,
            json!({"route": "/reports", "capabilities": ["reports", "transactions"], "entity_type": "transaction", "entity_id": "tx-1"})
        );
    }

    #[test]
    fn a_pinned_held_transaction_with_the_chart_selection() {
        let s = compose(
            screen::context_for("/transactions", "tab=pricing"),
            Some((Part::Pinned, pin("tx-2", true))),
            Some(&sel()),
            None,
        );
        assert_tray_is_json(&s);
        let parts: Vec<Part> = s.chips.iter().map(|c| c.part).collect();
        assert_eq!(parts, vec![Part::Pinned, Part::Chart, Part::Page]);
        assert_eq!(
            s.screen,
            json!({
                "route": "/transactions",
                "capabilities": ["transactions", "review"],
                "entity_type": "transaction",
                "entity_id": "tx-2",
                "filters": {
                    "tab": "pricing",
                    "chart_flows": "external",
                    "chart_segment": "outgoing",
                    "chart_bucket": "2025-Q1",
                },
            })
        );
    }

    #[test]
    fn the_chart_only_travels_on_transactions() {
        let mut c = sel();
        c.bucket_key = Some("2025 / Q1".into());
        c.segment = None;
        let s = compose(
            screen::context_for("/transactions", ""),
            None,
            Some(&c),
            None,
        );
        assert_tray_is_json(&s);
        assert_eq!(s.screen["filters"], json!({"chart_flows": "external"}));
        let s = compose(screen::context_for("/reports", ""), None, Some(&c), None);
        assert_tray_is_json(&s);
        assert_eq!(s.chips.len(), 1);
    }

    /// A page whose address names a transaction says so on its chip, and stops sending it once
    /// the user removed that transaction from the tray.
    #[test]
    fn a_transaction_the_page_names_is_shown_or_removed() {
        let page = || screen::context_for("/source-of-funds", "tx=tx-5");
        let s = compose(page(), None, None, None);
        assert_tray_is_json(&s);
        assert_eq!(s.screen["entity_id"], "tx-5");
        assert!(
            s.item.is_none(),
            "named by the page, not read for the model"
        );
        let s = compose(page(), None, None, Some("tx-5"));
        assert_tray_is_json(&s);
        assert_eq!(s.screen["entity_id"], Value::Null);
        assert_eq!(s.screen["entity_type"], Value::Null);
        // Another removed transaction leaves the page's own alone.
        let s = compose(page(), None, None, Some("tx-6"));
        assert_eq!(s.screen["entity_id"], "tx-5");
    }

    /// The tray as the composer draws it, headless, in both languages: every chip and remove
    /// button is labelled (the scene check's own rules) and no string is missing.
    #[test]
    fn the_tray_is_labelled_and_translated() {
        fn tray_app() -> App {
            let mut app = crate::app::tests::quiet_app();
            app.route = crate::app::Route("/transactions".into());
            app.prefs.ai_features = true;
            app.assistant.focus = Some(pin("tx-2", true));
            app.txs.filters.chart = Some(sel());
            app
        }
        fn tray_view(app: &App, _cx: &mut rune::Cx<App>) -> rune::El {
            crate::i18n::set_lang(app.prefs.lang);
            let tk = app.tk();
            rune_kit::root(
                rune::col()
                    .w_full()
                    .padding(16.0)
                    .child(super::super::composer::context_tray(app, &tk, false)),
                vec![],
            )
        }
        for lang in [crate::i18n::Lang::En, crate::i18n::Lang::De] {
            let mut rt = rune::App::new(rune::AppSpec::new(
                "tray",
                (900.0, 400.0),
                tray_app,
                tray_view,
            ))
            .headless_runtime((900.0, 400.0), rune::Env::new(true, 1.0));
            rt.state.prefs.lang = lang;
            for _ in 0..3 {
                rt.frame(1.0 / 60.0);
            }
            let found = crate::checks::a11y_findings(&rt.accessibility_json(), &rt.elements());
            assert!(found.is_empty(), "{lang:?}: {found:?}");
            let missing: Vec<String> = crate::i18n::missing()
                .into_iter()
                .filter(|k| k.contains("assistant:panel."))
                .collect();
            assert!(missing.is_empty(), "{lang:?}: {missing:?}");
            let tree = rt.accessibility_json().to_string();
            assert!(
                tree.contains("2025-03-05"),
                "{lang:?}: the pinned item's chip"
            );
            assert!(!tree.contains("tx-2"), "{lang:?}: no raw id on screen");
        }
    }

    /// On the app: the open transaction joins on any route once the book is known, a removal
    /// keeps it out until another opens, and a pin wins over it.
    #[test]
    fn the_open_transaction_is_ambient_until_removed() {
        let mut app = crate::app::tests::quiet_app();
        app.route = crate::app::Route("/reports".into());
        let open = |app: &mut App, id: &str| {
            let mut o = crate::txdetail::Open::new(id, crate::txdetail::Tab::Details, vec![]);
            o.record = Some(json!({"id": id, "date": "2025-03-05 10:00"}));
            app.detail.open = Some(Box::new(o));
        };
        open(&mut app, "tx-1");
        assert!(
            build(&app).item.is_none(),
            "no item before the book is known"
        );
        app.reads.insert(
            "ui.profiles.snapshot".into(),
            crate::app::Reply {
                kind: "ui.profiles.snapshot".into(),
                data: json!({"activeWorkspaceId": "w", "activeProfileId": "p"}),
                error: None,
                error_detail: Value::Null,
                elapsed: std::time::Duration::ZERO,
            },
        );
        let s = build(&app);
        assert_tray_is_json(&s);
        assert_eq!(s.chips[0].part, Part::Open);
        assert_eq!(s.item.as_ref().unwrap().book, book());
        // Shared as its typed entity, not read for the model: the user did not ask about it.
        assert!(!s.item.as_ref().unwrap().read);

        remove(&mut app, Part::Open);
        assert!(!poll(&mut app));
        let s = build(&app);
        assert_tray_is_json(&s);
        assert!(s.item.is_none());
        assert_eq!(s.screen["entity_id"], Value::Null);

        open(&mut app, "tx-3");
        assert!(poll(&mut app));
        assert_eq!(build(&app).screen["entity_id"], "tx-3");

        app.assistant.focus = Some(pin("tx-9", false));
        let s = build(&app);
        assert_tray_is_json(&s);
        assert_eq!(s.chips[0].part, Part::Pinned);
        assert_eq!(s.screen["entity_id"], "tx-9");
        assert!(s.item.as_ref().unwrap().read, "a pin is read for the model");

        // Unpinning the transaction that is also open stops sharing it in one click.
        app.assistant.focus = Some(pin("tx-3", false));
        remove(&mut app, Part::Pinned);
        let s = build(&app);
        assert_tray_is_json(&s);
        assert!(s.item.is_none());
        assert_eq!(s.screen["entity_id"], Value::Null);
    }
}
