//! What the Review screen reads and decides, as the web's pure helpers have it:
//! quarantine pages and causes (`components/kb/quarantine/{types,explain,priceEntry}.ts`), transfer
//! candidates (`components/review/transferModel.ts`) and the review-plan artifact checks
//! (`components/ai/reviewWorkflow.ts`). No UI here.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use crate::i18n::{t, tr};

// ---- Small JSON readers ---------------------------------------------------------------------

pub fn s(v: &Value, k: &str) -> String {
    match &v[k] {
        Value::String(x) => x.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

pub fn opt(v: &Value, k: &str) -> Option<String> {
    v[k].as_str().filter(|x| !x.is_empty()).map(str::to_owned)
}

pub fn num(v: &Value, k: &str) -> f64 {
    v[k].as_f64().unwrap_or(0.0)
}

pub fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// `formatSats`: the count in the UI language's grouping, then ` sats`.
pub fn sats_of(sats: f64) -> String {
    format!("{} sats", crate::format::count(sats.round().abs() as usize))
}

/// `formatMsat`: msat as whole sats.
pub fn msat(value: f64) -> String {
    sats_of((value.abs() / 1000.0).round())
}

// ---- Quarantine pages -----------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Item {
    pub v: Value,
    pub id: String,
    pub external_id: String,
    pub occurred_at: String,
    pub wallet: String,
    pub direction: String,
    pub asset: String,
    pub amount: f64,
    pub amount_msat: f64,
    pub reason: String,
    pub category: Option<String>,
    pub is_downstream: bool,
    pub blocks_reports: bool,
    pub group_key: Option<String>,
    pub created_at: String,
}

impl Item {
    pub fn parse(v: &Value) -> Item {
        Item {
            id: s(v, "transaction_id"),
            external_id: s(v, "external_id"),
            occurred_at: s(v, "occurred_at"),
            wallet: s(v, "wallet"),
            direction: s(v, "direction"),
            asset: s(v, "asset"),
            amount: num(v, "amount"),
            amount_msat: num(v, "amount_msat"),
            reason: s(v, "reason"),
            category: opt(v, "category"),
            is_downstream: v["is_downstream"].as_bool().unwrap_or(false),
            blocks_reports: v["blocks_reports"].as_bool().unwrap_or(false),
            group_key: opt(v, "group_key"),
            created_at: s(v, "created_at"),
            v: v.clone(),
        }
    }
    pub fn evidence(&self) -> &Value {
        &self.v["evidence"]
    }
    pub fn root(&self) -> Option<&Value> {
        Some(&self.v["root"]).filter(|r| r.is_object())
    }
    pub fn pair_id(&self) -> String {
        s(self.evidence(), "pair_id")
    }
    pub fn pair_legs(&self) -> Option<(&Value, &Value)> {
        let l = &self.evidence()["pair_legs"];
        (l["out"].is_object() && l["in"].is_object()).then(|| (&l["out"], &l["in"]))
    }
    pub fn category_or_derived(&self) -> String {
        self.category.clone().unwrap_or_else(|| {
            category_for_reason(&self.reason, self.evidence(), &self.v["detail"]).to_owned()
        })
    }
}

#[derive(Clone, Debug)]
pub struct Action {
    pub kind: String,
    pub wallet_id: Option<String>,
    pub wallet_label: Option<String>,
    pub gap_id: Option<String>,
    pub transaction_id: Option<String>,
    /// `restore_pairing`: the dismissal that undid an automatic link between exactly this held
    /// row's two transactions; deleting it links them back.
    pub dismissal_id: Option<String>,
}

impl Action {
    fn parse(v: &Value) -> Action {
        Action {
            kind: s(v, "kind"),
            wallet_id: opt(v, "wallet_id"),
            wallet_label: opt(v, "wallet_label"),
            gap_id: opt(v, "gap_id"),
            transaction_id: opt(v, "transaction_id"),
            dismissal_id: opt(v, "dismissal_id"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Group {
    pub key: String,
    pub category: String,
    pub reason: String,
    pub root_transaction_id: Option<String>,
    pub root_occurred_at: Option<String>,
    pub root_wallet: Option<String>,
    pub root_asset: Option<String>,
    pub downstream_count: usize,
    pub blocks_reports: bool,
    pub earliest_occurred_at: Option<String>,
    pub evidence: Value,
    pub actions: Vec<Action>,
    pub root_transaction_ids: Vec<String>,
    pub root_count: usize,
}

impl Group {
    pub fn parse(v: &Value) -> Group {
        Group {
            key: s(v, "key"),
            category: s(v, "category"),
            reason: s(v, "reason"),
            root_transaction_id: opt(v, "root_transaction_id"),
            root_occurred_at: opt(v, "root_occurred_at"),
            root_wallet: opt(v, "root_wallet"),
            root_asset: opt(v, "root_asset"),
            downstream_count: num(v, "downstream_count") as usize,
            blocks_reports: v["blocks_reports"].as_bool().unwrap_or(false),
            earliest_occurred_at: opt(v, "earliest_occurred_at"),
            evidence: if v["evidence"].is_object() {
                v["evidence"].clone()
            } else {
                json!({})
            },
            actions: v["actions"]
                .as_array()
                .map(|a| a.iter().map(Action::parse).collect())
                .unwrap_or_default(),
            root_transaction_ids: strs(&v["root_transaction_ids"]),
            root_count: num(v, "root_count") as usize,
        }
    }
}

/// A quarantine page (or the loaded attention pages merged): the first page's summary and every
/// loaded row once, in the daemon's order.
#[derive(Clone, Debug, Default)]
pub struct Snap {
    pub summary: Value,
    pub items: Vec<Item>,
}

impl Snap {
    pub fn parse(v: &Value) -> Snap {
        Snap {
            summary: v["summary"].clone(),
            items: v["items"]
                .as_array()
                .map(|a| a.iter().map(Item::parse).collect())
                .unwrap_or_default(),
        }
    }
    /// `mergeAttentionPages`.
    pub fn merge(pages: &[Value]) -> Option<Snap> {
        let first = pages.first()?;
        let mut seen = HashSet::new();
        let mut items = vec![];
        for p in pages {
            for it in p["items"].as_array().into_iter().flatten() {
                let item = Item::parse(it);
                if seen.insert(item.id.clone()) {
                    items.push(item);
                }
            }
        }
        Some(Snap {
            summary: first["summary"].clone(),
            items,
        })
    }
    pub fn count(&self) -> usize {
        num(&self.summary, "count") as usize
    }
    pub fn attention_count(&self) -> usize {
        self.summary["attention_count"]
            .as_f64()
            .map(|n| n as usize)
            .unwrap_or_else(|| self.count())
    }
    pub fn waiting_count(&self) -> usize {
        if let Some(n) = self.summary["waiting_count"].as_f64() {
            return n as usize;
        }
        self.summary["by_category"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|e| e["category"] == "downstream")
            .map(|e| num(e, "count") as usize)
            .unwrap_or(0)
    }
    pub fn scope_count(&self) -> usize {
        self.summary["scope_count"]
            .as_f64()
            .or_else(|| self.summary["count"].as_f64())
            .unwrap_or(0.0) as usize
    }
    pub fn groups(&self) -> Vec<Group> {
        self.summary["groups"]
            .as_array()
            .map(|a| a.iter().map(Group::parse).collect())
            .unwrap_or_default()
    }
    pub fn group_count(&self) -> Option<usize> {
        self.summary["group_count"].as_f64().map(|n| n as usize)
    }
    pub fn freshness(&self) -> &Value {
        &self.summary["freshness"]
    }
    pub fn last_error(&self) -> Option<String> {
        let e = &self.freshness()["last_error"];
        e.is_object().then(|| s(e, "message"))
    }
    pub fn needs_processing(&self) -> bool {
        self.freshness()["needs_processing"]
            .as_bool()
            .unwrap_or(false)
    }
    pub fn fiat_currency(&self) -> Option<String> {
        opt(&self.summary, "fiat_currency")
    }
    /// `quarantineBookScope`.
    pub fn book_scope(&self) -> Option<Value> {
        let w = opt(&self.summary, "workspace_id")?;
        let p = opt(&self.summary, "profile_id")?;
        Some(json!({ "workspace_id": w, "profile_id": p }))
    }
}

/// `nextAttentionOffset`: the next page's offset, or none once every row is loaded.
pub fn next_offset(page: &Value) -> Option<usize> {
    let summary = &page["summary"];
    if !summary.is_object() {
        return None;
    }
    let len = page["items"].as_array().map(Vec::len).unwrap_or(0);
    let next = num(summary, "offset") as usize + len;
    let total = summary["scope_count"]
        .as_f64()
        .or_else(|| summary["count"].as_f64())
        .unwrap_or(0.0) as usize;
    (len > 0 && next < total).then_some(next)
}

/// `withLoadedCauses`: the summary's causes (capped at 50) plus every cause of loaded rows the
/// summary leaves out, read from those rows.
pub fn with_loaded_causes(groups: Vec<Group>, items: &[Item]) -> Vec<Group> {
    let known: HashSet<String> = groups.iter().map(|g| g.key.clone()).collect();
    let mut order: Vec<String> = vec![];
    let mut unlisted: HashMap<String, Vec<&Item>> = HashMap::new();
    for it in items {
        let Some(k) = &it.group_key else { continue };
        if known.contains(k) {
            continue;
        }
        if !unlisted.contains_key(k) {
            order.push(k.clone());
        }
        unlisted.entry(k.clone()).or_default().push(it);
    }
    let mut out = groups;
    for key in order {
        let rows = &unlisted[&key];
        let roots: Vec<&&Item> = rows.iter().filter(|r| !r.is_downstream).collect();
        let first = roots.first().map(|r| **r).unwrap_or(rows[0]);
        let mut dates: Vec<String> = rows
            .iter()
            .map(|r| r.occurred_at.clone())
            .filter(|d| !d.is_empty())
            .collect();
        dates.sort();
        let root = roots.first();
        out.push(Group {
            key: key.clone(),
            category: first.category_or_derived(),
            reason: first.reason.clone(),
            root_transaction_id: root.map(|r| r.id.clone()),
            root_occurred_at: root.map(|r| r.occurred_at.clone()),
            root_wallet: root.map(|r| r.wallet.clone()),
            root_asset: root.map(|r| r.asset.clone()),
            downstream_count: 0,
            blocks_reports: rows.iter().any(|r| r.blocks_reports),
            earliest_occurred_at: dates.first().cloned(),
            evidence: if first.evidence().is_object() {
                first.evidence().clone()
            } else {
                json!({})
            },
            actions: first.v["actions"]
                .as_array()
                .map(|a| a.iter().map(Action::parse).collect())
                .unwrap_or_default(),
            root_transaction_ids: roots.iter().map(|r| r.id.clone()).collect(),
            root_count: roots.len(),
        });
    }
    out
}

/// What a cause card lists (`causeRows`): its root ids, how many the whole cause holds, every
/// loaded row of it, and the pairs to answer one by one for a suspense left by pairs.
pub struct CauseRows<'a> {
    pub root_ids: Vec<String>,
    pub root_count: usize,
    pub members: Vec<&'a Item>,
    pub pairs: Vec<&'a Item>,
}

pub fn cause_rows<'a>(g: &Group, items: &'a [Item]) -> CauseRows<'a> {
    let root_ids = if !g.root_transaction_ids.is_empty() {
        g.root_transaction_ids.clone()
    } else {
        g.root_transaction_id.iter().cloned().collect()
    };
    let mut members: Vec<&Item> = items
        .iter()
        .filter(|i| i.group_key.as_deref() == Some(g.key.as_str()))
        .collect();
    if members.is_empty() {
        members = root_ids
            .iter()
            .filter_map(|id| items.iter().find(|i| &i.id == id && i.group_key.is_none()))
            .collect();
    }
    let pairs = if cause_key(&g.reason, &g.evidence, &Value::Null) == Some("reviewedSuspensePair") {
        members
            .iter()
            .copied()
            .filter(|i| !i.is_downstream && !i.pair_id().is_empty() && i.pair_legs().is_some())
            .collect()
    } else {
        vec![]
    };
    CauseRows {
        root_count: g.root_count.max(root_ids.len()),
        root_ids,
        members,
        pairs,
    }
}

/// `causeOffersMore`.
pub fn cause_offers_more(g: &Group, items: &[Item]) -> bool {
    let r = cause_rows(g, items);
    let listed = if r.pairs.is_empty() {
        r.members.len()
    } else {
        r.pairs.len()
    };
    listed > 0 && r.root_count > listed
}

// ---- Explaining a cause (explain.ts) ---------------------------------------------------------

fn reason_cause(reason: &str) -> Option<&'static str> {
    Some(match reason {
        "ownership_transfer_source_missing" => "ownershipSourceMissing",
        "ownership_transfer_source_ambiguous" => "ownershipSourceAmbiguous",
        "ownership_transfer_destination_ambiguous" => "ownershipDestinationAmbiguous",
        "ownership_transfer_destination_missing_ref" => "ownershipDestinationMissingRef",
        "ownership_transfer_amount_mismatch" => "ownershipAmountMismatch",
        "ownership_transfer_conservation_mismatch" => "ownershipConservation",
        "ownership_transfer_fee_evidence_incomplete" => "ownershipFeeEvidence",
        "ownership_transfer_asset_evidence_incomplete" => "ownershipAssetEvidence",
        "ownership_transfer_ambiguous_output" => "ownershipAmbiguousOutput",
        "ownership_transfer_duplicate_outbound" => "ownershipDuplicateOutbound",
        "liquid_transfer_graph_incomplete" => "liquidGraphIncomplete",
        "recorded_transfer_authority_conflict" => "recordedAuthorityConflict",
        "transfer_pair_chronology_mismatch" => "chronologyMismatch",
        "transfer_network_mismatch" => "networkMismatch",
        "transfer_fee_implausible" => "feeImplausible",
        "privacy_hop_unresolved" => "privacyHop",
        "samourai_native_event_unverified" => "samouraiUnverified",
        "channel_open_unresolved" => "channelOpen",
        "channel_close_unresolved" => "channelClose",
        "native_transition_ambiguous" => "nativeTransitionAmbiguous",
        "native_transition_amount_mismatch" => "nativeTransitionAmountMismatch",
        "native_transition_fee_timing_unresolved" => "nativeTransitionFeeTiming",
        "custody_authored_migration_incomplete" => "migrationIncomplete",
        "synthetic_import_duplicate_review_required" => "importDuplicate",
        "book_reset_decision_held" => "resetDecisionHeld",
        "custody_basis_barrier" => "basisBarrier",
        "transfer_pair_dependency_blocked" => "pairDependency",
        "basis_provenance_incomplete" => "provenanceIncomplete",
        "derived_transfer_group_blocked" => "groupBlocked",
        "at_swap_basis_carry_unresolved" | "bitcoin_rail_carry_basis_unresolved" => {
            "carryUnresolved"
        }
        "missing_spot_price" => "missingPrice",
        "pricing_review_required" => "pricingReview",
        "price_currency_mismatch" => "priceCurrency",
        "at_swap_price_required" => "swapPriceRequired",
        "insufficient_lots" => "insufficientLots",
        "missing_cost_basis" => "missingCostBasis",
        "unclassified_income_kind" => "unclassifiedIncome",
        "non_sale_disposal_kind" => "nonSaleDisposal",
        "acquisition_valuation_unsupported" => "valuationUnsupported",
        "conflicting_spend" => "conflictingSpend",
        "pending_onchain_confirmation" => "pendingConfirmation",
        "unscoped_transfer_review" => "unscopedTransfer",
        "owned_fanout_unresolved" => "ownedFanout",
        "manual_multi_pair_ambiguous" | "transfer_mismatch" => "pairAmbiguous",
        "custody_component_blocked" => "componentProblem",
        "custody_interpreter_blocked" => "quantityOther",
        _ => return None,
    })
}

fn blocker_cause(code: &str) -> Option<&'static str> {
    Some(match code {
        "custody_gap_review_required" => "gapHold",
        "implicit_wallet_delta_unallocated" => "implicitDelta",
        "unclaimed_source_residual" => "unclaimedResidual",
        "source_overlap_quantity_unresolved" => "sourceOverlap",
        "search_capacity_incomplete" | "capacity_source_suspense_required" => "searchCapacity",
        "reviewed_residual_suspense" => "reviewedSuspense",
        _ => return None,
    })
}

