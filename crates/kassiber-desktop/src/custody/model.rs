//! The Transfers screen's pure model: the reads it merges into one ledger of moves
//! (`linkedMovesModel.ts`), exact amounts (`satsFormat.ts`), rails (`lib/bitcoinRail.ts`), the
//! custody lineage (`custodyGapsModel.ts`) and the transfer helpers it shares with Review
//! (`components/review/transferModel.ts`). Amounts are exact msat as `i128`; nothing passes
//! through a float.

use serde_json::Value;

/// A field as text (`""` when absent).
pub fn s(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// A field as text, None when absent, null or empty.
pub fn opt(v: &Value, k: &str) -> Option<String> {
    match v.get(k) {
        Some(Value::String(x)) if !x.is_empty() => Some(x.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

pub fn f(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}

pub fn b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

/// An exact integer from a JSON number or its decimal text (`BigInt(Math.trunc(n))`).
pub fn exact(v: &Value) -> Option<i128> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .map(i128::from)
            .or_else(|| n.as_u64().map(i128::from))
            .or_else(|| {
                n.as_f64()
                    .filter(|x| x.is_finite())
                    .map(|x| x.trunc() as i128)
            }),
        Value::String(t) => t.trim().parse::<i128>().ok(),
        _ => None,
    }
}

pub fn exact_of(v: &Value, k: &str) -> Option<i128> {
    v.get(k).and_then(exact)
}

// ---- Exact amounts (satsFormat.ts) -------------------------------------------------------------

/// `formatMsatAsBtc`: `₿0.00600000`, eight decimals plus any sub-sat digits.
pub fn msat_btc(msat: i128) -> String {
    let a = msat.unsigned_abs();
    let whole = a / 100_000_000_000;
    let frac = format!("{:011}", a % 100_000_000_000);
    let sub = frac[8..].trim_end_matches('0');
    format!("₿{whole}.{}{sub}", &frac[..8])
}

/// `formatMsatAsSats`: grouped whole sats in the UI locale, a sub-sat remainder appended.
pub fn msat_sats(msat: i128) -> String {
    let neg = msat < 0;
    let a = msat.unsigned_abs();
    let de = crate::i18n::lang() == crate::i18n::Lang::De;
    let mut txt = group(&(a / 1000).to_string(), if de { '\u{a0}' } else { ',' });
    let rem = a % 1000;
    if rem != 0 {
        txt.push(if de { ',' } else { '.' });
        txt.push_str(format!("{rem:03}").trim_end_matches('0'));
    }
    format!("{}{txt} sats", if neg { "−" } else { "" })
}

fn group(digits: &str, sep: char) -> String {
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

/// `formatMsatAsAsset`: bitcoin as `₿…`, another asset as its number and code.
pub fn msat_asset(msat: i128, asset: &str, bitcoin: bool) -> String {
    if bitcoin {
        return msat_btc(msat);
    }
    let a = msat.unsigned_abs();
    let whole = a / 100_000_000_000;
    let frac = format!("{:011}", a % 100_000_000_000);
    let frac = frac.trim_end_matches('0');
    if frac.is_empty() {
        format!("{whole} {asset}")
    } else {
        format!("{whole}.{frac} {asset}")
    }
}

/// `formatMsatIn`.
pub fn msat_in(msat: i128, asset: &str) -> String {
    msat_asset(msat, asset, is_bitcoin(asset))
}

/// `BtcFigure`: the dimmed leading `₿0.0…` and the rest, when the amount is under one coin.
pub fn btc_figure_parts(msat: i128) -> (Option<String>, String) {
    let t = msat_btc(msat);
    let Some(rest) = t.strip_prefix("₿0.") else {
        return (None, t);
    };
    let zeros = rest.chars().take_while(|c| *c == '0').count();
    if zeros == rest.len() {
        return (None, t);
    }
    (
        Some(format!("₿0.{}", &rest[..zeros])),
        rest[zeros..].to_owned(),
    )
}

// ---- Assets and rails ------------------------------------------------------------------------

pub const BITCOIN_ASSETS: [&str; 5] = ["BTC", "XBT", "XXBT", "LBTC", "L-BTC"];

pub fn is_bitcoin(asset: &str) -> bool {
    BITCOIN_ASSETS.contains(&asset.trim().to_uppercase().as_str())
}

pub const BITCOIN_SWAP_KINDS: [&str; 6] = [
    "chain-swap",
    "peg-in",
    "peg-out",
    "reverse-submarine-swap",
    "submarine-swap",
    "swap-refund",
];

/// `isDifferentAssetMove`.
pub fn is_different_asset(out_asset: &str, in_asset: &str, kind: Option<&str>) -> bool {
    if out_asset.trim().eq_ignore_ascii_case(in_asset.trim()) {
        return false;
    }
    if is_bitcoin(out_asset) && is_bitcoin(in_asset) {
        return false;
    }
    !kind.is_some_and(|k| BITCOIN_SWAP_KINDS.contains(&k))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rail {
    Onchain,
    Lightning,
    Liquid,
}

impl Rail {
    pub fn key(self) -> &'static str {
        match self {
            Rail::Onchain => "onchain",
            Rail::Lightning => "lightning",
            Rail::Liquid => "liquid",
        }
    }
    /// `RAIL_DETAILS[rail].label` (hardcoded English in the web).
    pub fn label(self) -> &'static str {
        match self {
            Rail::Onchain => "On-chain",
            Rail::Lightning => "Lightning",
            Rail::Liquid => "Liquid",
        }
    }
}

/// `railForLeg`.
pub fn rail_for_leg(asset: Option<&str>, wallet_kind: Option<&str>) -> Rail {
    let a = asset.unwrap_or("").to_uppercase();
    let k = wallet_kind.unwrap_or("").to_lowercase();
    if a == "LBTC" || k.contains("liquid") {
        return Rail::Liquid;
    }
    if k.contains("phoenix")
        || k.contains("lightning")
        || k.contains("coreln")
        || k.contains("core-ln")
        || k == "lnd"
        || k == "nwc"
    {
        return Rail::Lightning;
    }
    Rail::Onchain
}

// ---- Reads ------------------------------------------------------------------------------------

/// One leg of a pair (`ui.transfers.list`).
#[derive(Clone, Debug, Default)]
pub struct PairLeg {
    pub transaction_id: String,
    pub wallet: String,
    pub wallet_kind: Option<String>,
    pub asset: String,
    pub occurred_at: String,
    pub amount_msat: i128,
    pub fiat_value: Option<f64>,
    pub unlinked_msat: Option<i128>,
}

impl PairLeg {
    fn parse(v: &Value) -> PairLeg {
        PairLeg {
            transaction_id: s(v, "transaction_id"),
            wallet: s(v, "wallet"),
            wallet_kind: opt(v, "wallet_kind"),
            asset: s(v, "asset"),
            occurred_at: s(v, "occurred_at"),
            amount_msat: exact_of(v, "amount_msat").unwrap_or(0),
            fiat_value: f(v, "fiat_value"),
            unlinked_msat: exact_of(v, "unlinked_msat"),
        }
    }
}

/// `TransferPairSummary`.
#[derive(Clone, Debug, Default)]
pub struct Pair {
    pub id: String,
    pub out_transaction_id: String,
    pub in_transaction_id: String,
    pub kind: String,
    pub policy: String,
    pub swap_fee_msat: Option<i128>,
    pub swap_fee_fiat_value: Option<f64>,
    pub pair_source: Option<String>,
    pub out_amount: Option<i128>,
    pub created_at: Option<String>,
    pub deleted_at: Option<String>,
    pub notes: Option<String>,
    pub shares_legs: Option<bool>,
    pub out: PairLeg,
    pub inn: PairLeg,
    /// The record as read, for the dialogs that need more of it.
    pub raw: Value,
}

impl Pair {
    pub fn parse(v: &Value) -> Pair {
        Pair {
            id: s(v, "id"),
            out_transaction_id: s(v, "out_transaction_id"),
            in_transaction_id: s(v, "in_transaction_id"),
            kind: s(v, "kind"),
            policy: s(v, "policy"),
            swap_fee_msat: exact_of(v, "swap_fee_msat"),
            swap_fee_fiat_value: f(v, "swap_fee_fiat_value"),
            pair_source: opt(v, "pair_source"),
            out_amount: exact_of(v, "out_amount"),
            created_at: opt(v, "created_at"),
            deleted_at: opt(v, "deleted_at"),
            notes: opt(v, "notes"),
            shares_legs: v.get("shares_legs").and_then(Value::as_bool),
            out: PairLeg::parse(&v["out"]),
            inn: PairLeg::parse(&v["in"]),
            raw: v.clone(),
        }
    }
    pub fn list(v: &Value) -> Vec<Pair> {
        v.as_array()
            .map(|a| a.iter().map(Pair::parse).collect())
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, Default)]
pub struct DismissalLeg {
    pub wallet: String,
    pub asset: String,
    pub amount_msat: i128,
    pub occurred_at: Option<String>,
}

impl DismissalLeg {
    fn parse(v: &Value) -> DismissalLeg {
        DismissalLeg {
            wallet: s(v, "wallet"),
            asset: s(v, "asset"),
            amount_msat: exact_of(v, "amount_msat").unwrap_or(0),
            occurred_at: opt(v, "occurred_at"),
        }
    }
}

/// `TransferDismissal`.
#[derive(Clone, Debug, Default)]
pub struct Dismissal {
    pub id: String,
    pub out_transaction_id: String,
    pub in_transaction_id: String,
    pub reason: Option<String>,
    pub created_at: String,
    pub active: bool,
    pub out: DismissalLeg,
    pub inn: DismissalLeg,
}

impl Dismissal {
    pub fn parse(v: &Value) -> Dismissal {
        Dismissal {
            id: s(v, "id"),
            out_transaction_id: s(v, "out_transaction_id"),
            in_transaction_id: s(v, "in_transaction_id"),
            reason: opt(v, "reason"),
            created_at: s(v, "created_at"),
            active: b(v, "active"),
            out: DismissalLeg::parse(&v["out"]),
            inn: DismissalLeg::parse(&v["in"]),
        }
    }
    pub fn list(v: &Value) -> Vec<Dismissal> {
        v.as_array()
            .map(|a| a.iter().map(Dismissal::parse).collect())
            .unwrap_or_default()
    }
}

/// `CustodyLineageItem`.
#[derive(Clone, Debug, Default)]
pub struct Lineage {
    pub out_transaction_id: String,
    pub in_transaction_id: String,
    pub occurred_at: Option<String>,
    pub asset: String,
    pub amount_msat: i128,
    pub fiat_value: Option<f64>,
    pub fee_msat: Option<i128>,
    pub fee_fiat_value: Option<f64>,
    pub from_wallet_label: String,
    pub to_wallet_label: String,
    pub custody_state: String,
    pub basis_state: String,
    pub evidence_reason: String,
    pub rail: String,
    pub source_rail: Option<String>,
    pub target_rail: Option<String>,
    pub source_asset: Option<String>,
    pub target_asset: Option<String>,
}

impl Lineage {
    pub fn parse(v: &Value) -> Lineage {
        Lineage {
            out_transaction_id: s(v, "out_transaction_id"),
            in_transaction_id: s(v, "in_transaction_id"),
            occurred_at: opt(v, "occurred_at"),
            asset: s(v, "asset"),
            amount_msat: exact_of(v, "amount_msat").unwrap_or(0),
            fiat_value: f(v, "fiat_value"),
            fee_msat: exact_of(v, "fee_msat"),
            fee_fiat_value: f(v, "fee_fiat_value"),
            from_wallet_label: s(v, "from_wallet_label"),
            to_wallet_label: s(v, "to_wallet_label"),
            custody_state: s(v, "custody_state"),
            basis_state: s(v, "basis_state"),
            evidence_reason: s(v, "evidence_reason"),
            rail: s(v, "rail"),
            source_rail: opt(v, "source_rail"),
            target_rail: opt(v, "target_rail"),
            source_asset: opt(v, "source_asset"),
            target_asset: opt(v, "target_asset"),
        }
    }
}

pub const AUTOMATIC_UNDO_DAYS: u32 = 3650;
pub const AUTOMATIC_UNDO_REASON: &str = "Automatic pairing undone from custody lineage";
pub const DISMISS_REASON: &str = "user dismissed from review queue";

/// `canUndoAutomaticPairing`.
pub fn can_undo_automatic(item: &Lineage) -> bool {
    item.custody_state == "internal_verified"
        && matches!(
            item.evidence_reason.as_str(),
            "exchange_txid_exact" | "bark_txid_exact"
        )
}

// ---- The ledger of moves (linkedMovesModel.ts) -----------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Automatic,
    Reviewed,
    Undone,
}

impl Source {
    pub fn key(self) -> &'static str {
        match self {
            Source::Automatic => "automatic",
            Source::Reviewed => "reviewed",
            Source::Undone => "undone",
        }
    }
}

