//! Python's line-oriented summary presentation, over the shared JSON report.
use super::*;

fn grouped(raw: &str) -> String {
    let (whole, fraction) = raw
        .split_once('.')
        .map_or((raw, None), |(a, b)| (a, Some(b)));
    let (sign, digits) = whole.strip_prefix('-').map_or(("", whole), |s| ("-", s));
    let mut out = sign.to_owned();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if let Some(fraction) = fraction {
        out.push('.');
        out.push_str(fraction);
    }
    out
}
fn count(row: &Value, key: &str) -> String {
    grouped(&int(row, key).to_string())
}
pub(super) fn number(v: &Value, places: usize) -> String {
    let n: f64 = py_str(v).parse().unwrap_or(0.0);
    if !n.is_finite() {
        return format!("{n}").to_lowercase();
    }
    grouped(&format!("{n:.places$}"))
}
fn kv(lines: &mut Vec<String>, label: &str, value: String) {
    let label = format!("{label}:");
    lines.push(format!(
        "{label}{} {value}",
        " ".repeat(28usize.saturating_sub(label.chars().count()))
    ));
}
// format_table comes from Python's PDF helper even for terminal summaries.
// Preserve its Latin-1 transliteration and ellipsis behavior in these cells.
pub(super) fn cell(raw: &str, width: usize, right: bool) -> String {
    let mut ascii = String::new();
    for c in raw.chars() {
        ascii.push_str(match c {
            '€' => "EUR",
            '₿' => "BTC",
            '↔' => "<->",
            '→' => "->",
            '←' => "<-",
            '—' | '–' | '‑' => "-",
            '⚠' => "(!)",
            '…' => "...",
            '’' | '‘' => "'",
            '“' | '”' => "\"",
            '‚' | '„' => ",",
            '≈' => "~",
            '•' => "-",
            _ => {
                ascii.push(if c as u32 <= 255 { c } else { '?' });
                continue;
            }
        });
    }
    if ascii.chars().count() > width {
        ascii = ascii
            .chars()
            .take(width.saturating_sub(3))
            .collect::<String>()
            + "...";
    }
    let padding = " ".repeat(width.saturating_sub(ascii.chars().count()));
    if right {
        padding + &ascii
    } else {
        ascii + &padding
    }
}
pub fn render(report: &Value) -> String {
    let scope = if truthy(value(report, "wallet")) {
        "wallet"
    } else {
        "profile"
    };
    let title = format!("Kassiber Summary Report - {}", text(report, scope));
    let mut lines = vec![
        title.clone(),
        "=".repeat(title.chars().count()),
        String::new(),
    ];
    for (label, key) in [
        ("Workspace", "workspace"),
        ("Profile", "profile"),
        ("Wallet scope", "wallet"),
        ("Fiat currency", "fiat_currency"),
        ("Tax country", "tax_country"),
        ("Tax long-term days", "tax_long_term_days"),
        ("Gains algorithm", "gains_algorithm"),
        ("Last processed at", "last_processed_at"),
        ("Processed tx count", "processed_tx_count"),
    ] {
        let v = if key == "wallet" && !truthy(value(report, key)) {
            "All wallets".into()
        } else if key == "processed_tx_count" {
            count(report, key)
        } else if key == "last_processed_at" && !truthy(value(report, key)) {
            String::new()
        } else {
            text(report, key)
        };
        kv(&mut lines, label, v);
    }
    lines.extend([String::new(), "Activity".into(), "--------".into()]);
    let metrics = value(report, "metrics");
    for (label, key) in [
        ("Wallets in scope", "wallets_in_scope"),
        ("Assets in scope", "assets_in_scope"),
        ("Transactions (active)", "active_transactions"),
        ("Transactions (excluded)", "excluded_transactions"),
        ("Inbound transactions", "inbound_transactions"),
        ("Outbound transactions", "outbound_transactions"),
        ("Journal entries", "journal_entries"),
        ("Quarantines", "quarantines"),
        ("Priced transactions", "priced_transactions"),
        ("Transactions with notes", "transactions_with_notes"),
        ("Transactions with tags", "transactions_with_tags"),
        ("First transaction", "first_transaction_at"),
        ("Last transaction", "last_transaction_at"),
    ] {
        let v = if key.ends_with("_at") {
            if truthy(value(metrics, key)) {
                text(metrics, key)
            } else {
                String::new()
            }
        } else {
            count(metrics, key)
        };
        kv(&mut lines, label, v);
    }
    lines.extend([
        String::new(),
        "Financial Summary".into(),
        "-----------------".into(),
    ]);
    for (label, section, key) in [
        ("Holdings cost basis", "holdings", "cost_basis"),
        ("Holdings market value", "holdings", "market_value"),
        ("Unrealized PnL", "holdings", "unrealized_pnl"),
        ("Realized proceeds", "realized", "proceeds"),
        ("Realized cost basis", "realized", "cost_basis"),
        ("Realized gain/loss", "realized", "gain_loss"),
    ] {
        kv(
            &mut lines,
            label,
            number(value(value(report, section), key), 2),
        );
    }
    lines.extend([String::new(), "Asset Flow".into(), "----------".into()]);
    let Value::List(flows) = value(report, "asset_flow") else {
        unreachable!()
    };
    if flows.is_empty() {
        lines.push("No active transactions in scope.".into());
    } else {
        let widths = [6, 6, 6, 6, 14, 14, 14];
        let row = |cells: Vec<String>| {
            cells
                .iter()
                .zip(widths)
                .enumerate()
                .map(|(i, (s, w))| cell(s, w, i > 0))
                .collect::<Vec<_>>()
                .join("  ")
                .trim_end()
                .to_owned()
        };
        lines.push(row([
            "Asset", "Tx", "In", "Out", "Inbound", "Outbound", "Fees",
        ]
        .into_iter()
        .map(String::from)
        .collect()));
        lines.push(
            widths
                .into_iter()
                .map(|w| "-".repeat(w))
                .collect::<Vec<_>>()
                .join("  "),
        );
        for flow in flows {
            lines.push(row(vec![
                text(flow, "asset"),
                count(flow, "tx_count"),
                count(flow, "inbound_count"),
                count(flow, "outbound_count"),
                number(value(flow, "inbound_amount"), 8),
                number(value(flow, "outbound_amount"), 8),
                number(value(flow, "fee_amount"), 8),
            ]));
        }
    }
    lines.join("\n")
}