fn cause_category(key: &str) -> &'static str {
    match key {
        "ownershipSourceMissing"
        | "ownershipDestinationMissingRef"
        | "channelOpen"
        | "channelClose"
        | "gapHold"
        | "implicitDelta" => "missing_wallet_history",
        "ownershipFeeEvidence"
        | "ownershipAssetEvidence"
        | "liquidGraphIncomplete"
        | "samouraiUnverified"
        | "conflictingSpend"
        | "pendingConfirmation" => "missing_chain_evidence",
        "missingPrice" | "pricingReview" | "priceCurrency" | "swapPriceRequired" => "missing_price",
        "insufficientLots" | "missingCostBasis" => "missing_acquisition_history",
        "nativeTransitionFeeTiming" | "valuationUnsupported" => "unsupported",
        "basisBarrier"
        | "pairDependency"
        | "provenanceIncomplete"
        | "groupBlocked"
        | "carryUnresolved" => "downstream",
        _ => "needs_decision",
    }
}

/// `causeKeyFor`.
pub fn cause_key(reason: &str, evidence: &Value, detail: &Value) -> Option<&'static str> {
    if reason == "custody_quantity_unresolved" {
        let blocker = evidence["blocker_code"]
            .as_str()
            .or_else(|| detail["blocker_code"].as_str())
            .unwrap_or("");
        if blocker == "reviewed_residual_suspense" && !s(evidence, "pair_id").is_empty() {
            return Some("reviewedSuspensePair");
        }
        if strs(&evidence["review_codes"])
            .iter()
            .any(|c| c == "legacy_pair_fee_allocation_review_required")
        {
            return Some("legacyPairFee");
        }
        if let Some(k) = blocker_cause(blocker) {
            return Some(k);
        }
        if blocker.starts_with("custody_component_") {
            return Some("componentProblem");
        }
        return Some("quantityOther");
    }
    if reason == "synthetic_import_duplicate_review_required" {
        let holds = strs(&evidence["import_hold_reasons"]);
        if holds.iter().any(|h| h == "refilled_excluded_copy") {
            return Some("importDuplicateRefill");
        }
        if holds.iter().any(|h| h == "pre_batch_excess") {
            return Some("importDuplicatePreBatch");
        }
    }
    reason_cause(reason)
}

