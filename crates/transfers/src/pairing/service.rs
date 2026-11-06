use super::*;
use kassiber_store::rusqlite::params;
use matcher::sort;

#[derive(Default, Clone)]
pub struct PairingOptions {
    pub matching: MatchOptions,
    pub confidence: Option<String>,
    pub asset_pair: Option<String>,
    pub route_pair: Option<String>,
    pub method: Option<String>,
    pub candidate_type: Option<String>,
    /// Automatic journal runs preserve explicit unpair decisions.
    pub skip_user_unpaired: bool,
    pub authored_source: Option<String>,
    /// Uncoerced transport filters retain Python's non-text split refusals.
    pub raw_pair_filters: Option<Value>,
}
#[derive(Default, Clone)]
pub struct ProjectionGuards {
    pub booked_move_transaction_ids: HashSet<String>,
    pub ownership_candidates: Vec<Candidate>,
}
impl ProjectionGuards {
    pub fn load(c: &Connection, scope: &Scope<'_>) -> Result<Self> {
        let profile=rows(c,"SELECT p.*, (SELECT COUNT(*) FROM transactions WHERE profile_id=p.id AND excluded=0) AS active_count FROM profiles p WHERE p.id=?",[scope.profile_id])?.into_iter().next().ok_or_else(||AppError::new("not_found","Review profile was not found"))?;
        if !kassiber_store::semantics::journal_current(c, scope.profile_id)
            .map_err(|e| AppError::new("internal_error", e.to_string()))?
            || !truthy(&value(&profile, "last_processed_at"))
            || n(&value(&profile, "last_processed_tx_count"))?
                != n(&value(&profile, "active_count"))?
            || n(&value(&profile, "journal_input_version"))?
                != n(&value(&profile, "last_processed_input_version"))?
        {
            return Ok(Self::default());
        }
        let booked_move_transaction_ids=rows(c,"SELECT source_transaction_id,target_transaction_id FROM journal_custody_decisions WHERE profile_id=? AND state IN ('internal_verified','internal_reviewed') AND target_transaction_id IS NOT NULL",[scope.profile_id])?.iter().flat_map(|r|[st(r,"source_transaction_id"),st(r,"target_transaction_id")]).filter(|v|!v.is_empty()).collect();
        let mut ownership_candidates = Vec::new();
        for row in rows(
            c,
            "SELECT detail_json FROM journal_quarantines WHERE profile_id=?",
            [scope.profile_id],
        )? {
            let Ok(detail) = parse_json(&st(&row, "detail_json")) else {
                continue;
            };
            for item in list(&value(&detail, "ownership_review_candidates")) {
                const REQUIRED: &[&str] = &[
                    "out_id",
                    "in_id",
                    "out_asset",
                    "in_asset",
                    "out_amount_msat",
                    "in_amount_msat",
                    "out_wallet_id",
                    "in_wallet_id",
                    "out_wallet_label",
                    "in_wallet_label",
                    "out_wallet_kind",
                    "in_wallet_kind",
                    "out_occurred_at",
                    "in_occurred_at",
                    "confidence",
                    "method",
                    "swap_fee_msat",
                    "swap_fee_kind",
                    "default_kind",
                    "default_policy",
                ];
                if !matches!(item, Value::Dict(_))
                    || REQUIRED.iter().any(|k| get(item, k).is_none())
                {
                    continue;
                }
                let mut data = project(item, REQUIRED);
                for (k, default) in [
                    ("conflict_set_id", s("")),
                    ("conflict_size", count(1)),
                    ("evidence_provider", s("")),
                    ("evidence_id", s("")),
                    ("evidence_kind", s("")),
                    ("evidence_status", s("")),
                    ("evidence_version", s("")),
                    ("evidence_taproot", s("")),
                    ("evidence_cooperative", s("")),
                    ("evidence_spend_path", s("")),
                    ("evidence_conflicts", Value::List(vec![])),
                    ("evidence_send_amount_msat", Value::Null),
                    ("evidence_receive_amount_msat", Value::Null),
                ] {
                    set(&mut data, k, get(item, k).cloned().unwrap_or(default));
                }
                ownership_candidates.push(Candidate {
                    data: std::sync::Arc::new(data),
                });
            }
        }
        Ok(Self {
            booked_move_transaction_ids,
            ownership_candidates,
        })
    }
}
fn load(c: &Connection, scope: &Scope<'_>) -> Result<Population> {
    Population::from_rows(rows(c,"SELECT t.id,t.profile_id,t.wallet_id,t.external_id,t.external_id_kind,t.payment_hash,t.payment_hash_source,t.swap_refund_funding_txid,t.swap_refund_funding_vout,t.occurred_at,t.direction,t.asset,t.amount,t.amount_includes_fee,t.fee,t.kind,t.raw_json,t.excluded,w.label AS wallet_label,w.kind AS wallet_kind,w.config_json AS config_json,observation.authority_version AS observation_authority_version,observation.observer_kinds_json AS observation_observer_kinds_json,observation.graph_hash AS observation_graph_hash,observation.quantity_hash AS observation_quantity_hash FROM transactions t JOIN wallets w ON w.id=t.wallet_id LEFT JOIN chain_observation_provenance observation ON observation.transaction_id=t.id WHERE t.profile_id=?",[scope.profile_id])?)
}
fn candidates(
    c: &Connection,
    scope: &Scope<'_>,
    pop: &Population,
    o: &PairingOptions,
    guards: &ProjectionGuards,
) -> Result<Vec<Candidate>> {
    let refs = pair_authoring::active_refs(c, scope)?;
    let dismissals=rows(c,"SELECT out_transaction_id,in_transaction_id,expires_at FROM transaction_pair_dismissals WHERE profile_id=?",[scope.profile_id])?;
    let raw = suggest(
        pop,
        &refs,
        &dismissals,
        &guards.booked_move_transaction_ids,
        &o.matching,
    )?;
    let ownership_keys: HashSet<_> = guards
        .ownership_candidates
        .iter()
        .map(Candidate::edge)
        .collect();
    let mut combined: Vec<_> = raw
        .into_iter()
        .filter(|c| !ownership_keys.contains(&c.edge()))
        .collect();
    combined.extend(guards.ownership_candidates.clone());
    let mut result = if guards.ownership_candidates.is_empty() {
        combined
    } else {
        finalize(combined)
    };
    let mut policies = HashMap::new();
    for candidate in &mut result {
        let pair = (candidate.text("out_asset"), candidate.text("in_asset"));
        let policy = if let Some(policy) = policies.get(&pair) {
            *policy
        } else {
            let policy = rules::recommended_policy(
                c,
                scope,
                &dict([("out_asset", s(&pair.0)), ("in_asset", s(&pair.1))]),
            )?;
            policies.insert(pair, policy);
            policy
        };
        candidate.set("default_policy", s(policy));
    }
    Ok(result)
}
fn transfer(c: &Candidate) -> bool {
    kassiber_pytext::upper(c.str("out_asset")) == kassiber_pytext::upper(c.str("in_asset"))
        || [
            "chain-swap",
            "peg-in",
            "peg-out",
            "submarine-swap",
            "reverse-submarine-swap",
            "swap-refund",
        ]
        .contains(&c.str("default_kind"))
}
fn route(asset: &str, kind: &str) -> String {
    let a = kassiber_pytext::upper(asset);
    let k = kassiber_pytext::lower(kind);
    if a == "LBTC" || k.contains("liquid") {
        "LBTC".into()
    } else if a == "BTC" && lightning(&k) {
        "LNBTC".into()
    } else {
        a
    }
}
fn filter(
    mut candidates: Vec<Candidate>,
    o: &PairingOptions,
    confidence: bool,
) -> Result<Vec<Candidate>> {
    if let Some(t) = o.candidate_type.as_ref().filter(|v| !v.is_empty()) {
        if !["transfer", "swap"].contains(&t.as_str()) {
            return Err(AppError::new(
                "validation",
                format!("Invalid candidate_type '{t}', expected 'transfer' or 'swap'"),
            ));
        }
        candidates.retain(|c| transfer(c) == (t == "transfer"));
    }
    if confidence {
        if let Some(v) = o.confidence.as_ref().filter(|v| !v.is_empty()) {
            candidates.retain(|c| c.str("confidence") == *v);
        }
    }
    if let Some(v) = o.method.as_ref().filter(|v| !v.is_empty()) {
        candidates.retain(|c| c.str("method") == *v);
    }
    for (name, selected) in [("asset_pair", &o.asset_pair), ("route_pair", &o.route_pair)] {
        if let Some(v) = selected.as_ref().filter(|v| !v.is_empty()) {
            if let Some(raw) = o.raw_pair_filters.as_ref().and_then(|args| get(args, name)) {
                if !matches!(raw, Value::Str(_)) {
                    return Err(AppError::new(
                        "internal_error",
                        format!(
                            "'{}' object has no attribute 'split'",
                            kassiber_envelope::type_name(raw)
                        ),
                    ));
                }
            }
            let Some((out, input)) = v.split_once('-') else {
                return Err(AppError::new(
                    "validation",
                    format!(
                        "Invalid {name} '{v}', expected OUT-IN like '{}'",
                        if name == "asset_pair" {
                            "LBTC-BTC"
                        } else {
                            "LNBTC-BTC"
                        }
                    ),
                ));
            };
            candidates.retain(|c| {
                let a = c.str("out_asset");
                let b = c.str("in_asset");
                if name == "asset_pair" {
                    a == out && b == input
                } else {
                    route(a, &c.text("out_wallet_kind")) == out
                        && route(b, &c.text("in_wallet_kind")) == input
                }
            });
        }
    }
    Ok(candidates)
}
// Python int/int rounds the exact ratio once. Casting each input to binary64
// first can cross an inclusive rule cap above 2^53 msat. Every i128 ratio is
// normal and finite, so round its 53-bit significand with an exact remainder.
fn fee_ratio(fee: i128, amount: i128) -> f64 {
    if fee == 0 {
        return 0.0;
    }
    let mut numerator = BigInt::from(fee.unsigned_abs());
    let mut denominator = BigInt::from(amount);
    let mut exponent = numerator.bits() as i32 - denominator.bits() as i32;
    let below = if exponent >= 0 {
        numerator < (&denominator << exponent as usize)
    } else {
        (&numerator << (-exponent) as usize) < denominator
    };
    exponent -= i32::from(below);
    let shift = 52 - exponent;
    if shift >= 0 {
        numerator <<= shift as usize;
    } else {
        denominator <<= (-shift) as usize;
    }
    let mut significand = &numerator / &denominator;
    let twice_remainder = (numerator % &denominator) << 1;
    if twice_remainder > denominator || (twice_remainder == denominator && significand.bit(0)) {
        significand += 1;
    }
    significand.to_u64_digits().1[0] as f64 * 2.0_f64.powi(exponent - 52)
}
fn matching_rule<'a>(candidate: &Candidate, rules: &'a [Value]) -> Option<&'a Value> {
    if candidate.conflict_size() > 1 || candidate.str("method") == "ownership_graph" {
        return None;
    }
    rules.iter().find(|rule| {
        if !truthy(&value(rule, "enabled")) {
            return false;
        }
        let predicate = value(rule, "predicate");
        let mut minimum = st(&predicate, "min_confidence");
        if minimum.is_empty() {
            minimum = "strong".into();
        }
        if !(candidate.str("confidence") == minimum || (minimum == "strong" && candidate.exact())) {
            return false;
        }
        let Value::Dict(fields) = predicate else {
            return false;
        };
        fields.iter().all(|(key, v)| {
            let kassiber_pyjson::Key::Str(key) = key else {
                return false;
            };
            let Some(key) = key.as_str() else {
                return false;
            };
            if matches!(v, Value::Null) || *v == s("") {
                return true;
            }
            match key {
                "min_confidence" => true,
                "max_fee_pct" => {
                    let expected = py_str(v).parse::<f64>().unwrap_or(f64::NAN);
                    let amount = n(&value(&candidate.data, "out_amount_msat")).unwrap_or(0);
                    amount > 0
                        && expected.is_finite()
                        && expected >= 0.0
                        && fee_ratio(candidate.fee(), amount) <= expected
                }
                _ => value(&candidate.data, key) == *v,
            }
        })
    })
}
fn load_rules(c: &Connection, scope: &Scope<'_>) -> Result<Vec<Value>> {
    let mut rules = list(&rules::list(c, scope)?).to_vec();
    let specificity = |r: &Value| {
        let p = value(r, "predicate");
        if let Value::Dict(fields) = p {
            fields
                .iter()
                .filter(|(_, v)| !matches!(v, Value::Null) && *v != s(""))
                .count()
        } else {
            0
        }
    };
    rules.sort_by_key(|r| (std::cmp::Reverse(specificity(r)), text(r, "id")));
    Ok(rules)
}
fn btc(n: i128) -> Value {
    let abs = n.unsigned_abs();
    let v = format!(
        "{}{}.{:011}",
        if n < 0 { "-" } else { "" },
        abs / 100_000_000_000,
        abs % 100_000_000_000
    );
    Value::Float(v.parse().unwrap())
}
fn project_candidate(c: &Candidate) -> Value {
    let mut out = dict::<&str>([]);
    for k in [
        "out_id",
        "in_id",
        "out_asset",
        "in_asset",
        "out_amount_msat",
        "out_amount",
        "in_amount_msat",
        "in_amount",
        "out_wallet_id",
        "in_wallet_id",
        "out_wallet_label",
        "in_wallet_label",
        "out_wallet_kind",
        "in_wallet_kind",
        "out_occurred_at",
        "in_occurred_at",
        "confidence",
        "method",
        "swap_fee_msat",
        "swap_fee",
        "swap_fee_kind",
        "default_kind",
        "default_policy",
        "candidate_type",
        "conflict_set_id",
        "conflict_size",
    ] {
        let v = match k {
            "out_amount" => btc(n(&value(&c.data, "out_amount_msat")).unwrap()),
            "in_amount" => btc(n(&value(&c.data, "in_amount_msat")).unwrap()),
            "swap_fee" => btc(c.fee()),
            "candidate_type" => s(if transfer(c) { "transfer" } else { "swap" }),
            _ => value(&c.data, k),
        };
        set(&mut out, k, v);
    }
    if truthy(&value(&c.data, "evidence_provider")) || truthy(&value(&c.data, "evidence_id")) {
        set(
            &mut out,
            "evidence",
            dict(
                [
                    ("provider", "evidence_provider"),
                    ("id", "evidence_id"),
                    ("kind", "evidence_kind"),
                    ("status", "evidence_status"),
                    ("version", "evidence_version"),
                    ("taproot", "evidence_taproot"),
                    ("cooperative", "evidence_cooperative"),
                    ("spend_path", "evidence_spend_path"),
                    ("conflicts", "evidence_conflicts"),
                    ("send_amount_msat", "evidence_send_amount_msat"),
                    ("receive_amount_msat", "evidence_receive_amount_msat"),
                ]
                .map(|(to, from)| (to, value(&c.data, from))),
            ),
        );
    }
    out
}
fn counts(candidates: &[Candidate], rules: &[Value]) -> Value {
    dict([
        ("total", count(candidates.len())),
        (
            "exact",
            count(candidates.iter().filter(|c| c.exact()).count()),
        ),
        (
            "strong",
            count(
                candidates
                    .iter()
                    .filter(|c| c.str("confidence") == "strong")
                    .count(),
            ),
        ),
        (
            "conflicts",
            count(
                candidates
                    .iter()
                    .filter(|c| c.conflict_size() > 1)
                    .map(|c| c.text("conflict_set_id"))
                    .collect::<HashSet<_>>()
                    .len(),
            ),
        ),
        (
            "rule_matches",
            count(
                candidates
                    .iter()
                    .filter(|c| matching_rule(c, rules).is_some())
                    .count(),
            ),
        ),
        (
            "ownership",
            count(
                candidates
                    .iter()
                    .filter(|c| c.str("method") == "ownership_graph")
                    .count(),
            ),
        ),
    ])
}
fn suggestions_from(
    c: &Connection,
    scope: &Scope<'_>,
    pop: &Population,
    o: &PairingOptions,
) -> Result<Value> {
    let guards = ProjectionGuards::load(c, scope)?;
    let mut candidates = candidates(c, scope, pop, o, &guards)?;
    sort(&mut candidates, true);
    let candidates = filter(candidates, o, true)?;
    let rules = load_rules(c, scope)?;
    let result = candidates
        .iter()
        .map(|c| {
            let mut data = project_candidate(c);
            if let Some(rule) = matching_rule(c, &rules) {
                set(
                    &mut data,
                    "rule_match",
                    dict([
                        ("rule_id", value(rule, "id")),
                        ("rule_name", value(rule, "name")),
                        ("kind", value(rule, "kind")),
                        ("policy", value(rule, "policy")),
                    ]),
                );
            }
            data
        })
        .collect();
    // Profile-wide and before the filters: what the matcher skipped, so an
    // empty or short candidate list can be told apart from "nothing to pair".
    let refs = pair_authoring::active_refs(c, scope)?;
    let dismissals=rows(c,"SELECT out_transaction_id,in_transaction_id,expires_at FROM transaction_pair_dismissals WHERE profile_id=?",[scope.profile_id])?;
    let exclusions = matcher::exclusion_counts(
        pop,
        &refs,
        &dismissals,
        &guards.booked_move_transaction_ids,
        &o.matching.now,
    );
    Ok(dict([
        ("candidates", Value::List(result)),
        ("counts", counts(&candidates, &rules)),
        ("exclusions", exclusions),
    ]))
}
fn suggestion_counts(
    c: &Connection,
    scope: &Scope<'_>,
    pop: &Population,
    o: &PairingOptions,
) -> Result<Value> {
    let candidates = candidates(c, scope, pop, o, &ProjectionGuards::load(c, scope)?)?;
    Ok(counts(&candidates, &load_rules(c, scope)?))
}
pub fn suggestions(c: &Connection, scope: &Scope<'_>, o: &PairingOptions) -> Result<Value> {
    suggestions_from(c, scope, &load(c, scope)?, o)
}
fn unpaired(c: &Connection, scope: &Scope<'_>) -> Result<HashSet<(String, String)>> {
    let records=rows(c,"SELECT term.legacy_source_id AS id,c.superseded_at AS deleted_at,COALESCE(source.anchor_transaction_id,source.transaction_id) AS out_transaction_id,COALESCE(target.anchor_transaction_id,target.transaction_id) AS in_transaction_id FROM custody_component_economic_terms term JOIN custody_components c ON c.id=term.component_id JOIN custody_component_legs source ON source.id=term.source_leg_id JOIN custody_component_legs target ON target.id=term.target_leg_id WHERE term.profile_id=? AND term.term_kind='transaction_pair' AND (term.component_id IN (SELECT DISTINCT component_id FROM custody_component_transaction_memberships WHERE profile_id=?) OR (c.state='superseded' AND NOT EXISTS (SELECT 1 FROM custody_components successor WHERE successor.supersedes_component_id=c.id))) ORDER BY term.created_at DESC,term.component_id,term.ordinal",[scope.profile_id,scope.profile_id])?;
    let mut by_id = BTreeMap::new();
    for r in records {
        by_id.insert(text(&r, "id"), r);
    }
    let records: Vec<_> = by_id.into_values().collect();
    let active: HashSet<_> = records
        .iter()
        .filter(|r| !truthy(&value(r, "deleted_at")))
        .map(|r| (text(r, "out_transaction_id"), text(r, "in_transaction_id")))
        .collect();
    Ok(records
        .iter()
        .filter(|r| truthy(&value(r, "deleted_at")))
        .map(|r| (text(r, "out_transaction_id"), text(r, "in_transaction_id")))
        .filter(|key| !active.contains(key))
        .collect())
}
fn create(
    c: &Connection,
    scope: &Scope<'_>,
    candidate: &Candidate,
    rule: Option<&Value>,
    o: &PairingOptions,
    source: &str,
    sources: &dyn Sources,
) -> Result<Value> {
    let args = dict([
        ("tx_out", value(&candidate.data, "out_id")),
        ("tx_in", value(&candidate.data, "in_id")),
        (
            "kind",
            rule.map(|r| value(r, "kind"))
                .unwrap_or_else(|| value(&candidate.data, "default_kind")),
        ),
        (
            "policy",
            rule.map(|r| value(r, "policy"))
                .unwrap_or_else(|| value(&candidate.data, "default_policy")),
        ),
        ("pair_source", s(source)),
        ("confidence_at_pair", value(&candidate.data, "confidence")),
        (
            "authored_source",
            s(o.authored_source.as_deref().unwrap_or("cli")),
        ),
    ]);
    pair_authoring::execute(c, scope, "transfers.pair", &args, sources)
}
fn mutate(
    c: &Connection,
    scope: &Scope<'_>,
    pop: &Population,
    o: &PairingOptions,
    guards: &ProjectionGuards,
    rules_pass: bool,
    sources: &dyn Sources,
) -> Result<Value> {
    let all = candidates(c, scope, pop, o, guards)?;
    let confidence = o.confidence.as_deref().unwrap_or("exact");
    if !rules_pass && !["exact", "strong"].contains(&confidence) {
        return Err(AppError::new(
            "validation",
            format!("Unsupported confidence '{confidence}'. Use 'exact' or 'strong'."),
        ));
    }
    let mut candidates = filter(all, o, rules_pass)?;
    if rules_pass {
        candidates.retain(|c| c.str("method") != "ownership_graph");
    }
    if o.skip_user_unpaired {
        let unpaired = unpaired(c, scope)?;
        candidates.retain(|c| !unpaired.contains(&c.edge()));
    }
    let rules = if rules_pass {
        load_rules(c, scope)?
    } else {
        Vec::new()
    };
    let mut applied = Vec::new();
    let mut remaining = 0;
    for candidate in &candidates {
        let rule = if rules_pass {
            let rule = matching_rule(candidate, &rules);
            if rule.is_none() {
                remaining += 1;
                continue;
            }
            rule
        } else {
            if candidate.conflict_size() > 1
                || candidate.str("method") == "ownership_graph"
                || (confidence == "exact" && !candidate.exact())
            {
                continue;
            }
            None
        };
        applied.push(create(
            c,
            scope,
            candidate,
            rule,
            o,
            if rules_pass {
                "rule_auto"
            } else if confidence == "exact" {
                "bulk_exact"
            } else {
                "bulk_selected"
            },
            sources,
        )?);
    }
    let mut summary = dict([("count", count(applied.len()))]);
    if rules_pass {
        set(&mut summary, "remaining", count(remaining));
    } else {
        set(
            &mut summary,
            "skipped_conflicts",
            count(candidates.iter().filter(|c| c.conflict_size() > 1).count()),
        );
    }
    let total: i128 = applied
        .iter()
        .map(|r| {
            n(&get(r, "swap_fee_msat")
                .filter(|v| truthy(v))
                .cloned()
                .unwrap_or(num(0)))
            .unwrap_or(0)
        })
        .sum();
    set(&mut summary, "total_swap_fee_msat", num(total));
    Ok(dict([
        ("applied", Value::List(applied)),
        ("summary", summary),
    ]))
}
fn pending<T>(c: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    if c.is_autocommit() {
        c.execute_batch("BEGIN").map_err(sql::error)?;
    }
    sql::atomic(c, f)
}
pub fn apply_rules(
    c: &Connection,
    scope: &Scope<'_>,
    o: &PairingOptions,
    guards: Option<&ProjectionGuards>,
    sources: &dyn Sources,
) -> Result<Value> {
    let pop = load(c, scope)?;
    let loaded;
    let guards = if let Some(g) = guards {
        g
    } else {
        loaded = ProjectionGuards::load(c, scope)?;
        &loaded
    };
    pending(c, || mutate(c, scope, &pop, o, guards, true, sources))
}
pub fn bulk_pair(
    c: &Connection,
    scope: &Scope<'_>,
    o: &PairingOptions,
    guards: Option<&ProjectionGuards>,
    sources: &dyn Sources,
) -> Result<Value> {
    let pop = load(c, scope)?;
    let loaded;
    let guards = if let Some(g) = guards {
        g
    } else {
        loaded = ProjectionGuards::load(c, scope)?;
        &loaded
    };
    pending(c, || mutate(c, scope, &pop, o, guards, false, sources))
}
/// R4-13 calls inside its BEGIN IMMEDIATE + journal savepoint. The caller owns
/// commit and rollback of pairing, candidate cache and the subsequent journal.
/// UUID and clock injection is explicit; product callers pass SystemSources.
pub fn auto_pair_before_journals(
    c: &Connection,
    scope: &Scope<'_>,
    sources: &dyn Sources,
) -> Result<Value> {
    pending(c, || {
        let pop = load(c, scope)?;
        let mut o = PairingOptions {
            skip_user_unpaired: true,
            ..PairingOptions::default()
        };
        o.matching.now = sources.now_precise(c)?;
        let before = suggestion_counts(c, scope, &pop, &o)?;
        let guards = ProjectionGuards::load(c, scope)?;
        o.matching.now = sources.now_precise(c)?;
        let rules = mutate(c, scope, &pop, &o, &guards, true, sources)?;
        o.matching.now = sources.now_precise(c)?;
        let bulk = mutate(c, scope, &pop, &o, &guards, false, sources)?;
        o.matching.now = sources.now_precise(c)?;
        let remaining = suggestion_counts(c, scope, &pop, &o)?;
        c.execute(
            "UPDATE profiles SET swap_candidate_count=? WHERE id=?",
            params![n(&value(&remaining, "total"))? as i64, scope.profile_id],
        )
        .map_err(sql::error)?;
        let a = value(&rules, "summary");
        let b = value(&bulk, "summary");
        // Freshness' public count projection omits the ownership subcount.
        let public_counts = |v: &Value| {
            project(
                v,
                &["total", "exact", "strong", "conflicts", "rule_matches"],
            )
        };
        Ok(dict([
            ("enabled", Value::Bool(true)),
            (
                "applied",
                num(n(&value(&a, "count"))? + n(&value(&b, "count"))?),
            ),
            ("rules_applied", value(&a, "count")),
            ("bulk_exact_applied", value(&b, "count")),
            ("skipped_conflicts", value(&b, "skipped_conflicts")),
            (
                "total_swap_fee_msat",
                num(n(&value(&a, "total_swap_fee_msat"))? + n(&value(&b, "total_swap_fee_msat"))?),
            ),
            ("before", public_counts(&before)),
            ("remaining", public_counts(&remaining)),
        ]))
    })
}

