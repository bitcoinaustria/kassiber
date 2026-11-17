//! The closed output schema of replay bundles: every canonical output table
//! and column, and every field of a decoded entry calculation, with a typed
//! class. The book's own outputs (stored, or recomputed from the real
//! snapshot) are mapped through the input projection's keyed ids; the
//! recomputation from the projected inputs is checked against the values
//! that projection generated. A cell that does not parse, or a column,
//! table or calculation field the schema does not name, is refused, never
//! copied.
use crate::projection::Projection;
use crate::typed;
use kassiber_journals::reproduce::{Cell, Row, Table};
use kassiber_pyjson::{Key, PyStr, Value as Py};
use std::collections::{BTreeMap, BTreeSet};

/// Why a cell was refused: the table and column only, no value.
#[derive(Debug, PartialEq)]
pub struct Unsafe {
    pub table: String,
    pub column: String,
}

pub enum Mode<'a, 'p> {
    /// The book's own outputs: identifiers and enum misses through the
    /// projection's mapping.
    Map(&'a Projection<'p>),
    /// The recomputation from projected inputs: identifiers must be ones
    /// the projection generated (or engine constants).
    Validate(&'a BTreeSet<String>),
    /// A recipient re-running a bundle: identifiers are the bundle's own.
    Verify,
}

#[derive(Clone, Copy)]
enum O {
    Id,
    LocationId,
    Enum(&'static [&'static str]),
    /// A quarantine reason (`registry::quarantine_reason`).
    Reason,
    /// An enum the book may author (account codes): list, else mapped.
    Authored(&'static [&'static str]),
    Int,
    Real,
    Amount,
    Time,
    Calculation,
}

const ASSETS: &[&str] = &[
    "BTC", "LBTC", "L-BTC", "btc", "lbtc", "l-btc", "LN", "SAT", "SATS",
];
const ENTRY_TYPES: &[&str] = &[
    "acquisition",
    "disposal",
    "fee",
    "income",
    "transfer_fee",
    "transfer_in",
    "transfer_out",
];
const SUMMARY_TYPES: &[&str] = &[
    "sell", "gift", "donate", "fee", "move", "airdrop", "hardfork", "interest", "mining",
    "staking", "income", "wages",
];
const GAINS_TYPES: &[&str] = &["long", "short"];
const AT_CATEGORIES: &[&str] = &[
    "alt",
    "alt_spekulation",
    "alt_taxfree",
    "income_capital_yield",
    "income_general",
    "neu",
    "neu_gain",
    "neu_loss",
    "neu_swap",
];
const ACCOUNT_CODES: &[&str] = &["treasury"];
const LOCATION_KINDS: &[&str] = &[
    "wallet",
    "external",
    "external_origin",
    "fee",
    "custody_suspense",
];
/// Engine-named locations that carry no book data.
const LOCATION_CONSTANTS: &[&str] = &[
    "unmatched_outbound_default",
    "unclassified_origin",
    "network_fee",
];
/// The reasons the custody and journal stages write; the engine's and the
/// countries' come from their own lists.
const QUARANTINE_REASONS: &[&str] = &[
    "basis_provenance_incomplete",
    "custody_basis_barrier",
    "custody_quantity_unresolved",
    "insufficient_lots",
    "missing_fee_price",
    "missing_spot_price",
    "privacy_hop_unresolved",
    "pricing_review_required",
    "transfer_fee_implausible",
    "at_swap_price_required",
    "rail_policy_required",
    "source_overlap_quantity_unresolved",
    "transfer_pair_dependency_blocked",
    "custody_gap_review_required",
    "custody_interpreter_blocked",
    "capacity_source_suspense_required",
    "synthetic_import_duplicate_review_required",
    "book_reset_decision_held",
    "custody_component_allocation_required",
    "custody_component_authored_active_invalid",
    "custody_component_claim_compile",
    "custody_component_evidence_drift",
    "custody_component_fee_orphaned",
    "custody_component_fields",
    "custody_component_leg_role_unknown",
    "custody_component_validation",
    "custody_component_value_only_loss_unsupported",
];

