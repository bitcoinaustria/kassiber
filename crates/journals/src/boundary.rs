//! Replaceable JSON adapter for today's projection dataclass and ledger response.
//! Native callers use `build` and the typed `LedgerResponse` directly.
use crate::{render::LedgerInputs, *};
use kassiber_tax_core::{
    ledger::{request::Regime, response::*},
    manifest::{DetailKind, EntryType, QuarantineReason},
    model::Dec,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
pub(crate) mod joins;

/// Wallets in original mapping insertion order. The JSON adapter uses an explicit
/// list because duplicate-label selection is last-wins, never sorted by id.
pub fn build<'a>(
    profile: &'a Value,
    c: &'a Value,
    wallets: &'a [Value],
    response: &'a LedgerResponse,
    ids: impl FnMut() -> String,
) -> Result<JournalState<'a>, Refusal> {
    build_operands(
        profile,
        &RenderOperands::from_json(c),
        wallets,
        response,
        ids,
    )
}

pub struct RenderOperands<'a> {
    pub rows: &'a [Value],
    pub owned_index: &'a Value,
    pub loan_legs: &'a [Value],
    pub channel_roles: &'a Value,
    pub components: &'a [Value],
    pub component_blockers: &'a Value,
    pub native_reconciliations: &'a [Value],
    pub quantity: &'a Value,
    pub direct_payout_records: &'a [Value],
    pub transfers: &'a Value,
    pub projection: &'a Value,
    pub ignored: &'a Value,
    pub accounting_ignored: &'a Value,
    pub warnings: &'a Value,
    pub rates: &'a Value,
    /// `projection["rows"]` as views of the finalized stage's rows.
    pub projection_rows: Option<&'a [kassiber_custody::finalized_projection::RowView<'a>]>,
    /// `quantity["projection"]`'s observations and postings as views.
    pub quantity_views: Option<&'a kassiber_custody::quantity::runtime::QuantityViews<'a>>,
}

impl<'a> RenderOperands<'a> {
    /// The quantity projection's observations and postings, from the views
    /// when the caller supplies them.
    fn quantity_refs(&self) -> (Vec<ObservationRef<'a>>, Vec<PostingRef<'a>>) {
        crate::model::quantity_refs(self.quantity, self.quantity_views)
    }
    pub fn from_json(c: &'a Value) -> Self {
        Self {
            rows: array(&c["rows"]),
            owned_index: &c["owned_index"],
            loan_legs: array(&c["loan_legs"]),
            channel_roles: &c["channel_roles"],
            components: array(&c["active_components"]),
            component_blockers: &c["component_blockers"],
            native_reconciliations: array(&c["interpretation"]["native_component_reconciliations"]),
            quantity: &c["quantity_state"],
            direct_payout_records: array(&c["direct_payout_records"]),
            transfers: &c["custody_transfers"],
            projection: &c["finalized_tax_projection"],
            ignored: &c["gap_ignored_transaction_ids"],
            accounting_ignored: &c["gap_accounting_ignored_transaction_ids"],
            warnings: &c["warnings"],
            rates: &c["rates"],
            projection_rows: None,
            quantity_views: None,
        }
    }
}

pub fn build_operands<'a>(
    profile: &'a Value,
    c: &RenderOperands<'a>,
    wallets: &'a [Value],
    response: &'a LedgerResponse,
    ids: impl FnMut() -> String,
) -> Result<JournalState<'a>, Refusal> {
    let input = LedgerInputs {
        profile,
        projection: c.projection,
        rows: c.projection_rows,
        wallets,
        direct_payout_records: c.direct_payout_records,
    };
    let span = kassiber_custody::timing::span("render");
    let ledger = render_ledger(&input, response, ids)?;
    drop(span);
    finish_operands(c, ledger)
}

/// Complete the diagnostic joins and pass-through state after native ledger
/// rendering. The caller retains custody operands through persistence. Neither
/// this stage nor `render_ledger` performs SQL or generates clock/UUID values.
pub fn finish<'a>(c: &'a Value, ledger: LedgerState<'a>) -> Result<JournalState<'a>, Refusal> {
    finish_operands(&RenderOperands::from_json(c), ledger)
}

