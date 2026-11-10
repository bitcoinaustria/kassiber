//! The Greenfield reads of `sync_btcpay.py`: the wallet-history probe,
//! invoice provenance over the incremental page checkpoint, and payouts.
//! Every request runs under the caller's grant; nothing here writes a book.
use super::client::{app_error, quote, Context, Failure, Greenfield};
use super::normalize::{self, either, field, text};
use super::payment_methods as methods;
use kassiber_envelope::{dict, get, s, truthy, AppError};
use kassiber_net::Consent;
use kassiber_pyjson::{Key, PyStr, Value};
use std::time::Instant;

type Result<T> = std::result::Result<T, AppError>;

pub const DEFAULT_PAGE_SIZE: i64 = 100;
const MAX_PAGES: i64 = 10_000;
const UNCHANGED_PAGE_WINDOW: i64 = 5;
const DEEP_AUDIT_PAGES: i64 = 1;
pub const WALLET_HISTORY_PERMISSION_HINT: &str = "Greenfield wallet endpoints require the read-only `btcpay.store.canviewwallet` permission on BTCPay Server 2.4 and later, or `btcpay.store.canmodifystoresettings` on 2.3 and earlier. Keep this store as invoice provenance with a read-only key, or map it to a watch-only wallet you already track.";
const INVOICE_PERMISSION_HINT: &str = "Grant the API key the BTCPay 'View invoices' permission.";

/// `emit_sync_progress`: a refusal (a cancelled job) stops the read.
pub type Progress<'a> = &'a dyn Fn(&Value) -> Result<()>;

/// The caller's grant and overall deadline for one action.
#[derive(Clone, Copy)]
pub struct Reads<'a> {
    pub client: &'a Greenfield,
    pub consent: &'a Consent,
    pub deadline: Instant,
    /// `emit_sync_progress`: a freshness job persists each event; a refusal
    /// (a cancelled job) stops the read.
    pub progress: Option<Progress<'a>>,
}

impl Reads<'_> {
    /// `_fetch_incremental_pages.load_page`: progress around one page read.
    fn page(
        &self,
        loaded: &mut i64,
        page_size: i64,
        read: impl FnOnce() -> Result<Vec<Value>>,
    ) -> Result<Vec<Value>> {
        let Some(progress) = self.progress else {
            return read();
        };
        progress(&dict([("phase", s("backend_fetch"))]))?;
        let page = read()?;
        *loaded += 1;
        progress(&dict([
            ("phase", s("backend_fetch")),
            ("pages_fetched", Value::Int((*loaded).into())),
            ("page_size", Value::Int(page_size.into())),
        ]))?;
        Ok(page)
    }
    fn get(&self, path: &str, query: &[(&str, String)]) -> std::result::Result<Value, Failure> {
        self.client.get_url(
            &self.client.url(path, query),
            Some(self.consent),
            self.deadline,
        )
    }
    pub fn json(&self, path: &str, query: &[(&str, String)], ctx: Context<'_>) -> Result<Value> {
        self.client
            .get_json(path, query, ctx, Some(self.consent), self.deadline)
    }
    pub fn optional(
        &self,
        path: &str,
        query: &[(&str, String)],
        tolerate: &[u16],
    ) -> Result<(Value, Option<super::client::HttpFailure>)> {
        self.client
            .get_optional(path, query, tolerate, Some(self.consent), self.deadline)
    }
}

fn json_array(payload: Value, label: &str) -> Result<Vec<Value>> {
    match payload {
        Value::List(items) => Ok(items),
        _ => Err(AppError::new(
            "protocol_error",
            format!("BTCPay response for {label} was not a JSON array"),
        )),
    }
}

fn store_path(store_id: &str) -> String {
    format!("/api/v1/stores/{}", quote(store_id))
}

fn wallet_transactions_path(store_id: &str, method: &str, legacy: bool) -> String {
    if legacy {
        let (currency, _) = methods::split(method);
        format!(
            "{}/payment-methods/onchain/{}/wallet/transactions",
            store_path(store_id),
            quote(&currency)
        )
    } else {
        format!(
            "{}/payment-methods/{}/wallet/transactions",
            store_path(store_id),
            quote(method)
        )
    }
}