pub use crate::sql::CapturedError as AutomaticPairingError;

/// Preserve ordinary SQLite failures separately from domain AppErrors so the
/// refresh coordinator can reproduce Python's skipped-pairing summary exactly.
/// This wrapper has the same pending-write and rollback contract as the entry.
pub fn auto_pair_before_journals_captured(
    c: &Connection,
    scope: &Scope<'_>,
    sources: &dyn Sources,
) -> std::result::Result<Value, AutomaticPairingError> {
    crate::sql::capture_sqlite(|| Ok(auto_pair_before_journals(c, scope, sources)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pairing_rule_fee_cap_rounds_the_integer_ratio_once() {
        // CPython's exact integer quotient is one ULP below dividing two
        // independently rounded floats on this valid signed-64 input.
        let amount = 1_511_910_649_648_900_384;
        let fee = 1_886_523_809_418_448;
        let cap = 0.0012477746683354212_f64;
        assert!(fee as f64 / amount as f64 > cap);
        assert_eq!(fee_ratio(fee, amount), cap);
        assert_eq!(fee_ratio(-fee, amount), cap);
        assert_eq!(fee_ratio(0, amount), 0.0);
        let candidate = Candidate {
            data: std::sync::Arc::new(dict([
                ("out_amount_msat", num(amount)),
                ("swap_fee_msat", num(fee)),
                ("confidence", s("strong")),
            ])),
        };
        let rule = |cap| {
            dict([
                ("enabled", Value::Bool(true)),
                ("predicate", dict([("max_fee_pct", Value::Float(cap))])),
            ])
        };
        assert!(matching_rule(&candidate, &[rule(cap)]).is_some());
        assert!(matching_rule(&candidate, &[rule(f64::from_bits(cap.to_bits() - 1))]).is_none());
        // Exact halfway quotients round to the even significand.
        assert_eq!(fee_ratio((1_i128 << 53) + 1, 1_i128 << 53), 1.0);
        assert_eq!(
            fee_ratio((1_i128 << 53) + 3, 1_i128 << 53),
            f64::from_bits(1.0_f64.to_bits() + 2)
        );
    }
    #[test]
    fn pairing_view_classification_preserves_literal_asset_codes() {
        let candidate = |out: &str, input: &str| Candidate {
            data: std::sync::Arc::new(dict([
                ("out_asset", s(out)),
                ("in_asset", s(input)),
                ("default_kind", s("manual")),
            ])),
        };
        assert!(transfer(&candidate("BTC", "btc")));
        assert!(!transfer(&candidate(" BTC ", "BTC")));
        assert!(!transfer(&candidate("XBT", "BTC")));
    }
}