#[derive(Clone, Debug)]
pub struct LinkedMove {
    pub key: String,
    pub source: Source,
    pub occurred_at: Option<String>,
    pub out_transaction_id: String,
    pub in_transaction_id: String,
    pub from_label: String,
    pub to_label: String,
    pub from_rail: Rail,
    pub to_rail: Rail,
    pub asset: String,
    pub amount_msat: i128,
    pub in_amount_msat: Option<i128>,
    pub in_occurred_at: Option<String>,
    pub fee_msat: Option<i128>,
    pub amount_fiat: Option<f64>,
    pub fee_fiat: Option<f64>,
    pub evidence_reason: Option<String>,
    pub lineage: Option<Lineage>,
    pub pair_id: Option<String>,
    pub pair_kind: Option<String>,
    pub basis_blocked: bool,
    pub undoable: bool,
    pub different_asset: bool,
    pub by_rule: bool,
    pub linked_at: Option<String>,
    pub pair_policy: Option<String>,
    pub pair_source: Option<String>,
}

pub fn move_key(out_id: &str, in_id: &str) -> String {
    format!("{out_id}\u{0}{in_id}")
}

pub fn is_undone_automatic(d: &Dismissal) -> bool {
    d.active && d.reason.as_deref() == Some(AUTOMATIC_UNDO_REASON)
}