/// One wallet-history page (`_wallet_page_fetcher`), falling back to the
/// BTCPay 1.x path on an unnamed 404. `legacy` carries that discovery.
pub fn wallet_page(
    reads: Reads<'_>,
    store_id: &str,
    method: &str,
    skip: i64,
    limit: i64,
    legacy: &mut bool,
) -> Result<Vec<Value>> {
    let query = [
        ("statusFilter", "Confirmed".to_owned()),
        ("skip", skip.to_string()),
        ("limit", limit.to_string()),
    ];
    let ctx = Context {
        context: Some("wallet history"),
        permission_hint: Some(WALLET_HISTORY_PERMISSION_HINT),
        not_found_hint: None,
    };
    let page = match reads.get(&wallet_transactions_path(store_id, method, *legacy), &query) {
        Ok(page) => page,
        Err(Failure::App(error)) => return Err(error),
        Err(Failure::Http(failure))
            if failure.status == 404
                && !*legacy
                && failure.error_code().is_none()
                && method != methods::BARK_PAYMENT_METHOD_ID =>
        {
            match reads.get(&wallet_transactions_path(store_id, method, true), &query) {
                Ok(page) => {
                    *legacy = true;
                    page
                }
                Err(Failure::App(error)) => return Err(error),
                Err(Failure::Http(legacy_failure)) => {
                    let chosen = if legacy_failure.status == 404 {
                        &failure
                    } else {
                        &legacy_failure
                    };
                    return Err(app_error(chosen, ctx));
                }
            }
        }
        Err(Failure::Http(failure)) => return Err(app_error(&failure, ctx)),
    };
    json_array(page, "wallet transactions")
}

/// `probe_btcpay_wallet`: one wallet-history request, no pagination.
pub fn probe_wallet(reads: Reads<'_>, store_id: &str, method: &str) -> Result<Value> {
    let page = wallet_page(reads, store_id, method, 0, 1, &mut false)?;
    Ok(dict([
        ("checked", Value::Bool(true)),
        ("rows_seen", Value::Int(page.len().into())),
    ]))
}

/// `probe_btcpay_instance`: one store-catalog request.
pub fn probe_instance(reads: Reads<'_>) -> Result<usize> {
    let stores = reads.json(
        "/api/v1/stores",
        &[],
        Context {
            context: Some("the store list"),
            permission_hint: Some("Grant the API key access to view stores."),
            not_found_hint: None,
        },
    )?;
    Ok(json_array(stores, "stores")?.len())
}

// --- Incremental page checkpoint --------------------------------------------------------

fn int_key(key: &Value) -> Option<i64> {
    match key {
        Value::Int(n) => i64::try_from(n.clone()).ok(),
        Value::Str(t) => t
            .as_str()
            .and_then(|t| kassiber_pytext::parse_int(t).ok())
            .and_then(|n| i64::try_from(n).ok()),
        _ => None,
    }
}

/// `_sorted_page_map`: dict entries only, ordered by numeric skip.
fn sorted_pages(pages: &Value) -> Vec<(PyStr, Value)> {
    let Value::Dict(pairs) = pages else {
        return Vec::new();
    };
    let mut entries: Vec<(i64, PyStr, Value)> = Vec::new();
    for (key, value) in pairs {
        let key = match key {
            Key::Str(k) => k.clone(),
            other => PyStr::from(format!("{other:?}").as_str()),
        };
        if !matches!(value, Value::Dict(_)) {
            continue;
        }
        let order = int_key(&Value::Str(key.clone())).unwrap_or(MAX_PAGES * DEFAULT_PAGE_SIZE);
        if let Some(slot) = entries.iter_mut().find(|(_, k, _)| *k == key) {
            slot.2 = value.clone();
        } else {
            entries.push((order, key, value.clone()));
        }
    }
    entries.sort_by_key(|(order, _, _)| *order);
    entries.into_iter().map(|(_, k, v)| (k, v)).collect()
}

fn positive(value: &Value, default: i64, maximum: i64) -> i64 {
    match int_key(value).or_else(|| match value {
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Float(f) if f.is_finite() => Some(f.trunc() as i64),
        _ => None,
    }) {
        Some(n) if n > 0 => n.min(maximum),
        _ => default,
    }
}

/// Detects a page that must be re-read even when its checkpoint matches.
pub type ForceChanged<'a> = &'a dyn Fn(&[Value]) -> bool;

pub struct PageFns<'a> {
    pub stable_ids: &'a dyn Fn(&[Value]) -> Result<Value>,
    pub fingerprint: &'a dyn Fn(&[Value]) -> Result<String>,
    pub force_changed: Option<ForceChanged<'a>>,
}

pub struct Paged {
    pub records: Vec<Value>,
    pub pages: Value,
    pub metadata: Value,
}