/// `unlinkedAutomaticPair`: a held send whose automatic link the owner removed, which the daemon
/// offers to link back (asked as the question it leaves open, as Transfers asks it).
pub fn unlinked_automatic_pair(reason: &str, actions: &[Action]) -> bool {
    reason == "recorded_transfer_authority_conflict"
        && actions
            .iter()
            .any(|a| a.kind == "restore_pairing" && a.dismissal_id.is_some())
}

pub fn category_for_reason(reason: &str, evidence: &Value, detail: &Value) -> &'static str {
    cause_key(reason, evidence, detail)
        .map(cause_category)
        .unwrap_or("needs_decision")
}

pub struct CauseCopy {
    pub title: String,
    pub why: String,
    pub provide: String,
    pub why_quotes_amounts: bool,
}

/// `causeCopy`: title, reason and what to provide.
#[cfg(test)]
pub fn cause_copy(
    reason: &str,
    category: &str,
    evidence: &Value,
    wallet: Option<&str>,
    asset: Option<&str>,
) -> CauseCopy {
    cause_copy_with_root(reason, category, evidence, wallet, asset, None)
}

/// `causeCopy` with the context's `rootLabel` (the detail sheet passes the row's root).
pub fn cause_copy_with_root(
    reason: &str,
    category: &str,
    evidence: &Value,
    wallet: Option<&str>,
    asset: Option<&str>,
    root_label: Option<&str>,
) -> CauseCopy {
    cause_copy_full(reason, category, evidence, wallet, asset, root_label, &[])
}

