//! What the Reports screen reads out of `ui.reports.capital_gains` (components/reports/
//! reportModel.ts) plus the page's readiness and provisional checks (routes/Reports.tsx). Every
//! figure comes from the daemon; this module only picks and labels them, in the UI language
//! (the `reports` namespace).

use serde_json::Value;

use crate::format;
use crate::i18n::{t, tc, tr};

/// The book regions the core supports; anything else reads as generic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    At,
    Generic,
}

pub fn region(code: &str) -> Region {
    if code.trim().eq_ignore_ascii_case("AT") {
        Region::At
    } else {
        Region::Generic
    }
}

/// `methodLabel`: the book's gains algorithm as the report used it, never substituted.
pub fn method_label(method: &str) -> String {
    match method.to_lowercase().as_str() {
        "fifo" => t("reports:method.fifo"),
        "lifo" => t("reports:method.lifo"),
        "hifo" => t("reports:method.hifo"),
        "lofo" => t("reports:method.lofo"),
        "moving_average" => t("reports:method.movingAverage"),
        "moving_average_at" => t("reports:method.movingAverageAt"),
        _ if method.is_empty() => "—".into(),
        _ => method.to_uppercase(),
    }
}

/// The method's short name for a stat tile's line ("By FIFO"); the Rules card keeps the long one.
pub fn method_short(method: &str) -> String {
    match method.to_lowercase().as_str() {
        "moving_average" => t("reports:methodShort.movingAverage"),
        "moving_average_at" => t("reports:methodShort.movingAverageAt"),
        _ if method.is_empty() => t("reports:methodShort.notSet"),
        m => tr(
            "reports:methodShort.other",
            &[("method", &m.to_uppercase())],
        ),
    }
}

/// One `breakdown` bucket of the report.
#[derive(Clone, Debug, Default)]
pub struct Bucket {
    pub key: String,
    pub row_count: f64,
    pub transaction_count: usize,
    pub transaction_ids: Vec<String>,
    pub proceeds: f64,
    pub cost: f64,
    pub income: f64,
    pub gain: f64,
}

/// A filing field (`kennzahlRows`).
#[derive(Clone, Debug, Default)]
pub struct Field {
    pub code: String,
    pub label: String,
    pub form: String,
    pub amount: Option<f64>,
    pub row_count: f64,
}

/// The parts of `ui.reports.capital_gains` the page reads.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub year: i32,
    pub available_years: Vec<i32>,
    pub jurisdiction: String,
    pub fiat: String,
    pub method: String,
    pub long_term_days: Option<f64>,
    pub row_count: f64,
    pub proceeds: f64,
    pub cost: f64,
    pub gain: f64,
    pub income: f64,
    pub breakdown: Vec<Bucket>,
    pub fields: Vec<Field>,
    pub e1kv_applies: Option<bool>,
    pub tax_rate: Option<String>,
    /// `indicativeTax` present: (estimated cents or None, progressive cents sum, notes).
    pub tax: Option<(Option<f64>, f64, Vec<String>)>,
    pub rows_before_fields: f64,
    pub needs_journals: bool,
    pub quarantines: f64,
    /// `taxYear`: the book's own tax year (first day `MM-DD`, zone) when it isn't 1 January
    /// (docs/reference/tax.md "Tax years"); None for a calendar-year book.
    pub tax_year: Option<((u32, u32), String)>,
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_owned()
}