fn lineage_rails(item: &Lineage) -> (Rail, Rail) {
    let mut parts = item.rail.split("->");
    let jf = parts.next().unwrap_or("").to_owned();
    let jt = parts
        .next()
        .map(str::to_owned)
        .unwrap_or_else(|| jf.clone());
    let pick = |a: &Option<String>, fb: &str| {
        a.clone()
            .filter(|x| !x.is_empty())
            .unwrap_or_else(|| fb.to_owned())
    };
    let from = rail_for_leg(
        Some(&pick(&item.source_asset, &item.asset)),
        Some(&pick(&item.source_rail, &jf)),
    );
    let to = rail_for_leg(
        Some(&pick(&item.target_asset, &item.asset)),
        Some(&pick(&item.target_rail, &jt)),
    );
    (from, to)
}

fn from_lineage(item: &Lineage) -> LinkedMove {
    let (from, to) = lineage_rails(item);
    LinkedMove {
        key: move_key(&item.out_transaction_id, &item.in_transaction_id),
        source: if item.custody_state == "internal_reviewed" {
            Source::Reviewed
        } else {
            Source::Automatic
        },
        occurred_at: item.occurred_at.clone(),
        out_transaction_id: item.out_transaction_id.clone(),
        in_transaction_id: item.in_transaction_id.clone(),
        from_label: item.from_wallet_label.clone(),
        to_label: item.to_wallet_label.clone(),
        from_rail: from,
        to_rail: to,
        asset: item.asset.clone(),
        amount_msat: item.amount_msat,
        in_amount_msat: None,
        in_occurred_at: None,
        fee_msat: item.fee_msat,
        amount_fiat: item.fiat_value,
        fee_fiat: item.fee_fiat_value,
        evidence_reason: Some(item.evidence_reason.clone()),
        lineage: Some(item.clone()),
        pair_id: None,
        pair_kind: None,
        basis_blocked: item.basis_state == "blocked_by_prior_custody_basis",
        undoable: can_undo_automatic(item),
        different_asset: false,
        by_rule: item.evidence_reason == "rule_auto",
        linked_at: None,
        pair_policy: None,
        pair_source: None,
    }
}

