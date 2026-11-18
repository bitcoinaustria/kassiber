//! The trace stage's bank statements panel (`BankStatementsPanel`): import a statement the owner
//! downloaded from their own bank, review the exact-amount matches the daemon suggests between
//! outgoing bank lines and Bitcoin purchases, and see which lines are accepted as evidence on a
//! purchase's fiat source. Collapsed by default; its reads start only while it is open, and every
//! request is local (docs/reference/bank-statements.md).

use rune::{El, TextEdit, Tx, col, row, text};
use serde_json::{Value, json};

use super::Query;
use super::kit;
use super::model;
use super::target::optional_section;
use crate::app::{App, Reply, ToastTone};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant};

pub const SUGGEST: &str = "ui.source_funds.bank_lines.suggest";
pub const LIST: &str = "ui.source_funds.bank_lines.list";
pub const IMPORT: &str = "ui.documents.import_statement";
pub const ACCEPT: &str = "ui.source_funds.bank_lines.accept";
pub const REJECT: &str = "ui.source_funds.bank_lines.reject";

/// The panel's own state, kept with the case.
#[derive(Default)]
pub struct Panel {
    pub open: bool,
    /// Bank CSV exports do not name the account; the owner states it once per import.
    pub account: TextEdit,
    pub message: Option<String>,
    pub error: Option<String>,
}

/// The panel's reads, kept beside the case's other queries.
#[derive(Default)]
pub struct Reads {
    pub suggest: Query,
    pub lines: Query,
}

// ---- Model (pure, for tests) -----------------------------------------------------------------

/// A bank line as one row: date, exact amount and currency, counterparty and masked IBAN.
pub fn line_text(line: &Value) -> String {
    let mut s = format!(
        "{} · {} {}",
        model::s(line, "booking_date"),
        model::s(line, "amount"),
        model::s(line, "currency")
    );
    let name = model::s(line, "counterparty_name");
    if !name.is_empty() {
        s.push_str(&format!(" · {name}"));
    }
    let iban = model::s(line, "counterparty_iban_masked");
    if !iban.is_empty() {
        s.push_str(&format!(" · {iban}"));
    }
    s
}

/// A purchase as one row: its date, exact fiat value and wallet.
pub fn purchase_text(buy: &Value) -> String {
    let at: String = model::s(buy, "occurred_at").chars().take(10).collect();
    format!(
        "{at} · {} {} · {}",
        model::s(buy, "fiat_value"),
        model::s(buy, "fiat_currency"),
        model::s(buy, "wallet")
    )
}

/// The suggestion's totals: bank total → purchase total, both in its currency.
pub fn totals_text(s: &Value) -> String {
    let cur = model::s(s, "currency");
    format!(
        "{} {cur} → {} {cur}",
        model::s(s, "bank_total"),
        model::s(s, "purchase_total")
    )
}

fn list<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

pub fn suggestions(data: Option<&Value>) -> Vec<Value> {
    data.map(|d| list(d, "suggestions").to_vec())
        .unwrap_or_default()
}

/// Purchases without an exact fiat value, and purchases or transfers with more than one
/// counterpart (never suggested).
pub fn open_counts(data: Option<&Value>) -> (usize, usize) {
    let Some(d) = data else { return (0, 0) };
    let unpriced = list(d, "purchases_without_exact_fiat_value").len();
    let ambiguous = list(d, "unmatched")
        .iter()
        .filter(|u| model::s(u, "reason") == "ambiguous")
        .count();
    (unpriced, ambiguous)
}

/// Only lines with an accepted decision count as evidence.
pub fn accepted(data: Option<&Value>) -> Vec<Value> {
    data.map(|d| {
        list(d, "lines")
            .iter()
            .filter(|l| {
                list(l, "decisions")
                    .iter()
                    .any(|d| model::s(d, "state") == "accepted")
            })
            .cloned()
            .collect()
    })
    .unwrap_or_default()
}

/// The decision a suggestion's buttons send: exactly its lines and purchases.
pub fn decision_args(s: &Value) -> Value {
    json!({
        "line_ids": s.get("line_ids").cloned().unwrap_or(json!([])),
        "transaction_ids": s.get("transaction_ids").cloned().unwrap_or(json!([])),
    })
}

/// The import request: the picked file, and the stated account when there is one.
pub fn import_args(file: &str, account: &str) -> Value {
    let mut a = json!({"file_path": file});
    let account = account.trim();
    if !account.is_empty() {
        a["account"] = json!(account);
    }
    a
}