struct Spec {
    table: &'static str,
    columns: &'static [(&'static str, O)],
    key: &'static [&'static str],
    transaction: Option<&'static str>,
    wallet: Option<&'static str>,
    year: Option<&'static str>,
}

const SPECS: &[Spec] = &[
    Spec {
        table: "journal_entries",
        columns: &[
            ("transaction_id", O::Id),
            ("wallet_id", O::Id),
            ("account_id", O::Id),
            ("occurred_at", O::Time),
            ("entry_type", O::Enum(ENTRY_TYPES)),
            ("asset", O::Enum(ASSETS)),
            ("quantity", O::Int),
            ("fiat_value", O::Real),
            ("fiat_value_exact", O::Amount),
            ("unit_cost", O::Real),
            ("unit_cost_exact", O::Amount),
            ("cost_basis", O::Real),
            ("cost_basis_exact", O::Amount),
            ("proceeds", O::Real),
            ("proceeds_exact", O::Amount),
            ("gain_loss", O::Real),
            ("gain_loss_exact", O::Amount),
            ("at_category", O::Enum(AT_CATEGORIES)),
            ("at_kennzahl", O::Int),
            ("capital_gains_type", O::Enum(GAINS_TYPES)),
            ("calculation", O::Calculation),
        ],
        key: &["transaction_id", "wallet_id", "entry_type", "asset"],
        transaction: Some("transaction_id"),
        wallet: Some("wallet_id"),
        year: Some("occurred_at"),
    },
    Spec {
        table: "journal_tax_summary",
        columns: &[
            ("year", O::Int),
            ("asset", O::Enum(ASSETS)),
            ("transaction_type", O::Enum(SUMMARY_TYPES)),
            ("capital_gains_type", O::Enum(GAINS_TYPES)),
            ("quantity", O::Int),
            ("proceeds", O::Real),
            ("cost_basis", O::Real),
            ("gain_loss", O::Real),
        ],
        key: &["year", "asset", "transaction_type", "capital_gains_type"],
        transaction: None,
        wallet: None,
        year: Some("year"),
    },
    Spec {
        table: "journal_wallet_holdings",
        columns: &[
            ("wallet_id", O::Id),
            ("account_code", O::Authored(ACCOUNT_CODES)),
            ("asset", O::Enum(ASSETS)),
            ("quantity", O::Int),
            ("cost_basis", O::Real),
        ],
        key: &["wallet_id", "account_code", "asset"],
        transaction: None,
        wallet: Some("wallet_id"),
        year: None,
    },
    Spec {
        table: "journal_account_holdings",
        columns: &[
            ("account_id", O::Id),
            ("account_code", O::Authored(ACCOUNT_CODES)),
            ("asset", O::Enum(ASSETS)),
            ("quantity", O::Int),
            ("cost_basis", O::Real),
        ],
        key: &["account_id", "account_code", "asset"],
        transaction: None,
        wallet: None,
        year: None,
    },
    Spec {
        table: "journal_quantity_balances",
        columns: &[
            ("location_kind", O::Enum(LOCATION_KINDS)),
            ("location_id", O::LocationId),
            ("asset", O::Enum(ASSETS)),
            ("amount_msat", O::Int),
        ],
        key: &["location_kind", "location_id", "asset"],
        transaction: None,
        wallet: None,
        year: None,
    },
    Spec {
        table: "journal_quarantines",
        columns: &[("reason", O::Reason)],
        key: &[],
        transaction: None,
        wallet: None,
        year: None,
    },
];

/// The decoded calculation's fields: `(name, class)`, nested by object.
#[derive(Clone, Copy)]
enum C {
    Int,
    Amount,
    Time,
    Bool,
    Id,
    Enum(&'static [&'static str]),
    /// A tax-country registry value, printed again from its constant.
    Parsed(fn(&str) -> Option<String>),
    /// Derived engine identity (embeds observation digests): left out.
    Skip,
    Obj(&'static [(&'static str, C)]),
    List(&'static [(&'static str, C)]),
}