/// `causeCopy` with the cause's `actions`: a link the owner removed reads as that question.
pub fn cause_copy_full(
    reason: &str,
    category: &str,
    evidence: &Value,
    wallet: Option<&str>,
    asset: Option<&str>,
    root_label: Option<&str>,
    actions: &[Action],
) -> CauseCopy {
    let key = if unlinked_automatic_pair(reason, actions) {
        Some("unlinkedAutomaticPair")
    } else {
        cause_key(reason, evidence, &Value::Null)
    };
    let wallet = opt(evidence, "wallet_label")
        .or_else(|| wallet.filter(|w| !w.is_empty()).map(str::to_owned))
        .unwrap_or_else(|| t("journals:quarantine.cause.fallback.wallet"));
    let wallets = evidence["missing_source_wallets"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|w| s(w, "label"))
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|x| !x.is_empty())
        .unwrap_or_else(|| t("journals:quarantine.cause.fallback.sendingWallet"));
    let fmt = |k: &str| {
        evidence[k]
            .as_f64()
            .filter(|n| n.is_finite())
            .map(msat)
            .unwrap_or_else(|| t("journals:quarantine.cause.fallback.amount"))
    };
    let since = evidence["lot_state_uncertain_since"]
        .as_str()
        .map(|x| x.chars().take(10).collect::<String>())
        .filter(|x| !x.is_empty())
        .unwrap_or_else(|| t("journals:quarantine.cause.fallback.date"));
    let cur = |k: &str| {
        opt(evidence, k).unwrap_or_else(|| t("journals:quarantine.cause.fallback.currency"))
    };
    let asset = asset.filter(|a| !a.is_empty()).unwrap_or("BTC").to_owned();
    let root = root_label
        .filter(|r| !r.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| t("journals:quarantine.cause.fallback.root"));
    let (required, available) = (fmt("required_msat"), fmt("available_msat"));
    let (pc, bc) = (cur("price_currency"), cur("book_currency"));
    let values: [(&str, &str); 9] = [
        ("wallet", &wallet),
        ("wallets", &wallets),
        ("asset", &asset),
        ("required", &required),
        ("available", &available),
        ("since", &since),
        ("root", &root),
        ("priceCurrency", &pc),
        ("bookCurrency", &bc),
    ];
    let base = match key {
        Some(k) => format!("journals:quarantine.cause.{k}"),
        None => format!(
            "journals:quarantine.cause.category.{}",
            if category.is_empty() {
                "needs_decision"
            } else {
                category
            }
        ),
    };
    CauseCopy {
        title: tr(&format!("{base}.title"), &values),
        why: tr(&format!("{base}.why"), &values),
        provide: tr(&format!("{base}.provide"), &values),
        why_quotes_amounts: evidence["required_msat"].is_number()
            || evidence["available_msat"].is_number(),
    }
}

/// `causeFacts`: observed facts the user can check.
pub fn cause_facts(evidence: &Value) -> Vec<String> {
    let mut f = vec![];
    if evidence["pair_txids_differ"].as_bool() == Some(true) {
        f.push(t("journals:quarantine.fact.pairTxidsDiffer"));
    }
    if evidence["pair_receipt_before_spend"].as_bool() == Some(true) {
        f.push(t("journals:quarantine.fact.pairReceiptFirst"));
    }
    f
}

/// `exclusionFitsReason`: excluding is an answer for prices and explicit classification
/// questions, never for custody, missing history or follow-on rows.
pub fn exclusion_fits_reason(reason: &str, category: &str, evidence: &Value) -> bool {
    if category == "missing_price" {
        return true;
    }
    if category != "needs_decision" {
        return false;
    }
    matches!(
        cause_key(reason, evidence, &Value::Null),
        Some(
            "unclassifiedIncome"
                | "nonSaleDisposal"
                | "ownershipDuplicateOutbound"
                | "importDuplicate"
                | "importDuplicateRefill"
                | "importDuplicatePreBatch"
        )
    )
}

/// `nextAfterRefresh`: the row after `current` in the re-read list, or (when the save cleared
/// it) the next one after it in the old list that is still held.
pub fn next_after_refresh(
    current: &str,
    previous: &[String],
    refreshed: &[String],
) -> Option<String> {
    if let Some(at) = refreshed.iter().position(|x| x == current) {
        return refreshed.get(at + 1).cloned();
    }
    let index = previous.iter().position(|x| x == current)?;
    previous[index + 1..]
        .iter()
        .find(|id| refreshed.contains(id))
        .cloned()
}

/// `sheetTabForCause`: which detail tab holds the evidence.
pub fn sheet_tab(reason: &str, category: &str, evidence: &Value) -> &'static str {
    let key = cause_key(reason, evidence, &Value::Null);
    if category == "missing_price" {
        return "pricing";
    }
    if matches!(
        key,
        Some("unclassifiedIncome" | "nonSaleDisposal" | "valuationUnsupported")
    ) || category == "missing_acquisition_history"
    {
        return "tax";
    }
    if matches!(
        key,
        Some(
            "reviewedSuspensePair"
                | "legacyPairFee"
                | "ownershipSourceAmbiguous"
                | "ownershipDestinationAmbiguous"
                | "nativeTransitionAmbiguous"
                | "chronologyMismatch"
                | "networkMismatch"
                | "unscopedTransfer"
                | "ownedFanout"
                | "pairAmbiguous"
        )
    ) {
        return "linked";
    }
    "details"
}