/// What an import reports back.
pub fn import_message(data: &Value) -> String {
    if data.get("already_imported").and_then(Value::as_bool) == Some(true) {
        return t("sourceFunds:bank.alreadyImported");
    }
    let n = |k: &str| data.get(k).and_then(Value::as_u64).unwrap_or(0).to_string();
    tr(
        "sourceFunds:bank.imported",
        &[("new", &n("lines_new")), ("existing", &n("lines_existing"))],
    )
}

// ---- Requests --------------------------------------------------------------------------------

fn panel(a: &mut App) -> Option<&mut Panel> {
    a.provenance.sof.case.as_mut().map(|c| &mut c.bank)
}

/// Starts the panel's reads while it is open on the trace stage.
pub fn want_reads(app: &mut App, enabled: bool) {
    super::want(
        app,
        |s| &mut s.bank.suggest,
        SUGGEST,
        json!({}),
        "provenance.sof.bank.suggest",
        enabled,
    );
    super::want(
        app,
        |s| &mut s.bank.lines,
        LIST,
        json!({"direction": "out"}),
        "provenance.sof.bank.lines",
        enabled,
    );
}

fn busy(app: &App) -> bool {
    let b = &app.provenance.sof.busy;
    b.contains("bank.import") || b.contains("bank.decide")
}

fn clear_notes(a: &mut App) {
    if let Some(p) = panel(a) {
        p.error = None;
        p.message = None;
    }
}

fn pick(a: &mut App, tx: &mut Tx) {
    clear_notes(a);
    let opts = rune::FileDialogOptions {
        title: t("sourceFunds:bank.pickTitle"),
        ..Default::default()
    }
    .filter(t("sourceFunds:bank.fileFilter"), &["xml", "csv"]);
    tx.open_file(opts, |a: &mut App, result, _tx: &mut Tx| match result {
        Ok(Some(p)) => import(a, &p.to_string_lossy()),
        Ok(None) => {}
        Err(e) => {
            if let Some(p) = panel(a) {
                p.error = Some(e.0);
            }
        }
    });
}

fn import(a: &mut App, file: &str) {
    let account = panel(a)
        .map(|p| p.account.text.to_owned())
        .unwrap_or_default();
    if super::send(
        a,
        IMPORT,
        import_args(file, &account),
        "provenance.sof.bank.import",
    ) {
        a.provenance.sof.busy.insert("bank.import");
        super::sent_from(a, "bank.import");
    }
}

fn decide(a: &mut App, kind: &'static str, s: &Value) {
    clear_notes(a);
    if super::send(a, kind, decision_args(s), "provenance.sof.bank.decide") {
        a.provenance.sof.busy.insert("bank.decide");
        super::sent_from(a, "bank.decide");
    }
}

/// `provenance.sof.bank.*` replies.
pub fn on_reply(a: &mut App, sub: &str, r: &Reply) {
    let generation = a.book_generation;
    match sub {
        "suggest" => super::landed(&mut a.provenance.sof.bank.suggest, r, generation),
        "lines" => super::landed(&mut a.provenance.sof.bank.lines, r, generation),
        "import" | "decide" => {
            let kind = if sub == "import" {
                "bank.import"
            } else {
                "bank.decide"
            };
            a.provenance.sof.busy.remove(kind);
            let note = match &r.error {
                Some(e) => (None, Some(e.clone())),
                None if sub == "import" => (Some(import_message(&r.data)), None),
                None => (None, None),
            };
            // The answer belongs to the case it was sent from: another case on screen keeps its
            // own notes, and the answer shows as a toast instead.
            if super::from_this_case(a, kind)
                && let Some(p) = panel(a)
            {
                if note.0.is_some() {
                    p.message = note.0;
                }
                p.error = note.1;
            } else if let Some(e) = note.1 {
                a.toast(
                    ToastTone::Error,
                    &t("sourceFunds:header.title"),
                    Some(e),
                    Some("sof"),
                );
            } else if let Some(m) = note.0 {
                a.toast(
                    ToastTone::Success,
                    &t("sourceFunds:bank.title"),
                    Some(m),
                    Some("sof"),
                );
            }
            // Accepted lines become evidence on a reviewed source: every case read refreshes.
            if r.error.is_none() {
                super::invalidate(a);
            }
        }
        _ => {}
    }
}

