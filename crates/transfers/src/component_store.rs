//! Append-only aggregate persistence. All interpretation comes from custody.
use super::*;
use kassiber_custody::{
    codec,
    components::review,
    snapshot::{Component, Record, Snapshot},
};
use kassiber_envelope::{set, truthy};
use kassiber_pyjson::{dumps, Options};
use kassiber_store::rusqlite::{params, params_from_iter};
use sql::{bind, error};
use std::collections::BTreeSet;

pub(super) fn json(v: &Value) -> Result<String> {
    dumps(
        v,
        &Options {
            sort_keys: true,
            ..Options::default().compact()
        },
    )
    .map_err(|e| AppError::new("internal_error", e.to_string()))
}
pub(super) fn hash(v: &Value, ascii: bool) -> Result<String> {
    codec::hash_py(v, ascii)
        .map(|x| x.0)
        .map_err(|e| AppError::new("internal_error", e))
}
fn record(v: &Value) -> Result<Record> {
    match codec::to_json(v) {
        serde_json::Value::Object(m) => Ok(m.into()),
        _ => Err(AppError::new("internal_error", "expected record")),
    }
}
fn records(
    c: &Connection,
    q: &str,
    p: impl kassiber_store::rusqlite::Params,
) -> Result<Vec<Record>> {
    rows(c, q, p)?.iter().map(record).collect()
}
fn convert_error(e: kassiber_custody::components::Error) -> AppError {
    AppError::new(
        if matches!(
            e.code,
            "ValueError" | "TypeError" | "UnicodeEncodeError" | "KeyError" | "OverflowError"
        ) {
            "internal_error"
        } else {
            e.code
        },
        e.message,
    )
}
#[allow(clippy::field_reassign_with_default)] // StageSnapshot owns a private shared-row cache.
pub(super) fn snapshot(c: &Connection, scope: &Scope<'_>) -> Result<Snapshot> {
    let profile = records(c, "SELECT * FROM profiles WHERE id=?", [scope.profile_id])?.remove(0);
    let wallets = records(
        c,
        "SELECT *,kind AS wallet_kind FROM wallets WHERE profile_id=?",
        [scope.profile_id],
    )?;
    let transactions=records(c,"SELECT t.*, w.kind AS wallet_kind,w.config_json,observation.authority_version AS observation_authority_version,observation.graph_hash AS observation_graph_hash,observation.quantity_hash AS observation_quantity_hash,observation.fee_attribution AS observation_fee_attribution,observation.application_revision AS observation_application_revision FROM transactions t JOIN wallets w ON w.id=t.wallet_id LEFT JOIN chain_observation_provenance observation ON observation.transaction_id=t.id WHERE t.profile_id=? ORDER BY t.occurred_at,t.created_at,t.id",[scope.profile_id])?;
    let headers = records(
        c,
        "SELECT * FROM custody_components WHERE profile_id=?",
        [scope.profile_id],
    )?;
    let mut components = Vec::new();
    for h in &headers {
        let id = h["id"].as_str().unwrap();
        components.push(Component {header:h.clone(),legs:records(c,"SELECT * FROM custody_component_legs WHERE component_id=? ORDER BY ordinal,id",[id])?,allocations:records(c,"SELECT * FROM custody_component_allocations WHERE component_id=? ORDER BY ordinal,id",[id])?,economic_terms:records(c,"SELECT * FROM custody_component_economic_terms WHERE component_id=? ORDER BY ordinal,id",[id])?,commitments:records(c,"SELECT * FROM custody_component_evidence_commitments WHERE component_id=? ORDER BY ordinal,id",[id])?});
    }
    let anchors = transactions
        .iter()
        .map(|r| {
            (
                r["id"].as_str().unwrap().to_string(),
                serde_json::Value::Object(r.clone().into_map()),
            )
        })
        .collect();
    // Initialize through Default because the stage's shared row cache is private.
    let mut snapshot = Snapshot::default();
    snapshot.schema_version = 1;
    snapshot.operation = "custody".into();
    snapshot.as_of = "2025-01-01T00:00:00Z".into();
    snapshot.profile = profile;
    snapshot.wallets = wallets.clone();
    snapshot.transactions = transactions;
    snapshot.components = components;
    snapshot.anchors = anchors;
    snapshot.anchor_wallets = wallets;
    snapshot.revision_headers = headers;
    snapshot.channel_records = records(c,"SELECT r.txid,r.outpoint,r.tag,r.wallet_id,r.channel_id,r.amount_msat,r.raw_json,w.config_json,b.chain,b.network FROM lightning_node_records r JOIN wallets w ON w.id=r.wallet_id LEFT JOIN backends b ON b.name=r.backend_name WHERE r.profile_id=? AND r.record_type='channel'",[scope.profile_id])?;
    Ok(snapshot)
}
/// `iter_authored_active_components(include_local_evidence=False)`: every
/// authored-active component with its validation and effective state.
pub fn authored_active_components(
    c: &Connection,
    scope: &Scope<'_>,
) -> Result<Vec<serde_json::Value>> {
    let computed =
        kassiber_custody::components::compute(&snapshot(c, scope)?).map_err(convert_error)?;
    Ok(computed["components"]
        .as_array()
        .cloned()
        .unwrap_or_default())
}
pub(super) fn invalidate(c: &Connection, profile: &str) -> Result<()> {
    c.execute("UPDATE profiles SET last_processed_at=NULL,last_processed_tx_count=0,journal_input_version=journal_input_version+1,ownership_review_counts_json=NULL WHERE id=?",[profile]).map_err(error)?;
    Ok(())
}
pub(super) fn insert(c: &Connection, table: &str, row: &Value) -> Result<()> {
    let Value::Dict(fields) = row else {
        unreachable!()
    };
    let names: Vec<_> = fields
        .iter()
        .map(|(k, _)| {
            if let kassiber_pyjson::Key::Str(k) = k {
                k.as_str().unwrap()
            } else {
                unreachable!()
            }
        })
        .collect();
    let bound = fields
        .iter()
        .enumerate()
        .map(|(i, (_, v))| bind(v, i + 1))
        .collect::<Result<Vec<_>>>()?;
    c.execute(
        &format!(
            "INSERT INTO {table}({}) VALUES({})",
            names.join(","),
            vec!["?"; names.len()].join(",")
        ),
        params_from_iter(bound),
    )
    .map_err(error)?;
    Ok(())
}
pub(super) fn optional(v: &Value, field: &str) -> Result<Value> {
    if matches!(v, Value::Null) {
        return Ok(Value::Null);
    }
    let Value::Str(t) = v else {
        return Err(AppError::new(
            "custody_component_validation",
            format!("{field} must be text"),
        )
        .with_details(dict([("field", s(field))])));
    };
    let t =
        kassiber_pytext::strip(t.as_str().ok_or_else(|| {
            AppError::new("internal_error", "surrogates not allowed in SQLite text")
        })?);
    Ok(if t.is_empty() { Value::Null } else { s(t) })
}
fn canonical(v: &Value) -> Result<Value> {
    review::canonical_timestamp(&codec::to_json(v))
        .map(s)
        .map_err(|e| {
            if e.code == "validation" {
                AppError::new("validation", e.message)
                    .with_hint("Use RFC3339 UTC like 2025-01-01T00:00:00Z")
            } else {
                convert_error(e)
            }
        })
}