pub fn action_label(a: &Action) -> String {
    let k = match a.kind.as_str() {
        "sync_wallet" => {
            let w = a
                .wallet_label
                .clone()
                .unwrap_or_else(|| t("journals:quarantine.cause.fallback.wallet"));
            return tr("journals:quarantine.cta.syncWallet", &[("wallet", &w)]);
        }
        "connect_wallet" => "connectWallet",
        "import_history" => "importHistory",
        "set_price" => "setPrice",
        "classify" => "classify",
        "pair_transfer" => "pairTransfer",
        "review_custody_gap" => "reviewCustodyGap",
        "attach_evidence" => "attachEvidence",
        "wait_for_confirmation" => "waitForConfirmation",
        "resolve_root" => "resolveRoot",
        "review_pair" => "reviewPair",
        "restore_pairing" => "restorePairing",
        "process_journals" => "processJournals",
        _ => "openTransaction",
    };
    t(&format!("journals:quarantine.cta.{k}"))
}

/// `quarantineRootLabel`.
pub fn root_label(root: &Value) -> String {
    let date: String = s(root, "occurred_at").chars().take(10).collect();
    [date, s(root, "wallet")]
        .into_iter()
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// `quarantineRowMeta`: when · wallet · short id.
pub fn row_meta(i: &Item) -> String {
    let id = &i.external_id;
    let n = id.chars().count();
    let short = if n > 16 {
        format!(
            "{}…{}",
            id.chars().take(8).collect::<String>(),
            id.chars().skip(n - 6).collect::<String>()
        )
    } else {
        id.clone()
    };
    [
        i.occurred_at.chars().take(10).collect::<String>(),
        i.wallet.clone(),
        short,
    ]
    .into_iter()
    .filter(|x| !x.is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}

/// `quarantineRowAmount`: signed sats for bitcoin, units otherwise.
pub fn row_amount(i: &Item) -> String {
    let sign = if i.direction == "outbound" {
        "−"
    } else {
        "+"
    };
    let asset = i.asset.to_uppercase();
    if asset == "BTC" || asset == "LBTC" {
        format!("{sign}{}", msat(i.amount_msat))
    } else {
        format!("{sign}{} {}", trim_num(i.amount.abs()), i.asset)
    }
}

/// JavaScript's `String(number)` for ordinary values.
pub fn trim_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e21 {
        format!("{}", v as i128)
    } else {
        let s = format!("{v}");
        s
    }
}

/// `samePairCase`: a freshly read row is still the pair case the owner picked.
pub fn same_pair_case(listed: &Item, current: Option<&Item>) -> bool {
    let Some(cur) = current else { return false };
    let (b, a) = (listed.evidence(), cur.evidence());
    let pid = s(b, "pair_id");
    if pid.is_empty() || pid != s(a, "pair_id") {
        return false;
    }
    if b["pair_fingerprint"] != a["pair_fingerprint"]
        || listed.reason != cur.reason
        || b["blocker_code"] != a["blocker_code"]
    {
        return false;
    }
    let reading = |e: &Value| {
        let leg = |side: &str| {
            let l = &e["pair_legs"][side];
            if l.is_object() {
                json!([
                    l["transaction_id"],
                    l["wallet"],
                    l["asset"],
                    l["amount_msat"],
                    l["occurred_at"],
                    l["external_id"]
                ])
            } else {
                Value::Null
            }
        };
        json!([e["pair_review"], leg("out"), leg("in")])
    };
    reading(b) == reading(a)
}

/// `reconcilePicks`: picks read against a fresh page, matched by pair: still held as picked,
/// cleared meanwhile, or changed.
pub fn reconcile_picks(picked: &[Item], fresh: &[Item]) -> (Vec<Item>, Vec<Item>, Vec<Item>) {
    let by_tx: HashMap<&str, &Item> = fresh.iter().map(|i| (i.id.as_str(), i)).collect();
    let mut by_pair: HashMap<String, &Item> = HashMap::new();
    for i in fresh {
        let p = i.pair_id();
        if !p.is_empty() {
            by_pair.entry(p).or_insert(i);
        }
    }
    let (mut cur, mut cleared, mut changed) = (vec![], vec![], vec![]);
    for it in picked {
        let pid = it.pair_id();
        let row = by_tx.get(it.id.as_str()).copied();
        let now = if row.is_some_and(|r| r.pair_id() == pid) {
            row
        } else {
            by_pair.get(&pid).copied()
        };
        if let Some(n) = now {
            if same_pair_case(it, Some(n)) {
                cur.push(it.clone())
            } else {
                changed.push(it.clone())
            }
        } else if row.is_some() {
            changed.push(it.clone());
        } else {
            cleared.push(it.clone());
        }
    }
    (cur, cleared, changed)
}

/// One review proposal covers at most this many repairs (the daemon's bound).
pub const MAX_FIX_OPERATIONS: usize = 50;
pub const UNPAIR_REASON: &str =
    "The owner reviewed this pair from quarantine and chose to unpair it.";
pub const ACCEPT_PRICES_REASON: &str =
    "The owner accepted the approximate price on the Review screen.";
pub const ENTERED_PRICE_REASON: &str = "The owner entered the price on the Review screen.";

/// `fixOperations`: unpair each picked pair once, bound to its reading.
pub fn fix_operations(items: &[Item]) -> Vec<Value> {
    let mut seen = HashSet::new();
    let mut ops = vec![];
    for i in items {
        let pid = i.pair_id();
        if pid.is_empty() || !seen.insert(pid.clone()) {
            continue;
        }
        ops.push(json!({ "type": "unpair", "pair_id": pid, "expected_fingerprint": s(i.evidence(), "pair_fingerprint"), "reason": UNPAIR_REASON }));
    }
    ops
}

// ---- Typed prices (priceEntry.ts) -------------------------------------------------------------

fn is_space_group(c: char) -> bool {
    c.is_whitespace() || c == '\'' || c == '’'
}

fn grouped_head(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && b.len() <= 3 && b[0] != b'0' && b.iter().all(u8::is_ascii_digit)
}

fn three(s: &str) -> bool {
    s.len() == 3 && s.bytes().all(|c| c.is_ascii_digit())
}

/// `exactDecimal`: the exact decimal the owner typed, or None.
pub fn exact_decimal(input: &str, dec: char) -> Option<String> {
    let text = input.trim();
    if text.is_empty()
        || text.chars().count() > 40
        || !text
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == ',' || is_space_group(c))
        || !text.chars().any(|c| c.is_ascii_digit())
    {
        return None;
    }
    let dots = text.matches('.').count();
    let commas = text.matches(',').count();
    let mut sep: Option<char> = None;
    if dots > 0 && commas > 0 {
        sep = Some(if text.rfind('.') > text.rfind(',') {
            '.'
        } else {
            ','
        });
    } else if dots + commas == 1 {
        let only = if dots > 0 { '.' } else { ',' };
        let (before, after) = text.split_once(only).unwrap();
        let looks_grouped = grouped_head(before) && three(after);
        sep = if only == dec || !looks_grouped {
            Some(only)
        } else {
            None
        };
    }
    let at = sep.and_then(|c| text.rfind(c));
    let (whole, fraction) = match at {
        Some(i) => (&text[..i], &text[i + 1..]),
        None => (text, ""),
    };
    if at.is_some() && (fraction.is_empty() || !fraction.bytes().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let marks: Vec<char> = whole.chars().filter(|c| !c.is_ascii_digit()).collect();
    if !marks.is_empty() {
        let kinds: HashSet<char> = marks
            .iter()
            .map(|&m| if is_space_group(m) { ' ' } else { m })
            .collect();
        if kinds.len() > 1 || sep.is_some_and(|s| kinds.contains(&s)) {
            return None;
        }
        let groups: Vec<&str> = whole.split(|c: char| !c.is_ascii_digit()).collect();
        if !grouped_head(groups[0]) || groups[1..].iter().any(|g| !three(g)) {
            return None;
        }
    }
    let digits: String = whole.chars().filter(char::is_ascii_digit).collect();
    let digits = if digits.is_empty() {
        "0".to_owned()
    } else {
        digits
    };
    let trimmed = digits.trim_start_matches('0');
    let integer = if trimmed.is_empty() { "0" } else { trimmed };
    Some(if fraction.is_empty() {
        integer.to_owned()
    } else {
        format!("{integer}.{fraction}")
    })
}

pub fn is_positive_decimal(v: &str) -> bool {
    v.chars().any(|c| ('1'..='9').contains(&c))
}

/// `formatExactDecimal`: every digit kept, in the UI language's format.
pub fn format_exact_decimal(v: &str) -> String {
    let (int, frac) = v
        .split_once('.')
        .map(|(a, b)| (a, Some(b)))
        .unwrap_or((v, None));
    let (group, dec) = match crate::i18n::lang() {
        crate::i18n::Lang::En => (',', '.'),
        crate::i18n::Lang::De => ('\u{a0}', ','),
    };
    let mut g = String::new();
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            g.push(group);
        }
        g.push(ch);
    }
    match frac {
        Some(f) => format!("{g}{dec}{f}"),
        None => g,
    }
}

