//! Profitability over a curated Lightning snapshot acquired by the book service.
//! These types have no fields for node pubkeys, credentials, invoices or routes.
use super::*;
use kassiber_pyjson::{BigInt, PyStr};
use std::io::Write;
use std::path::Path;

pub const DEFAULT_OPEN_COST_SAT: i64 = 2_500;

/// Internal adapter lookup, distinct from a user-facing connection block.
/// Wallet configuration is parsed only to extract its backend reference.
#[derive(Clone, Debug, PartialEq)]
pub struct ConnectionRef {
    pub id: Value,
    pub label: Value,
    pub kind: String,
    pub profile_id: Value,
    pub backend_name: Option<PyStr>,
}

impl ConnectionRef {
    fn payload(&self) -> Value {
        dict([
            ("id", self.id.clone()),
            ("label", self.label.clone()),
            ("kind", s(self.kind.as_str())),
        ])
    }
}

/// Match the reference's combined id/label query, including its ambiguity
/// refusal when one wallet's id equals another wallet's label. Do not trim ref.
pub fn resolve_connection(conn: &Connection, args: &Value) -> Result<ConnectionRef> {
    let reference = ["connection", "wallet", "label"]
        .into_iter()
        .map(|k| value(args, k))
        .find(|v| truthy(v))
        .unwrap_or(&Value::Null);
    let Value::Str(reference) = reference else {
        return Err(
            AppError::new("validation", "Specify which Lightning connection to read.")
                .with_hint("Pass `connection` (wallet id or label)."),
        );
    };
    let (_, profile) = kassiber_setup::references::scope(conn, args)?;
    let reference = reference.as_str().ok_or_else(|| utf8_error(reference))?;
    let found = rows(
        conn,
        "SELECT id,label,kind,profile_id,config_json FROM wallets WHERE profile_id=?1 AND (id=?2 OR lower(label)=lower(?2))",
        [&text(&profile, "id"), reference],
    )?;
    if found.is_empty() {
        return Err(AppError::new(
            "not_found",
            format!("Lightning connection '{reference}' not found."),
        )
        .with_hint("Run `kassiber wallets list` to see configured connections."));
    }
    if found.len() > 1 {
        return Err(AppError::new(
            "ambiguous",
            format!("Lightning connection '{reference}' is ambiguous in the active profile."),
        )
        .with_hint("Pass the wallet id instead of the label to disambiguate."));
    }
    let row = &found[0];
    let kind = get(row, "kind")
        .filter(|v| truthy(v))
        .map(py_str)
        .unwrap_or_default();
    if !["coreln", "lnd", "nwc"].contains(&kind.as_str()) {
        let label = get(row, "label")
            .filter(|v| truthy(v))
            .map(py_str)
            .unwrap_or_else(|| reference.to_owned());
        return Err(AppError::new(
            "validation",
            format!(
                "Connection '{label}' is not a Lightning node (kind={}).",
                kassiber_pytext::repr_str(&kind)
            ),
        )
        .with_hint("Lightning kinds are coreln, lnd, nwc."));
    }
    let config = get(row, "config_json")
        .and_then(|v| match v {
            Value::Str(s) => s.as_str(),
            _ => None,
        })
        .map(kassiber_pytext::loads);
    let config = match config {
        Some(Err(e)) if matches!(e.kind, kassiber_pytext::JsonErrorKind::TooDeep { .. }) => {
            return Err(AppError::new("internal_error", e.to_string()));
        }
        Some(Ok(config)) => config,
        _ => Value::Null,
    };
    let backend_name = match get(&config, "backend") {
        Some(Value::Str(s)) if !s.is("") => Some(s.clone()),
        _ => None,
    };
    Ok(ConnectionRef {
        id: value(row, "id").clone(),
        label: value(row, "label").clone(),
        kind,
        profile_id: value(row, "profile_id").clone(),
        backend_name,
    })
}

/// The daemon defaults invalid int inputs to 30, then clamps valid integers.
/// An infinite float escapes its TypeError/ValueError handler in Python.
pub fn daemon_window_days(raw: &Value) -> Result<u16> {
    let parsed = match raw {
        Value::Int(n) => n.clone(),
        Value::Bool(b) => i64::from(*b).into(),
        Value::Str(t) => t
            .as_str()
            .and_then(|t| kassiber_pytext::parse_int(t).ok())
            .unwrap_or_else(|| 30.into()),
        Value::Float(n) if n.is_infinite() => {
            return Err(AppError::new(
                "internal_error",
                "cannot convert float infinity to integer",
            ))
        }
        Value::Float(n) if n.is_finite() => {
            kassiber_pytext::parse_int(&format!("{:.0}", n.trunc()))
                .map_err(|_| AppError::new("internal_error", "invalid Lightning window"))?
        }
        _ => 30.into(),
    };
    Ok(u16::try_from(parsed.max(1.into()).min(365.into())).expect("clamped window"))
}

