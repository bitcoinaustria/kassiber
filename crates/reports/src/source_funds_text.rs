use super::*;

fn list<'a>(row: &'a Value, key: &str) -> &'a [Value] {
    match value(row, key) {
        Value::List(v) => v,
        _ => &[],
    }
}
fn fallback(row: &Value, key: &str) -> String {
    get(row, key)
        .filter(|v| truthy(v))
        .map(py_str)
        .unwrap_or_default()
}
fn number(v: &Value) -> String {
    format!("{:.8}", py_str(v).parse::<f64>().unwrap_or(0.0))
}
fn table(
    lines: &mut Vec<String>,
    headers: &[&str],
    records: Vec<Vec<String>>,
    widths: &[usize],
    right: &[usize],
) {
    let render = |row: Vec<String>| {
        row.into_iter()
            .zip(widths)
            .enumerate()
            .map(|(i, (v, &w))| super::summary_text::cell(&v, w, right.contains(&i)))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_owned()
    };
    lines.push(render(headers.iter().map(|s| (*s).into()).collect()));
    lines.push(
        widths
            .iter()
            .map(|w| "-".repeat(*w))
            .collect::<Vec<_>>()
            .join("  "),
    );
    lines.extend(records.into_iter().map(render));
}
pub fn render(report: &Value) -> Result<String> {
    let target = value(report, "target");
    let purpose = value(report, "purpose");
    let planned = text(purpose, "type") == "planned_exchange_sale";
    let title = "Kassiber Source of Funds Report";
    let mut lines = vec![
        title.into(),
        "=".repeat(title.len()),
        String::new(),
        format!("Workspace:       {}", text(report, "workspace")),
        format!("Profile:         {}", text(report, "profile")),
        format!("Purpose:         {}", text(purpose, "label")),
        format!("Reveal mode:     {}", text(report, "reveal_mode")),
        format!(
            "{}:          {}",
            if planned {
                "Bitcoin being sold"
            } else {
                "Target"
            },
            text(target, "label")
        ),
        format!(
            "{}:   {} {}",
            if planned {
                "Planned amount"
            } else {
                "Target amount"
            },
            number(value(target, "required_amount")),
            text(target, "asset")
        ),
        format!(
            "Exportable:      {}",
            py_str(value(value(report, "explain_gates"), "exportable"))
        ),
        String::new(),
    ];
    if planned {
        lines.extend([
            "Planned Sale".into(),
            "------------".into(),
            format!(
                "Destination:     {}",
                if truthy(value(purpose, "planned_destination")) {
                    text(purpose, "planned_destination")
                } else {
                    "(not specified)".into()
                }
            ),
            format!(
                "Note:            {}",
                if truthy(value(purpose, "planned_note")) {
                    text(purpose, "planned_note")
                } else {
                    "(none)".into()
                }
            ),
            fallback(purpose, "fiat_purchase_note"),
            String::new(),
        ]);
    }
    let overview = value(report, "overview");
    let range = value(overview, "time_range");
    if truthy(overview) {
        lines.extend([
            "Overview".into(),
            "--------".into(),
            format!(
                "Time range:      {} - {}",
                fallback(range, "start"),
                fallback(range, "end")
            ),
            format!("Transactions:    {}", text(overview, "transaction_count")),
            format!("Reviewed links:  {}", text(overview, "link_count")),
            format!("Data sources:    {}", text(overview, "data_source_count")),
            format!(
                "Source types:    {}",
                text(overview, "source_category_count")
            ),
            String::new(),
        ]);
    }
    let paragraphs = list(value(report, "narrative"), "paragraphs");
    if !paragraphs.is_empty() {
        lines.extend([
            "Origin and Transaction Flow".into(),
            "---------------------------".into(),
        ]);
        lines.extend(paragraphs.iter().map(py_str));
        lines.push(String::new());
    }
    let sources = list(report, "data_sources");
    if !sources.is_empty() {
        lines.extend(["Data Sources".into(), "------------".into()]);
        table(
            &mut lines,
            &["Name", "Kind", "Transactions", "Sources", "Assets"],
            sources
                .iter()
                .map(|r| {
                    vec![
                        text(r, "label"),
                        text(r, "kind"),
                        text(r, "transaction_count"),
                        text(r, "source_count"),
                        list(r, "assets")
                            .iter()
                            .map(py_str)
                            .collect::<Vec<_>>()
                            .join(","),
                    ]
                })
                .collect(),
            &[28, 20, 12, 10, 16],
            &[2, 3],
        );
        lines.push(String::new());
    }
    let preview = value(report, "disclosure_preview");
    let txids = list(preview, "txids")
        .iter()
        .map(py_str)
        .collect::<Vec<_>>()
        .join(", ");
    let attachments = list(preview, "attachments")
        .iter()
        .map(|r| text(r, "label"))
        .collect::<Vec<_>>()
        .join(", ");
    lines.extend([
        "Disclosure Preview".into(),
        "------------------".into(),
        format!(
            "Txids: {}",
            if txids.is_empty() {
                "(none in this reveal mode)"
            } else {
                &txids
            }
        ),
        format!(
            "Evidence attachments: {}",
            if attachments.is_empty() {
                "(none)"
            } else {
                &attachments
            }
        ),
        text(preview, "privacy_note"),
        String::new(),
        "Source Mix".into(),
        "----------".into(),
    ]);
    let mix = list(report, "source_mix");
    let allocations = value(report, "allocations");
    if !mix.is_empty() {
        let gross = list(allocations, "gross_source_requirement")
            .iter()
            .map(|r| format!("{} {}", number(value(r, "amount")), text(r, "asset")))
            .collect::<Vec<_>>()
            .join(" + ");
        if !gross.is_empty() && !truthy(value(allocations, "gross_matches_target")) {
            lines.extend([
                format!(
                    "Selected target:         {} {}",
                    number(value(allocations, "target_amount")),
                    fallback(allocations, "asset")
                ),
                format!("Gross upstream demand:   {gross}"),
            ]);
            let explanation = value(allocations, "route_difference_explanation");
            if text(explanation, "kind") == "network_fees" {
                let hops = int(explanation, "hop_count");
                lines.push(format!("That is {} {} plus {} {} of network fees recorded on {hops} disclosed transaction{}.",number(value(allocations,"target_amount")),fallback(allocations,"asset"),number(value(explanation,"fee")),fallback(allocations,"asset"),if hops==1{""}else{"s"}));
            } else if truthy(value(allocations, "route_difference")) {
                let difference = py_str(value(allocations, "route_difference"))
                    .parse::<f64>()
                    .unwrap_or(0.0)
                    .abs();
                lines.push(format!("The route difference ({difference:.8} {}) is not automatically classified as a fee.",fallback(allocations,"asset")));
            }
            lines.push(String::new());
        }
        table(
            &mut lines,
            &["Source", "Amount", "Asset", "Count"],
            mix.iter()
                .map(|r| {
                    vec![
                        text(r, "source_type"),
                        number(value(r, "amount")),
                        if truthy(value(r, "asset")) {
                            text(r, "asset")
                        } else {
                            text(allocations, "asset")
                        },
                        text(r, "count"),
                    ]
                })
                .collect(),
            &[32, 16, 8, 8],
            &[1, 3],
        );
    } else {
        lines.push("No reviewed root sources yet.".into());
    }
    lines.extend([String::new(), "Findings".into(), "--------".into()]);
    let findings = list(report, "findings");
    if findings.is_empty() {
        lines.push("No blockers or warnings.".into());
    } else {
        for f in findings {
            lines.push(
                format!(
                    "{} {}: {} {}",
                    kassiber_pytext::upper(&text(f, "severity")),
                    text(f, "code"),
                    text(f, "message"),
                    text(f, "ref")
                )
                .trim_end()
                .into(),
            );
            let hint = value(f, "next_step");
            if truthy(value(hint, "headline")) {
                lines.push(format!("  -> {}", text(hint, "headline")));
            }
        }
    }
    lines.extend([String::new(), "Flow Links".into(), "----------".into()]);
    let edges = list(value(report, "graph"), "edges");
    if edges.is_empty() {
        lines.push("No reviewed links yet.".into());
    } else {
        table(
            &mut lines,
            &["Type", "State", "Method", "Amount", "Policy", "Explanation"],
            edges
                .iter()
                .map(|e| {
                    vec![
                        text(e, "link_type"),
                        text(e, "state"),
                        text(e, "method"),
                        format!(
                            "{} {}",
                            number(value(e, "allocation_amount")),
                            text(e, "asset")
                        ),
                        text(e, "allocation_policy"),
                        text(e, "explanation"),
                    ]
                })
                .collect(),
            &[18, 10, 24, 18, 10, 52],
            &[],
        );
    }
    lines.extend([String::new(),"Limitations".into(),"-----------".into(),"Kassiber reports reviewed local evidence. It does not certify ownership, perform chain-surveillance scoring, or provide legal/AML advice.".into(),"Opening balances are rendered as attested prior-history stops, not as real root sources.".into(),"Suggested links and unconfirmed chain observations are never used as PDF proof.".into()]);
    Ok(lines.join("\n"))
}