struct Walk<'a, 'b> {
    fetch: &'a mut dyn FnMut(i64) -> Result<Vec<Value>>,
    normalize_page: &'a mut dyn FnMut(&[Value]) -> Result<Vec<Value>>,
    fns: &'a PageFns<'b>,
    page_size: i64,
    previous: Vec<(PyStr, Value)>,
    next: Vec<(PyStr, Value)>,
    records: Vec<Value>,
    pages_fetched: i64,
    fetched: std::collections::BTreeSet<i64>,
    changed: Vec<i64>,
    unchanged: Vec<i64>,
    stopped_by_known: bool,
    streak: i64,
    max_pages_message: &'a str,
}

impl Walk<'_, '_> {
    fn load(&mut self, skip: i64) -> Result<Vec<Value>> {
        if self.pages_fetched >= MAX_PAGES {
            return Err(AppError::new("config_error", self.max_pages_message));
        }
        let page = (self.fetch)(skip)?;
        self.pages_fetched += 1;
        self.fetched.insert(skip);
        Ok(page)
    }

    fn process(&mut self, skip: i64, page: &[Value]) -> Result<()> {
        let key = PyStr::from(skip.to_string().as_str());
        let stable_ids = (self.fns.stable_ids)(page)?;
        let fingerprint = (self.fns.fingerprint)(page)?;
        let before = self
            .previous
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.clone());
        let same = before.as_ref().is_some_and(|before| {
            matches!(get(before, "stable_ids"), Some(Value::List(_)))
                && get(before, "stable_ids") == Some(&stable_ids)
                && get(before, "fingerprint") == Some(&s(fingerprint.as_str()))
                && get(before, "page_size") == Some(&Value::Int(self.page_size.into()))
        }) && !self.fns.force_changed.is_some_and(|force| force(page));
        let checkpoint = if same {
            self.stopped_by_known = true;
            self.unchanged.push(skip);
            self.streak += 1;
            before.expect("previous page")
        } else {
            let normalized = (self.normalize_page)(page)?;
            self.records.extend(normalized);
            self.changed.push(skip);
            self.streak = 0;
            dict([
                ("fingerprint", s(fingerprint)),
                ("stable_ids", stable_ids),
                ("rows", Value::Int(page.len().into())),
                ("page_size", Value::Int(self.page_size.into())),
            ])
        };
        if let Some(slot) = self.next.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = checkpoint;
        } else {
            self.next.push((key, checkpoint));
        }
        Ok(())
    }
}

