//! Headless scenes for the source-of-funds journey (parity/scenes.d/sof.json). The case targets
//! are demo-book fixtures (parity/fixtures.json): `sof_blocked` is an outgoing Merchant 2022
//! payment whose history stops at a missing-history blocker after auto-assembly; `full` is the
//! exportable case, seeded on a temporary copy of the book (`sof-seeded`: an assembled hop
//! covered by a reviewed fiat-purchase root source, and a recipient).

use super::{Stage, model};
use crate::app::{App, ThemeMode};
use crate::i18n::Lang;

fn a_tx() -> &'static str {
    crate::fixtures::id("sof_blocked")
}

fn b_tx() -> &'static str {
    crate::fixtures::id("full")
}

fn open(a: &mut App, route: &str) {
    a.prefs.developer_tools = true;
    a.navigate(route);
    super::prime(a);
}

fn with(a: &mut App, f: impl FnOnce(&mut super::Case)) {
    if let Some(c) = a.provenance.sof.case.as_mut() {
        f(c);
    }
    super::prime(a);
}

fn trace(a: &mut App, tx: &str) {
    open(a, &format!("/source-of-funds?tx={tx}"));
    super::forward(a);
    super::prime(a);
}

fn disclose(a: &mut App, tx: &str) {
    // The exportable case is seeded on a copy of the book (the blocked one needs no seed).
    if tx == b_tx() {
        crate::fixtures::use_book(a, "sof-seeded");
    }
    trace(a, tx);
    super::forward(a);
    super::prime(a);
}

fn select_upstream(a: &mut App) {
    let id = super::report(a).and_then(|r| {
        model::arr(&r["simplified_flow"], "levels")
            .iter()
            .find(|l| model::s(l, "role") == "upstream")
            .and_then(|l| l["nodes"][0]["id"].as_str().map(str::to_owned))
    });
    if let (Some(id), Some(c)) = (id, a.provenance.sof.case.as_mut()) {
        c.graph.entry("trace").or_default().selected = Some((true, id));
    }
}

pub const SCENES: &[crate::app::Scene] = &[
    ("provenance-sof-target", |a| open(a, "/source-of-funds")),
    ("provenance-sof-target-light", |a| {
        a.prefs.theme = ThemeMode::Light;
        open(a, "/source-of-funds");
    }),
    ("provenance-sof-target-de", |a| {
        a.prefs.lang = Lang::De;
        open(a, "/source-of-funds");
    }),
    ("provenance-sof-loading", |a| {
        a.prefs.developer_tools = true;
        a.navigate("/source-of-funds");
    }),
    ("provenance-sof-filters", |a| {
        open(a, "/source-of-funds");
        with(a, |c| c.show_filters = true);
    }),
    ("provenance-sof-filter-menu", |a| {
        open(a, "/source-of-funds");
        with(a, |c| c.show_filters = true);
        a.provenance.sof.popup = Some(super::Popup::Select("f-date"));
    }),
    ("provenance-sof-empty", |a| {
        open(a, "/source-of-funds");
        with(a, |c| {
            c.search = rune::TextEdit::new("no-such-transaction-zz")
        });
    }),
    ("provenance-sof-options", |a| {
        open(a, "/source-of-funds");
        with(a, |c| {
            c.open.insert("target-options".into());
        });
    }),
    ("provenance-sof-planned", |a| {
        open(a, "/source-of-funds");
        with(a, |c| {
            c.open.insert("target-options".into());
            c.purpose = "planned_exchange_sale".into();
        });
    }),
    ("provenance-sof-coverage", |a| {
        open(a, "/source-of-funds");
        with(a, |c| c.show_coverage = true);
    }),
    ("provenance-sof-selected", |a| {
        open(a, &format!("/source-of-funds?tx={}", a_tx()))
    }),
    ("provenance-sof-trace", |a| trace(a, a_tx())),
    ("provenance-sof-trace-light", |a| {
        a.prefs.theme = ThemeMode::Light;
        trace(a, a_tx());
    }),
    ("provenance-sof-trace-de", |a| {
        a.prefs.lang = Lang::De;
        trace(a, a_tx());
    }),
    ("provenance-sof-trace-advanced", |a| {
        trace(a, a_tx());
        with(a, |c| c.show_advanced = true);
    }),
    ("provenance-sof-trace-brief", |a| {
        trace(a, a_tx());
        with(a, |c| {
            c.show_advanced = true;
            c.open.insert("evidence".into());
        });
    }),
    ("provenance-sof-trace-gap", |a| {
        trace(a, a_tx());
        let f = super::report(a)
            .and_then(|r| model::arr(&r["explain_gates"], "blockers").first().cloned());
        if let Some(f) = f {
            super::gap_action(a, "open_source_creator", &f);
        }
        super::prime(a);
    }),
    ("provenance-sof-trace-node", |a| {
        trace(a, a_tx());
        select_upstream(a);
    }),
    ("provenance-sof-disclose", |a| disclose(a, b_tx())),
    ("provenance-sof-disclose-de", |a| {
        a.prefs.lang = Lang::De;
        disclose(a, b_tx());
    }),
    ("provenance-sof-disclose-details", |a| {
        disclose(a, b_tx());
        with(a, |c| {
            c.open.insert("preview-details".into());
        });
    }),
    ("provenance-sof-disclose-options", |a| {
        disclose(a, b_tx());
        with(a, |c| {
            c.open.insert("report-options".into());
        });
    }),
    ("provenance-sof-disclose-recipient", |a| {
        disclose(a, b_tx());
        let id = a.provenance.sof.recipients.data().and_then(|d| {
            model::arr(d, "recipients")
                .first()
                .map(|r| model::s(r, "id").to_owned())
        });
        with(a, |c| {
            c.open.insert("report-options".into());
            if let Some(id) = id {
                c.recipient = id;
            }
        });
    }),
    ("provenance-sof-disclose-graphics", |a| {
        disclose(a, b_tx());
        with(a, |c| {
            c.open.insert("print-graphics".into());
        });
    }),
    ("provenance-sof-disclose-blocked", |a| disclose(a, a_tx())),
    ("provenance-sof-export", |a| {
        disclose(a, b_tx());
        super::forward(a);
        super::prime(a);
    }),
    ("provenance-sof-export-light", |a| {
        a.prefs.theme = ThemeMode::Light;
        disclose(a, b_tx());
        super::forward(a);
        super::prime(a);
    }),
    // Writes the PDF into the disposable book's exports folder and answers the Save As with
    // "cancel" (the managed file stays).
    ("provenance-sof-exported", |a| {
        disclose(a, b_tx());
        super::forward(a);
        super::prime(a);
        super::export(a, false);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while a.provenance.sof.pending_save.is_none()
            && a.provenance
                .sof
                .case
                .as_ref()
                .is_some_and(|c| c.exporting.is_some())
            && std::time::Instant::now() < deadline
        {
            a.pump();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if let Some(w) = a.provenance.sof.pending_save.take() {
            super::finish(a, &w, Ok(None));
        }
        let _ = Stage::Export;
    }),
];