pub enum PriceCheck {
    Ok(Vec<Value>),
    Bad {
        invalid: Vec<String>,
        too_many: bool,
    },
}

/// `priceEntryOperations`.
pub fn price_entry_operations(
    order: &[String],
    entries: &HashMap<String, String>,
    rate: bool,
    dec: char,
) -> PriceCheck {
    let mut invalid = vec![];
    let mut ops = vec![];
    for id in order {
        let typed = entries.get(id).map(|s| s.trim()).unwrap_or("");
        if typed.is_empty() {
            continue;
        }
        match exact_decimal(typed, dec).filter(|v| is_positive_decimal(v)) {
            None => invalid.push(id.clone()),
            Some(v) => {
                let mut op = json!({ "type": "price_override", "transaction_id": id, "reason": ENTERED_PRICE_REASON });
                op[if rate { "fiat_rate" } else { "fiat_value" }] = json!(v);
                ops.push(op);
            }
        }
    }
    let too_many = ops.len() > MAX_FIX_OPERATIONS;
    if !invalid.is_empty() || too_many || ops.is_empty() {
        PriceCheck::Bad { invalid, too_many }
    } else {
        PriceCheck::Ok(ops)
    }
}

/// The UI language's decimal separator.
pub fn decimal_separator() -> char {
    match crate::i18n::lang() {
        crate::i18n::Lang::En => '.',
        crate::i18n::Lang::De => ',',
    }
}

// ---- Transfer candidates (transferModel.ts) ---------------------------------------------------

pub const DISMISS_REASON: &str = "user dismissed from review queue";

pub fn candidate_key(c: &Value) -> String {
    format!("{}->{}", s(c, "out_id"), s(c, "in_id"))
}

/// `candidateTerms`: the kind and treatment linking records, a matching rule's, else the
/// matcher's default.
pub fn candidate_terms(c: &Value) -> (String, String) {
    let rule = &c["rule_match"];
    let kind = opt(rule, "kind").unwrap_or_else(|| s(c, "default_kind"));
    let policy = opt(rule, "policy").unwrap_or_else(|| s(c, "default_policy"));
    (kind, policy)
}

/// `needsReview`: not exact, or competing with another match for one of its sides. These open
/// the receipt picker, not a one-click link.
pub fn needs_review(c: &Value) -> bool {
    s(c, "confidence") != "exact" || num(c, "conflict_size") > 1.0
}

/// `suggestionReason`: why the matcher suggests a pair, in one short phrase.
pub fn suggestion_reason(c: &Value) -> String {
    match s(c, "method").as_str() {
        "payment_hash" => t("review:inbox.transfers.reason.paymentHash"),
        "provider_swap_id" => t("review:inbox.transfers.reason.provider"),
        "htlc_refund" => t("review:inbox.transfers.reason.refund"),
        "ownership_graph" => t("review:inbox.transfers.reason.ownership"),
        _ => {
            let minutes = match (
                crate::format::parse_ts(c["out_occurred_at"].as_str().unwrap_or("")),
                crate::format::parse_ts(c["in_occurred_at"].as_str().unwrap_or("")),
            ) {
                (Some(a), Some(b)) => Some(((b - a).abs() / 60.0).round()),
                _ => None,
            };
            let time = match minutes {
                None => String::new(),
                Some(m) if m < 90.0 => {
                    crate::i18n::tc("review:inbox.transfers.reason.minutes", m as usize, &[])
                }
                Some(m) if m < 48.0 * 60.0 => crate::i18n::tc(
                    "review:inbox.transfers.reason.hours",
                    (m / 60.0).round() as usize,
                    &[],
                ),
                Some(m) => crate::i18n::tc(
                    "review:inbox.transfers.reason.days",
                    (m / 1440.0).round() as usize,
                    &[],
                ),
            };
            // A larger arrival is no fee, so it is only a close amount.
            let fee = num(c, "swap_fee_msat");
            let key = if fee == 0.0 {
                "sameAmount"
            } else if fee > 0.0 {
                "closeAmount"
            } else {
                "similarAmount"
            };
            tr(
                &format!("review:inbox.transfers.reason.{key}"),
                &[("time", &time)],
            )
        }
    }
}