/// `_fetch_incremental_pages`.
pub fn incremental_pages(
    fetch: &mut dyn FnMut(i64) -> Result<Vec<Value>>,
    normalize_page: &mut dyn FnMut(&[Value]) -> Result<Vec<Value>>,
    fns: &PageFns<'_>,
    page_size: i64,
    previous_pages: &Value,
    previous_pagination: &Value,
    max_pages_message: &str,
) -> Result<Paged> {
    let previous = sorted_pages(previous_pages);
    let empty = dict::<&str>([]);
    let pagination = if matches!(previous_pagination, Value::Dict(_)) {
        previous_pagination
    } else {
        &empty
    };
    let unchanged_window = positive(
        field(pagination, "unchanged_page_window"),
        UNCHANGED_PAGE_WINDOW,
        100,
    );
    let deep_audit_pages = positive(field(pagination, "deep_audit_pages"), DEEP_AUDIT_PAGES, 20);
    let mut walk = Walk {
        fetch,
        normalize_page,
        fns,
        page_size,
        next: previous.clone(),
        previous,
        records: Vec::new(),
        pages_fetched: 0,
        fetched: Default::default(),
        changed: Vec::new(),
        unchanged: Vec::new(),
        stopped_by_known: false,
        streak: 0,
        max_pages_message,
    };
    let mut terminal: Option<i64> = None;
    let mut skip = 0;
    let stop_reason;
    loop {
        let page = walk.load(skip)?;
        walk.process(skip, &page)?;
        if (page.len() as i64) < page_size {
            terminal = Some(skip);
            stop_reason = "end_of_results";
            break;
        }
        if !walk.previous.is_empty() && walk.streak >= unchanged_window {
            stop_reason = "unchanged_page_window";
            break;
        }
        skip += page_size;
    }
    let mut deep_audit = Value::Null;
    let mut next_deep = Value::Null;
    if stop_reason == "unchanged_page_window" && deep_audit_pages > 0 {
        let minimum = skip + page_size;
        let saved = int_key(field(pagination, "next_deep_audit_skip"));
        let mut audit_skip = saved.filter(|s| *s >= minimum).unwrap_or(minimum);
        let start = audit_skip;
        let mut audited = 0;
        let mut audit_stop = "deep_audit_window";
        while audited < deep_audit_pages {
            if walk.fetched.contains(&audit_skip) {
                audit_skip += page_size;
                continue;
            }
            let page = walk.load(audit_skip)?;
            walk.process(audit_skip, &page)?;
            audited += 1;
            if (page.len() as i64) < page_size {
                terminal = Some(terminal.map_or(audit_skip, |t| t.min(audit_skip)));
                audit_stop = "end_of_results";
                audit_skip = minimum;
                break;
            }
            audit_skip += page_size;
        }
        next_deep = Value::Int(audit_skip.into());
        deep_audit = dict([
            ("start_skip", Value::Int(start.into())),
            ("pages", Value::Int(audited.into())),
            ("stop_reason", s(audit_stop)),
            ("next_skip", Value::Int(audit_skip.into())),
        ]);
    }
    let mut next = walk.next;
    if let Some(terminal) = terminal {
        next.retain(|(key, _)| int_key(&Value::Str(key.clone())).is_none_or(|n| n <= terminal));
    }
    let mut pagination_out = vec![
        ("unchanged_page_window", Value::Int(unchanged_window.into())),
        ("deep_audit_pages", Value::Int(deep_audit_pages.into())),
        ("last_stop_reason", s(stop_reason)),
        ("next_deep_audit_skip", next_deep),
    ];
    if !matches!(deep_audit, Value::Null) {
        pagination_out.push(("last_deep_audit", deep_audit.clone()));
    }
    let ints =
        |items: Vec<i64>| Value::List(items.into_iter().map(|n| Value::Int(n.into())).collect());
    let pages_value = Value::Dict(
        sorted_pages(&Value::Dict(
            next.into_iter().map(|(k, v)| (Key::Str(k), v)).collect(),
        ))
        .into_iter()
        .map(|(k, v)| (Key::Str(k), v))
        .collect(),
    );
    Ok(Paged {
        records: walk.records,
        pages: pages_value,
        metadata: dict([
            ("pages_fetched", Value::Int(walk.pages_fetched.into())),
            ("stopped_by_known_page", Value::Bool(walk.stopped_by_known)),
            ("stop_reason", s(stop_reason)),
            ("changed_pages", ints(walk.changed)),
            ("unchanged_pages", ints(walk.unchanged)),
            ("deep_audit", deep_audit),
            ("pagination", dict(pagination_out)),
        ]),
    })
}

// --- Invoice provenance -----------------------------------------------------------------