pub fn pair_is_different_asset(p: &Pair) -> bool {
    is_different_asset(&p.out.asset, &p.inn.asset, Some(&p.kind))
}

fn from_pair(p: &Pair) -> LinkedMove {
    LinkedMove {
        key: move_key(&p.out_transaction_id, &p.in_transaction_id),
        source: Source::Reviewed,
        occurred_at: Some(p.out.occurred_at.clone()).filter(|x| !x.is_empty()),
        out_transaction_id: p.out_transaction_id.clone(),
        in_transaction_id: p.in_transaction_id.clone(),
        from_label: p.out.wallet.clone(),
        to_label: p.inn.wallet.clone(),
        from_rail: rail_for_leg(Some(&p.out.asset), p.out.wallet_kind.as_deref()),
        to_rail: rail_for_leg(Some(&p.inn.asset), p.inn.wallet_kind.as_deref()),
        asset: p.out.asset.clone(),
        amount_msat: p.out.amount_msat,
        in_amount_msat: Some(p.inn.amount_msat),
        in_occurred_at: Some(p.inn.occurred_at.clone()).filter(|x| !x.is_empty()),
        fee_msat: p.swap_fee_msat,
        amount_fiat: p.out.fiat_value,
        fee_fiat: p.swap_fee_fiat_value,
        evidence_reason: None,
        lineage: None,
        pair_id: Some(p.id.clone()),
        pair_kind: Some(p.kind.clone()),
        basis_blocked: false,
        undoable: false,
        different_asset: pair_is_different_asset(p),
        by_rule: p.pair_source.as_deref() == Some("rule_auto"),
        linked_at: p.created_at.clone(),
        pair_policy: Some(p.policy.clone()),
        pair_source: p.pair_source.clone(),
    }
}

fn from_dismissal(d: &Dismissal) -> LinkedMove {
    LinkedMove {
        key: move_key(&d.out_transaction_id, &d.in_transaction_id),
        source: Source::Undone,
        occurred_at: d
            .out
            .occurred_at
            .clone()
            .or_else(|| d.inn.occurred_at.clone()),
        out_transaction_id: d.out_transaction_id.clone(),
        in_transaction_id: d.in_transaction_id.clone(),
        from_label: d.out.wallet.clone(),
        to_label: d.inn.wallet.clone(),
        from_rail: rail_for_leg(Some(&d.out.asset), None),
        to_rail: rail_for_leg(Some(&d.inn.asset), None),
        asset: d.out.asset.clone(),
        amount_msat: d.out.amount_msat,
        in_amount_msat: Some(d.inn.amount_msat),
        in_occurred_at: d.inn.occurred_at.clone(),
        fee_msat: None,
        amount_fiat: None,
        fee_fiat: None,
        evidence_reason: None,
        lineage: None,
        pair_id: None,
        pair_kind: None,
        basis_blocked: false,
        undoable: false,
        different_asset: false,
        by_rule: false,
        linked_at: None,
        pair_policy: None,
        pair_source: None,
    }
}