const PRICING: &[(&str, C)] = &[
    ("fiat_currency", C::Parsed(crate::registry::fiat)),
    ("fiat_rate_exact", C::Amount),
    ("fiat_value_exact", C::Amount),
    (
        "pricing_source_kind",
        C::Enum(&[
            "manual",
            "market",
            "import",
            "exchange",
            "provider",
            "reviewed",
            "cache",
            "fallback",
            "transaction",
            "receipt",
            "rates_cache",
            "manual_override",
        ]),
    ),
    (
        "pricing_quality",
        C::Enum(&[
            "exact",
            "coarse",
            "estimated",
            "derived",
            "fallback",
            "missing",
            "approximate",
            "hourly",
            "daily",
        ]),
    ),
    (
        "fiat_price_source",
        C::Enum(&[
            "coingecko",
            "kraken",
            "coinbase",
            "binance",
            "bitpanda",
            "bitstamp",
            "ecb",
            "mempool",
            "reviewed-receipt",
            "manual",
            "import",
        ]),
    ),
    ("pricing_timestamp", C::Time),
    (
        "pricing_provider",
        C::Enum(&[
            "coingecko",
            "kraken",
            "coinbase",
            "binance",
            "bitpanda",
            "bitstamp",
            "ecb",
            "mempool",
            "reviewed-receipt",
            "manual",
            "import",
        ]),
    ),
    (
        "pricing_pair",
        C::Enum(&[
            "BTC-EUR", "BTC-USD", "BTC-CHF", "BTC-GBP", "BTCEUR", "BTCUSD",
        ]),
    ),
    ("pricing_fetched_at", C::Time),
    (
        "pricing_granularity",
        C::Enum(&[
            "exact",
            "minute",
            "hourly",
            "hour",
            "daily",
            "day",
            "transaction",
        ]),
    ),
    (
        "pricing_method",
        C::Enum(&[
            "quarantine_price_override",
            "import",
            "manual",
            "nearest",
            "interpolated",
            "close",
            "rates_cache",
        ]),
    ),
];
const SOURCE: &[(&str, C)] = &[
    ("transaction_id", C::Id),
    ("engine_event_id", C::Skip),
    ("occurred_at", C::Time),
    ("pricing", C::Obj(PRICING)),
    ("asset", C::Enum(ASSETS)),
    ("spot_price_exact", C::Amount),
    ("fiat_fee_exact", C::Amount),
    ("crypto_fee_msat", C::Int),
    ("crypto_fee_msat_exact", C::Amount),
];
const FRAGMENT: &[(&str, C)] = &[
    ("quantity_msat", C::Int),
    ("quantity_msat_exact", C::Amount),
    ("cost_basis_exact", C::Amount),
    ("proceeds_exact", C::Amount),
    ("gain_loss_exact", C::Amount),
    ("expenses_exact", C::Amount),
    ("unit_basis_override_exact", C::Amount),
    (
        "basis_authority",
        C::Enum(&["engine_unit_override", "acquired_lot", "income", "engine"]),
    ),
    ("acquisition_basis_exact", C::Amount),
    ("acquisition_quantity_msat", C::Int),
    ("acquisition_quantity_msat_exact", C::Amount),
    ("event", C::Obj(SOURCE)),
    ("lot", C::Obj(SOURCE)),
];
const CALCULATION: &[(&str, C)] = &[
    ("schema_version", C::Int),
    ("engine", C::Enum(&["kassiber_tax"])),
    ("method", C::Parsed(crate::registry::method)),
    ("fragments", C::List(FRAGMENT)),
    ("loss_deductible", C::Bool),
    ("valuation", C::Enum(&["market_value", "seventy_percent"])),
];

struct Context<'m, 'a, 'p> {
    mode: &'m Mode<'a, 'p>,
    table: &'static str,
    column: &'static str,
}