/// `fetch_btcpay_invoice_provenance`: normalized invoices plus the metadata
/// the caller stores as its checkpoint.
pub fn invoice_provenance(
    reads: Reads<'_>,
    store_id: &PyStr,
    page_size: i64,
    checkpoint: &Value,
) -> Result<(Vec<Value>, Value)> {
    let store = super::url::from_py(store_id);
    if store.is_empty() {
        return Err(AppError::new("validation", "BTCPay store id is required"));
    }
    if page_size <= 0 {
        return Err(AppError::new(
            "validation",
            "BTCPay page_size must be positive",
        ));
    }
    let path = format!("{}/invoices", store_path(&store));
    let fetch = |skip: i64| -> Result<Vec<Value>> {
        let page = reads.json(
            &path,
            &[
                ("skip", skip.to_string()),
                ("take", page_size.to_string()),
                ("includePaymentMethods", "true".into()),
            ],
            Context {
                context: Some("invoices"),
                permission_hint: Some(INVOICE_PERMISSION_HINT),
                not_found_hint: None,
            },
        )?;
        json_array(page, "invoices")
    };
    let mut hydrated_count: i64 = 0;
    let mut titles: Option<Vec<(PyStr, PyStr)>> = None;
    let mut normalize_page = |page: &[Value]| -> Result<Vec<Value>> {
        let mut hydrated = Vec::new();
        for invoice in page {
            let Value::Dict(_) = invoice else {
                hydrated.push(invoice.clone());
                continue;
            };
            if let Value::List(list) = field(invoice, "paymentMethods") {
                let mut copy = invoice.clone();
                kassiber_envelope::set(
                    &mut copy,
                    "payments",
                    normalize::payments_from_methods(list),
                );
                hydrated.push(copy);
                continue;
            }
            if matches!(field(invoice, "payments"), Value::List(items) if !items.is_empty()) {
                hydrated.push(invoice.clone());
                continue;
            }
            let id = either(&[field(invoice, "id"), field(invoice, "invoiceId")]);
            if !truthy(id) || !normalize::invoice_may_have_payments(invoice) {
                hydrated.push(invoice.clone());
                continue;
            }
            let methods_payload = reads.json(
                &format!(
                    "{}/invoices/{}/payment-methods",
                    store_path(&store),
                    quote(&super::url::from_py(&text(id)))
                ),
                &[("onlyAccountedPayments", "true".into())],
                Context {
                    context: Some("invoice payments"),
                    permission_hint: Some(INVOICE_PERMISSION_HINT),
                    not_found_hint: None,
                },
            )?;
            let list = json_array(methods_payload, "invoice payment methods")?;
            hydrated_count += 1;
            let mut copy = invoice.clone();
            kassiber_envelope::set(&mut copy, "paymentMethods", Value::List(list.clone()));
            kassiber_envelope::set(
                &mut copy,
                "payments",
                normalize::payments_from_methods(&list),
            );
            hydrated.push(copy);
        }
        let none = |_: &PyStr| None;
        let mut normalized = hydrated
            .iter()
            .map(|invoice| normalize::normalize_invoice(store_id, invoice, &none))
            .collect::<Result<Vec<_>>>()?;
        let unlabeled: Vec<usize> = normalized
            .iter()
            .enumerate()
            .filter(|(_, invoice)| {
                field(invoice, "origin_kind") == &s("payment_request")
                    && field(invoice, "origin_label") == field(invoice, "payment_request_id")
            })
            .map(|(index, _)| index)
            .collect();
        if !unlabeled.is_empty() {
            if titles.is_none() {
                let (payload, error) = reads.optional(
                    &format!("{}/payment-requests", store_path(&store)),
                    &[],
                    &[403, 404],
                )?;
                let mut loaded = Vec::new();
                if let (None, Value::List(items)) = (error, payload) {
                    for item in items {
                        let id = field(&item, "id");
                        let title = field(&item, "title");
                        if matches!(item, Value::Dict(_)) && truthy(id) && truthy(title) {
                            let id = text(id);
                            if let Some(slot) = loaded
                                .iter_mut()
                                .find(|(k, _): &&mut (PyStr, PyStr)| *k == id)
                            {
                                slot.1 = text(title);
                            } else {
                                loaded.push((id, text(title)));
                            }
                        }
                    }
                }
                titles = Some(loaded);
            }
            let known = titles.as_ref().expect("titles");
            for index in unlabeled {
                let request = text(field(&normalized[index], "payment_request_id"));
                if let Some((_, title)) = known.iter().find(|(id, _)| *id == request) {
                    if !title.is("") {
                        kassiber_envelope::set(
                            &mut normalized[index],
                            "origin_label",
                            Value::Str(title.clone()),
                        );
                    }
                }
            }
        }
        Ok(normalized)
    };
    let fns = PageFns {
        stable_ids: &normalize::invoice_page_ids,
        fingerprint: &normalize::invoice_page_fingerprint,
        force_changed: Some(&normalize::invoice_page_has_open),
    };
    let mut loaded = 0;
    let mut paced = |skip: i64| reads.page(&mut loaded, page_size, || fetch(skip));
    let paged = incremental_pages(
        &mut paced,
        &mut normalize_page,
        &fns,
        page_size,
        field(checkpoint, "btcpay_invoice_pages"),
        field(checkpoint, "btcpay_invoice_pagination"),
        &format!("BTCPay invoice sync exceeded {MAX_PAGES} pages; aborting for safety"),
    )?;
    let meta = &paged.metadata;
    let metadata = dict([
        ("btcpay_invoice_pages", paged.pages.clone()),
        (
            "btcpay_invoice_pagination",
            field(meta, "pagination").clone(),
        ),
        ("pages_fetched", field(meta, "pages_fetched").clone()),
        (
            "stopped_by_known_page",
            field(meta, "stopped_by_known_page").clone(),
        ),
        ("stop_reason", field(meta, "stop_reason").clone()),
        ("deep_audit", field(meta, "deep_audit").clone()),
        ("changed_pages", field(meta, "changed_pages").clone()),
        ("invoices_hydrated", Value::Int(hydrated_count.into())),
    ]);
    Ok((paged.records, metadata))
}

