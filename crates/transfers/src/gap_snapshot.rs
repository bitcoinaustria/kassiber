//! Advisory gap pages and historical review displays. No ledger recomputation.
use super::component_rows as cr;
use super::*;
use kassiber_custody::{codec, gaps};
use kassiber_envelope::{s, set, truthy};
use kassiber_pyjson::BigInt;
use std::collections::{BTreeMap, BTreeSet};

fn internal(message: impl Into<String>) -> AppError {
    AppError::new("internal_error", message)
}
fn list<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    if let Some(Value::List(a)) = get(v, k) {
        a
    } else {
        &[]
    }
}
fn number(v: &Value, k: &str) -> Result<BigInt> {
    integer(get(v, k).filter(|v| truthy(v)).unwrap_or(&0.into()))
}
fn latest(c: &Connection, scope: &Scope<'_>) -> Result<BTreeMap<String, Value>> {
    let rows=rows(c,"SELECT r.* FROM custody_gap_reviews r JOIN (SELECT gap_id,MAX(revision) AS revision FROM custody_gap_reviews WHERE profile_id=? GROUP BY gap_id) latest ON latest.gap_id=r.gap_id AND latest.revision=r.revision WHERE r.profile_id=? ORDER BY r.gap_id,r.created_at,r.id",[scope.profile_id,scope.profile_id])?;
    let mut result: BTreeMap<String, Value> = BTreeMap::new();
    let signature = |v: &Value| {
        Value::List(vec![
            value(v, "candidate_fingerprint"),
            value(v, "action"),
            get(v, "event_kind")
                .filter(|v| truthy(v))
                .cloned()
                .unwrap_or_else(|| s("review_decision")),
            value(v, "component_id"),
        ])
    };
    let mut conflicts = BTreeSet::new();
    for row in rows {
        let id = text(&row, "gap_id");
        if result
            .get(&id)
            .is_some_and(|old| signature(old) != signature(&row))
        {
            conflicts.insert(id.clone());
        }
        result.insert(id, row);
    }
    for id in conflicts {
        set(
            result.get_mut(&id).unwrap(),
            "action",
            s("conflicting_review"),
        );
    }
    Ok(result)
}
fn component_status(c: &Connection, scope: &Scope<'_>, id: &str) -> (bool, String, String) {
    let Ok(component) = cr::get_component(c, scope, id, false) else {
        return (false, "component_missing".into(), String::new());
    };
    let native = get(&component, "native_support_status")
        .map(|v| text(v, "status"))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unverified".into());
    if text(&component, "effective_state") != "active" {
        return (
            false,
            if native == "contradicted" {
                "component_native_support_contradicted"
            } else {
                "component_not_effective"
            }
            .into(),
            native,
        );
    }
    let evidence = value(&component, "evidence_status");
    if !get(&evidence, "valid").is_some_and(truthy) {
        return (
            false,
            get(&evidence, "status")
                .filter(|v| truthy(v))
                .map(py_str)
                .unwrap_or_else(|| "component_evidence_unusable".into()),
            native,
        );
    }
    (true, String::new(), native)
}
fn state(
    c: &Connection,
    scope: &Scope<'_>,
    candidate: &Value,
    review: Option<&Value>,
) -> Result<(String, Option<String>, Option<String>)> {
    let conflict = number(candidate, "conflict_size")? > 1.into();
    let fallback = || {
        (
            if conflict {
                "conflicting"
            } else {
                "needs_review"
            }
            .into(),
            conflict.then(|| "competing_candidates".into()),
            None,
        )
    };
    let Some(r) = review else {
        return Ok(fallback());
    };
    if text(r, "action") == "conflicting_review" {
        return Ok((
            "conflicting".into(),
            Some("concurrent_review_conflict".into()),
            None,
        ));
    }
    if text(r, "event_kind") == "bridge_reopened" {
        return Ok(("needs_review".into(), Some("bridge_reopened".into()), None));
    }
    let resolved = text(r, "action") == "resolved";
    let expected = if resolved {
        gaps::authored_claim_fingerprint(&codec::to_json(candidate))
    } else {
        gaps::candidate_fingerprint(&codec::to_json(candidate))
    }
    .map_err(internal)?;
    let matches = text(r, "candidate_fingerprint") == expected;
    if !matches && !resolved {
        return Ok(fallback());
    }
    if resolved {
        let (usable, reason, native) = component_status(c, scope, &text(r, "component_id"));
        let native = if native.is_empty() {
            "unverified".to_owned()
        } else {
            native
        };
        if matches || (usable && ["partial", "corroborated"].contains(&native.as_str())) {
            return Ok((
                if usable { "resolved" } else { "conflicting" }.into(),
                (!usable).then_some(reason),
                Some(native),
            ));
        }
        return Ok((
            "conflicting".into(),
            Some(if usable {
                "candidate_evidence_drift".into()
            } else {
                reason
            }),
            Some(native),
        ));
    }
    if text(r, "action") == "dismissed" {
        return Ok(("dismissed".into(), None, None));
    }
    Ok(fallback())
}
fn correction(review: &Value) -> Value {
    dict([
        ("component_id", s(text(review, "component_id"))),
        ("strategy", s("create_revision_then_activate")),
    ])
}
fn historical(c: &Connection, scope: &Scope<'_>, review: &Value) -> Option<Value> {
    let mut snapshot = parse_json(&text(review, "snapshot_json"))
        .ok()
        .filter(|v| matches!(v, Value::Dict(_)))?;
    let action = text(review, "action");
    if action == "conflicting_review" {
        set(&mut snapshot, "status", s("conflicting"));
        set(
            &mut snapshot,
            "status_reason",
            s("concurrent_review_conflict"),
        );
    } else if text(review, "event_kind") == "bridge_reopened" {
        set(&mut snapshot, "status", s("needs_review"));
        set(&mut snapshot, "status_reason", s("bridge_reopened"));
        set(&mut snapshot, "correction", correction(review));
    } else if action == "resolved" {
        let (usable, reason, native) = component_status(c, scope, &text(review, "component_id"));
        set(
            &mut snapshot,
            "status",
            s(if usable { "resolved" } else { "conflicting" }),
        );
        if !usable {
            set(&mut snapshot, "status_reason", s(reason));
        }
        set(
            &mut snapshot,
            "native_support_status",
            s(if native.is_empty() {
                "unverified".into()
            } else {
                native
            }),
        );
        set(&mut snapshot, "correction", correction(review));
    } else if action == "dismissed" {
        set(&mut snapshot, "status", s("dismissed"));
    } else {
        return None;
    }
    Some(snapshot)
}
const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
fn encode_cursor(version: &Value, ordinal: usize, id: &Value) -> Result<Value> {
    let json = kassiber_pyjson::dumps(
        &Value::List(vec![version.clone(), ordinal.into(), id.clone()]),
        &kassiber_pyjson::Options {
            separators: Some((",".into(), ":".into())),
            ..Default::default()
        },
    )
    .map_err(|e| internal(e.to_string()))?;
    let mut encoded = String::from("cgr3.");
    for chunk in json.as_bytes().chunks(3) {
        let bits = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for shift in (0..chunk.len() + 1).map(|i| 18 - i * 6) {
            encoded.push(ALPHABET[((bits >> shift) & 63) as usize] as char);
        }
    }
    Ok(s(encoded))
}
fn unbase64(raw: &str) -> Option<Vec<u8>> {
    if !raw.is_ascii() {
        return None;
    }
    let mut out = Vec::new();
    let mut bits = 0u32;
    let mut count = 0;
    let mut padding = 0;
    for byte in raw
        .bytes()
        .chain(std::iter::repeat_n(b'=', (4 - raw.len() % 4) % 4))
    {
        if byte == b'=' {
            if count >= 2 {
                padding += 1;
                if count + padding >= 4 {
                    return Some(out);
                }
            }
            continue;
        }
        let normalized = match byte {
            b'+' => b'-',
            b'/' => b'_',
            b => b,
        };
        let Some(n) = ALPHABET.iter().position(|b| *b == normalized) else {
            continue;
        };
        padding = 0;
        bits = (bits << 6) | n as u32;
        count += 1;
        if count >= 2 {
            out.push((bits >> (6 * count - 8 * (count - 1))) as u8);
        }
        if count == 4 {
            count = 0;
            bits = 0;
        }
    }
    (count == 0).then_some(out)
}
fn decode_cursor(raw: &str, version: &Value) -> Result<(BigInt, Value)> {
    let malformed = || internal("cursor is malformed");
    let decoded = raw
        .strip_prefix("cgr3.")
        .and_then(unbase64)
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| parse_json(&s).ok())
        .ok_or_else(malformed)?;
    let Value::List(parts) = decoded else {
        return Err(malformed());
    };
    if parts.len() != 3 {
        return Err(malformed());
    }
    let Value::List(v) = &parts[0] else {
        return Err(malformed());
    };
    if v.len() != 2
        || v.iter().any(|v| !matches!(v,Value::Int(n)if n>=&0.into()))
        || !matches!(&parts[1],Value::Int(n)if n>=&0.into())
        || !matches!(&parts[2], Value::Str(_))
    {
        return Err(malformed());
    }
    if &parts[0] != version {
        return Err(internal("cursor expired because custody evidence changed"));
    }
    let Value::Int(n) = &parts[1] else {
        unreachable!()
    };
    Ok((n.clone(), parts[2].clone()))
}