impl Context<'_, '_, '_> {
    fn refuse(&self) -> Unsafe {
        Unsafe {
            table: self.table.to_owned(),
            column: self.column.to_owned(),
        }
    }

    fn id(&self, text: &str) -> Result<String, Unsafe> {
        match self.mode {
            Mode::Map(projection) => Ok(projection.id(text)),
            Mode::Validate(generated) if generated.contains(text) => Ok(text.to_owned()),
            Mode::Validate(_) => Err(self.refuse()),
            Mode::Verify => Ok(text.to_owned()),
        }
    }

    fn location(&self, text: &str) -> Result<String, Unsafe> {
        if let Some(constant) = LOCATION_CONSTANTS.iter().find(|c| **c == text) {
            return Ok((*constant).to_owned());
        }
        let parts: Vec<&str> = text.split(':').collect();
        if parts.len() == 4 && parts[0] == "component" && parts[2] == "allocation" {
            return Ok(format!(
                "component:{}:allocation:{}",
                self.id(parts[1])?,
                self.id(parts[3])?
            ));
        }
        self.id(text)
    }

    fn enumerated(&self, values: &'static [&'static str], text: &str) -> Result<String, Unsafe> {
        values
            .iter()
            .find(|v| **v == text)
            .map(|v| (*v).to_owned())
            .ok_or_else(|| self.refuse())
    }

    fn authored(&self, values: &'static [&'static str], text: &str) -> Result<String, Unsafe> {
        if let Ok(value) = self.enumerated(values, text) {
            return Ok(value);
        }
        match self.mode {
            Mode::Map(projection) => Ok(projection.enum_value(text)),
            Mode::Validate(generated) if generated.contains(text) => Ok(text.to_owned()),
            Mode::Validate(_) => Err(self.refuse()),
            Mode::Verify => Ok(text.to_owned()),
        }
    }

    fn cell(&self, class: O, cell: &Cell) -> Result<Cell, Unsafe> {
        Ok(match (class, cell) {
            (_, Cell::Null) => Cell::Null,
            (O::Int, Cell::Int(n)) => Cell::Int(*n),
            (O::Real, Cell::Real(bits)) => Cell::Real(*bits),
            (O::Real, Cell::Int(n)) => Cell::real(*n as f64),
            (O::Amount, Cell::Text(text)) => {
                Cell::Text(typed::amount(text).ok_or_else(|| self.refuse())?)
            }
            (O::Time, Cell::Text(text)) => {
                Cell::Text(typed::time(text).ok_or_else(|| self.refuse())?)
            }
            (O::Id, Cell::Text(text)) => Cell::Text(self.id(text)?),
            (O::LocationId, Cell::Text(text)) => Cell::Text(self.location(text)?),
            (O::Enum(values), Cell::Text(text)) => Cell::Text(self.enumerated(values, text)?),
            (O::Reason, Cell::Text(text)) => Cell::Text(
                crate::registry::quarantine_reason(QUARANTINE_REASONS, text)
                    .ok_or_else(|| self.refuse())?,
            ),
            (O::Authored(values), Cell::Text(text)) => Cell::Text(self.authored(values, text)?),
            (O::Calculation, Cell::Text(text)) => Cell::Text(self.calculation(text)?),
            _ => return Err(self.refuse()),
        })
    }

    fn calculation(&self, text: &str) -> Result<String, Unsafe> {
        let parsed = kassiber_pytext::loads(text).map_err(|_| self.refuse())?;
        let projected = self.calc_object(CALCULATION, &parsed)?;
        kassiber_pyjson::dumps(
            &projected,
            &kassiber_pyjson::Options {
                sort_keys: true,
                ..Default::default()
            },
        )
        .map_err(|_| self.refuse())
    }

    fn calc_object(&self, schema: &'static [(&'static str, C)], value: &Py) -> Result<Py, Unsafe> {
        let Py::Dict(items) = value else {
            return Err(self.refuse());
        };
        let mut out = Vec::new();
        for (key, value) in items {
            let Key::Str(name) = key else {
                return Err(self.refuse());
            };
            let name = name.as_str().ok_or_else(|| self.refuse())?;
            let (name, class) = schema
                .iter()
                .find(|(n, _)| *n == name)
                .ok_or_else(|| self.refuse())?;
            if matches!(class, C::Skip) {
                continue;
            }
            out.push((Key::from(*name), self.calc_value(*class, value)?));
        }
        Ok(Py::Dict(out))
    }

    fn calc_value(&self, class: C, value: &Py) -> Result<Py, Unsafe> {
        let text = |value: &Py| match value {
            Py::Str(text) => text
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| self.refuse()),
            _ => Err(self.refuse()),
        };
        let string = |text: String| Py::Str(PyStr::from(text.as_str()));
        Ok(match (class, value) {
            (_, Py::Null) => Py::Null,
            (C::Int, Py::Int(n)) => Py::Int(
                n.to_string()
                    .parse::<i64>()
                    .map_err(|_| self.refuse())?
                    .into(),
            ),
            (C::Int, Py::Str(_)) => string(typed::int(&text(value)?).ok_or_else(|| self.refuse())?),
            (C::Bool, Py::Bool(b)) => Py::Bool(*b),
            (C::Amount, Py::Str(_)) => {
                string(typed::amount(&text(value)?).ok_or_else(|| self.refuse())?)
            }
            (C::Time, Py::Str(_)) => {
                string(typed::time(&text(value)?).ok_or_else(|| self.refuse())?)
            }
            (C::Id, Py::Str(_)) => string(self.id(&text(value)?)?),
            (C::Enum(values), Py::Str(_)) => string(self.enumerated(values, &text(value)?)?),
            (C::Parsed(parse), Py::Str(_)) => {
                string(parse(&text(value)?).ok_or_else(|| self.refuse())?)
            }
            (C::Obj(schema), Py::Dict(_)) => self.calc_object(schema, value)?,
            (C::List(schema), Py::List(items)) => Py::List(
                items
                    .iter()
                    .map(|item| self.calc_object(schema, item))
                    .collect::<Result<_, _>>()?,
            ),
            _ => return Err(self.refuse()),
        })
    }
}