pub(super) fn activate(
    c: &Connection,
    scope: &Scope<'_>,
    base: &Value,
    spec: &mut Value,
    source: &str,
    reason: Option<String>,
) -> Result<String> {
    let lineage = text(spec, "lineage_id");
    let prior=rows(c,"SELECT * FROM custody_components WHERE profile_id=? AND lineage_id=? AND state='active' ORDER BY revision DESC LIMIT 1",[scope.profile_id,&lineage])?.into_iter().next();
    if let Some(prior) = &prior {
        if !rows(
            c,
            "SELECT id FROM custody_components WHERE id=?",
            [text(spec, "component_id")],
        )?
        .is_empty()
        {
            super::pair_spec::retarget(spec, &text(prior, "id"));
        }
        if let Some(draft)=rows(c,"SELECT id FROM custody_components WHERE profile_id=? AND lineage_id=? AND state='draft' AND id!=?",params![scope.profile_id,lineage,text(prior,"id")])?.first() {
            return Err(AppError::new("custody_component_draft_exists","component lineage already has a draft revision").with_details(dict([("draft_component_id",value(draft,"id"))])));
        }
    } else if let Some(old) = rows(
        c,
        "SELECT id FROM custody_components WHERE profile_id=? AND lineage_id=? LIMIT 1",
        [scope.profile_id, &lineage],
    )?
    .first()
    {
        return Err(AppError::new(
            "custody_component_lineage_exists",
            "component lineage already exists; create a revision instead",
        )
        .with_details(dict([
            ("lineage_id", s(&lineage)),
            ("component_id", value(old, "id")),
        ])));
    }
    // Python normalizes child economics before the header; preserve refusal precedence.
    for term in list(spec, "terms") {
        for field in [
            "swap_fee_kind",
            "confidence_at_review",
            "review_source",
            "review_notes",
            "payout_asset",
            "payout_occurred_at",
            "payout_fiat_value_exact",
            "payout_external_id",
            "counterparty",
        ] {
            optional(&value(term, field), field)?;
        }
    }
    let id = text(spec, "component_id");
    let now = canonical(&value(base, "created_at"))?;
    let revision=rows(c,"SELECT COALESCE(MAX(revision),0)+1 AS revision FROM custody_components WHERE profile_id=? AND lineage_id=?",[scope.profile_id,&lineage])?.remove(0);
    let evidence = dict([
        ("review_id", value(base, "id")),
        ("term_kind", value(&list(spec, "terms")[0], "term_kind")),
    ]);
    let mut header = dict([
        ("id", s(&id)),
        ("lineage_id", s(lineage)),
        ("workspace_id", s(scope.workspace_id)),
        ("profile_id", s(scope.profile_id)),
        ("revision", value(&revision, "revision")),
        ("component_type", value(spec, "component_type")),
        ("conservation_mode", value(spec, "conservation_mode")),
        ("state", s("draft")),
        ("evidence_kind", s("reviewed_custody")),
        ("evidence_grade", s("reviewed")),
        ("evidence_json", s(json(&evidence)?)),
        ("conversion_policy", value(spec, "conversion_policy")),
        (
            "conversion_reviewed",
            Value::Int(u8::from(truthy(&value(spec, "conversion_reviewed"))).into()),
        ),
        ("conversion_metadata_json", s("{}")),
        (
            "expected_leg_count",
            (list(spec, "legs").len() as i64).into(),
        ),
        (
            "expected_allocation_count",
            (list(spec, "allocations").len() as i64).into(),
        ),
        (
            "expected_economic_term_count",
            (list(spec, "terms").len() as i64).into(),
        ),
        (
            "authored_source",
            super::component_rows::authored_source(&s(source))?,
        ),
        ("notes", optional(&value(base, "notes"), "notes")?),
        (
            "change_reason",
            if prior.is_some() {
                s(reason.unwrap_or_else(|| "transaction_pair review revised".into()))
            } else {
                Value::Null
            },
        ),
        ("created_at", now.clone()),
    ]);
    if let Some(prior) = &prior {
        set(&mut header, "supersedes_component_id", value(prior, "id"));
    }
    insert(c, "custody_components", &header)?;
    for (collection, table) in [
        ("legs", "custody_component_legs"),
        ("allocations", "custody_component_allocations"),
        ("terms", "custody_component_economic_terms"),
    ] {
        for (ordinal, raw) in list(spec, collection).iter().enumerate() {
            let mut row = raw.clone();
            set(&mut row, "component_id", s(&id));
            set(&mut row, "workspace_id", s(scope.workspace_id));
            set(&mut row, "profile_id", s(scope.profile_id));
            set(&mut row, "ordinal", (ordinal as i64).into());
            set(&mut row, "created_at", now.clone());
            if collection == "legs" {
                for key in [
                    "chain",
                    "network",
                    "valuation_unit",
                    "transaction_id",
                    "wallet_id",
                    "location_ref",
                    "notes",
                ] {
                    set(&mut row, key, optional(&value(raw, key), key)?);
                }
                let anchor = value(&row, "transaction_id");
                set(&mut row, "anchor_transaction_id", anchor);
                if !matches!(value(&row, "occurred_at"), Value::Null) {
                    let when = canonical(&value(&row, "occurred_at"))?;
                    set(&mut row, "occurred_at", when);
                }
            }
            if collection == "terms" {
                for key in [
                    "swap_fee_kind",
                    "confidence_at_review",
                    "review_source",
                    "review_notes",
                    "payout_asset",
                    "payout_occurred_at",
                    "payout_fiat_value_exact",
                    "payout_external_id",
                    "counterparty",
                ] {
                    set(&mut row, key, optional(&value(raw, key), key)?);
                }
            }
            insert(c, table, &row)?;
        }
    }
    let snap = snapshot(c, scope)?;
    let component = snap
        .components
        .iter()
        .find(|c| c.header["id"] == id)
        .unwrap();
    let validation = review::validate_activation(&snap, component).map_err(convert_error)?;
    if validation["activatable"] != true {
        return Err(AppError::new(
            "custody_component_incomplete",
            "custody component is incomplete or does not conserve",
        )
        .with_details(dict([
            ("component_id", s(&id)),
            (
                "validation",
                review::validation_projection(&validation)
                    .map_err(|e| AppError::new("internal_error", e))?,
            ),
        ])));
    }
    let review::ActivationEvidence {
        snapshots,
        commitments: pairs,
    } = review::activation_evidence(&snap, component).map_err(convert_error)?;
    c.execute("UPDATE custody_components SET state='superseded',superseded_at=?,superseded_by_component_id=? WHERE profile_id=? AND lineage_id=? AND state='active' AND id!=?",params![py_str(&now),id,scope.profile_id,text(spec,"lineage_id"),id]).map_err(error)?;
    let txids: BTreeSet<_> = component
        .legs
        .iter()
        .filter_map(|r| r["transaction_id"].as_str())
        .collect();
    c.execute(
        "DELETE FROM custody_component_transaction_memberships WHERE component_id=?",
        [&id],
    )
    .map_err(error)?;
    for tx in txids {
        c.execute("DELETE FROM custody_component_transaction_memberships WHERE profile_id=? AND transaction_id=? AND component_id IN (SELECT id FROM custody_components WHERE state!='active')",[scope.profile_id,tx]).map_err(error)?;
        c.execute("INSERT INTO custody_component_transaction_memberships(component_id,profile_id,transaction_id,created_at) VALUES(?,?,?,?)",params![id,scope.profile_id,tx,py_str(&now)]).map_err(error)?;
    }
    for snapshot in snapshots {
        let snapshot =
            codec::from_json(&snapshot).map_err(|e| AppError::new("internal_error", e))?;
        insert(
            c,
            "custody_authored_evidence_snapshots",
            &dict([
                ("workspace_id", s(scope.workspace_id)),
                ("profile_id", s(scope.profile_id)),
                ("subject_kind", s("custody_component")),
                ("subject_id", s(&id)),
                ("detail_hash", value(&snapshot, "detail_hash")),
                ("quantity_hash", value(&snapshot, "quantity_hash")),
                ("payload_json", value(&snapshot, "payload_json")),
                ("created_at", now.clone()),
            ]),
        )?;
    }
    for (ordinal, (quantity, detail)) in pairs.iter().enumerate() {
        let commitment_id = hash(
            &Value::List(vec![
                s("custody-component-evidence-v1"),
                s(&id),
                (ordinal as i64).into(),
            ]),
            true,
        )?;
        insert(
            c,
            "custody_component_evidence_commitments",
            &dict([
                ("id", s(commitment_id)),
                ("component_id", s(&id)),
                ("workspace_id", s(scope.workspace_id)),
                ("profile_id", s(scope.profile_id)),
                ("ordinal", (ordinal as i64).into()),
                ("quantity_hash", s(quantity)),
                ("detail_hash", s(detail)),
                ("created_at", now.clone()),
            ]),
        )?;
    }
    c.execute("UPDATE custody_components SET state='active',activated_at=?,superseded_at=NULL,superseded_by_component_id=NULL,expected_evidence_count=? WHERE id=? AND state='draft'",params![py_str(&now),pairs.len() as i64,id]).map_err(error)?;
    if let Some(prior) = prior {
        c.execute("UPDATE custody_components SET superseded_by_component_id=? WHERE id=? AND state='superseded'",[&id,&text(&prior,"id")]).map_err(error)?;
    }
    invalidate(c, scope.profile_id)?;
    Ok(id)
}
pub(super) fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    match get(v, key) {
        Some(Value::List(a)) => a,
        _ => &[],
    }
}

fn s(v: impl AsRef<str>) -> Value {
    kassiber_envelope::s(v.as_ref())
}