/// The report needs only open channels, in adapter order. Closed channels do
/// not contribute rows. These types cannot carry Tier-1 material or pubkeys.
#[derive(Clone)]
pub struct Channel {
    pub id: PyStr,
    pub peer_alias: PyStr,
    pub capacity_sat: BigInt,
    pub earned_routing_sat: Option<BigInt>,
}

#[derive(Clone)]
pub struct Routing {
    pub window_label: PyStr,
    pub routing_revenue_sat: BigInt,
    pub payment_cost_sat: BigInt,
    pub rebalance_cost_sat: BigInt,
    pub onchain_cost_sat: BigInt,
    /// Authoritative adapter profit, deliberately not recomputed from sat totals.
    pub net_profit_sat: BigInt,
    pub forward_count: BigInt,
    pub payment_count: BigInt,
    pub rebalance_count: BigInt,
}

pub struct Snapshot {
    pub routing: Option<Routing>,
    pub channels: Vec<Channel>,
}

pub struct Report {
    connection: ConnectionRef,
    routing: Routing,
    channels: Vec<Channel>,
    open_cost_sat: BigInt,
}

/// This reshaping is independent of tax journals and report-readiness checks,
/// as in Python. Acquisition and its exact-action consent belong to adapters.
pub fn build_report(
    connection: ConnectionRef,
    snapshot: Snapshot,
    open_cost_sat: BigInt,
) -> Report {
    let routing = snapshot.routing.unwrap_or_else(|| Routing {
        window_label: "No routing window reported".into(),
        routing_revenue_sat: 0.into(),
        payment_cost_sat: 0.into(),
        rebalance_cost_sat: 0.into(),
        onchain_cost_sat: 0.into(),
        net_profit_sat: 0.into(),
        forward_count: 0.into(),
        payment_count: 0.into(),
        rebalance_count: 0.into(),
    });
    Report {
        connection,
        routing,
        channels: snapshot.channels,
        open_cost_sat,
    }
}