/// Project canonical output tables through the closed output schema.
pub fn project(tables: &[Table], mode: &Mode<'_, '_>) -> Result<Vec<Table>, Unsafe> {
    let mut out = Vec::with_capacity(tables.len());
    for table in tables {
        let spec = SPECS
            .iter()
            .find(|s| s.table == table.name)
            .ok_or_else(|| Unsafe {
                table: "unknown".into(),
                column: "table".into(),
            })?;
        let mut rows = Vec::with_capacity(table.rows.len());
        for row in &table.rows {
            let mut columns = BTreeMap::new();
            for (name, cell) in &row.columns {
                let (name, class) =
                    spec.columns
                        .iter()
                        .find(|(n, _)| n == name)
                        .ok_or_else(|| Unsafe {
                            table: spec.table.into(),
                            column: "unknown".into(),
                        })?;
                let context = Context {
                    mode,
                    table: spec.table,
                    column: name,
                };
                columns.insert((*name).to_owned(), context.cell(*class, cell)?);
            }
            let text = |name: &str| match columns.get(name) {
                Some(Cell::Text(text)) => Some(text.clone()),
                Some(Cell::Int(n)) => Some(n.to_string()),
                _ => None,
            };
            let key = if spec.key.is_empty() {
                // Quarantines are keyed by their transaction.
                let base = row.key.split('#').next().unwrap_or_default();
                Context {
                    mode,
                    table: spec.table,
                    column: "transaction_id",
                }
                .id(base)?
            } else {
                spec.key
                    .iter()
                    .map(|k| text(k).unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join("|")
            };
            let wallet = spec.wallet.and_then(text).or_else(|| {
                (spec.table == "journal_quantity_balances"
                    && columns.get("location_kind") == Some(&Cell::text("wallet")))
                .then(|| text("location_id"))
                .flatten()
            });
            let year = spec.year.and_then(text).and_then(|t| {
                let year: String = t.chars().take(4).collect();
                (year.len() == 4 && year.bytes().all(|b| b.is_ascii_digit())).then_some(year)
            });
            let transaction = if spec.key.is_empty() {
                Some(key.clone())
            } else {
                spec.transaction.and_then(text)
            };
            rows.push(Row {
                key,
                transaction_id: transaction,
                wallet_id: wallet,
                year,
                columns,
            });
        }
        out.push(Table::new(spec.table, rows));
    }
    Ok(out)
}