impl Report {
    pub fn parse(d: &Value) -> Report {
        let tax = d.get("indicativeTax").filter(|t| t.is_object()).map(|t| {
            let est = t["estimatedTaxEurCents"].as_f64();
            let prog: f64 = t["progressiveRateAmountsEurCents"]
                .as_object()
                .map(|m| m.values().map(num).sum())
                .unwrap_or(0.0);
            let notes = t["notes"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|n| n.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            (est, prog, notes)
        });
        Report {
            tax_year: d["taxYear"]["taxYearStart"]
                .as_str()
                .and_then(|t| {
                    Some((
                        (t.get(0..2)?.parse().ok()?, t.get(3..5)?.parse().ok()?),
                        d["taxYear"]["timezone"]
                            .as_str()
                            .unwrap_or("UTC")
                            .to_owned(),
                    ))
                })
                .filter(|((m, dd), _): &((u32, u32), String)| {
                    (1..=12).contains(m) && *dd >= 1 && (*m, *dd) != (1, 1)
                }),
            year: d["year"].as_i64().unwrap_or(0) as i32,
            available_years: d["availableYears"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|y| y.as_i64().map(|y| y as i32))
                        .collect()
                })
                .unwrap_or_default(),
            jurisdiction: s(&d["jurisdictionCode"]),
            fiat: d["fiatCurrency"].as_str().unwrap_or("EUR").to_uppercase(),
            method: s(&d["method"]),
            long_term_days: d["longTermDays"].as_f64(),
            row_count: num(&d["totals"]["rowCount"]),
            proceeds: num(&d["totals"]["proceedsEur"]),
            cost: num(&d["totals"]["costEur"]),
            gain: num(&d["totals"]["gainLossEur"]),
            income: num(&d["totals"]["incomeEur"]),
            breakdown: d["breakdown"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|b| Bucket {
                            key: s(&b["key"]),
                            row_count: num(&b["rowCount"]),
                            transaction_count: b["transactionCount"].as_u64().unwrap_or(0) as usize,
                            transaction_ids: b["transactionIds"]
                                .as_array()
                                .map(|x| {
                                    x.iter()
                                        .filter_map(|i| i.as_str().map(str::to_owned))
                                        .collect()
                                })
                                .unwrap_or_default(),
                            proceeds: num(&b["proceedsEur"]),
                            cost: num(&b["costEur"]),
                            income: num(&b["incomeEur"]),
                            gain: num(&b["gainLossEur"]),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            fields: d["kennzahlRows"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|f| Field {
                            code: f["code"]
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| f["code"].to_string()),
                            label: s(&f["label"]),
                            form: s(&f["form"]),
                            amount: f["amount"].as_f64(),
                            row_count: num(&f["rowCount"]),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            e1kv_applies: d["e1kvApplies"].as_bool(),
            tax_rate: d["indicativeTax"]["rate"].as_str().map(str::to_owned),
            tax,
            rows_before_fields: num(&d["rowsBeforeCryptoKennzahlen"]),
            needs_journals: d["status"]["needsJournals"].as_bool().unwrap_or(false),
            quarantines: num(&d["status"]["quarantines"]),
        }
    }

    pub fn region(&self) -> Region {
        region(&self.jurisdiction)
    }

    fn bucket(&self, key: &str) -> Option<&Bucket> {
        self.breakdown.iter().find(|b| b.key == key)
    }

    /// The part of the gain from tax-free Altbestand.
    pub fn tax_free_gain(&self) -> f64 {
        self.bucket("tax_free").map(|b| b.gain).unwrap_or(0.0)
    }

    /// `reportE1kvApplies`.
    pub fn e1kv_applies(&self) -> bool {
        self.region() == Region::At && self.e1kv_applies != Some(false)
    }

    /// `reportUnassignedRows`.
    pub fn unassigned_rows(&self) -> f64 {
        self.bucket("unassigned")
            .map(|b| b.row_count)
            .unwrap_or(0.0)
    }

    /// `reportTaxFieldRows`: the book's filing fields, or the Austrian ones awaiting figures.
    pub fn tax_field_rows(&self) -> Vec<Field> {
        if !self.fields.is_empty() {
            return self.fields.clone();
        }
        if self.region() != Region::At {
            return vec![];
        }
        let e1kv = self.e1kv_applies();
        AUSTRIAN_FIELDS
            .iter()
            .filter(|(_, form, _)| e1kv || *form != "E 1kv")
            .map(|(code, form, _)| Field {
                code: (*code).into(),
                label: field_label(code),
                form: (*form).into(),
                amount: None,
                row_count: 0.0,
            })
            .collect()
    }

    /// A year's name in the picker: `2025`, or `2025-26` for a book whose tax year doesn't
    /// start on 1 January (named by its first day's year).
    pub fn year_label(&self, y: i32) -> String {
        match &self.tax_year {
            Some(_) => format!("{y}-{:02}", (y + 1).rem_euclid(100)),
            None => y.to_string(),
        }
    }

    /// The trigger's period: `1 Jan – 31 Dec 2025`, or the book's own year
    /// (`1 Jul 2025 – 30 Jun 2026`).
    pub fn period_of(&self, y: i32) -> String {
        let Some(((m, d), _)) = &self.tax_year else {
            return period(y);
        };
        let first = crate::format::Day { y, m: *m, d: *d };
        let last = crate::format::Day::from_epoch_days(
            crate::format::Day {
                y: y + 1,
                m: *m,
                d: *d,
            }
            .epoch_days()
                - 1,
        );
        tr(
            "reports:date.range",
            &[
                ("start", &day_month_year(first)),
                ("end", &day_month_year(last)),
            ],
        )
    }

    /// The rules row's tax year: the registry's for the country, the book's own when it has one.
    pub fn tax_year_rule(&self) -> String {
        match &self.tax_year {
            Some(((m, d), zone)) => tr(
                "reports:taxYear.starts",
                &[("start", &day_month(*m, *d)), ("zone", zone)],
            ),
            None => describe_tax_year(&self.jurisdiction),
        }
    }

    /// The years the picker lists: the report's, plus the shown one, newest first.
    pub fn years(&self, effective: i32) -> Vec<i32> {
        let mut v: Vec<i32> = if self.available_years.is_empty() {
            vec![effective]
        } else {
            self.available_years.clone()
        };
        if !v.contains(&effective) {
            v.push(effective);
        }
        v.sort_by(|a, b| b.cmp(a));
        v.dedup();
        v
    }
}

/// `AUSTRIAN_FIELD_COPY`: (code, form, note key); the label is `reports:field.<code>`.
pub const AUSTRIAN_FIELDS: [(&str, &str, Option<&str>); 4] = [
    ("172", "E 1kv", None),
    ("174", "E 1kv", None),
    ("176", "E 1kv", None),
    ("801", "E 1", Some("reports:field.801Note")),
];

fn field_copy(code: &str) -> Option<&'static (&'static str, &'static str, Option<&'static str>)> {
    AUSTRIAN_FIELDS.iter().find(|f| f.0 == code)
}

/// A known Austrian field's label in the UI language.
fn field_label(code: &str) -> String {
    t(&format!("reports:field.{code}"))
}

/// First tax year with the crypto Kennzahlen; the core decides, this names it.
pub const E1KV_FIRST_TAX_YEAR: i32 = 2022;

/// `formatTaxRate`: `27.5 %` (de `27,5 %`) from the core's decimal rate string.
pub fn tax_rate(rate: Option<&str>) -> String {
    let Some(v) = rate
        .and_then(|r| r.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
    else {
        return String::new();
    };
    let pct = (v * 100.0 * 100.0).round() / 100.0;
    let mut s = format!("{pct}");
    if s.ends_with(".0") {
        s.truncate(s.len() - 2);
    }
    if crate::i18n::lang() == crate::i18n::Lang::De {
        s = s.replace('.', ",");
    }
    format!("{s} %")
}

/// `1 Jan` (de `1. Jän.`).
fn day_month(m: u32, d: u32) -> String {
    tr(
        "reports:date.dayMonth",
        &[
            ("day", &d.to_string()),
            ("month", format::short_month_ui(m)),
        ],
    )
}

/// `1 Jan 2025` (de `1. Jän. 2025`).
fn day_month_year(day: format::Day) -> String {
    tr(
        "reports:date.dayMonthYear",
        &[
            ("day", &day.d.to_string()),
            ("month", format::short_month_ui(day.m)),
            ("year", &day.y.to_string()),
        ],
    )
}

/// `formatReportPeriod`: `1 Jan – 31 Dec 2025`.
pub fn period(year: i32) -> String {
    tr(
        "reports:date.range",
        &[
            ("start", &day_month(1, 1)),
            (
                "end",
                &day_month_year(format::Day {
                    y: year,
                    m: 12,
                    d: 31,
                }),
            ),
        ],
    )
}

/// (timezone, tax-year start (month, day)) from the tax country registry.
fn country_zone(code: &str) -> Option<(&'static str, (u32, u32))> {
    Some(match code.trim().to_lowercase().as_str() {
        "generic" => ("UTC", (1, 1)),
        "at" => ("Europe/Vienna", (1, 1)),
        "us" => ("UTC", (1, 1)),
        "de" => ("Europe/Berlin", (1, 1)),
        "ch" => ("Europe/Zurich", (1, 1)),
        "ie" => ("Europe/Dublin", (1, 1)),
        "gb" => ("Europe/London", (4, 6)),
        "es" => ("Europe/Madrid", (1, 1)),
        "jp" => ("Asia/Tokyo", (1, 1)),
        _ => return None,
    })
}

/// `describeTaxYear`.
pub fn describe_tax_year(jurisdiction: &str) -> String {
    let (zone, (m, d)) = country_zone(jurisdiction).unwrap_or(("UTC", (1, 1)));
    if m == 1 && d == 1 {
        tr("reports:taxYear.calendar", &[("zone", zone)])
    } else {
        tr(
            "reports:taxYear.calendarNotApplied",
            &[("zone", zone), ("start", &day_month(m, d))],
        )
    }
}

/// `signedFiat`: `+ € 1.234,56` / `− € 12,00`, unsigned at zero.
pub fn signed_fiat(v: f64, cur: &str) -> String {
    let sign = if v > 0.0 {
        "+ "
    } else if v < 0.0 {
        "\u{2212} "
    } else {
        ""
    };
    format!("{sign}{}", format::fiat(v.abs(), cur))
}

/// `transactionsHref`: the year, or exactly the given transactions.
pub fn transactions_href(year: i32, ids: &[String]) -> String {
    if ids.is_empty() {
        format!("/transactions?period={year}")
    } else {
        format!("/transactions?period={year}&txids={}", ids.join("%2C"))
    }
}

#[derive(Clone, Debug)]
pub struct BreakdownRow {
    pub key: String,
    pub code: Option<String>,
    pub label: String,
    pub note: Option<String>,
    pub form: String,
    pub transaction_count: usize,
    pub href: Option<String>,
    pub exact: bool,
    pub amount: Option<f64>,
    pub attention: bool,
    pub proceeds: f64,
    pub cost: f64,
}

#[derive(Clone, Debug)]
pub struct Group {
    pub title: String,
    pub reported: bool,
    pub rows: Vec<BreakdownRow>,
}

fn links(year: i32, b: Option<&Bucket>) -> (usize, Option<String>, bool) {
    let count = b.map(|b| b.transaction_count).unwrap_or(0);
    if count == 0 {
        return (0, None, false);
    }
    let ids = b.map(|b| b.transaction_ids.as_slice()).unwrap_or(&[]);
    (count, Some(transactions_href(year, ids)), !ids.is_empty())
}

/// `reportBreakdown`: the Austrian fields with what stays off the return, or a generic book's
/// holding periods and income.
pub fn breakdown(r: &Report) -> Vec<Group> {
    let year = r.year;
    if r.region() != Region::At {
        let mut rows = vec![];
        for (key, label) in [
            ("short_term", "reports:breakdown.row.shortTerm"),
            ("long_term", "reports:breakdown.row.longTerm"),
            ("income", "reports:breakdown.row.income"),
        ] {
            let Some(b) = r.bucket(key).filter(|b| b.row_count != 0.0) else {
                continue;
            };
            let (n, href, exact) = links(year, Some(b));
            rows.push(BreakdownRow {
                key: key.into(),
                code: None,
                label: t(label),
                note: None,
                form: String::new(),
                transaction_count: n,
                href,
                exact,
                amount: Some(if key == "income" { b.income } else { b.gain }),
                attention: false,
                proceeds: b.proceeds,
                cost: b.cost,
            });
        }
        return if rows.is_empty() {
            vec![]
        } else {
            vec![Group {
                title: String::new(),
                reported: true,
                rows,
            }]
        };
    }
    let field_rows: Vec<BreakdownRow> = r
        .tax_field_rows()
        .into_iter()
        .map(|f| {
            let copy = field_copy(&f.code);
            let b = r.bucket(&f.code);
            let (n, href, exact) = links(year, b);
            BreakdownRow {
                key: f.code.clone(),
                code: Some(format!("KZ {}", f.code)),
                label: copy.map(|c| field_label(c.0)).unwrap_or(f.label.clone()),
                note: copy.and_then(|c| c.2).map(t),
                form: copy.map(|c| c.1.to_owned()).unwrap_or_else(|| {
                    if f.form.is_empty() {
                        "E 1kv".into()
                    } else {
                        f.form.clone()
                    }
                }),
                transaction_count: n,
                href,
                exact,
                amount: f.amount,
                attention: false,
                proceeds: b.map(|b| b.proceeds).unwrap_or(0.0),
                cost: b.map(|b| b.cost).unwrap_or(0.0),
            }
        })
        .collect();
    let mut groups = vec![];
    let e1kv: Vec<BreakdownRow> = field_rows
        .iter()
        .filter(|r| r.form == "E 1kv")
        .cloned()
        .collect();
    let e1: Vec<BreakdownRow> = field_rows
        .iter()
        .filter(|r| r.form != "E 1kv")
        .cloned()
        .collect();
    if !e1kv.is_empty() {
        groups.push(Group {
            title: "E 1kv".into(),
            reported: true,
            rows: e1kv,
        });
    }
    if !e1.is_empty() {
        groups.push(Group {
            title: t("reports:breakdown.group.outsideE1kv"),
            reported: true,
            rows: e1,
        });
    }
    let extra: [(&str, &str, &str); 3] = [
        (
            "tax_free",
            "reports:breakdown.row.taxFree",
            "reports:breakdown.row.taxFreeNote",
        ),
        (
            "swaps",
            "reports:breakdown.row.swaps",
            "reports:breakdown.row.swapsNote",
        ),
        (
            "unassigned",
            "reports:breakdown.row.unassigned",
            "reports:breakdown.row.unassignedNote",
        ),
    ];
    let mut off = vec![];
    for (key, label, note) in extra {
        let Some(b) = r.bucket(key).filter(|b| b.row_count != 0.0) else {
            continue;
        };
        let (n, href, exact) = links(year, Some(b));
        off.push(BreakdownRow {
            key: key.into(),
            code: None,
            label: t(label),
            note: Some(t(note)),
            form: String::new(),
            transaction_count: n,
            href,
            exact,
            amount: if key == "swaps" { None } else { Some(b.gain) },
            attention: key == "unassigned",
            proceeds: b.proceeds,
            cost: b.cost,
        });
    }
    if !off.is_empty() {
        groups.push(Group {
            title: t("reports:breakdown.group.notReported"),
            reported: false,
            rows: off,
        });
    }
    groups
}

/// `ReadinessTone`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Good,
    Warning,
    Alert,
    Neutral,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    WalletCards,
    RefreshCw,
    ShieldAlert,
    Loader,
    Alert,
    Check,
}

#[derive(Clone, Debug)]
pub struct Readiness {
    pub title: String,
    pub detail: String,
    pub tone: Tone,
    pub glyph: Glyph,
    pub href: Option<&'static str>,
}

/// What the page knows about the book's completeness (`ui.overview.snapshot`'s fiat block).
#[derive(Clone, Debug, Default)]
pub struct Completeness {
    pub cost_basis_complete: bool,
    pub reasons: Vec<String>,
    pub missing_prices: usize,
}

fn n(v: f64) -> usize {
    v.max(0.0) as usize
}

/// `buildReportReadiness`.
pub fn readiness(
    r: &Report,
    row_count: f64,
    year: i32,
    completeness: Option<&Completeness>,
    pending: bool,
    blocking: usize,
) -> Readiness {
    let ready =
        |title: String, detail: String, tone: Tone, glyph: Glyph, href: Option<&'static str>| {
            Readiness {
                title,
                detail,
                tone,
                glyph,
                href,
            }
        };
    if blocking > 0 {
        return ready(
            t("overview:readiness.sourcesBlocking.title"),
            tc("reports:readiness.sourcesBlocking", blocking, &[]),
            Tone::Alert,
            Glyph::WalletCards,
            Some("/connections"),
        );
    }
    let has_filing_rows = r
        .fields
        .iter()
        .any(|f| f.row_count > 0.0 || f.amount.unwrap_or(0.0).abs() > 0.005);
    if r.needs_journals {
        return ready(
            t("reports:readiness.processLedger.title"),
            t("reports:readiness.processLedger.detail"),
            Tone::Warning,
            Glyph::RefreshCw,
            Some("/journals"),
        );
    }
    if r.quarantines > 0.0 {
        return ready(
            t("overview:readiness.reviewQueueOpen.title"),
            tc("reports:readiness.reviewQueue", n(r.quarantines), &[]),
            Tone::Alert,
            Glyph::ShieldAlert,
            Some("/review"),
        );
    }
    if pending {
        return ready(
            t("reports:readiness.checking.title"),
            t("reports:readiness.checking.detail"),
            Tone::Neutral,
            Glyph::Loader,
            None,
        );
    }
    let has = |reason: &str| completeness.is_some_and(|c| c.reasons.iter().any(|x| x == reason));
    if has("custody_unresolved") {
        return ready(
            t("overview:readiness.custodyUnresolved.title"),
            t("reports:readiness.custody"),
            Tone::Alert,
            Glyph::ShieldAlert,
            Some("/journals"),
        );
    }
    if has("missing_prices") {
        let c = completeness.map(|c| c.missing_prices).unwrap_or(0);
        return ready(
            t("overview:readiness.missingPrices.title"),
            tc("reports:readiness.missingPrices", c, &[]),
            Tone::Warning,
            Glyph::Alert,
            Some("/review"),
        );
    }
    if !completeness.is_some_and(|c| c.cost_basis_complete) {
        return ready(
            t("reports:readiness.unknown.title"),
            t("reports:readiness.unknown.detail"),
            Tone::Warning,
            Glyph::Alert,
            Some("/journals"),
        );
    }
    let austrian = r.region() == Region::At;
    let unassigned = r.unassigned_rows();
    if austrian && !r.e1kv_applies() {
        let first = E1KV_FIRST_TAX_YEAR.to_string();
        let mut detail = tr(
            "reports:readiness.noFields.detail",
            &[("first", &first), ("year", &year.to_string())],
        );
        if unassigned > 0.0 {
            detail.push(' ');
            detail.push_str(&tc("reports:readiness.noFields.tail", n(unassigned), &[]));
        }
        return ready(
            tr("reports:readiness.noFields.title", &[("year", &first)]),
            detail,
            Tone::Warning,
            Glyph::Alert,
            None,
        );
    }
    let before = r.rows_before_fields;
    if austrian && before > 0.0 {
        return ready(
            tr(
                "reports:readiness.rowsBefore.title",
                &[("year", &E1KV_FIRST_TAX_YEAR.to_string())],
            ),
            tc(
                "reports:readiness.rowsBefore.detail",
                n(before),
                &[("year", &year.to_string())],
            ),
            Tone::Warning,
            Glyph::Alert,
            None,
        );
    }
    if unassigned > 0.0 {
        return ready(
            t("reports:readiness.unassigned.title"),
            tc("reports:readiness.unassigned.detail", n(unassigned), &[]),
            Tone::Warning,
            Glyph::Alert,
            None,
        );
    }
    if row_count == 0.0 && !has_filing_rows {
        return ready(
            t("reports:readiness.noRows.title"),
            tr(
                "reports:readiness.noRows.detail",
                &[("year", &year.to_string())],
            ),
            Tone::Neutral,
            Glyph::Alert,
            None,
        );
    }
    ready(
        t("journals:review.nextAction.ready"),
        t("reports:readiness.ready"),
        Tone::Good,
        Glyph::Check,
        None,
    )
}

