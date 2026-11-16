//! A development harness while the detail panel lives elsewhere: `/dev/graph?tx=<id>` renders
//! the graph section full-page for one transaction. Scenes call `scene_open`, which loads the
//! row (`ui.transactions.resolve`) and its graph before the first frame.

use std::time::{Duration, Instant};

use rune::{Cx, El, col, text};
use serde_json::{Value, json};

use crate::app::App;
use crate::theme::Tk;

/// Opens the harness on `id` and waits (headless scenes only) until row and graph arrived.
pub fn scene_open(app: &mut App, id: &str) {
    app.navigate(&format!("/dev/graph?tx={id}"));
    app.graph.dev_record = Some(id.to_owned());
    app.reads.remove("ui.transactions.resolve");
    app.send(
        "ui.transactions.resolve",
        json!({"query": id}),
        "graph-dev:resolve",
    );
    super::ensure(app, id);
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        app.pump();
        let row_ready = app.read("ui.transactions.resolve").is_some();
        let sh = super::shown(app, id);
        let graph_ready = !matches!(sh.entry, None | Some(super::Entry::Loading));
        // Swap legs load after the record's own graph.
        let legs_ready = !app
            .pending
            .values()
            .any(|(tag, _, _)| tag.starts_with("graph:"));
        if row_ready && graph_ready && legs_ready {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub fn dev_page(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let id = app
        .graph
        .dev_record
        .clone()
        .or_else(|| {
            app.route
                .0
                .split("tx=")
                .nth(1)
                .map(|s| s.split('&').next().unwrap_or("").to_owned())
        })
        .unwrap_or_default();
    let record: Value = app
        .read("ui.transactions.resolve")
        .map(|v| v["transaction"].clone())
        .filter(|r| r["id"].as_str() == Some(id.as_str()))
        .unwrap_or_else(|| json!({ "id": id }));
    col()
        .key("graph-dev")
        .w_full()
        .padding(tk.r(1.25))
        .gap(tk.r(0.75))
        .child(
            text(format!(
                "{} · {}",
                crate::i18n::t("transactions:graph.sectionTitle"),
                record["account"].as_str().unwrap_or(&id)
            ))
            .font(tk.tsm(600.0))
            .color(tk.fg),
        )
        .child(if app.graph.expanded {
            // Graph focus fills the harness page below its title.
            col()
                .w_full()
                .h(app.shell.window_h.max(600.0) - tk.r(6.0))
                .child(super::focus_view(
                    app,
                    cx,
                    tk,
                    &record,
                    record["account"].as_str().map(str::to_owned),
                ))
        } else {
            super::section(app, cx, tk, &record, layer)
        })
}
