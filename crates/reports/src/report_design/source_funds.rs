//! The source-of-funds report in the shared design, from a saved case
//! snapshot (`load_case_snapshot`), its hash and the export clock. Austrian
//! books read German headings; the review's own prose (narrative, findings,
//! gap and privacy notes) is the snapshot's text.
use super::*;
use crate::report_design::{
    col, fact, list, Block, Cell, Column, Document, Fact, Flow, FlowColumn, FlowLink, FlowNode,
    Kind, Lang, Legend, Section, Table,
};

/// Keys of the checklist items and fixed texts Python writes in English;
/// the English table holds Python's text verbatim, so one is found by text.
const TEXTS: [&str; 6] = [
    "sof.check.root",
    "sof.check.hops",
    "sof.check.target",
    "sof.check.snapshot",
    "sof.check.at_eur",
    "sof.subtitle.at_eur",
];

fn translated(lang: Lang, raw: &str) -> String {
    TEXTS
        .iter()
        .find(|key| Lang::En.get(key) == Some(raw))
        .and_then(|key| lang.get(key))
        .unwrap_or(raw)
        .to_owned()
}
fn label(raw: &str) -> String {
    raw.replace('_', " ")
}
fn present(v: &Value) -> bool {
    !matches!(v, Value::Null) && *v != s("")
}
fn table(id: &'static str, title: String, columns: Vec<Column>, rows: Vec<Vec<Cell>>) -> Table {
    Table {
        id,
        title,
        columns,
        rows,
        total: None,
        note: String::new(),
    }
}
fn note(text: impl Into<String>) -> Block {
    Block::Note {
        text: text.into(),
        accent: false,
    }
}
fn section(id: &'static str, title: &str, blocks: Vec<Block>) -> Section {
    Section {
        id,
        title: title.into(),
        lead: String::new(),
        page_break: false,
        landscape: false,
        blocks,
    }
}
/// An exact decimal from a snapshot amount (`Decimal(str(value))`).
fn exact(v: &Value) -> Option<Decimal> {
    present(v)
        .then(|| py_str(v).trim().parse::<Decimal>().ok())
        .flatten()
}

/// Python's `visible_levels`: long paths keep their head and tail around
/// one omitted level, wide levels their first nodes and a count of the rest.
fn visible_levels(levels: &[Value], max_levels: usize, max_nodes: usize) -> Vec<Value> {
    let omitted = |id: String, label: String, kind: &str| {
        dict([
            ("id", s(id)),
            ("label", s(label)),
            ("kind", s(kind)),
            ("asset", s("")),
            ("amount", Value::Null),
            ("node_type", s("omitted")),
            ("deferred_privacy_hop", Value::Bool(false)),
        ])
    };
    let mut levels = levels.to_vec();
    if max_levels > 0 && levels.len() > max_levels {
        let hidden = levels.len() - (max_levels - 1);
        let head = ((max_levels - 1) / 2 + 1).max(1);
        let tail = max_levels - 1 - head;
        let mut next = levels[..head].to_vec();
        next.push(dict([
            ("role", s("omitted")),
            (
                "nodes",
                Value::List(vec![omitted(
                    "synthetic:omitted".into(),
                    format!("{hidden} path levels omitted"),
                    "continued path",
                )]),
            ),
        ]));
        if tail > 0 {
            next.extend_from_slice(&levels[levels.len() - tail..]);
        }
        levels = next;
    }
    for (index, level) in levels.iter_mut().enumerate() {
        let nodes = list(level, "nodes").to_vec();
        if max_nodes > 0 && nodes.len() > max_nodes {
            let extra = nodes.len() - (max_nodes - 1);
            let mut next = nodes[..max_nodes - 1].to_vec();
            next.push(omitted(
                format!(
                    "synthetic:extra:{}",
                    get(level, "level")
                        .map(py_str)
                        .unwrap_or_else(|| index.to_string())
                ),
                format!("{extra} more items"),
                "additional reviewed nodes",
            ));
            replace(level, "nodes", Value::List(next));
        }
    }
    levels
}