fn newest_first(a: &LinkedMove, b: &LinkedMove) -> std::cmp::Ordering {
    let l = a.occurred_at.as_deref().unwrap_or("");
    let r = b.occurred_at.as_deref().unwrap_or("");
    r.cmp(l).then_with(|| a.key.cmp(&b.key))
}

/// `buildLinkedMoves`.
pub fn build_linked_moves(
    lineage: &[Lineage],
    pairs: &[Pair],
    dismissals: &[Dismissal],
) -> Vec<LinkedMove> {
    let mut order: Vec<String> = vec![];
    let mut moves: std::collections::HashMap<String, LinkedMove> = Default::default();
    for item in lineage {
        let m = from_lineage(item);
        if !moves.contains_key(&m.key) {
            order.push(m.key.clone());
            moves.insert(m.key.clone(), m);
        }
    }
    for p in pairs {
        let key = move_key(&p.out_transaction_id, &p.in_transaction_id);
        if let Some(listed) = moves.get_mut(&key) {
            listed.source = Source::Reviewed;
            listed.pair_id = Some(p.id.clone());
            listed.pair_kind = Some(p.kind.clone());
            if p.swap_fee_msat.is_some_and(|f| f != 0) {
                listed.fee_msat = p.swap_fee_msat;
                listed.fee_fiat = p.swap_fee_fiat_value;
            }
            listed.in_amount_msat = Some(p.inn.amount_msat);
            listed.in_occurred_at = Some(p.inn.occurred_at.clone()).filter(|x| !x.is_empty());
            listed.linked_at = p.created_at.clone();
            listed.pair_policy = Some(p.policy.clone());
            listed.pair_source = p.pair_source.clone();
            listed.from_rail = rail_for_leg(Some(&p.out.asset), p.out.wallet_kind.as_deref());
            listed.to_rail = rail_for_leg(Some(&p.inn.asset), p.inn.wallet_kind.as_deref());
            listed.undoable = false;
            listed.different_asset = pair_is_different_asset(p);
            listed.by_rule = listed.by_rule || p.pair_source.as_deref() == Some("rule_auto");
        } else {
            order.push(key.clone());
            moves.insert(key, from_pair(p));
        }
    }
    for d in dismissals {
        if !is_undone_automatic(d) {
            continue;
        }
        let key = move_key(&d.out_transaction_id, &d.in_transaction_id);
        if moves.contains_key(&key) {
            continue;
        }
        order.push(key.clone());
        moves.insert(key, from_dismissal(d));
    }
    let mut out: Vec<LinkedMove> = order.into_iter().filter_map(|k| moves.remove(&k)).collect();
    out.sort_by(newest_first);
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Automatic,
    Reviewed,
    DifferentAsset,
}

impl Filter {
    pub const ALL: [Filter; 4] = [
        Filter::All,
        Filter::Automatic,
        Filter::Reviewed,
        Filter::DifferentAsset,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Filter::All => "all",
            Filter::Automatic => "automatic",
            Filter::Reviewed => "reviewed",
            Filter::DifferentAsset => "differentAsset",
        }
    }
}