/// `reportFiguresProvisional`.
pub fn provisional(r: &Report, completeness: Option<&Completeness>, pending: bool) -> bool {
    r.needs_journals
        || r.quarantines > 0.0
        || r.unassigned_rows() > 0.0
        || (!pending && !completeness.is_some_and(|c| c.cost_basis_complete))
}

/// `reportYearFromSearch`: `?year=` when it is a plausible integer year.
pub fn year_from_route(route: &str) -> Option<i32> {
    let q = route.split_once('?').map(|(_, q)| q)?;
    q.split('#')
        .next()?
        .split('&')
        .find_map(|p| p.strip_prefix("year="))
        .and_then(|v| v.trim().parse::<i32>().ok())
        .filter(|y| (1900..=9999).contains(y))
}

/// The `result` param of `/reports?result=<json>` (a capital-gain explanation reference).
pub fn result_param(route: &str) -> Option<String> {
    let q = route.split_once('?').map(|(_, q)| q)?;
    let raw = q
        .split('#')
        .next()?
        .split('&')
        .find_map(|p| p.strip_prefix("result="))?;
    Some(decode(raw))
}

pub fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(v) => {
                    out.push(v);
                    i += 2;
                }
                Err(_) => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `encodeURIComponent`.
pub fn encode(s: &str) -> String {
    s.bytes()
        .map(|c| match c {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => (c as char).to_string(),
            _ => format!("%{c:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn generic() -> Report {
        Report::parse(&json!({
            "jurisdictionCode": "GENERIC", "fiatCurrency": "EUR", "longTermDays": 365, "year": 2025,
            "availableYears": [2025, 2024], "method": "fifo",
            "totals": {"rowCount": 2, "costEur": 10.0, "proceedsEur": 30.0, "incomeEur": 0.0, "gainLossEur": 20.0},
            "breakdown": [{"key": "long_term", "rowCount": 2, "transactionCount": 2, "transactionIds": ["a", "b"], "gainLossEur": 20.0, "proceedsEur": 30.0, "costEur": 10.0}],
            "kennzahlRows": [], "e1kvApplies": false, "indicativeTax": null, "status": {"needsJournals": false, "quarantines": 0}
        }))
    }

    fn complete() -> Completeness {
        Completeness {
            cost_basis_complete: true,
            reasons: vec![],
            missing_prices: 0,
        }
    }

    #[test]
    fn generic_breakdown_links_exact_transactions() {
        let g = breakdown(&generic());
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].rows[0].label, "Long-term gains");
        assert_eq!(
            g[0].rows[0].href.as_deref(),
            Some("/transactions?period=2025&txids=a%2Cb")
        );
        assert!(g[0].rows[0].exact);
    }

    #[test]
    fn austrian_fields_await_figures() {
        let mut r = generic();
        r.jurisdiction = "AT".into();
        r.e1kv_applies = Some(true);
        let g = breakdown(&r);
        assert_eq!(g[0].title, "E 1kv");
        assert_eq!(g[0].rows.len(), 3);
        assert_eq!(g[1].title, "E 1 · outside the E 1kv");
        assert_eq!(g[1].rows[0].code.as_deref(), Some("KZ 801"));
        r.e1kv_applies = Some(false);
        assert_eq!(breakdown(&r).len(), 1);
    }

    #[test]
    fn readiness_order() {
        let r = generic();
        let c = complete();
        crate::i18n::set_lang(crate::i18n::Lang::De);
        assert_eq!(
            readiness(&r, 2.0, 2025, Some(&c), false, 0).title,
            "Bereit für Berichte"
        );
        crate::i18n::set_lang(crate::i18n::Lang::En);
        assert_eq!(
            readiness(&r, 2.0, 2025, Some(&c), false, 0).title,
            "Ready for reports"
        );
        assert_eq!(
            readiness(&r, 2.0, 2025, Some(&c), false, 1).title,
            "Sources block reports"
        );
        assert_eq!(
            readiness(&r, 2.0, 2025, None, true, 0).title,
            "Checking completeness"
        );
        assert_eq!(
            readiness(&r, 2.0, 2025, None, false, 0).title,
            "Completeness unknown"
        );
        let missing = Completeness {
            cost_basis_complete: false,
            reasons: vec!["missing_prices".into()],
            missing_prices: 1,
        };
        assert_eq!(
            readiness(&r, 2.0, 2025, Some(&missing), false, 0).detail,
            "1 transaction has no price; figures are provisional."
        );
        let mut q = generic();
        q.quarantines = 3.0;
        assert_eq!(
            readiness(&q, 2.0, 2025, Some(&c), false, 0).detail,
            "3 quarantined items should be resolved before filing."
        );
        assert!(provisional(&q, Some(&c), false));
        assert!(!provisional(&r, Some(&c), false));
        assert!(!provisional(&r, None, true));
    }

    #[test]
    fn formats() {
        assert_eq!(tax_rate(Some("0.275")), "27.5 %");
        assert_eq!(tax_rate(Some("x")), "");
        assert_eq!(
            describe_tax_year("GB"),
            "Calendar year, Europe/London · 6 Apr tax year not applied yet"
        );
        assert_eq!(describe_tax_year("GENERIC"), "Calendar year, UTC");
        // A book with its own tax year names and bounds its years by their first day.
        let r = Report::parse(
            &serde_json::json!({"year": 2025, "taxYear": {"taxYearStart": "07-01", "timezone": "UTC", "yearLabel": "2025-26"}}),
        );
        assert_eq!(
            (r.year_label(2025), r.period_of(2025), r.tax_year_rule()),
            (
                "2025-26".into(),
                "1 Jul 2025 – 30 Jun 2026".into(),
                "Starts 1 Jul, UTC".into()
            )
        );
        let r = Report::parse(&serde_json::json!({"year": 2025}));
        assert_eq!(
            (r.year_label(2025), r.period_of(2025)),
            ("2025".into(), "1 Jan – 31 Dec 2025".into())
        );
        assert_eq!(year_from_route("/reports?year=2022"), Some(2022));
        assert_eq!(year_from_route("/reports"), None);
        assert_eq!(decode(&encode("{\"a\":1}")), "{\"a\":1}");
    }

    /// The page's own copy follows the UI language: Austrian months, decimal comma, plurals.
    #[test]
    fn german_copy() {
        crate::i18n::set_lang(crate::i18n::Lang::De);
        let r = Report::parse(
            &serde_json::json!({"year": 2025, "taxYear": {"taxYearStart": "07-01", "timezone": "UTC"}}),
        );
        let got = (
            tax_rate(Some("0.275")),
            period(2025),
            r.period_of(2025),
            r.tax_year_rule(),
            describe_tax_year("GB"),
            method_short("fifo"),
        );
        let mut at = generic();
        at.jurisdiction = "AT".into();
        at.e1kv_applies = Some(true);
        let groups = breakdown(&at);
        let mut q = generic();
        q.quarantines = 1.0;
        let detail = readiness(&q, 2.0, 2025, Some(&complete()), false, 0).detail;
        crate::i18n::set_lang(crate::i18n::Lang::En);
        assert_eq!(
            got,
            (
                "27,5 %".into(),
                "1. Jän. – 31. Dez. 2025".into(),
                "1. Juli 2025 – 30. Juni 2026".into(),
                "Beginnt am 1. Juli, UTC".into(),
                "Kalenderjahr, Europe/London · Steuerjahr ab 6. Apr. noch nicht angewendet".into(),
                "Nach FIFO".into(),
            )
        );
        assert_eq!(groups[1].title, "E 1 · außerhalb der E 1kv");
        assert_eq!(
            groups[1].rows[0].label,
            "Einkünfte aus Spekulationsgeschäften (Altbestand)"
        );
        assert_eq!(
            detail,
            "1 Eintrag in Quarantäne sollte vor der Einreichung geklärt werden."
        );
    }
}