/// `fetch_btcpay_payouts`: refunds, pull-payment claims and store payouts. A
/// key without the payouts permission yields none, reported in the metadata.
pub fn payouts(reads: Reads<'_>, store_id: &PyStr) -> Result<(Vec<Value>, Value)> {
    let store = super::url::from_py(store_id);
    if store.is_empty() {
        return Err(AppError::new("validation", "BTCPay store id is required"));
    }
    let (payload, error) = reads.optional(
        &format!("{}/payouts", store_path(&store)),
        &[("includeCancelled", "false".into())],
        &[403, 404],
    )?;
    if let Some(error) = error {
        return Ok((
            Vec::new(),
            dict([
                ("payouts_available", Value::Bool(false)),
                (
                    "payouts_permission_missing",
                    Value::Bool(error.status == 403),
                ),
                ("payouts_seen", Value::Int(0.into())),
            ]),
        ));
    }
    let payouts = json_array(payload, "payouts")?;
    let mut pulls: Vec<(PyStr, Value)> = Vec::new();
    if payouts
        .iter()
        .any(|p| matches!(p, Value::Dict(_)) && truthy(field(p, "pullPaymentId")))
    {
        let (payload, error) = reads.optional(
            &format!("{}/pull-payments", store_path(&store)),
            &[("includeArchived", "true".into())],
            &[403, 404],
        )?;
        if let (None, Value::List(items)) = (error, payload) {
            for item in items {
                if matches!(item, Value::Dict(_)) && truthy(field(&item, "id")) {
                    let id = text(field(&item, "id"));
                    if let Some(slot) = pulls.iter_mut().find(|(k, _)| *k == id) {
                        slot.1 = item;
                    } else {
                        pulls.push((id, item));
                    }
                }
            }
        }
    }
    let lookup = |id: &PyStr| pulls.iter().find(|(k, _)| k == id).map(|(_, v)| v.clone());
    let mut normalized = Vec::new();
    for payout in &payouts {
        if !matches!(payout, Value::Dict(_)) {
            return Err(AppError::new(
                "protocol_error",
                "BTCPay payout record was not a JSON object",
            ));
        }
        if let Some(record) = normalize::normalize_payout(store_id, payout, &lookup)? {
            normalized.push(record);
        }
    }
    let seen = normalized.len();
    Ok((
        normalized,
        dict([
            ("payouts_available", Value::Bool(true)),
            ("payouts_permission_missing", Value::Bool(false)),
            ("payouts_seen", Value::Int(seen.into())),
        ]),
    ))
}

// --- Wallet history ---------------------------------------------------------------------

/// `_is_confirmed_transaction`.
fn confirmed(tx: &Value) -> Result<bool> {
    let Value::Dict(_) = tx else {
        return Err(AppError::new(
            "protocol_error",
            "BTCPay transaction record was not a JSON object",
        ));
    };
    let confirmations = field(tx, "confirmations");
    if !matches!(confirmations, Value::Null) && *confirmations != s("") {
        let invalid = || {
            AppError::new(
                "protocol_error",
                format!(
                    "Invalid BTCPay confirmations value '{}'",
                    kassiber_envelope::py_str(confirmations)
                ),
            )
        };
        let count = match confirmations {
            Value::Bool(b) => i64::from(*b),
            Value::Int(n) => return Ok(*n > 0.into()),
            Value::Float(n) if n.is_nan() => return Err(invalid()),
            Value::Float(n) if n.is_infinite() => {
                return Err(AppError::new(
                    "internal_error",
                    "cannot convert float infinity to integer",
                ))
            }
            Value::Float(n) => n.trunc() as i64,
            Value::Str(text) => {
                let text = text.as_str().ok_or_else(invalid)?;
                return kassiber_pytext::parse_int(text)
                    .map(|n| n > 0.into())
                    .map_err(|_| invalid());
            }
            _ => return Err(invalid()),
        };
        return Ok(count > 0);
    }
    let status = field(tx, "status");
    let status = if truthy(status) {
        kassiber_pytext::lower(kassiber_pytext::strip(&kassiber_envelope::py_str(status)))
    } else {
        String::new()
    };
    Ok(status.is_empty() || status == "confirmed")
}

/// `_to_record`'s CSV-shaped row, before `normalize_btcpay_record`.
fn wallet_row(tx: &Value, method: &str) -> Result<Value> {
    let currency = if method.is_empty() {
        "BTC".to_owned()
    } else {
        kassiber_pytext::upper(method.split('-').next().unwrap_or_default())
    };
    let timestamp = field(tx, "timestamp");
    if matches!(timestamp, Value::Null) {
        return Err(AppError::new(
            "protocol_error",
            "BTCPay transaction is missing 'timestamp'",
        ));
    }
    let occurred = normalize::unix_to_iso(timestamp)?;
    let or_empty = |key: &str| {
        let value = field(tx, key);
        if truthy(value) {
            value.clone()
        } else {
            s("")
        }
    };
    let amount = match field(tx, "amount") {
        Value::Null => "0".to_owned(),
        value => kassiber_envelope::py_str(value),
    };
    Ok(dict([
        ("TransactionId", or_empty("transactionHash")),
        ("Timestamp", s(occurred.as_str())),
        ("confirmed_at", s(occurred.as_str())),
        ("Currency", s(currency)),
        ("Amount", s(amount)),
        ("Comment", or_empty("comment")),
        ("Labels", normalize::parse_labels(field(tx, "labels"))),
    ]))
}