pub fn filter_moves(moves: &[LinkedMove], filter: Filter) -> Vec<&LinkedMove> {
    moves
        .iter()
        .filter(|m| match filter {
            Filter::All => m.source != Source::Undone,
            Filter::DifferentAsset => m.source != Source::Undone && m.different_asset,
            Filter::Automatic => m.source == Source::Automatic,
            Filter::Reviewed => m.source == Source::Reviewed,
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub automatic: usize,
    pub reviewed: usize,
    pub undone: usize,
    pub different_asset: usize,
}

impl Counts {
    pub fn of(self, f: Filter) -> usize {
        match f {
            Filter::All => self.automatic + self.reviewed,
            Filter::Automatic => self.automatic,
            Filter::Reviewed => self.reviewed,
            Filter::DifferentAsset => self.different_asset,
        }
    }
}

pub fn count_moves(moves: &[LinkedMove]) -> Counts {
    let mut c = Counts::default();
    for m in moves {
        match m.source {
            Source::Automatic => c.automatic += 1,
            Source::Reviewed => c.reviewed += 1,
            Source::Undone => c.undone += 1,
        }
        if m.different_asset && m.source != Source::Undone {
            c.different_asset += 1;
        }
    }
    c
}

pub fn move_year(occurred_at: Option<&str>) -> Option<i32> {
    occurred_at
        .and_then(|s| s.get(..4))
        .and_then(|y| y.parse().ok())
}

/// Years that hold a move, newest first.
pub fn move_years(moves: &[&LinkedMove]) -> Vec<i32> {
    let mut ys: Vec<i32> = moves
        .iter()
        .filter_map(|m| move_year(m.occurred_at.as_deref()))
        .collect();
    ys.sort_unstable_by(|a, b| b.cmp(a));
    ys.dedup();
    ys
}

/// `withoutHeld`.
pub fn without_held(moves: Vec<LinkedMove>, held: &[Dismissal]) -> Vec<LinkedMove> {
    if held.is_empty() {
        return moves;
    }
    let keys: std::collections::HashSet<String> = held
        .iter()
        .map(|d| move_key(&d.out_transaction_id, &d.in_transaction_id))
        .collect();
    moves
        .into_iter()
        .filter(|m| !(m.source == Source::Undone && keys.contains(&m.key)))
        .collect()
}

/// `heldUndonePairings`.
pub fn held_undone(
    dismissals: &[Dismissal],
    held_ids: &std::collections::HashSet<String>,
) -> Vec<Dismissal> {
    dismissals
        .iter()
        .filter(|d| {
            is_undone_automatic(d)
                && (held_ids.contains(&d.out_transaction_id)
                    || held_ids.contains(&d.in_transaction_id))
        })
        .cloned()
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Loading,
    Stale,
    Unavailable,
    SortOut { total: usize },
    Settled,
    Empty,
}

/// `transfersVerdict`.
pub fn verdict(
    loading: bool,
    unavailable: bool,
    stale: bool,
    held: usize,
    suggested: usize,
    linked: usize,
) -> Verdict {
    if loading {
        Verdict::Loading
    } else if stale {
        Verdict::Stale
    } else if unavailable {
        Verdict::Unavailable
    } else if held + suggested > 0 {
        Verdict::SortOut {
            total: held + suggested,
        }
    } else if linked > 0 {
        Verdict::Settled
    } else {
        Verdict::Empty
    }
}

/// `RecentlyUnlinked`.
#[derive(Clone, Debug)]
pub struct Unlinked {
    pub key: String,
    pub out_transaction_id: String,
    pub in_transaction_id: String,
    pub from_label: String,
    pub to_label: String,
    pub from_rail: Rail,
    pub to_rail: Rail,
    pub occurred_at: Option<String>,
    pub asset: String,
    pub amount_msat: i128,
    pub unlinked_at: Option<String>,
    pub pair: Option<Pair>,
    pub dismissal: Option<Dismissal>,
}

/// `recentlyUnlinked`.
pub fn recently_unlinked(
    unlinked: &[Pair],
    dismissals: &[Dismissal],
    held: &[Dismissal],
    linked: &[LinkedMove],
) -> Vec<Unlinked> {
    let taken: std::collections::HashSet<&str> = linked
        .iter()
        .filter(|m| m.source != Source::Undone)
        .flat_map(|m| [m.out_transaction_id.as_str(), m.in_transaction_id.as_str()])
        .collect();
    let held_keys: std::collections::HashSet<String> = held
        .iter()
        .map(|d| move_key(&d.out_transaction_id, &d.in_transaction_id))
        .collect();
    let mut rows: Vec<Unlinked> = vec![];
    let has = |rows: &Vec<Unlinked>, k: &str| rows.iter().any(|r| r.key == k);
    for p in unlinked {
        let key = move_key(&p.out_transaction_id, &p.in_transaction_id);
        if has(&rows, &key)
            || taken.contains(p.out_transaction_id.as_str())
            || taken.contains(p.in_transaction_id.as_str())
        {
            continue;
        }
        rows.push(Unlinked {
            key,
            out_transaction_id: p.out_transaction_id.clone(),
            in_transaction_id: p.in_transaction_id.clone(),
            from_label: p.out.wallet.clone(),
            to_label: p.inn.wallet.clone(),
            from_rail: rail_for_leg(Some(&p.out.asset), p.out.wallet_kind.as_deref()),
            to_rail: rail_for_leg(Some(&p.inn.asset), p.inn.wallet_kind.as_deref()),
            occurred_at: Some(p.out.occurred_at.clone()).filter(|x| !x.is_empty()),
            asset: p.out.asset.clone(),
            amount_msat: p.out.amount_msat,
            unlinked_at: p.deleted_at.clone(),
            pair: Some(p.clone()),
            dismissal: None,
        });
    }
    for d in dismissals {
        if !is_undone_automatic(d) {
            continue;
        }
        let key = move_key(&d.out_transaction_id, &d.in_transaction_id);
        if has(&rows, &key)
            || held_keys.contains(&key)
            || taken.contains(d.out_transaction_id.as_str())
            || taken.contains(d.in_transaction_id.as_str())
        {
            continue;
        }
        rows.push(Unlinked {
            key,
            out_transaction_id: d.out_transaction_id.clone(),
            in_transaction_id: d.in_transaction_id.clone(),
            from_label: d.out.wallet.clone(),
            to_label: d.inn.wallet.clone(),
            from_rail: rail_for_leg(Some(&d.out.asset), None),
            to_rail: rail_for_leg(Some(&d.inn.asset), None),
            occurred_at: d
                .out
                .occurred_at
                .clone()
                .or_else(|| d.inn.occurred_at.clone()),
            asset: d.out.asset.clone(),
            amount_msat: d.out.amount_msat,
            unlinked_at: Some(d.created_at.clone()),
            pair: None,
            dismissal: Some(d.clone()),
        });
    }
    rows.sort_by(|a, b| {
        let l = a.unlinked_at.as_deref().unwrap_or("");
        let r = b.unlinked_at.as_deref().unwrap_or("");
        r.cmp(l).then_with(|| a.key.cmp(&b.key))
    });
    rows
}

/// `splitWalletName`: the part that may be cut and the whole-word end (≤ 14 chars) that stays.
pub fn split_wallet_name(label: &str) -> (String, String) {
    const TAIL: usize = 14;
    if label.chars().count() <= TAIL + 4 {
        return (label.to_owned(), String::new());
    }
    let words: Vec<&str> = label.split(' ').collect();
    let mut cut = words.len();
    while cut > 1 && words[cut - 1..].join(" ").chars().count() <= TAIL {
        cut -= 1;
    }
    if cut == words.len() {
        return (label.to_owned(), String::new());
    }
    (
        words[..cut].join(" "),
        format!(" {}", words[cut..].join(" ")),
    )
}

/// `moveType` of a linked move: the key under `review:transfersPage.type.*`.
pub fn move_type(pair_kind: Option<&str>, from: Rail, to: Rail) -> &'static str {
    match pair_kind {
        Some("peg-in") => return "pegIn",
        Some("peg-out") => return "pegOut",
        Some("submarine-swap") => return "submarine",
        Some("reverse-submarine-swap") => return "reverseSubmarine",
        Some("chain-swap") => return "chainSwap",
        Some("swap-refund") => return "refund",
        Some("coinjoin" | "whirlpool") => return "coinjoin",
        _ => {}
    }
    if from == to {
        "transfer"
    } else if to == Rail::Liquid && from != Rail::Lightning {
        "pegIn"
    } else if from == Rail::Liquid && to != Rail::Lightning {
        "pegOut"
    } else if to == Rail::Lightning {
        "submarine"
    } else if from == Rail::Lightning {
        "reverseSubmarine"
    } else {
        "transfer"
    }
}

pub fn move_is_taxable(m: &LinkedMove) -> bool {
    m.pair_policy.as_deref() == Some("taxable")
}

pub fn can_unlink(m: &LinkedMove) -> bool {
    m.pair_id.is_some() || m.undoable
}

/// `automaticLinkKind`.
pub fn automatic_link_kind(evidence: Option<&str>) -> &'static str {
    match evidence.unwrap_or("") {
        "row_matched" | "verified_native_transfer" => "sameTransaction",
        "recorded_fanout" => "fanout",
        "multi_source_consolidation" => "consolidation",
        "channel_lifecycle" => "channel",
        "exchange_txid_exact" => "exchange",
        "bark_txid_exact" => "bark",
        _ => "chain",
    }
}