pub fn finish_operands<'a>(
    c: &RenderOperands<'a>,
    ledger: LedgerState<'a>,
) -> Result<JournalState<'a>, Refusal> {
    let span = kassiber_custody::timing::span("balance_comparison");
    let (observations, postings) = c.quantity_refs();
    let quantity_differences =
        joins::differences(c, &observations, &postings, &ledger.wallet_holdings)?;
    drop(span);
    let span = kassiber_custody::timing::span("ownership_review");
    let (counts, candidates) = joins::ownership_review(c, &ledger.quarantines)?;
    drop(span);
    fn asdict(value: &Value) -> Value {
        match value {
            Value::Object(m) => Value::Object(
                m.iter()
                    .filter(|(k, _)| k.as_str() != "$type")
                    .map(|(k, v)| (k.clone(), asdict(v)))
                    .collect(),
            ),
            Value::Array(a) => Value::Array(a.iter().map(asdict).collect()),
            _ => value.clone(),
        }
    }
    let reconciliations = c
        .native_reconciliations
        .iter()
        .map(asdict)
        .collect::<Vec<_>>();
    Ok(JournalState {
        entries: ledger.entries,
        quarantines: ledger.quarantines,
        cross_asset_pairs: &c.projection["cross_asset_pairs"],
        direct_swap_payouts: ledger.direct_swap_payouts,
        tax_summary: ledger.tax_summary,
        account_holdings: ledger.account_holdings,
        wallet_holdings: ledger.wallet_holdings,
        ownership_review_counts: counts,
        ownership_review_candidates: candidates,
        custody_component_blockers: c.component_blockers,
        custody_native_reconciliations: json!(reconciliations),
        custody_quantity: c.quantity,
        custody_transfers: c.transfers,
        quantity_differences,
        latest_rates: c.rates,
        warnings: c.warnings,
        custody_gap_ignored_transaction_ids: c.ignored,
        custody_gap_accounting_ignored_transaction_ids: c.accounting_ignored,
        quantity_views: c.quantity_views,
    })
}
fn get<T: DeserializeOwned>(v: &Value, k: &str) -> Result<T, Refusal> {
    let value = v
        .get(k)
        .ok_or_else(|| Refusal::new("KeyError", format!("'{k}'")))?;
    serde_json::from_value(value.clone())
        .map_err(|e| Refusal::new("EngineContractError", format!("RequestError: {k}: {e}")))
}
fn response_array<'a>(v: &'a Value, key: &str) -> Result<&'a [Value], Refusal> {
    v.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| {
            Refusal::new(
                "EngineContractError",
                format!("RequestError: {key}: expected a list"),
            )
        })
}
fn entry_type(v: &Value) -> Result<EntryType, Refusal> {
    let s = v.as_str().unwrap_or("");
    EntryType::ALL
        .into_iter()
        .find(|t| t.as_str() == s)
        .ok_or_else(|| {
            Refusal::new(
                "EngineContractError",
                format!("RequestError: unknown entry type {s:?}"),
            )
        })
}
fn event(v: &Value) -> Result<FragmentEvent, Refusal> {
    Ok(FragmentEvent {
        row: get(v, "row")?,
        spot_price: get(v, "spot_price")?,
        fiat_fee: get(v, "fiat_fee")?,
        crypto_fee: get(v, "crypto_fee")?,
    })
}
fn fragment(v: &Value) -> Result<FragmentOut, Refusal> {
    Ok(FragmentOut {
        crypto_amount: get(v, "crypto_amount")?,
        cost_basis: get(v, "cost_basis")?,
        proceeds: get(v, "proceeds")?,
        gain_loss: get(v, "gain_loss")?,
        unit_basis_override: get(v, "unit_basis_override")?,
        event: event(&v["event"])?,
        lot: if v["lot"].is_null() {
            None
        } else {
            let l = &v["lot"];
            let e = event(l)?;
            Some(FragmentLot {
                row: e.row,
                spot_price: e.spot_price,
                fiat_fee: e.fiat_fee,
                crypto_fee: e.crypto_fee,
                basis: get(l, "basis")?,
                crypto_in: get(l, "crypto_in")?,
            })
        },
    })
}
/// The engine's ledger for a request text, read back through its response text.
pub fn ledger_text(request: &str) -> Result<LedgerResponse, Refusal> {
    let response = kassiber_custody::codec::loads(&kassiber_tax_core::ledger(request))?;
    ledger_response(&kassiber_custody::codec::to_json(&response))
}
/// Thin decoder: does not re-run tax computation or derive a second ledger.
pub fn ledger_response(v: &Value) -> Result<LedgerResponse, Refusal> {
    if v["ok"] != true {
        return Err(ledger_error(&v["error"]));
    }
    let mut entries = vec![];
    for e in response_array(v, "entries")? {
        let t = entry_type(&e["entry_type"])?;
        entries.push(match t {
            EntryType::Acquisition => EntryOut::Acquisition(AcquisitionOut {
                entry_type: t,
                row: get(e, "row")?,
                wallet: get(e, "wallet")?,
                asset: get(e, "asset")?,
                quantity: get(e, "quantity")?,
                fiat_value: get(e, "fiat_value")?,
                unit_cost: get(e, "unit_cost")?,
                description: get(e, "description")?,
            }),
            EntryType::TransferOut | EntryType::TransferIn => {
                EntryOut::TransferLeg(TransferLegOut {
                    entry_type: t,
                    audit: get(e, "audit")?,
                })
            }
            _ => EntryOut::Realized(RealizedOut {
                entry_type: t,
                row: get(e, "row")?,
                wallet: get(e, "wallet")?,
                asset: get(e, "asset")?,
                quantity: get(e, "quantity")?,
                proceeds: get(e, "proceeds")?,
                cost_basis: get(e, "cost_basis")?,
                gain_loss: get(e, "gain_loss")?,
                description: get(e, "description")?,
                at_category: get(e, "at_category")?,
                capital_gains_type: long_short(&e["capital_gains_type"])?,
                fragments: response_array(e, "fragments")?
                    .iter()
                    .map(fragment)
                    .collect::<Result<_, _>>()?,
            }),
        });
    }
    let quarantines = response_array(v, "quarantines")?
        .iter()
        .map(|q| {
            Ok(QuarantineOut {
                row: get(q, "row")?,
                reason: QuarantineReason::ALL
                    .into_iter()
                    .find(|r| q["reason"] == r.as_str())
                    .ok_or_else(|| {
                        Refusal::new(
                            "EngineContractError",
                            "RequestError: unknown quarantine reason",
                        )
                    })?,
                detail_kind: DetailKind::ALL
                    .into_iter()
                    .find(|r| q["detail_kind"] == r.as_str())
                    .ok_or_else(|| {
                        Refusal::new(
                            "EngineContractError",
                            format!(
                                "RequestError: unknown quarantine detail kind '{}'",
                                string(&q["detail_kind"])
                            ),
                        )
                    })?,
                detail: get(q, "detail")?,
            })
        })
        .collect::<Result<_, Refusal>>()?;
    let intra_audit = response_array(v, "intra_audit")?
        .iter()
        .map(|a| {
            Ok(AuditOut {
                pair: get(a, "pair")?,
                sent: get(a, "sent")?,
                received: get(a, "received")?,
                fee: get(a, "fee")?,
                sent_msat: get(a, "sent_msat")?,
                received_msat: get(a, "received_msat")?,
                fee_msat: get(a, "fee_msat")?,
                spot_price: get::<Option<Dec>>(a, "spot_price")?,
                at_regime: get::<Option<Regime>>(a, "at_regime")?,
                regime_flows: if a["regime_flows"].is_null() {
                    None
                } else {
                    let f = &a["regime_flows"];
                    Some(RegimeFlows {
                        out: Flows {
                            alt: get(&f["out"], "alt")?,
                            neu: get(&f["out"], "neu")?,
                        },
                        inbound: Flows {
                            alt: get(&f["in"], "alt")?,
                            neu: get(&f["in"], "neu")?,
                        },
                    })
                },
                at_pool: get(a, "at_pool")?,
                description: get(a, "description")?,
            })
        })
        .collect::<Result<_, Refusal>>()?;
    let holdings = response_array(v, "holdings")?
        .iter()
        .map(|h| {
            Ok(HoldingOut {
                asset: get(h, "asset")?,
                wallet: get(h, "wallet")?,
                quantity: get(h, "quantity")?,
                cost_basis: get(h, "cost_basis")?,
            })
        })
        .collect::<Result<_, Refusal>>()?;
    let tax_summary = response_array(v, "tax_summary")?
        .iter()
        .map(|s| {
            Ok(SummaryOut {
                year: get(s, "year")?,
                asset: get(s, "asset")?,
                transaction_type: match s["transaction_type"].as_str() {
                    Some("sell") => "sell",
                    Some("gift") => "gift",
                    Some("donate") => "donate",
                    Some("fee") => "fee",
                    Some("airdrop") => "airdrop",
                    Some("hardfork") => "hardfork",
                    Some("interest") => "interest",
                    Some("mining") => "mining",
                    Some("staking") => "staking",
                    Some("income") => "income",
                    Some("wages") => "wages",
                    _ => {
                        return Err(Refusal::new(
                            "EngineContractError",
                            format!(
                                "RequestError: unknown summary transaction_type {}",
                                s["transaction_type"]
                            ),
                        ))
                    }
                },
                long_term: get(s, "long_term")?,
                quantity: get(s, "quantity")?,
                proceeds: get(s, "proceeds")?,
                cost_basis: get(s, "cost_basis")?,
                gain_loss: get(s, "gain_loss")?,
            })
        })
        .collect::<Result<_, Refusal>>()?;
    let carried_rows = response_array(v, "carried_rows")?
        .iter()
        .map(|c| {
            Ok(CarriedRow {
                row: get(c, "row")?,
                value: get(c, "value")?,
                rate: c.get("rate").map(|_| get(c, "rate")).transpose()?.flatten(),
            })
        })
        .collect::<Result<_, Refusal>>()?;
    Ok(LedgerResponse {
        schema_version: 2,
        ok: true,
        engine_version: kassiber_tax_core::ENGINE_VERSION,
        entries,
        quarantines,
        intra_audit,
        tax_summary,
        holdings,
        carried_rows,
    })
}
pub fn response_refusal(
    profile: &Value,
    c: &Value,
    wallets: &[Value],
    response: &Value,
    ids: impl FnMut() -> String,
) -> Refusal {
    let inputs = LedgerInputs {
        profile,
        projection: &c["finalized_tax_projection"],
        rows: None,
        wallets,
        direct_payout_records: array(&c["direct_payout_records"]),
    };
    match crate::render::response_refusal(&inputs, response, ids) {
        Err(error) => error,
        Ok(()) => Refusal::new(
            "EngineContractError",
            "RequestError: response cannot be represented by the typed ledger boundary",
        ),
    }
}
fn long_short(v: &Value) -> Result<&'static str, Refusal> {
    match v.as_str() {
        Some("long") => Ok("long"),
        Some("short") => Ok("short"),
        _ => Err(Refusal::new(
            "EngineContractError",
            "RequestError: unknown capital gains type",
        )),
    }
}
pub fn empty_response() -> LedgerResponse {
    LedgerResponse {
        schema_version: 2,
        ok: true,
        engine_version: kassiber_tax_core::ENGINE_VERSION,
        entries: vec![],
        quarantines: vec![],
        intra_audit: vec![],
        tax_summary: vec![],
        holdings: vec![],
        carried_rows: vec![],
    }
}
pub fn staged_error(error: kassiber_tax_core::ledger::response::StagedError) -> Refusal {
    ledger_error(&serde_json::to_value(error).expect("staged error fields"))
}