pub(super) fn execute(
    c: &Connection,
    scope: &Scope<'_>,
    operation: &str,
    args: &Value,
) -> Result<Value> {
    let reviewing = operation == "transfers.gaps.review";
    let limit = if reviewing {
        1.into()
    } else {
        get(args, "limit").cloned().unwrap_or(100.into())
    };
    if !matches!(&limit,Value::Int(n)if n>=&1.into()&&n<=&500.into()) {
        return Err(internal("limit must be an integer between 1 and 500"));
    }
    let limit = usize::try_from(integer(&limit)?).unwrap();
    let cursor = if reviewing {
        None
    } else {
        get(args, "cursor").filter(|v| !matches!(v, Value::Null))
    };
    if cursor.is_some_and(|v| !matches!(v, Value::Str(_))) {
        return Err(internal("cursor must be a string"));
    }
    if matches!(cursor,Some(Value::Str(text)) if text.as_str().is_none()) {
        return Err(internal("cursor is malformed"));
    }
    let cursor = cursor.map(py_str).filter(|v| v != "0");
    let profile = super::gap_candidates::profile(c, scope)?;
    let version = Value::List(vec![
        value(&profile, "active_count"),
        Value::Int(number(&profile, "journal_input_version")?),
    ]);
    let after = cursor
        .as_deref()
        .map(|s| decode_cursor(s, &version))
        .transpose()?;
    let loaded = super::gap_candidates::load(c, scope)?;
    let reviews = latest(c, scope)?;
    let candidates = list(&loaded.result, "candidates");
    let mut current = Vec::new();
    let mut residuals: BTreeMap<(String, String), BigInt> = BTreeMap::new();
    for candidate in candidates {
        let raw = gaps::snapshot_gap(&codec::to_json(candidate), &loaded.display_rows)
            .map_err(internal)?;
        let converted = codec::from_json(&raw).map_err(internal)?;
        let mut gap = project(
            &converted,
            &[
                "gap_id",
                "status",
                "asset",
                "source_wallet_label",
                "destination_wallet_labels",
                "source_total_msat",
                "source_fee_msat",
                "source_debit_msat",
                "return_total_msat",
                "retained_msat",
                "residual_msat",
                "excess_msat",
                "started_at",
                "ended_at",
                "confidence",
                "reason_codes",
                "promotion_eligible",
                "competitor_score_margin",
                "downstream",
            ],
        );
        // The pure JSON helper uses sorted maps; the Python packet has explicit order.
        let downstream = project(
            &value(&gap, "downstream"),
            &["affected_disposals", "affected_years"],
        );
        set(&mut gap, "downstream", downstream);
        set(
            &mut gap,
            "candidate_fingerprint",
            s(gaps::candidate_fingerprint(&codec::to_json(candidate)).map_err(internal)?),
        );
        let review = reviews.get(&text(candidate, "gap_id"));
        let (status, reason, native) = state(c, scope, candidate, review)?;
        set(&mut gap, "status", s(status.as_str()));
        if let Some(reason) = reason {
            set(&mut gap, "status_reason", s(reason));
        }
        if let Some(native) = native {
            set(&mut gap, "native_support_status", s(native));
        }
        if let Some(r) = review.filter(|r| text(r, "action") == "resolved") {
            set(&mut gap, "correction", correction(r));
        }
        if ["needs_review", "conflicting"].contains(&status.as_str()) {
            let cluster = get(candidate, "conflict_set_id")
                .filter(|v| truthy(v))
                .map(py_str)
                .unwrap_or_else(|| text(candidate, "gap_id"));
            let amount = number(candidate, "residual_msat")?;
            let entry = residuals
                .entry((text(candidate, "asset"), cluster))
                .or_default();
            *entry = entry.clone().max(amount);
        }
        current.push(gap);
    }
    let mut by_asset: BTreeMap<String, BigInt> = BTreeMap::new();
    for ((asset, _), amount) in residuals {
        *by_asset.entry(asset).or_default() += amount;
    }
    let candidate_btc = by_asset.get("BTC").cloned().unwrap_or_default();
    let candidate_assets = Value::List(
        by_asset
            .into_iter()
            .map(|(asset, amount)| dict([("asset", s(asset)), ("amount_msat", Value::Int(amount))]))
            .collect(),
    );
    let issue_query="SELECT issue_id,issue_type,state,asset,amount_msat,occurred_at,reason,blocks_from FROM journal_quantity_issues WHERE profile_id=? ORDER BY COALESCE(occurred_at,''),issue_id";
    let issues = rows(c, issue_query, [scope.profile_id]).map_err(|_| {
        AppError::new(
            "custody_quantity_state_unavailable",
            "Custody quantity blocker state is unavailable.",
        )
        .with_hint(
            "Repair or migrate the local book, then rebuild journals before relying on reports.",
        )
        .with_details(dict([
            ("profile_id", s(scope.profile_id)),
            ("operation", s("read_journal_quantity_issues")),
            ("error_type", s("OperationalError")),
        ]))
    })?;
    let mut unresolved: BTreeMap<String, (BigInt, usize)> = BTreeMap::new();
    let mut quantified = 0;
    for issue in &issues {
        if !matches!(value(issue, "amount_msat"), Value::Null) {
            quantified += 1;
            let asset = get(issue, "asset")
                .filter(|v| truthy(v))
                .map(py_str)
                .unwrap_or_else(|| "UNKNOWN".into());
            let entry = unresolved.entry(asset).or_default();
            entry.0 += number(issue, "amount_msat")?;
            entry.1 += 1;
        }
    }
    let canonical_btc = unresolved
        .get("BTC")
        .map(|v| v.0.clone())
        .unwrap_or_default();
    let canonical_assets = Value::List(
        unresolved
            .into_iter()
            .map(|(asset, (amount, count))| {
                dict([
                    ("asset", s(asset)),
                    ("amount_msat", Value::Int(amount)),
                    ("issue_count", count.into()),
                ])
            })
            .collect(),
    );
    let (status, status_text) = match loaded.journal_status.as_str() {
        "current" if issues.is_empty() => ("no_known_custody_gaps", "No known custody gaps"),
        "current" => ("known_custody_gaps", "Known custody gaps require review"),
        "no_transactions" => (
            "not_evaluated",
            "Custody gaps not evaluated: no active transactions",
        ),
        _ => (
            "needs_processing",
            "Custody gap status needs journal processing",
        ),
    };
    let mut summary = dict([("total", candidates.len().into())]);
    for status in ["needs_review", "conflicting", "resolved", "dismissed"] {
        set(
            &mut summary,
            status,
            current
                .iter()
                .filter(|g| text(g, "status") == status)
                .count()
                .into(),
        );
    }
    let search_complete = get(&loaded.result, "search_complete").is_some_and(truthy);
    let search_count = if search_complete {
        candidates.len().into()
    } else {
        value(&loaded.result, "candidate_count")
    };
    let attributes = [
        (
            "unresolved_msat",
            Value::Int(if issues.is_empty() {
                candidate_btc.clone()
            } else {
                canonical_btc.clone()
            }),
        ),
        ("candidate_residual_msat", Value::Int(candidate_btc)),
        ("candidate_residual_by_asset", candidate_assets),
        ("canonical_unresolved_msat", Value::Int(canonical_btc)),
        ("canonical_issue_count", issues.len().into()),
        ("canonical_unresolved_by_asset", canonical_assets),
        (
            "canonical_unquantified_issue_count",
            (issues.len() - quantified).into(),
        ),
        ("canonical_status", s(status)),
        ("canonical_status_text", s(status_text)),
        (
            "derived_state_current",
            Value::Bool(loaded.journal_status == "current"),
        ),
        (
            "qualification",
            s(concat!(
                "This reports gaps detectable from current imported evidence; it ",
                "does not assert that every wallet was imported."
            )),
        ),
        ("search_complete", Value::Bool(search_complete)),
        (
            "search_status",
            s(if search_complete {
                "complete"
            } else {
                "capacity_limited"
            }),
        ),
        ("search_limit_kind", value(&loaded.result, "limit_kind")),
        ("search_candidate_count", search_count),
    ];
    for (key, v) in attributes {
        set(&mut summary, key, v);
    }
    if reviewing {
        let id = value(args, "gap_id");
        let stored_id = if let Value::Str(s) = &id {
            s.as_str()
        } else {
            None
        };
        let gap = current
            .into_iter()
            .find(|g| value(g, "gap_id") == id)
            .or_else(|| {
                stored_id
                    .and_then(|id| reviews.get(id))
                    .and_then(|r| historical(c, scope, r))
                    .filter(|g| value(g, "gap_id") == id)
            });
        return Ok(dict([
            ("summary", summary),
            ("gaps", Value::List(gap.into_iter().collect())),
            ("next_cursor", Value::Null),
        ]));
    }
    let start = if let Some((ordinal, id)) = after {
        let n = usize::try_from(ordinal)
            .ok()
            .filter(|&n| n < candidates.len() && value(&candidates[n], "gap_id") == id)
            .ok_or_else(|| internal("cursor expired because custody candidates changed"))?;
        n + 1
    } else {
        0
    };
    let page: Vec<_> = current.into_iter().skip(start).take(limit).collect();
    let next = if start + limit < candidates.len() && !page.is_empty() {
        encode_cursor(
            &version,
            start + page.len() - 1,
            &value(page.last().unwrap(), "gap_id"),
        )?
    } else {
        Value::Null
    };
    Ok(dict([
        ("summary", summary),
        ("gaps", Value::List(page)),
        ("next_cursor", next),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gap_display_cursor_codec_matches_python_compact_payload_and_refusals() {
        let version = Value::List(vec![3.into(), 0.into()]);
        let token = encode_cursor(&version, 0, &s("gap")).unwrap();
        assert_eq!(token, s("cgr3.W1szLDBdLDAsImdhcCJd"));
        assert_eq!(
            decode_cursor(&py_str(&token), &version).unwrap(),
            (0.into(), s("gap"))
        );
        let lone = kassiber_pytext::loads("\"gap\\ud800\"").unwrap();
        let token_with_lone = encode_cursor(&version, 0, &lone).unwrap();
        assert_eq!(
            decode_cursor(&py_str(&token_with_lone), &version).unwrap(),
            (0.into(), lone)
        );
        for token in [
            "",
            "0",
            "cgr3.",
            "cgr3.A",
            "cgr3.Zg",
            "cgr3.W10",
            "cgr3.bnVsbA",
        ] {
            assert_eq!(
                decode_cursor(token, &version).unwrap_err().message,
                "cursor is malformed"
            );
        }
        let changed = Value::List(vec![3.into(), 1.into()]);
        assert_eq!(
            decode_cursor(&py_str(&token), &changed)
                .unwrap_err()
                .message,
            "cursor expired because custody evidence changed"
        );
    }
}