/// `fixedLinkKind`.
pub fn fixed_link_kind(evidence: Option<&str>) -> &'static str {
    match automatic_link_kind(evidence) {
        "exchange" | "bark" => "chain",
        k => k,
    }
}

/// `setAsideSuggestions`.
pub fn set_aside(dismissals: &[Dismissal]) -> Vec<Dismissal> {
    let mut out: Vec<Dismissal> = dismissals
        .iter()
        .filter(|d| d.active && d.reason.as_deref() == Some(DISMISS_REASON))
        .cloned()
        .collect();
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    out
}

/// `lineageEvidenceLabel`: the `custodyGaps:lineage.evidence.*` key.
pub fn lineage_evidence_key(evidence: &str, custody_state: &str) -> &'static str {
    match evidence {
        "recorded_fanout" => "custodyGaps:lineage.evidence.recordedFanout",
        "multi_source_consolidation" => "custodyGaps:lineage.evidence.multiSourceConsolidation",
        "row_matched" => "custodyGaps:lineage.evidence.rowMatched",
        "verified_native_transfer" => "custodyGaps:lineage.evidence.nativeTransfer",
        "exchange_txid_exact" => "custodyGaps:lineage.evidence.exchangeTxid",
        "bark_txid_exact" => "custodyGaps:lineage.evidence.barkTxid",
        "reviewed_transfer_pair" | "manual" | "bulk_exact" | "bulk_selected" | "rule_auto" => {
            "custodyGaps:lineage.evidence.reviewedTransfer"
        }
        "reviewed_gap_bridge" => "custodyGaps:lineage.evidence.reviewedGapBridge",
        "channel_lifecycle" => "custodyGaps:lineage.evidence.channelLifecycle",
        _ if custody_state == "internal_reviewed" => "custodyGaps:lineage.evidence.reviewedDefault",
        _ => "custodyGaps:lineage.evidence.verifiedDefault",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn btc_text_is_exact() {
        assert_eq!(msat_btc(600_000_000), "₿0.00600000");
        assert_eq!(msat_btc(600_000_123), "₿0.00600000123");
        assert_eq!(msat_btc(-150_000_000_000), "₿1.50000000");
        assert_eq!(msat_asset(12_050_000_000_000, "USDT", false), "120.5 USDT");
        assert_eq!(
            btc_figure_parts(600_000_000),
            (Some("₿0.00".into()), "600000".into())
        );
        assert_eq!(btc_figure_parts(150_000_000_000).0, None);
    }

    #[test]
    fn sats_text() {
        assert_eq!(msat_sats(1_234_567_000), "1,234,567 sats");
        assert_eq!(msat_sats(1_500), "1.5 sats");
    }

    #[test]
    fn splits_wallet_names() {
        assert_eq!(
            split_wallet_name("Regtest Liquid Treasury 2024 Rotation"),
            ("Regtest Liquid Treasury".into(), " 2024 Rotation".into())
        );
        assert_eq!(split_wallet_name("Short"), ("Short".into(), "".into()));
    }

    #[test]
    fn merges_reviewed_pairs_once() {
        let lin = Lineage::parse(
            &json!({"out_transaction_id":"a","in_transaction_id":"b","occurred_at":"2024-01-01T00:00:00Z","asset":"BTC","amount_msat":"1000","custody_state":"internal_reviewed","evidence_reason":"manual","rail":"bitcoin","basis_state":"eligible","from_wallet_label":"A","to_wallet_label":"B"}),
        );
        let pair = Pair::parse(
            &json!({"id":"p","out_transaction_id":"a","in_transaction_id":"b","kind":"manual","policy":"carrying-value","swap_fee_msat":0,"out":{"transaction_id":"a","wallet":"A","asset":"BTC","occurred_at":"2024-01-01T00:00:00Z","amount_msat":1000},"in":{"transaction_id":"b","wallet":"B","asset":"BTC","occurred_at":"2024-01-01T00:00:00Z","amount_msat":1000}}),
        );
        let moves = build_linked_moves(&[lin], &[pair], &[]);
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].source, Source::Reviewed);
        assert_eq!(moves[0].pair_id.as_deref(), Some("p"));
    }

    #[test]
    fn move_types_from_rails() {
        assert_eq!(move_type(None, Rail::Onchain, Rail::Liquid), "pegIn");
        assert_eq!(move_type(None, Rail::Liquid, Rail::Onchain), "pegOut");
        assert_eq!(move_type(None, Rail::Onchain, Rail::Lightning), "submarine");
        assert_eq!(
            move_type(Some("chain-swap"), Rail::Onchain, Rail::Liquid),
            "chainSwap"
        );
    }

    #[test]
    fn verdict_order() {
        assert_eq!(verdict(false, true, true, 1, 0, 0), Verdict::Stale);
        assert_eq!(verdict(false, true, false, 1, 0, 0), Verdict::Unavailable);
        assert_eq!(
            verdict(false, false, false, 1, 2, 0),
            Verdict::SortOut { total: 3 }
        );
        assert_eq!(verdict(false, false, false, 0, 0, 3), Verdict::Settled);
    }

    #[test]
    fn automatic_undo_covers_exchange_and_bark_only() {
        let lineage = |reason: &str, state: &str| {
            Lineage::parse(&json!({
                "out_transaction_id": "out-bark",
                "in_transaction_id": "in-store",
                "occurred_at": "2026-11-13T11:51:50Z",
                "asset": "BTC",
                "amount_msat": "600000000",
                "custody_state": state,
                "evidence_reason": reason,
                "rail": "bitcoin",
                "basis_state": "eligible",
                "from_wallet_label": "BTCPay Bark",
                "to_wallet_label": "Store on-chain"
            }))
        };
        assert!(can_undo_automatic(&lineage(
            "exchange_txid_exact",
            "internal_verified"
        )));
        assert!(can_undo_automatic(&lineage(
            "bark_txid_exact",
            "internal_verified"
        )));
        assert!(!can_undo_automatic(&lineage(
            "row_matched",
            "internal_verified"
        )));
        assert!(!can_undo_automatic(&lineage(
            "bark_txid_exact",
            "internal_reviewed"
        )));
        let moves =
            build_linked_moves(&[lineage("bark_txid_exact", "internal_verified")], &[], &[]);
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].source, Source::Automatic);
        assert!(moves[0].undoable);
        assert!(can_unlink(&moves[0]));
    }
}