// ---- View ------------------------------------------------------------------------------------

fn small(tk: &Tk, s: String) -> El {
    text(s).font(tk.txs(400.0)).color(tk.muted_fg).wrap()
}

fn suggestion_card(tk: &Tk, i: usize, s: &Value, pending: bool) -> El {
    let lines = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(tk.r(0.25))
        .role("list")
        .label(t("sourceFunds:bank.linesLabel"))
        .children(list(s, "lines").iter().map(|l| {
            text(line_text(l))
                .font(kit::tnum(tk.tsm(400.0)))
                .color(tk.fg)
                .wrap()
        }));
    let buys = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(tk.r(0.25))
        .role("list")
        .label(t("sourceFunds:bank.purchasesLabel"))
        .children(list(s, "transactions").iter().map(|b| {
            text(purchase_text(b))
                .font(kit::tnum(tk.tsm(400.0)))
                .color(tk.fg)
                .wrap()
        }));
    let reasons = col()
        .w_full()
        .gap(tk.r(0.125))
        .role("list")
        .label(t("sourceFunds:bank.reasonsLabel"))
        .children(
            list(s, "reasons")
                .iter()
                .filter_map(Value::as_str)
                .map(|r| small(tk, format!("• {r}"))),
        );
    let accept = s.clone();
    let reject = s.clone();
    let ok = kit::button(
        tk,
        format!("sof-bank-accept-{i}"),
        Variant::Default,
        Size::Sm,
        Some(&I::CHECK),
        &t("sourceFunds:bank.accept"),
        !pending,
    );
    let no = kit::button(
        tk,
        format!("sof-bank-reject-{i}"),
        Variant::Outline,
        Size::Sm,
        Some(&I::X),
        &t("sourceFunds:bank.reject"),
        !pending,
    );
    let (ok, no) = if pending {
        (ok, no)
    } else {
        (
            ok.on_press(move |a: &mut App| decide(a, ACCEPT, &accept)),
            no.on_press(move |a: &mut App| decide(a, REJECT, &reject)),
        )
    };
    kit::bordered(tk)
        .key(format!("sof-bank-suggestion-{i}"))
        .padding(tk.r(0.75))
        .gap(tk.r(0.5))
        .child(
            row()
                .w_full()
                .justify_between()
                .flex_wrap()
                .gap(tk.r(0.5))
                .child(
                    text(t("sourceFunds:bank.exactMatch"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                )
                .child(
                    text(totals_text(s))
                        .font(kit::tnum(tk.tsm(400.0)))
                        .color(tk.fg),
                ),
        )
        .child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .items_start()
                .child(lines)
                .child(buys),
        )
        .child(reasons)
        .child(row().gap(tk.r(0.5)).child(ok).child(no))
}

fn body(app: &App, tk: &Tk) -> El {
    let reads = &app.provenance.sof.bank;
    let p = &app.provenance.sof.case.as_ref().expect("case").bank;
    let pending = busy(app);
    let mut out = col()
        .w_full()
        .gap(tk.r(1.0))
        .px(tk.r(1.0))
        .pb(tk.r(1.0))
        .child(small(tk, t("sourceFunds:bank.description")))
        .child(
            col()
                .w_full()
                .max_w(tk.r(28.0))
                .gap(tk.r(0.25))
                .child(kit::field(
                    tk,
                    &t("sourceFunds:bank.accountLabel"),
                    kit::input(
                        tk,
                        "sof-bank-account",
                        &p.account,
                        &t("sourceFunds:bank.accountLabel"),
                        &t("sourceFunds:bank.accountPlaceholder"),
                        |a| &mut a.provenance.sof.case.as_mut().expect("case").bank.account,
                        tk.r(2.0),
                    ),
                ))
                .child(small(tk, t("sourceFunds:bank.accountHelp"))),
        );
    let choose = kit::button(
        tk,
        "sof-bank-import",
        Variant::Outline,
        Size::Sm,
        Some(&I::FILE_UP),
        &t("sourceFunds:bank.import"),
        !pending,
    );
    let mut bar = row()
        .w_full()
        .gap(tk.r(0.75))
        .center_y()
        .flex_wrap()
        .child(if pending {
            choose
        } else {
            choose.on_press_tx(pick)
        });
    if let Some(m) = &p.message {
        bar = bar.child(
            text(m.clone())
                .font(tk.tsm(400.0))
                .color(tk.fg)
                .role("status"),
        );
    }
    out = out.child(bar);
    if let Some(e) = &p.error {
        out = out.child(
            text(e.clone())
                .font(tk.tsm(400.0))
                .color(tk.destructive)
                .role("alert")
                .wrap(),
        );
    }
    let data = reads.suggest.data();
    if reads.suggest.fetching() || reads.suggest.loading() {
        out = out.child(small(tk, t("sourceFunds:case.loading")).role("status"));
    } else {
        let list = suggestions(data);
        out = out.child(if list.is_empty() {
            small(tk, t("sourceFunds:bank.noSuggestions"))
        } else {
            col()
                .w_full()
                .gap(tk.r(0.75))
                .role("list")
                .label(t("sourceFunds:bank.suggestionsTitle"))
                .children(
                    list.iter()
                        .enumerate()
                        .map(|(i, s)| suggestion_card(tk, i, s, pending)),
                )
        });
    }
    let (unpriced, ambiguous) = open_counts(data);
    if unpriced > 0 {
        out = out.child(small(tk, tc("sourceFunds:bank.unpriced", unpriced, &[])));
    }
    if ambiguous > 0 {
        out = out.child(small(tk, tc("sourceFunds:bank.ambiguous", ambiguous, &[])));
    }
    let ok = accepted(reads.lines.data());
    if !ok.is_empty() {
        out = out.child(
            col()
                .w_full()
                .gap(tk.r(0.25))
                .child(
                    text(t("sourceFunds:bank.acceptedTitle"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                )
                .children(ok.iter().map(|l| {
                    text(line_text(l))
                        .font(kit::tnum(tk.tsm(400.0)))
                        .color(tk.fg)
                        .wrap()
                })),
        );
    }
    out
}

/// The collapsible panel under the trace stage.
pub fn view(app: &App, tk: &Tk) -> El {
    let open = app
        .provenance
        .sof
        .case
        .as_ref()
        .is_some_and(|c| c.bank.open);
    optional_section(
        tk,
        "sof-bank",
        open,
        &I::LANDMARK,
        &t("sourceFunds:bank.title"),
        None,
        |a| {
            if let Some(p) = panel(a) {
                p.open ^= true;
            }
        },
        if open { body(app, tk) } else { rune::el() },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line() -> Value {
        json!({"id": "L1", "booking_date": "2025-03-03", "amount": "-1000.00", "currency": "EUR",
            "counterparty_name": "Payward Ltd", "counterparty_iban_masked": "AT** **** **** **** 3201"})
    }

    fn suggestion() -> Value {
        json!({"id": "abc", "currency": "EUR", "bank_total": "1000.00", "purchase_total": "1000.00",
            "line_ids": ["L1"], "transaction_ids": ["T1"], "lines": [line()],
            "transactions": [{"id": "T1", "occurred_at": "2025-03-05T10:00:00Z", "fiat_value": "1000.00",
                "fiat_currency": "EUR", "wallet": "Kraken", "exchange": "Kraken"}],
            "reasons": ["bank amount equals the purchase value 1000.00", "booked 2 days before the purchase"]})
    }

    #[test]
    fn a_suggestion_reads_its_lines_purchases_and_exact_amounts() {
        let s = suggestion();
        assert_eq!(totals_text(&s), "1000.00 EUR → 1000.00 EUR");
        assert_eq!(
            line_text(&list(&s, "lines")[0]),
            "2025-03-03 · -1000.00 EUR · Payward Ltd · AT** **** **** **** 3201"
        );
        assert_eq!(
            purchase_text(&list(&s, "transactions")[0]),
            "2025-03-05 · 1000.00 EUR · Kraken"
        );
        // A line without a counterparty or IBAN drops those parts.
        assert_eq!(
            line_text(&json!({"booking_date": "2025-01-02", "amount": "-5.00", "currency": "EUR"})),
            "2025-01-02 · -5.00 EUR"
        );
    }

    #[test]
    fn decisions_send_exactly_the_suggestions_ids() {
        assert_eq!(
            decision_args(&suggestion()),
            json!({"line_ids": ["L1"], "transaction_ids": ["T1"]})
        );
    }

    fn reply(kind: &str, data: Value, error: Option<&str>) -> Reply {
        Reply {
            kind: kind.into(),
            data,
            error: error.map(str::to_owned),
            error_detail: Value::Null,
            elapsed: Default::default(),
        }
    }

    fn open_case(app: &mut App, target: &str) {
        app.provenance.sof.mounted = Some(format!("book|{target}"));
        app.provenance.sof.case = Some(super::super::Case::new(
            &super::super::Draft::default(),
            target,
        ));
    }

    /// An import or decision answered after another case opened: its note stays out of the case
    /// on screen (it shows as a toast), and the next answer for that case lands in it.
    #[test]
    fn a_reply_lands_in_the_case_it_was_sent_from() {
        let mut app = crate::app::tests::quiet_app();
        open_case(&mut app, "tx-a");
        import(&mut app, "/x/statement.xml");
        decide(&mut app, ACCEPT, &suggestion());
        open_case(&mut app, "tx-b");
        let shown = |app: &App| {
            app.toasts
                .last()
                .map(|t| (t.tone, t.body.clone().unwrap_or_default()))
        };
        on_reply(
            &mut app,
            "import",
            &reply(IMPORT, json!({"lines_new": 3, "lines_existing": 0}), None),
        );
        let (tone, body) = shown(&app).expect("the import's answer shows");
        assert_eq!(tone, ToastTone::Success);
        assert!(body.contains('3'), "{body}");
        on_reply(
            &mut app,
            "decide",
            &reply(ACCEPT, json!({}), Some("refused")),
        );
        assert_eq!(
            shown(&app),
            Some((ToastTone::Error, "refused".to_owned())),
            "the decision's error shows"
        );
        let b = &app.provenance.sof.case.as_ref().expect("case").bank;
        assert_eq!((&b.message, &b.error), (&None, &None));
        assert!(app.provenance.sof.sent_from.values().all(|q| q.is_empty()));
        assert!(!busy(&app));

        import(&mut app, "/x/giro.csv");
        on_reply(
            &mut app,
            "import",
            &reply(IMPORT, json!({"already_imported": true}), None),
        );
        let b = &app.provenance.sof.case.as_ref().expect("case").bank;
        assert_eq!(b.message, Some(t("sourceFunds:bank.alreadyImported")));
        assert_eq!(b.error, None);
    }

    #[test]
    fn an_import_names_the_account_only_when_stated() {
        assert_eq!(
            import_args("/x/statement.xml", "  "),
            json!({"file_path": "/x/statement.xml"})
        );
        assert_eq!(
            import_args("/x/giro.csv", " Giro "),
            json!({"file_path": "/x/giro.csv", "account": "Giro"})
        );
        assert_eq!(
            import_message(&json!({"already_imported": true})),
            t("sourceFunds:bank.alreadyImported")
        );
        let m = import_message(
            &json!({"lines_new": 3, "lines_existing": 2, "already_imported": false}),
        );
        assert!(m.contains('3') && m.contains('2'), "{m}");
    }

    #[test]
    fn only_accepted_lines_count_as_evidence() {
        let ok = json!({"id": "L1", "decisions": [{"transaction_id": "T1", "state": "accepted", "source_id": "S1"}]});
        let no = json!({"id": "L2", "decisions": [{"transaction_id": "T1", "state": "rejected", "source_id": null}]});
        let plain = json!({"id": "L3"});
        let got = accepted(Some(&json!({"lines": [ok, no, plain]})));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["id"], "L1");
        assert!(accepted(None).is_empty());
    }

    #[test]
    fn open_counts_name_unpriced_and_ambiguous_but_not_unmatched() {
        let d = json!({"suggestions": [], "purchases_without_exact_fiat_value": ["T2", "T3"],
            "unmatched": [{"line_id": "L9", "reason": "ambiguous"}, {"transaction_id": "T4", "reason": "no_exact_amount"}]});
        assert_eq!(open_counts(Some(&d)), (2, 1));
        assert_eq!(open_counts(None), (0, 0));
        assert!(suggestions(Some(&d)).is_empty());
    }

    #[test]
    fn every_panel_string_has_a_bundle_key() {
        for k in [
            "title",
            "description",
            "import",
            "pickTitle",
            "fileFilter",
            "suggestionsTitle",
            "noSuggestions",
            "linesLabel",
            "purchasesLabel",
            "reasonsLabel",
            "accept",
            "reject",
            "acceptedTitle",
            "exactMatch",
            "accountLabel",
            "accountPlaceholder",
            "accountHelp",
        ] {
            assert!(crate::i18n::has(&format!("sourceFunds:bank.{k}")), "{k}");
        }
    }
}