/// `fetch_btcpay_records`: confirmed wallet-history rows (CSV-shaped, before
/// the importer's normalization) and the page metadata kept as checkpoint.
pub fn wallet_records(
    reads: Reads<'_>,
    store_id: &PyStr,
    method: &str,
    page_size: i64,
    checkpoint: &Value,
) -> Result<(Vec<Value>, Value)> {
    let store = super::url::from_py(store_id);
    if store.is_empty() {
        return Err(AppError::new("validation", "BTCPay store id is required"));
    }
    let method = methods::require_wallet_history(method, method)?;
    if page_size <= 0 {
        return Err(AppError::new(
            "validation",
            "BTCPay page_size must be positive",
        ));
    }
    let previous_pagination = field(checkpoint, "btcpay_pagination");
    let mut legacy = truthy(field(previous_pagination, "legacy_wallet_paths"));
    let paged = {
        let mut loaded = 0;
        let mut fetch = |skip: i64| {
            reads.page(&mut loaded, page_size, || {
                wallet_page(reads, &store, &method, skip, page_size, &mut legacy)
            })
        };
        let mut normalize_page = |page: &[Value]| -> Result<Vec<Value>> {
            let mut rows = Vec::new();
            for tx in page {
                if confirmed(tx)? {
                    rows.push(wallet_row(tx, &method)?);
                }
            }
            Ok(rows)
        };
        let fns = PageFns {
            stable_ids: &normalize::wallet_page_ids,
            fingerprint: &normalize::wallet_page_fingerprint,
            force_changed: None,
        };
        incremental_pages(
            &mut fetch,
            &mut normalize_page,
            &fns,
            page_size,
            field(checkpoint, "btcpay_pages"),
            previous_pagination,
            &format!("BTCPay sync exceeded {MAX_PAGES} pages; aborting for safety"),
        )?
    };
    let meta = &paged.metadata;
    let mut pagination = field(meta, "pagination").clone();
    if legacy {
        kassiber_envelope::set(&mut pagination, "legacy_wallet_paths", Value::Bool(true));
    }
    let metadata = dict([
        ("btcpay_pages", paged.pages.clone()),
        ("btcpay_pagination", pagination),
        ("pages_fetched", field(meta, "pages_fetched").clone()),
        (
            "stopped_by_known_page",
            field(meta, "stopped_by_known_page").clone(),
        ),
        ("stop_reason", field(meta, "stop_reason").clone()),
        ("deep_audit", field(meta, "deep_audit").clone()),
        ("changed_pages", field(meta, "changed_pages").clone()),
    ]);
    Ok((paged.records, metadata))
}

/// `fetch_btcpay_records` for `BTC-ARK`: the Bark plugin's confirmed rows,
/// unchanged. `check` books each one as `bark_records` would, page by page,
/// so a row Kassiber cannot book stops the walk where Python's does.
pub fn bark_wallet_rows(
    reads: Reads<'_>,
    store_id: &PyStr,
    page_size: i64,
    checkpoint: &Value,
    check: &mut dyn FnMut(&Value) -> Result<()>,
) -> Result<(Vec<Value>, Value)> {
    let store = super::url::from_py(store_id);
    if store.is_empty() {
        return Err(AppError::new("validation", "BTCPay store id is required"));
    }
    let method = methods::BARK_PAYMENT_METHOD_ID;
    if page_size <= 0 {
        return Err(AppError::new(
            "validation",
            "BTCPay page_size must be positive",
        ));
    }
    let previous_pagination = field(checkpoint, "btcpay_pagination");
    let mut legacy = truthy(field(previous_pagination, "legacy_wallet_paths"));
    let paged = {
        let mut loaded = 0;
        let mut fetch = |skip: i64| {
            reads.page(&mut loaded, page_size, || {
                wallet_page(reads, &store, method, skip, page_size, &mut legacy)
            })
        };
        let mut normalize_page = |page: &[Value]| -> Result<Vec<Value>> {
            let mut rows = Vec::new();
            for tx in page {
                if confirmed(tx)? {
                    check(tx)?;
                    rows.push(tx.clone());
                }
            }
            Ok(rows)
        };
        let fns = PageFns {
            stable_ids: &super::bark::page_ids,
            fingerprint: &super::bark::page_fingerprint,
            force_changed: None,
        };
        incremental_pages(
            &mut fetch,
            &mut normalize_page,
            &fns,
            page_size,
            field(checkpoint, "btcpay_pages"),
            previous_pagination,
            &format!("BTCPay sync exceeded {MAX_PAGES} pages; aborting for safety"),
        )?
    };
    let meta = &paged.metadata;
    let mut pagination = field(meta, "pagination").clone();
    if legacy {
        kassiber_envelope::set(&mut pagination, "legacy_wallet_paths", Value::Bool(true));
    }
    let metadata = dict([
        ("btcpay_pages", paged.pages.clone()),
        ("btcpay_pagination", pagination),
        ("pages_fetched", field(meta, "pages_fetched").clone()),
        (
            "stopped_by_known_page",
            field(meta, "stopped_by_known_page").clone(),
        ),
        ("stop_reason", field(meta, "stop_reason").clone()),
        ("deep_audit", field(meta, "deep_audit").clone()),
        ("changed_pages", field(meta, "changed_pages").clone()),
    ]);
    Ok((paged.records, metadata))
}