/// `bulkPairable`.
pub fn bulk_pairable(cs: &[Value]) -> Vec<Value> {
    cs.iter()
        .filter(|c| {
            s(c, "confidence") == "exact"
                && num(c, "conflict_size") <= 1.0
                && s(c, "method") != "ownership_graph"
                && candidate_terms(c).1 == "carrying-value"
        })
        .take(50)
        .cloned()
        .collect()
}

const BITCOIN_SWAP_KINDS: [&str; 6] = [
    "chain-swap",
    "peg-in",
    "peg-out",
    "reverse-submarine-swap",
    "submarine-swap",
    "swap-refund",
];

/// `moveType`: `transfer`, `bitcoinSwap` or `swap`.
pub fn move_type(c: &Value) -> &'static str {
    if s(c, "out_asset").to_uppercase() == s(c, "in_asset").to_uppercase() {
        return "transfer";
    }
    if s(c, "candidate_type") == "transfer"
        || BITCOIN_SWAP_KINDS.contains(&s(c, "default_kind").as_str())
    {
        return "bitcoinSwap";
    }
    "swap"
}

pub const BITCOIN_ASSETS: [&str; 5] = ["BTC", "XBT", "XXBT", "LBTC", "L-BTC"];

pub fn is_bitcoin(asset: &str) -> bool {
    BITCOIN_ASSETS.contains(&asset.trim().to_uppercase().as_str())
}

/// `legAmount`: ₿ for bitcoin on any rail, the asset's own unit otherwise (msat scale).
pub fn leg_amount(msat: f64, asset: &str) -> String {
    let q = msat.abs() / 100_000_000_000.0;
    if is_bitcoin(asset) {
        crate::format::btc(q, 8, false)
    } else {
        let s = format!("{q:.8}");
        let s = s.trim_end_matches('0').trim_end_matches('.').to_owned();
        format!("{s} {asset}")
    }
}

// ---- Review artifacts (reviewWorkflow.ts) -------------------------------------------------------

const REVIEW_OPERATION_TYPES: [&str; 16] = [
    "pair",
    "unpair",
    "price_override",
    "accept_coarse_prices",
    "kind_override",
    "exclude",
    "include",
    "dismiss",
    "classify",
    "transfer_pair",
    "pair_update",
    "gap_resolve",
    "component",
    "split_payout",
    "duplicate_resolve",
    "acquisition_kind",
];

fn effects_ok(v: &Value) -> bool {
    let c = |k: &str| v[k].as_u64().is_some();
    v.is_object()
        && c("entries_count")
        && c("quarantine_count")
        && v["report_ready"].is_boolean()
        && v["quarantines"].is_array()
}

/// `reviewArtifact`: a schema-1 preview the dialogs may confirm. Operation types are not
/// re-checked against the web's list beyond being named (the daemon owns that list).
pub fn artifact_ok(v: &Value) -> bool {
    let _ = REVIEW_OPERATION_TYPES;
    let digest = v["digest"].as_str().unwrap_or("");
    v["schema_version"] == 1
        && v["workspace_id"].is_string()
        && v["profile_id"].is_string()
        && v["base_input_version"].as_u64().is_some()
        && digest.len() == 64
        && digest
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        && v["operations"].as_array().is_some_and(|o| {
            !o.is_empty()
                && o.len() <= 50
                && o.iter().all(|x| x.is_object() && x["type"].is_string())
        })
        && effects_ok(&v["before"])
        && effects_ok(&v["after"])
}

/// `reviewTaxDelta`: the tax context and changed years, or None for an older artifact.
pub fn tax_delta(a: &Value) -> Option<(Value, Vec<Value>)> {
    let after = if a["after"].is_object() {
        &a["after"]
    } else {
        &a["verification"]
    };
    let tax = &after["tax"];
    let years = a["delta"]["years"].as_array()?;
    if !tax["fiat_currency"].is_string() || !tax["years"].is_array() {
        return None;
    }
    if !years.iter().all(|y| {
        y["year"].as_i64().is_some()
            && y["realized_gain_loss"].is_string()
            && y["income"].is_string()
    }) {
        return None;
    }
    Some((tax.clone(), years.clone()))
}

/// A random v4-style UUID for idempotency keys (`crypto.randomUUID()`).
pub fn uuid() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut x = nanos
        ^ N.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed)
        ^ (std::process::id() as u64).rotate_left(32);
    let mut next = || {
        x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    let (a, b) = (next(), next());
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        a >> 32,
        (a >> 16) & 0xffff,
        a & 0xfff,
        (b >> 48) & 0x3fff | 0x8000,
        b & 0xffff_ffff_ffff
    )
}

#[cfg(test)]
mod tests {

    #[test]
    fn next_after_refresh_walks_what_is_still_held() {
        let ids = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        // Still held: the row after it in the re-read list.
        assert_eq!(
            super::next_after_refresh("b", &ids(&["a", "b", "c"]), &ids(&["a", "b", "c"])),
            Some("c".into())
        );
        // Cleared by the save: the next old row that is still held.
        assert_eq!(
            super::next_after_refresh("b", &ids(&["a", "b", "c", "d"]), &ids(&["a", "d"])),
            Some("d".into())
        );
        assert_eq!(
            super::next_after_refresh("d", &ids(&["a", "d"]), &ids(&["a", "d"])),
            None
        );
        assert_eq!(
            super::next_after_refresh("x", &ids(&["a"]), &ids(&["a"])),
            None
        );
    }

    #[test]
    fn exclusion_fits_only_prices_and_classification_decisions() {
        use serde_json::Value;
        assert!(super::exclusion_fits_reason(
            "missing_spot_price",
            "missing_price",
            &Value::Null
        ));
        assert!(super::exclusion_fits_reason(
            "unclassified_income_kind",
            "needs_decision",
            &Value::Null
        ));
        assert!(!super::exclusion_fits_reason(
            "insufficient_lots",
            "missing_acquisition_history",
            &Value::Null
        ));
        assert!(!super::exclusion_fits_reason(
            "transfer_pair_chronology_mismatch",
            "needs_decision",
            &Value::Null
        ));
    }
    use super::*;