/// The source-of-funds document. `model` holds `report` (the saved case
/// snapshot), `snapshot_hash` and `generated_at`.
pub fn source_funds(model: &Value) -> Document {
    let report = value(model, "report");
    let context = value(report, "report_context");
    let country = text(context, "tax_country");
    let lang = Lang::for_country(&country);
    let t = |key: &str| lang.t(key);
    let options = value(report, "report_options");
    let sats = text(options, "amount_precision") == "sats";
    let mask = truthy(value(options, "mask_recipient"));
    let omitted: Vec<String> = list(options, "omit_sections").iter().map(py_str).collect();
    let target = value(report, "target");
    let purpose = value(report, "purpose");
    let recipient = value(report, "recipient");
    let overview = value(report, "overview");
    let allocations = value(report, "allocations");
    let currency = text(context, "fiat_currency");
    let planned = text(purpose, "type") == "planned_exchange_sale";
    let masked = || t("sof.masked").to_owned();
    // Amounts follow the snapshot's precision option: BTC, or whole sats.
    let amount_kind = if sats { Kind::Count } else { Kind::Btc };
    let amount_label = |key: &str| format!("{} ({})", t(key), if sats { "sats" } else { "BTC" });
    let amt = |v: &Value| -> Cell {
        match exact(v) {
            None => Cell::Empty,
            Some(d) if sats => d
                .mul(&Decimal::from(100_000_000), &DECIMAL)
                .ok()
                .and_then(|d| d.to_fixed_places(0, Rounding::HalfEven).ok())
                .and_then(|t| t.parse::<i64>().ok())
                .map(Cell::Count)
                .unwrap_or(Cell::Empty),
            Some(d) => Cell::Btc(d),
        }
    };
    let fiat_cell = |v: &Value| exact(v).map(Cell::Fiat).unwrap_or(Cell::Empty);
    let when = |v: &Value| {
        if truthy(v) {
            Cell::Text(lang.date_time(&py_str(v)))
        } else {
            Cell::Empty
        }
    };
    let title = if text(context, "template_key") == "at_eur_basic" {
        t("sof.title.at_eur")
    } else {
        t("sof.title")
    };
    let book = format!(
        "{} / {}",
        text(report, "workspace"),
        text(report, "profile")
    );
    let mut sections = Vec::new();

    // 1. Scope: what is disclosed, to whom, from which snapshot
    let mut scope = vec![
        Fact {
            display: Some(lang.date_time(&text(model, "generated_at"))),
            ..fact(
                "generated_at",
                t("scope.generated_at"),
                Cell::Text(text(model, "generated_at")),
            )
        },
        fact("book", t("scope.book"), Cell::Text(book.clone())),
        fact(
            "jurisdiction",
            t("sof.jurisdiction"),
            Cell::Text(
                t(if country == "at" {
                    "country.at"
                } else {
                    "country.generic"
                })
                .into(),
            ),
        ),
        fact(
            "fiat_currency",
            t("scope.fiat_currency"),
            Cell::Text(currency.clone()),
        ),
        fact(
            "purpose",
            t("sof.purpose"),
            Cell::Text(
                lang.get(&format!("sof.purpose.{}", text(purpose, "type")))
                    .map(str::to_owned)
                    .unwrap_or_else(|| text(purpose, "label")),
            ),
        ),
        fact(
            "reveal_mode",
            t("sof.reveal_mode"),
            Cell::Text(
                lang.get(&format!("sof.reveal.{}", text(report, "reveal_mode")))
                    .map(str::to_owned)
                    .unwrap_or_else(|| text(report, "reveal_mode")),
            ),
        ),
    ];
    if truthy(recipient) {
        scope.push(fact(
            "recipient",
            t("sof.recipient"),
            Cell::Text(format!(
                "{} ({})",
                if mask {
                    masked()
                } else {
                    text(recipient, "label")
                },
                text(recipient, "kind")
            )),
        ));
    }
    scope.extend([
        fact(
            "target",
            t(if planned {
                "sof.target.sale"
            } else {
                "sof.target"
            }),
            Cell::Text(text(target, "label")),
        ),
        Fact {
            display: Some(match amt(value(target, "required_amount")) {
                Cell::Empty => String::new(),
                cell => format!(
                    "{} {}",
                    cell.display(lang),
                    if sats {
                        "sats".into()
                    } else {
                        text(target, "asset")
                    }
                ),
            }),
            ..fact(
                "amount",
                t("col.amount"),
                amt(value(target, "required_amount")),
            )
        },
        Fact {
            display: Some(match fiat_cell(value(overview, "target_fiat_value")) {
                Cell::Empty => t("sof.not_priced").into(),
                cell => format!(
                    "{} {}",
                    cell.display(lang),
                    text(overview, "target_fiat_currency")
                ),
            }),
            ..fact(
                "fiat_value",
                t("sof.fiat_value"),
                fiat_cell(value(overview, "target_fiat_value")),
            )
        },
        fact(
            "snapshot_hash",
            t("sof.snapshot_hash"),
            Cell::Text(text(model, "snapshot_hash")),
        ),
    ]);
    let mut blocks = vec![
        note(translated(lang, &{
            let subtitle = text(context, "report_subtitle");
            if subtitle.is_empty() {
                t("sof.subtitle").to_owned()
            } else {
                subtitle
            }
        })),
        Block::Facts(scope),
    ];
    if planned {
        let or = |key: &str, fallback: &str| {
            if mask {
                masked()
            } else {
                match text(purpose, key) {
                    v if v.is_empty() => fallback.to_owned(),
                    v => v,
                }
            }
        };
        blocks.push(Block::Facts(vec![
            fact(
                "planned_destination",
                t("sof.planned_destination"),
                Cell::Text(or("planned_destination", t("sof.not_specified"))),
            ),
            fact(
                "planned_note",
                t("sof.planned_note"),
                Cell::Text(or("planned_note", t("doc.none"))),
            ),
            fact(
                "fiat_purchase_note",
                t("sof.fiat_purchase_note"),
                Cell::Text(text(purpose, "fiat_purchase_note").replace("None", "")),
            ),
        ]));
    }
    sections.push(Section {
        lead: String::new(),
        ..section("scope", t("scope.title"), blocks)
    });

    // 2. Overview
    let range = value(overview, "time_range");
    sections.push(section(
        "overview",
        t("sof.overview"),
        vec![Block::Facts(vec![
            fact(
                "target_date",
                t("col.date"),
                when(value(overview, "target_date")),
            ),
            fact(
                "target_wallet",
                t("sof.wallet_source"),
                Cell::Text(match text(overview, "target_wallet") {
                    w if w.is_empty() => text(target, "wallet"),
                    w => w,
                }),
            ),
            Fact {
                display: Some(format!(
                    "{} – {}",
                    when(value(range, "start")).display(lang),
                    when(value(range, "end")).display(lang)
                )),
                ..fact("time_range", t("sof.time_range"), Cell::Empty)
            },
            fact(
                "transactions",
                t("col.transactions"),
                Cell::Count(int(overview, "transaction_count")),
            ),
            fact(
                "reviewed_links",
                t("sof.reviewed_links"),
                Cell::Count(int(overview, "link_count")),
            ),
            fact(
                "data_sources",
                t("sof.data_sources"),
                Cell::Count(int(overview, "data_source_count")),
            ),
            fact(
                "source_categories",
                t("sof.source_categories"),
                Cell::Count(int(overview, "source_category_count")),
            ),
        ])],
    ));

    // 3. Narrative
    let narrative = value(report, "narrative");
    let paragraphs = list(narrative, "paragraphs");
    let mut blocks: Vec<Block> = paragraphs.iter().map(|p| note(py_str(p))).collect();
    if blocks.is_empty() {
        blocks.push(note(t("sof.no_narrative")));
    } else if text(narrative, "generated_by") == "local_rule_summary" {
        blocks.push(note(t("sof.narrative_local")));
    }
    sections.push(section("narrative", t("sof.narrative"), blocks));

    // 4. The simplified flow as a diagram
    let flow = value(report, "simplified_flow");
    let detailed = text(options, "diagram_detail") == "detailed";
    let (max_levels, max_nodes) = if detailed { (16, 10) } else { (6, 4) };
    let levels = visible_levels(list(flow, "levels"), max_levels, max_nodes);
    let mut blocks = Vec::new();
    if levels.is_empty() {
        blocks.push(note(t("sof.no_flow")));
    } else {
        if truthy(value(flow, "note")) {
            blocks.push(note(text(flow, "note")));
        }
        let mut styles = std::collections::BTreeSet::new();
        let columns = levels
            .iter()
            .enumerate()
            .map(|(col, level)| {
                let role = match text(level, "role") {
                    r if r.is_empty() => "flow".to_owned(),
                    r => r,
                };
                FlowColumn {
                    title: lang
                        .get(&format!("sof.role.{role}"))
                        .map(str::to_owned)
                        .unwrap_or_else(|| label(&role)),
                    nodes: list(level, "nodes")
                        .iter()
                        .enumerate()
                        .map(|(row, node)| {
                            let deferred = truthy(value(node, "deferred_privacy_hop"));
                            let style = if role == "target" {
                                "target"
                            } else if deferred {
                                "privacy"
                            } else if text(node, "node_type") == "omitted" {
                                "omitted"
                            } else if text(node, "node_type") == "source" {
                                if ["missing_history", "opening_balance_attestation"]
                                    .contains(&text(node, "kind").as_str())
                                {
                                    "attested"
                                } else {
                                    "source"
                                }
                            } else {
                                "hop"
                            };
                            styles.insert(style);
                            let id = match text(node, "id") {
                                id if id.is_empty() || id == "None" => {
                                    format!("synthetic:{col}:{row}")
                                }
                                id => id,
                            };
                            FlowNode {
                                id,
                                label: text(node, "label"),
                                detail: if deferred {
                                    t("sof.privacy_deferred").into()
                                } else {
                                    label(&text(node, "kind")).replace("None", "")
                                },
                                amount: match amt(value(node, "amount")) {
                                    Cell::Empty => String::new(),
                                    cell => format!(
                                        "{} {}",
                                        cell.display(lang),
                                        if sats {
                                            "sats".into()
                                        } else {
                                            text(node, "asset")
                                        }
                                    )
                                    .trim()
                                    .to_owned(),
                                },
                                style,
                            }
                        })
                        .collect(),
                }
            })
            .collect();
        let links = list(flow, "edges")
            .iter()
            .map(|edge| {
                let kind = text(edge, "link_type");
                let privacy = truthy(value(edge, "deferred_privacy_hop"))
                    || ["coinjoin", "payjoin"].contains(&kind.as_str());
                let swap = ["swap", "peg_in", "peg_out", "lightning_swap"].contains(&kind.as_str());
                let share = exact(value(edge, "share_of_target"));
                let fraction = share
                    .as_ref()
                    .map(|d| d.to_canonical().parse::<f64>().unwrap_or(0.0) / 100.0)
                    .unwrap_or(0.0);
                FlowLink {
                    from: text(edge, "from"),
                    to: text(edge, "to"),
                    label: share.map(|d| lang.percent(&d)).unwrap_or_default(),
                    style: if privacy {
                        "privacy"
                    } else if swap {
                        "swap"
                    } else {
                        "plain"
                    },
                    weight: (0.6 + fraction * 2.4).clamp(0.6, 3.0),
                }
            })
            .collect::<Vec<_>>();
        let legend = [
            ("target", "box-strong", "sof.legend.target"),
            ("source", "box-strong", "sof.legend.source"),
            ("attested", "box-dashed", "sof.legend.attested"),
            ("hop", "box", "sof.legend.hop"),
            ("privacy", "box-dashed", "sof.legend.privacy"),
        ]
        .into_iter()
        .filter(|(style, _, _)| styles.contains(style))
        .map(|(_, mark, key)| Legend {
            label: t(key).into(),
            mark,
            tone: if mark == "box-dashed" { "muted" } else { "ink" },
        })
        .collect();
        blocks.push(Block::Flow(Flow {
            columns,
            links,
            legend,
        }));
        let deferred = list(flow, "deferred_privacy_hops").len();
        if deferred > 0 {
            blocks.push(note(lang.tf(
                "sof.deferred_hops",
                &[("count", &lang.integer(deferred as i128))],
            )));
        }
    }
    sections.push(section("flow", t("sof.flow"), blocks));

    // 5. Data sources
    let sources = table(
        "data_sources",
        String::new(),
        vec![
            col("name", t("sof.col.name"), Kind::Text, 1.4),
            col("kind", t("col.kind"), Kind::Text, 0.9),
            col("provenance", t("sof.col.provenance"), Kind::Text, 0.8),
            col("transactions", t("col.transactions"), Kind::Count, 0.8),
            col("sources", t("sof.col.sources"), Kind::Count, 0.6),
            col("assets", t("col.assets"), Kind::Text, 0.7),
            col("period", t("at.period"), Kind::Text, 1.7),
        ],
        list(report, "data_sources")
            .iter()
            .map(|item| {
                let provenance = text(item, "provenance");
                let period =
                    if truthy(value(item, "first_seen")) || truthy(value(item, "last_seen")) {
                        format!(
                            "{} – {}",
                            when(value(item, "first_seen")).display(lang),
                            when(value(item, "last_seen")).display(lang)
                        )
                    } else {
                        String::new()
                    };
                vec![
                    txt_or(item, "label"),
                    Cell::Text(label(&text(item, "kind"))),
                    Cell::Text(
                        lang.get(&format!("sof.provenance.{provenance}"))
                            .map(str::to_owned)
                            .unwrap_or_else(|| label(&provenance)),
                    ),
                    Cell::Count(int(item, "transaction_count")),
                    Cell::Count(int(item, "source_count")),
                    Cell::Text(
                        list(item, "assets")
                            .iter()
                            .map(py_str)
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                    Cell::Text(period),
                ]
            })
            .collect(),
    );
    sections.push(section(
        "data_sources",
        t("sof.data_sources"),
        vec![if sources.rows.is_empty() {
            note(t("sof.no_data_sources"))
        } else {
            Block::Table(sources)
        }],
    ));

    // 6. Evidence checklist and review gates
    let checklist = list(context, "evidence_checklist");
    let mut blocks = vec![if checklist.is_empty() {
        note(t("sof.no_checklist"))
    } else {
        Block::Table(table(
            "evidence_checklist",
            t("sof.checklist").into(),
            vec![
                col("item", "#", Kind::Count, 0.15),
                col("requirement", t("sof.col.requirement"), Kind::Text, 3.0),
            ],
            checklist
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    vec![
                        Cell::Count(i as i64 + 1),
                        Cell::Text(translated(lang, &py_str(item))),
                    ]
                })
                .collect(),
        ))
    }];
    let gates = value(report, "explain_gates");
    let exportable = truthy(value(gates, "exportable"));
    blocks.push(Block::Facts(vec![
        Fact {
            accent: !exportable,
            ..fact(
                "gate_status",
                t("sof.gate_status"),
                Cell::Text(
                    t(if exportable {
                        "sof.exportable"
                    } else {
                        "sof.blocked"
                    })
                    .into(),
                ),
            )
        },
        Fact {
            accent: !list(gates, "blockers").is_empty(),
            ..fact(
                "blockers",
                t("sof.blockers"),
                Cell::Count(list(gates, "blockers").len() as i64),
            )
        },
        fact(
            "warnings",
            t("sof.warnings"),
            Cell::Count(list(gates, "warnings").len() as i64),
        ),
    ]));
    let findings = list(report, "findings");
    blocks.push(if findings.is_empty() {
        note(t("sof.no_findings"))
    } else {
        Block::Table(table(
            "findings",
            t("sof.findings").into(),
            vec![
                col("severity", t("sof.col.severity"), Kind::Text, 0.7),
                col("code", t("at.col.code"), Kind::Code, 1.1),
                col("message", t("at.col.message"), Kind::Text, 3.0),
                col("reference", t("at.col.reference"), Kind::Code, 1.2),
            ],
            findings
                .iter()
                .map(|finding| {
                    let mut message = text(finding, "message");
                    let next = value(finding, "next_step");
                    if truthy(value(next, "headline")) {
                        message = format!(
                            "{message} {}: {}",
                            t("sof.next_step"),
                            text(next, "headline")
                        );
                    }
                    vec![
                        txt_or(finding, "severity"),
                        txt_or(finding, "code"),
                        Cell::Text(message),
                        txt_or(finding, "ref"),
                    ]
                })
                .collect(),
        ))
    });
    sections.push(section("review_gates", t("sof.review_gates"), blocks));

    // 7. Missing history and gaps, when there are any
    let gaps = list(report, "gaps");
    if !gaps.is_empty() {
        sections.push(Section {
            lead: t("sof.gaps_lead").into(),
            ..section(
                "gaps",
                t("sof.gaps"),
                vec![Block::Table(table(
                    "gaps",
                    String::new(),
                    vec![
                        col("severity", t("sof.col.severity"), Kind::Text, 0.7),
                        col("gap", t("sof.col.gap"), Kind::Text, 3.0),
                        col("amount", amount_label("col.amount"), amount_kind, 1.1),
                        col("asset", t("col.asset"), Kind::Text, 0.5),
                        col("reference", t("at.col.reference"), Kind::Code, 1.3),
                    ],
                    gaps.iter()
                        .map(|gap| {
                            vec![
                                txt_or(gap, "severity"),
                                txt_or(gap, "message"),
                                amt(value(gap, "amount")),
                                txt_or(gap, "asset"),
                                txt_or(gap, "ref"),
                            ]
                        })
                        .collect(),
                ))],
            )
        });
    }

    // 8. Source mix and the root sources behind it
    let mix = list(report, "source_mix");
    let mut blocks = Vec::new();
    let gross = list(allocations, "gross_source_requirement");
    if !gross.is_empty() && !truthy(value(allocations, "gross_matches_target")) {
        let phrase = gross
            .iter()
            .map(|row| {
                format!(
                    "{} {}",
                    amt(value(row, "amount")).display(lang),
                    text(row, "asset").replace("None", "")
                )
                .trim()
                .to_owned()
            })
            .collect::<Vec<_>>()
            .join(", ");
        blocks.push(note(lang.tf(
            "sof.gross",
            &[
                (
                    "target",
                    &amt(value(allocations, "target_amount")).display(lang),
                ),
                ("asset", &text(allocations, "asset")),
                ("gross", &phrase),
            ],
        )));
        let explanation = value(allocations, "route_difference_explanation");
        if text(explanation, "kind") == "network_fees" {
            blocks.push(note(lang.tf(
                "sof.network_fees",
                &[
                    ("fee", &amt(value(explanation, "fee")).display(lang)),
                    ("asset", &text(allocations, "asset")),
                    ("hops", &lang.integer(int(explanation, "hop_count").into())),
                ],
            )));
        } else if let Some(difference) = exact(value(allocations, "route_difference")) {
            if !difference.is_zero() {
                let difference = difference.abs(&DECIMAL).unwrap_or(difference);
                blocks.push(note(lang.tf(
                    "sof.route_difference",
                    &[
                        ("difference", &Cell::Btc(difference).display(lang)),
                        ("asset", &text(allocations, "asset")),
                    ],
                )));
            }
        }
    }
    let legacy = mix
        .iter()
        .any(|row| matches!(value(row, "asset"), Value::Null));
    if legacy {
        blocks.push(note(t("sof.legacy")));
    }
    if mix.is_empty() {
        blocks.push(note(t("sof.no_sources")));
    } else {
        blocks.push(Block::Table(table(
            "source_mix",
            String::new(),
            vec![
                col("source", t("sof.col.source"), Kind::Text, 1.8),
                col("amount", amount_label("col.amount"), amount_kind, 1.1),
                col("asset", t("col.asset"), Kind::Text, 0.6),
                col("share", t("sof.col.share"), Kind::Percent, 0.7),
                col("count", t("sof.col.count"), Kind::Count, 0.6),
            ],
            mix.iter()
                .map(|row| {
                    let asset = value(row, "asset");
                    vec![
                        Cell::Text(label(&text(row, "source_type"))),
                        amt(value(row, "amount")),
                        if present(asset) {
                            Cell::Text(py_str(asset))
                        } else {
                            Cell::Text(text(allocations, "asset"))
                        },
                        if present(asset) {
                            exact(value(row, "percent_of_target"))
                                .map(Cell::Percent)
                                .unwrap_or(Cell::Empty)
                        } else {
                            Cell::Empty
                        },
                        Cell::Count(int(row, "count")),
                    ]
                })
                .collect(),
        )));
    }
    let graph = value(report, "graph");
    let mut roots: Vec<&Value> = list(graph, "nodes")
        .iter()
        .filter(|n| text(n, "node_type") == "source")
        .collect();
    roots.sort_by_key(|n| {
        let at = value(n, "acquired_at");
        if truthy(at) {
            py_str(at)
        } else {
            String::new()
        }
    });
    if !roots.is_empty() {
        blocks.push(Block::Table(Table {
            note: t("sof.roots_note").into(),
            ..table(
                "root_sources",
                t("sof.roots").into(),
                vec![
                    col("date", t("col.date"), Kind::Text, 1.1),
                    col("source", t("sof.col.source"), Kind::Text, 1.5),
                    col("kind", t("col.kind"), Kind::Text, 0.9),
                    col(
                        "allocated",
                        amount_label("sof.col.allocated"),
                        amount_kind,
                        1.0,
                    ),
                    col("asset", t("col.asset"), Kind::Text, 0.5),
                    col("original_fiat", t("sof.col.original_fiat"), Kind::Fiat, 1.0),
                    col("fiat_currency", t("scope.fiat_currency"), Kind::Text, 0.5),
                    col("review", t("sof.col.review"), Kind::Text, 0.7),
                ],
                roots
                    .iter()
                    .map(|node| {
                        let required = value(node, "required_amount");
                        vec![
                            when(value(node, "acquired_at")),
                            txt_or(node, "label"),
                            Cell::Text(label(&text(node, "source_type"))),
                            amt(if matches!(required, Value::Null) {
                                value(node, "amount")
                            } else {
                                required
                            }),
                            txt_or(node, "asset"),
                            fiat_cell(value(node, "fiat_value")),
                            txt_or(node, "fiat_currency"),
                            txt_or(node, "review_state"),
                        ]
                    })
                    .collect(),
            )
        }));
    }
    sections.push(section("source_mix", t("sof.source_mix"), blocks));

    // 9. The wide detail tables, on landscape pages, unless omitted
    let shown = |key: &str| !omitted.iter().any(|o| o == key);
    let levels_data = list(report, "flow_levels");
    let nodes_by_id: Vec<(String, &Value)> = list(graph, "nodes")
        .iter()
        .filter(|n| truthy(value(n, "id")))
        .map(|n| (text(n, "id"), n))
        .collect();
    let graph_node = |id: &str| nodes_by_id.iter().find(|(k, _)| k == id).map(|(_, n)| *n);
    if shown("flow_levels") {
        let rows: Vec<Vec<Cell>> = levels_data
            .iter()
            .flat_map(|level| {
                list(level, "nodes").iter().map(move |node| {
                    let kind = ["source_type", "direction", "node_type"]
                        .iter()
                        .map(|k| text(node, k))
                        .find(|v| !v.is_empty() && v != "None")
                        .unwrap_or_default();
                    let required = value(node, "required_amount");
                    let time = if truthy(value(node, "occurred_at")) {
                        value(node, "occurred_at")
                    } else {
                        value(node, "acquired_at")
                    };
                    vec![
                        txt_or(level, "level"),
                        Cell::Text(label(&text(level, "role"))),
                        txt_or(node, "label"),
                        Cell::Text(label(&kind)),
                        when(time),
                        amt(if matches!(required, Value::Null) {
                            value(node, "amount")
                        } else {
                            required
                        }),
                        txt_or(node, "asset"),
                    ]
                })
            })
            .collect();
        sections.push(Section {
            landscape: true,
            ..section(
                "flow_levels",
                t("sof.flow_levels"),
                vec![if rows.is_empty() {
                    note(t("sof.no_flow_levels"))
                } else {
                    Block::Table(table(
                        "flow_levels",
                        String::new(),
                        vec![
                            col("level", t("sof.col.level"), Kind::Text, 0.4),
                            col("role", t("sof.col.role"), Kind::Text, 0.9),
                            col("node", t("sof.col.node"), Kind::Text, 2.0),
                            col("kind", t("col.kind"), Kind::Text, 1.1),
                            col("date", t("col.date"), Kind::Text, 1.1),
                            col("amount", amount_label("col.amount"), amount_kind, 1.1),
                            col("asset", t("col.asset"), Kind::Text, 0.5),
                        ],
                        rows,
                    ))
                }],
            )
        });
    }
    if shown("transaction_details") {
        // One table across levels with the level as a column; the levels'
        // counts and fiat totals precede it.
        let mut level_rows = Vec::new();
        let mut rows = Vec::new();
        for level in levels_data {
            let nodes = list(level, "nodes");
            if nodes.is_empty() {
                continue;
            }
            level_rows.push(vec![
                txt_or(level, "level"),
                Cell::Count(int(level, "transaction_count")),
                Cell::Count(int(level, "source_count")),
                exact(value(level, "fiat_value_total"))
                    .map(Cell::Fiat)
                    .unwrap_or(Cell::Empty),
                txt_or(level, "fiat_currency"),
            ]);
            for node in nodes {
                let full = graph_node(&text(node, "id"));
                let pick = |key: &str| {
                    let own = value(node, key);
                    if present(own) {
                        own.clone()
                    } else {
                        full.map(|f| value(f, key).clone()).unwrap_or(Value::Null)
                    }
                };
                let is_source = text(node, "node_type") == "source";
                let direction = py_str(&pick("direction")).replace("None", "");
                let inbound = is_source || direction == "inbound";
                let required = value(node, "required_amount");
                let amount = amt(if matches!(required, Value::Null) {
                    value(node, "amount")
                } else {
                    required
                });
                let fee = pick("fee");
                let fee = match exact(&fee) {
                    Some(d) if d.is_positive() => amt(&fee),
                    _ => Cell::Empty,
                };
                let provenance = if is_source {
                    "attested".to_owned()
                } else {
                    py_str(&pick("data_provenance")).replace("None", "")
                };
                let fiat_value = match get(node, "fiat_value_allocated") {
                    Some(v) => v,
                    None => value(node, "fiat_value"),
                };
                let time = if truthy(value(node, "occurred_at")) {
                    value(node, "occurred_at")
                } else {
                    value(node, "acquired_at")
                };
                rows.push(vec![
                    txt_or(level, "level"),
                    when(time),
                    if is_source {
                        txt_or(node, "label")
                    } else {
                        txt_or(node, "wallet")
                    },
                    Cell::Text(if is_source {
                        label(&text(node, "source_type"))
                    } else {
                        lang.get(&format!("direction.{direction}"))
                            .map(str::to_owned)
                            .unwrap_or_else(|| label(&direction))
                    }),
                    if inbound { amount.clone() } else { Cell::Empty },
                    if inbound { Cell::Empty } else { amount },
                    fee,
                    txt_or(node, "asset"),
                    fiat_cell(fiat_value),
                    Cell::Text(match text(node, "external_id") {
                        e if !e.is_empty() && e != "None" => e,
                        _ if is_source => String::new(),
                        _ => text(node, "label"),
                    }),
                    Cell::Text(
                        lang.get(&format!("sof.provenance.{provenance}"))
                            .map(str::to_owned)
                            .unwrap_or_else(|| label(&provenance)),
                    ),
                ]);
            }
        }
        let blocks = if rows.is_empty() {
            vec![note(t("sof.no_transactions"))]
        } else {
            vec![
                note(t("sof.details_lead")),
                Block::Table(table(
                    "levels",
                    String::new(),
                    vec![
                        col("level", t("sof.col.level"), Kind::Text, 0.5),
                        col("transactions", t("col.transactions"), Kind::Count, 0.9),
                        col("sources", t("sof.col.sources"), Kind::Count, 0.7),
                        col("fiat_total", t("sof.col.fiat_total"), Kind::Fiat, 1.0),
                        col("fiat_currency", t("scope.fiat_currency"), Kind::Text, 0.6),
                    ],
                    level_rows,
                )),
                Block::Table(table(
                    "transaction_details",
                    String::new(),
                    vec![
                        col("level", t("sof.col.level"), Kind::Text, 0.4),
                        col("date", t("col.date"), Kind::Text, 1.0),
                        col("source", t("sof.col.source"), Kind::Text, 1.1),
                        col("kind", t("col.kind"), Kind::Text, 0.8),
                        col(
                            "inbound",
                            amount_label("direction.inbound"),
                            amount_kind,
                            1.0,
                        ),
                        col(
                            "outbound",
                            amount_label("direction.outbound"),
                            amount_kind,
                            1.0,
                        ),
                        col("fee", amount_label("col.fee"), amount_kind, 0.9),
                        col("asset", t("col.asset"), Kind::Text, 0.5),
                        col(
                            "fiat",
                            lang.tf("col.value", &[("fiat", &currency)]),
                            Kind::Fiat,
                            0.9,
                        ),
                        col("reference", t("at.col.reference"), Kind::Code, 1.6),
                        col("data_source", t("sof.col.provenance"), Kind::Text, 0.7),
                    ],
                    rows,
                )),
            ]
        };
        sections.push(Section {
            landscape: true,
            ..section("transaction_details", t("sof.transaction_details"), blocks)
        });
    }
    let endpoint = |id: &Value| -> String {
        match graph_node(&py_str(id)) {
            None => String::new(),
            Some(node) if text(node, "node_type") == "source" => match text(node, "label") {
                l if !l.is_empty() => l,
                _ => label(&text(node, "source_type")),
            },
            Some(node) => match text(node, "wallet") {
                w if !w.is_empty() && w != "None" => w,
                _ => text(node, "label"),
            },
        }
    };
    if shown("flow_links") {
        let edges = list(graph, "edges");
        sections.push(Section {
            landscape: true,
            ..section(
                "flow_links",
                t("sof.flow_links"),
                if edges.is_empty() {
                    vec![note(t("sof.no_links"))]
                } else {
                    vec![
                        note(t("sof.links_reviewed")),
                        Block::Table(table(
                            "flow_links",
                            String::new(),
                            vec![
                                col("kind", t("col.kind"), Kind::Text, 0.9),
                                col("from", t("sof.col.from"), Kind::Text, 1.2),
                                col("to", t("sof.col.to"), Kind::Text, 1.2),
                                col("amount", amount_label("col.amount"), amount_kind, 1.0),
                                col("asset", t("col.asset"), Kind::Text, 0.5),
                                col("method", t("sof.col.method"), Kind::Text, 0.9),
                                col("explanation", t("sof.col.explanation"), Kind::Text, 2.2),
                            ],
                            edges
                                .iter()
                                .map(|edge| {
                                    let kind = text(edge, "link_type");
                                    let explanation = match text(edge, "explanation") {
                                        e if !e.is_empty() && e != "None" => e,
                                        _ => lang
                                            .get(&format!("sof.link.{kind}"))
                                            .unwrap_or("")
                                            .to_owned(),
                                    };
                                    vec![
                                        Cell::Text(label(&kind)),
                                        Cell::Text(endpoint(value(edge, "from"))),
                                        Cell::Text(endpoint(value(edge, "to"))),
                                        amt(value(edge, "allocation_amount")),
                                        txt_or(edge, "asset"),
                                        Cell::Text(label(&text(edge, "method"))),
                                        Cell::Text(explanation),
                                    ]
                                })
                                .collect(),
                        )),
                    ]
                },
            )
        });
    }
    if shown("graph_nodes") {
        let nodes = list(graph, "nodes");
        sections.push(Section {
            landscape: true,
            ..section(
                "graph_nodes",
                t("sof.graph_nodes"),
                vec![if nodes.is_empty() {
                    note(t("sof.no_nodes"))
                } else {
                    Block::Table(table(
                        "graph_nodes",
                        String::new(),
                        vec![
                            col("kind", t("col.kind"), Kind::Text, 0.9),
                            col("label", t("sof.col.node"), Kind::Text, 1.5),
                            col("date", t("col.date"), Kind::Text, 1.0),
                            col("asset", t("col.asset"), Kind::Text, 0.5),
                            col("amount", amount_label("col.amount"), amount_kind, 1.0),
                            col("details", t("sof.col.details"), Kind::Text, 2.4),
                        ],
                        nodes
                            .iter()
                            .map(|node| {
                                let source = text(node, "node_type") == "source";
                                let mut details = Vec::new();
                                if source {
                                    if present(value(node, "fiat_value")) {
                                        details.push(
                                            lang.tf(
                                                "sof.original_fiat",
                                                &[(
                                                    "value",
                                                    &format!(
                                                        "{} {}",
                                                        fiat_cell(value(node, "fiat_value"))
                                                            .display(lang),
                                                        text(node, "fiat_currency")
                                                    ),
                                                )],
                                            ),
                                        );
                                    }
                                } else {
                                    if truthy(value(node, "wallet")) {
                                        details.push(lang.tf(
                                            "sof.wallet",
                                            &[("wallet", &text(node, "wallet"))],
                                        ));
                                    }
                                    if truthy(value(node, "direction")) {
                                        let d = text(node, "direction");
                                        details.push(
                                            lang.get(&format!("direction.{d}"))
                                                .map(str::to_owned)
                                                .unwrap_or_else(|| label(&d)),
                                        );
                                    }
                                }
                                if truthy(value(node, "description")) {
                                    details.push(text(node, "description"));
                                }
                                vec![
                                    Cell::Text(if source {
                                        match text(node, "source_type") {
                                            st if !st.is_empty() && st != "None" => label(&st),
                                            _ => t("sof.source").into(),
                                        }
                                    } else {
                                        t("sof.transaction").into()
                                    }),
                                    txt_or(node, "label"),
                                    when(if source {
                                        value(node, "acquired_at")
                                    } else {
                                        value(node, "occurred_at")
                                    }),
                                    txt_or(node, "asset"),
                                    amt(value(node, "required_amount")),
                                    Cell::Text(details.join(" | ")),
                                ]
                            })
                            .collect(),
                    ))
                }],
            )
        });
    }

    // 10. What sharing this report discloses
    let preview = value(report, "disclosure_preview");
    let txids = list(preview, "txids");
    let explorer: Vec<(String, &Value)> = list(preview, "explorer_links")
        .iter()
        .filter(|l| truthy(value(l, "txid")) && truthy(value(l, "url")))
        .map(|l| (text(l, "txid"), l))
        .collect();
    let mut blocks = vec![if txids.is_empty() {
        Block::Facts(vec![fact(
            "txids",
            t("sof.txids"),
            Cell::Text(t("sof.no_txids").into()),
        )])
    } else {
        Block::Table(table(
            "disclosed_txids",
            String::new(),
            vec![
                col("txid", t("sof.col.txid"), Kind::Code, 2.6),
                col("explorer", t("sof.col.explorer"), Kind::Text, 2.0),
            ],
            txids
                .iter()
                .map(|txid| {
                    let id = py_str(txid);
                    let link = explorer.iter().find(|(k, _)| *k == id).map(|(_, l)| *l);
                    vec![
                        Cell::Text(id),
                        Cell::Text(match link {
                            Some(l) => text(l, "url"),
                            None => t("sof.not_public").into(),
                        }),
                    ]
                })
                .collect(),
        ))
    }];
    let attachments = list(preview, "attachments");
    if attachments.is_empty() {
        blocks.push(note(t("sof.no_attachments")));
    } else {
        blocks.push(Block::Table(table(
            "disclosed_attachments",
            t("sof.attachments").into(),
            vec![
                col("label", t("sof.col.name"), Kind::Text, 1.4),
                col("type", t("col.kind"), Kind::Text, 0.8),
                col("media", t("sof.col.media"), Kind::Text, 0.8),
                col("sha256", "SHA-256", Kind::Code, 2.2),
                col("location", t("sof.col.location"), Kind::Text, 1.4),
            ],
            attachments
                .iter()
                .map(|item| {
                    vec![
                        txt_or(item, "label"),
                        txt_or(item, "attachment_type"),
                        txt_or(item, "media_type"),
                        txt_or(item, "sha256"),
                        Cell::Text(match text(item, "source_url") {
                            u if !u.is_empty() && u != "None" => u,
                            _ => text(item, "stored_relpath").replace("None", ""),
                        }),
                    ]
                })
                .collect(),
        )));
    }
    let wallets_named: Vec<String> = list(preview, "wallets_named").iter().map(py_str).collect();
    let mut footprint = vec![
        fact(
            "txids_disclosed",
            t("sof.txids_disclosed"),
            Cell::Count(txids.len() as i64),
        ),
        fact(
            "evidence_files",
            t("sof.evidence_files"),
            Cell::Count(attachments.len() as i64),
        ),
    ];
    if !wallets_named.is_empty() {
        footprint.push(fact(
            "wallets_named",
            t("sof.wallets_named"),
            Cell::Text(wallets_named.join(", ")),
        ));
    }
    let excluded: Vec<String> = list(preview, "excluded").iter().map(py_str).collect();
    if !excluded.is_empty() {
        footprint.push(fact(
            "excluded",
            t("sof.excluded"),
            Cell::Text(excluded.join(", ")),
        ));
    }
    blocks.push(Block::Facts(footprint));
    if truthy(value(preview, "ownership_note")) {
        blocks.push(note(text(preview, "ownership_note")));
    }
    if truthy(value(preview, "privacy_note")) {
        blocks.push(note(text(preview, "privacy_note")));
    }
    sections.push(section("disclosure", t("sof.disclosure"), blocks));

    // 11. Limitations
    let mut blocks = vec![
        note(t("sof.limit.evidence")),
        note(t("sof.limit.opening")),
        note(t("sof.limit.suggested")),
    ];
    if !list(flow, "deferred_privacy_hops").is_empty() {
        blocks.push(note(t("sof.limit.privacy")));
    }
    sections.push(section("limitations", t("sof.limitations"), blocks));

    Document {
        kind: "source_funds",
        title: title.into(),
        kicker: t(if country == "at" {
            "doc.kicker.at"
        } else {
            "doc.kicker"
        })
        .into(),
        book,
        period: text(target, "label"),
        period_start: String::new(),
        period_end: String::new(),
        generated_at: text(model, "generated_at"),
        input_version: None,
        fiat: currency,
        country,
        lang,
        sections,
    }
}

/// A text cell, empty for a missing value.
fn txt_or(row: &Value, key: &str) -> Cell {
    match value(row, key) {
        Value::Null => Cell::Text(String::new()),
        v => Cell::Text(py_str(v)),
    }
}