/// `bark_wallet_history_status`: `None` when the store's Bark plugin serves
/// wallet history to this key; otherwise `not_served` (a plugin before 0.10,
/// or Bark not set up), `missing_permission` or `unavailable`.
pub fn bark_wallet_history_status(
    reads: Reads<'_>,
    store_id: &str,
) -> Result<Option<&'static str>> {
    let path = format!(
        "/api/v1/stores/{}/payment-methods/{}/wallet",
        super::client::quote(store_id),
        methods::BARK_PAYMENT_METHOD_ID
    );
    match reads.optional(&path, &[], &[403, 404]) {
        Ok((_, None)) => Ok(None),
        Ok((_, Some(error))) => Ok(Some(if error.status == 403 {
            "missing_permission"
        } else {
            "not_served"
        })),
        // barkd unreachable (BTCPay answers 503 until it is back).
        Err(error) if error.retryable => Ok(Some("unavailable")),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod wallet_history_tests {
    use super::*;

    fn py(text: &str) -> Value {
        kassiber_pytext::loads(text).unwrap()
    }

    #[test]
    fn confirmations_then_status_decide_and_bad_values_refuse() {
        for (raw, expected) in [
            (r#"{"confirmations": 3}"#, true),
            (r#"{"confirmations": 0, "status": "Confirmed"}"#, false),
            (r#"{"confirmations": "2"}"#, true),
            (r#"{"confirmations": " 1 "}"#, true),
            (r#"{"confirmations": 0.9}"#, false),
            (r#"{"confirmations": true}"#, true),
            (r#"{"confirmations": "", "status": " Confirmed "}"#, true),
            (r#"{"status": "pending"}"#, false),
            (r#"{}"#, true),
        ] {
            assert_eq!(confirmed(&py(raw)).unwrap(), expected, "{raw}");
        }
        let error = confirmed(&py(r#"{"confirmations": "2.0"}"#)).unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.message.as_str()),
            ("protocol_error", "Invalid BTCPay confirmations value '2.0'")
        );
        assert_eq!(confirmed(&py("[1]")).unwrap_err().code, "protocol_error");
    }

    #[test]
    fn wallet_rows_take_the_method_currency_and_python_text() {
        let row = wallet_row(
            &py(r#"{"transactionHash": "", "amount": 0.25, "timestamp": 1700000000, "labels": ["a"]}"#),
            "lbtc-CHAIN",
        )
        .unwrap();
        assert_eq!(
            kassiber_pyjson::dumps(&row, &Default::default()).unwrap(),
            r#"{"TransactionId": "", "Timestamp": "2023-11-14T22:13:20Z", "confirmed_at": "2023-11-14T22:13:20Z", "Currency": "LBTC", "Amount": "0.25", "Comment": "", "Labels": ["a"]}"#
        );
        let row = wallet_row(&py(r#"{"timestamp": 1700000000}"#), "").unwrap();
        assert_eq!(field(&row, "Currency"), &s("BTC"));
        assert_eq!(field(&row, "Amount"), &s("0"));
        let error = wallet_row(&py(r#"{"amount": 1}"#), "BTC-CHAIN").unwrap_err();
        assert_eq!(error.message, "BTCPay transaction is missing 'timestamp'");
    }
}