    #[test]
    fn typed_prices_read_like_the_web() {
        assert_eq!(exact_decimal("2800", ',').as_deref(), Some("2800"));
        assert_eq!(exact_decimal("2.800,50", ',').as_deref(), Some("2800.50"));
        assert_eq!(exact_decimal("2,800.50", '.').as_deref(), Some("2800.50"));
        assert_eq!(exact_decimal("2 800,5", ',').as_deref(), Some("2800.5"));
        assert_eq!(
            exact_decimal("1'234'567.891234567", '.').as_deref(),
            Some("1234567.891234567")
        );
        assert_eq!(exact_decimal("0,5", '.').as_deref(), Some("0.5"));
        assert_eq!(exact_decimal("007", '.').as_deref(), Some("7"));
        assert_eq!(exact_decimal("2.800", ',').as_deref(), Some("2800"));
        assert_eq!(exact_decimal("2.800", '.').as_deref(), Some("2.800"));
        assert_eq!(exact_decimal("2,800", '.').as_deref(), Some("2800"));
        assert_eq!(exact_decimal("2,800", ',').as_deref(), Some("2.800"));
        assert_eq!(exact_decimal("12.5", ',').as_deref(), Some("12.5"));
        assert_eq!(exact_decimal("0.500", ',').as_deref(), Some("0.500"));
        assert_eq!(exact_decimal("0,500", '.').as_deref(), Some("0.500"));
        assert_eq!(exact_decimal("000.500", ',').as_deref(), Some("0.500"));
        assert_eq!(exact_decimal("0.500.000", ','), None);
        for bad in [
            "",
            "-5",
            "1e3",
            "5.",
            "1,2,3",
            "1.2.3,4.5",
            "12,34.567,8",
            "abc",
            "€ 5",
            "1 0",
            "10 20",
            "1 234.567,89",
            "1,234.567,89",
            "2 800 ,5",
        ] {
            assert_eq!(exact_decimal(bad, '.'), None, "{bad:?}");
        }
    }

    #[test]
    fn price_operations_hold_back_bad_rows() {
        let ids: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let mut e = HashMap::new();
        e.insert("a".to_string(), "2.800,5".to_string());
        e.insert("c".to_string(), "".to_string());
        match price_entry_operations(&ids, &e, false, ',') {
            PriceCheck::Ok(ops) => assert_eq!(
                ops,
                vec![
                    json!({"type":"price_override","transaction_id":"a","reason":ENTERED_PRICE_REASON,"fiat_value":"2800.5"})
                ]
            ),
            _ => panic!(),
        }
        e.insert("b".to_string(), "x".to_string());
        assert!(
            matches!(price_entry_operations(&ids, &e, false, ','), PriceCheck::Bad { ref invalid, too_many: false } if invalid == &vec!["b".to_string()])
        );
        crate::i18n::set_lang(crate::i18n::Lang::En);
        assert_eq!(
            format_exact_decimal("1234567.891234567"),
            "1,234,567.891234567"
        );
        assert_eq!(format_exact_decimal("12"), "12");
    }

    #[test]
    fn candidates_link_with_a_rule_terms_and_ask_for_review_when_unsure() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        let c = json!({"confidence": "exact", "conflict_size": 1, "default_kind": "manual", "default_policy": "carrying-value", "method": "amount_time", "swap_fee_msat": 0,
            "out_occurred_at": "2026-01-01T10:00:00Z", "in_occurred_at": "2026-01-01T10:12:00Z",
            "rule_match": {"rule_id": "r1", "rule_name": "Pegs", "kind": "peg-out", "policy": "taxable"}});
        assert_eq!(candidate_terms(&c), ("peg-out".into(), "taxable".into()));
        // A rule that makes it taxable keeps it out of the bulk step.
        assert!(bulk_pairable(std::slice::from_ref(&c)).is_empty());
        assert!(!needs_review(&c));
        assert!(needs_review(
            &json!({"confidence": "strong", "conflict_size": 1})
        ));
        assert!(needs_review(
            &json!({"confidence": "exact", "conflict_size": 2})
        ));
        assert_eq!(suggestion_reason(&c), "Same amount, 12 min apart");
        let late = json!({"swap_fee_msat": 1000, "out_occurred_at": "2026-01-01T10:00:00Z", "in_occurred_at": "2026-01-01T15:00:00Z"});
        assert_eq!(
            suggestion_reason(&late),
            "Same amount less a fee, 5 h apart"
        );
        let days = json!({"swap_fee_msat": -5, "out_occurred_at": "2026-01-01T10:00:00Z", "in_occurred_at": "2026-01-04T10:00:00Z"});
        assert_eq!(suggestion_reason(&days), "Similar amount, 3 days apart");
        assert_eq!(
            suggestion_reason(&json!({"method": "payment_hash"})),
            "Same payment hash"
        );
    }

    #[test]
    fn a_removed_automatic_link_reads_as_its_question() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        let acts = |v: Value| {
            v.as_array()
                .unwrap()
                .iter()
                .map(Action::parse)
                .collect::<Vec<_>>()
        };
        let unlinked = acts(
            json!([{"kind": "restore_pairing", "dismissal_id": "d-9"}, {"kind": "pair_transfer", "transaction_id": "out"}]),
        );
        let reason = "recorded_transfer_authority_conflict";
        assert!(unlinked_automatic_pair(reason, &unlinked));
        assert!(!unlinked_automatic_pair(
            reason,
            &acts(json!([{"kind": "restore_pairing"}]))
        ));
        let copy = cause_copy_full(
            reason,
            "",
            &json!({}),
            Some("BTCPay Bark"),
            None,
            None,
            &unlinked,
        );
        assert_eq!(copy.title, "You unlinked this send from BTCPay Bark");
        assert_eq!(
            cause_copy(reason, "", &json!({}), None, None).title,
            "One side comes from a file, the other from the chain"
        );
        assert_eq!(
            category_for_reason(reason, &json!({}), &Value::Null),
            "needs_decision"
        );
        assert_eq!(action_label(&unlinked[0]), "Link it back");
    }

    #[test]
    fn uuids_look_like_v4() {
        let a = uuid();
        assert_eq!(a.len(), 36);
        assert_ne!(a, uuid());
    }
}