impl Report {
    /// Desktop/CLI projection. AI redaction remains on the Python reference;
    /// this full operator payload must never be registered as an AI tool.
    pub fn payload(&self) -> Value {
        let r = &self.routing;
        dict([
            (
                "connection",
                dict([
                    ("id", report_text(&self.connection.id)),
                    ("label", report_text(&self.connection.label)),
                    ("kind", s(self.connection.kind.as_str())),
                ]),
            ),
            ("windowLabel", Value::Str(r.window_label.clone())),
            (
                "summary",
                dict([
                    (
                        "routingRevenueSat",
                        Value::Int(r.routing_revenue_sat.clone()),
                    ),
                    ("paymentCostSat", Value::Int(r.payment_cost_sat.clone())),
                    ("rebalanceCostSat", Value::Int(r.rebalance_cost_sat.clone())),
                    ("onchainCostSat", Value::Int(r.onchain_cost_sat.clone())),
                    ("netProfitSat", Value::Int(r.net_profit_sat.clone())),
                    ("forwardCount", Value::Int(r.forward_count.clone())),
                    ("paymentCount", Value::Int(r.payment_count.clone())),
                    ("rebalanceCount", Value::Int(r.rebalance_count.clone())),
                ]),
            ),
            (
                "channels",
                Value::List(
                    self.channels
                        .iter()
                        .map(|c| {
                            let earned = c.earned_routing_sat.clone().unwrap_or_else(|| 0.into());
                            dict([
                                ("channelId", Value::Str(c.id.clone())),
                                ("peerAlias", Value::Str(c.peer_alias.clone())),
                                ("capacitySat", Value::Int(c.capacity_sat.clone())),
                                ("earnedRoutingSat", Value::Int(earned.clone())),
                                ("openCostSat", Value::Int(self.open_cost_sat.clone())),
                                ("coversOpenCost", Value::Bool(earned >= self.open_cost_sat)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }

    pub fn csv_rows(&self) -> Result<Vec<Vec<PyStr>>> {
        let mut rows = vec![vec![
            "section".into(),
            "key".into(),
            "value_sat".into(),
            "value_btc".into(),
            "detail".into(),
        ]];
        let r = &self.routing;
        for (label, amount) in [
            ("routing_revenue", &r.routing_revenue_sat),
            ("payment_cost", &r.payment_cost_sat),
            ("rebalance_cost", &r.rebalance_cost_sat),
            ("onchain_cost", &r.onchain_cost_sat),
            ("net_profit", &r.net_profit_sat),
        ] {
            rows.push(vec![
                "summary".into(),
                label.into(),
                integer_text(amount)?.into(),
                sat_to_btc(amount)?.into(),
                r.window_label.clone(),
            ]);
        }
        for (label, count) in [
            ("forward_count", &r.forward_count),
            ("payment_count", &r.payment_count),
            ("rebalance_count", &r.rebalance_count),
        ] {
            rows.push(vec![
                "summary".into(),
                label.into(),
                integer_text(count)?.into(),
                "".into(),
                r.window_label.clone(),
            ]);
        }
        for c in &self.channels {
            let earned = c.earned_routing_sat.clone().unwrap_or_else(|| 0.into());
            let detail = concat(&[
                "capacity=".into(),
                integer_text(&c.capacity_sat)?.into(),
                " sat; open_cost=".into(),
                integer_text(&self.open_cost_sat)?.into(),
                " sat; covers_open_cost=".into(),
                if earned >= self.open_cost_sat {
                    "yes".into()
                } else {
                    "no".into()
                },
                "; id=".into(),
                c.id.clone(),
            ]);
            rows.push(vec![
                "channel".into(),
                c.peer_alias.clone(),
                integer_text(&earned)?.into(),
                sat_to_btc(&earned)?.into(),
                detail,
            ]);
        }
        Ok(rows)
    }

    /// Retain Python's direct open/truncate behavior: no parent creation,
    /// atomic replacement, saved report snapshot, dependency capture or filing.
    pub fn export_csv(&self, path: &Path) -> Result<Value> {
        let rows = self.csv_rows()?;
        let mut file = std::fs::File::create(path).map_err(|e| export::io_error(e, Some(path)))?;
        for row in &rows {
            let line = csv_line(row);
            let encoded = line.as_str().ok_or_else(|| utf8_error(&line))?;
            file.write_all(encoded.as_bytes())
                .map_err(|e| export::io_error(e, Some(path)))?;
        }
        export::sync_written(&file, path).map_err(|e| export::io_error(e, Some(path)))?;
        Ok(dict([
            ("connection", self.connection.payload()),
            ("file", s(path.to_string_lossy().into_owned())),
            ("rows", Value::Int((rows.len() - 1).into())),
        ]))
    }
}

fn integer_text(n: &BigInt) -> Result<String> {
    kassiber_pyjson::dumps(&Value::Int(n.clone()), &Default::default())
        .map_err(|e| AppError::new("internal_error", e.to_string()))
}

fn report_text(raw: &Value) -> Value {
    match raw {
        Value::Str(_) if truthy(raw) => raw.clone(),
        _ if truthy(raw) => s(py_str(raw)),
        _ => s(""),
    }
}

fn sat_to_btc(n: &BigInt) -> Result<String> {
    // Python int/int division rounds the exact rational to binary64; converting
    // the numerator to f64 first loses a different bit for large integers.
    let f = format!("{}e-8", integer_text(n)?)
        .parse::<f64>()
        .map_err(|_| AppError::new("internal_error", "invalid Lightning amount"))?;
    if !f.is_finite() {
        return Err(AppError::new(
            "internal_error",
            "integer division result too large for a float",
        ));
    }
    Ok(format!("{f:.8}"))
}

fn concat(parts: &[PyStr]) -> PyStr {
    PyStr::from_code_points(parts.iter().flat_map(PyStr::code_points))
        .expect("existing code points")
}

fn csv_line(row: &[PyStr]) -> PyStr {
    let mut points = Vec::new();
    for (i, cell) in row.iter().enumerate() {
        if i != 0 {
            points.push(44);
        }
        let quoted = cell.code_points().any(|c| matches!(c, 10 | 13 | 34 | 44));
        if quoted {
            points.push(34);
        }
        for point in cell.code_points() {
            points.push(point);
            if quoted && point == 34 {
                points.push(34);
            }
        }
        if quoted {
            points.push(34);
        }
    }
    points.extend([13, 10]);
    PyStr::from_code_points(points).expect("existing code points")
}

fn utf8_error(line: &PyStr) -> AppError {
    let points: Vec<_> = line.code_points().collect();
    let start = points
        .iter()
        .position(|c| (0xd800..=0xdfff).contains(c))
        .expect("surrogate");
    let end = start
        + points[start..]
            .iter()
            .take_while(|c| (0xd800..=0xdfff).contains(*c))
            .count();
    let message = if end == start + 1 {
        format!("'utf-8' codec can't encode character '\\u{:04x}' in position {start}: surrogates not allowed", points[start])
    } else {
        format!(
            "'utf-8' codec can't encode characters in position {start}-{}: surrogates not allowed",
            end - 1
        )
    };
    AppError::new("internal_error", message)
}