pub(crate) fn ledger_error(e: &Value) -> Refusal {
    let class = string(&e["inner"]["class"]);
    let message = string(&e["inner"]["message"]);
    let family = match class.as_str() {
        "ValueError" => "EngineValueError",
        "TypeError" => "EngineTypeError",
        "RuntimeError" => "EngineRuntimeError",
        "BuiltinValueError" => "ValueError",
        "AppError" => "AppError",
        "InvalidOperation" => "InvalidOperation",
        "DivisionByZero" => "DivisionByZero",
        "Overflow" => "Overflow",
        _ => "EngineContractError",
    };
    let mut error = Refusal::new(family, message.clone());
    if family == "EngineContractError" {
        error.message = format!("{class}: {message}");
        error.error_class = Some(class.clone());
    }
    if family == "AppError" {
        error.code = Some("app_error".into());
    }
    match e["stage"].as_str() {
        Some("request" | "unsupported") => {
            error.family = "EngineContractError".into();
            error.message = format!("{class}: {message}");
            error.error_class = Some(class.clone());
            error.code = None;
            error.retryable = None;
        }
        Some("validate" | "compute_multi" | "compute") => {
            let cause = error;
            let cause_message = cause.message.clone();
            error = Refusal::new("AppError", "");
            error.cause = Some(Box::new(cause));
            error.family = "AppError".into();
            let prefix = match e["stage"].as_str() {
                Some("validate") => "RP2 cross-asset input validation failed: ".into(),
                Some("compute_multi") => "RP2 multi-asset tax calculation failed: ".into(),
                _ => format!(
                    "RP2 tax calculation failed for asset '{}': ",
                    string(&e["asset"])
                ),
            };
            error.message = format!("{prefix}{cause_message}");
            error.code = Some(
                if e["stage"] == "validate" {
                    "rp2_input_validation"
                } else {
                    "app_error"
                }
                .into(),
            );
        }
        Some("prepare" | "classify") => {
            if family == "EngineContractError" {
                error.message = format!("{class}: {message}");
            }
            if family == "AppError" {
                error.code = Some("app_error".into());
            }
        }
        _ => {
            error = Refusal::new("EngineContractError", "");
            error.error_class = Some("RequestError".into());
            error.message = format!(
                "RequestError: unknown ledger failure stage '{}'",
                string(&e["stage"])
            );
        }
    }
    error
}
/// Canonical oracle encoding, outside the measured render span.
pub fn canonical(state: &JournalState<'_>) -> Result<String, Refusal> {
    let mut v =
        serde_json::to_value(state).map_err(|e| Refusal::new("ValueError", e.to_string()))?;
    for k in ["account_holdings", "wallet_holdings"] {
        if v[k]["$pairs"].as_array().is_some_and(Vec::is_empty) {
            v[k] = json!({});
        }
    }
    pyjson(&v, true)
}
