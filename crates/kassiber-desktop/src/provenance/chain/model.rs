//! The chain analysis data model and its pure helpers, ported from `lib/chainAnalysis.ts`,
//! `lib/chainAnalysisNavigation.ts`, `lib/chainAnalysisWatchScope.ts`,
//! `lib/chainAnalysisWorkbench.ts` and `components/evidence-graph/{layout,viewport}.ts`.
//!
//! Results stay as the daemon's JSON (`Value`) where the screen only shows them; the graph's
//! nodes and edges are parsed once per result.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value, json};

// ---- Query --------------------------------------------------------------------------------

/// `AnalysisQuery`.
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub mode: String,
    pub subject: Option<String>,
    pub target: Option<String>,
    pub chain: Option<String>,
    pub network: Option<String>,
    pub direction: String,
    pub depth: i64,
    pub node_limit: i64,
    pub edge_limit: i64,
    pub include_relations: bool,
    pub include_hypotheses: bool,
    pub observer: String,
    pub min_amount_msat: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
}

impl Default for Query {
    /// `DEFAULT_ANALYSIS_QUERY`.
    fn default() -> Self {
        Query {
            mode: "overview".into(),
            subject: None,
            target: None,
            chain: None,
            network: None,
            direction: "both".into(),
            depth: 4,
            node_limit: 400,
            edge_limit: 1200,
            include_relations: true,
            include_hypotheses: false,
            observer: "owner".into(),
            min_amount_msat: None,
            start: None,
            end: None,
        }
    }
}

fn opt_str(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_owned)
}

impl Query {
    /// A query as the daemon or a saved case returns it.
    pub fn from_json(v: &Value) -> Query {
        let d = Query::default();
        Query {
            mode: opt_str(v, "mode").unwrap_or(d.mode),
            subject: opt_str(v, "subject"),
            target: opt_str(v, "target"),
            chain: opt_str(v, "chain"),
            network: opt_str(v, "network"),
            direction: opt_str(v, "direction").unwrap_or(d.direction),
            depth: v.get("depth").and_then(Value::as_i64).unwrap_or(d.depth),
            node_limit: v
                .get("node_limit")
                .and_then(Value::as_i64)
                .unwrap_or(d.node_limit),
            edge_limit: v
                .get("edge_limit")
                .and_then(Value::as_i64)
                .unwrap_or(d.edge_limit),
            include_relations: v
                .get("include_relations")
                .and_then(Value::as_bool)
                .unwrap_or(d.include_relations),
            include_hypotheses: v
                .get("include_hypotheses")
                .and_then(Value::as_bool)
                .unwrap_or(d.include_hypotheses),
            observer: opt_str(v, "observer").unwrap_or(d.observer),
            min_amount_msat: opt_str(v, "min_amount_msat"),
            start: opt_str(v, "start"),
            end: opt_str(v, "end"),
        }
    }

    /// The query as JSON, undefined fields left out (what `JSON.stringify` sends).
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("mode".into(), json!(self.mode));
        for (k, v) in [
            ("subject", &self.subject),
            ("target", &self.target),
            ("chain", &self.chain),
            ("network", &self.network),
        ] {
            if let Some(v) = v {
                m.insert(k.into(), json!(v));
            }
        }
        m.insert("direction".into(), json!(self.direction));
        m.insert("depth".into(), json!(self.depth));
        m.insert("node_limit".into(), json!(self.node_limit));
        m.insert("edge_limit".into(), json!(self.edge_limit));
        m.insert("include_relations".into(), json!(self.include_relations));
        m.insert("include_hypotheses".into(), json!(self.include_hypotheses));
        m.insert("observer".into(), json!(self.observer));
        for (k, v) in [
            ("min_amount_msat", &self.min_amount_msat),
            ("start", &self.start),
            ("end", &self.end),
        ] {
            if let Some(v) = v {
                m.insert(k.into(), json!(v));
            }
        }
        Value::Object(m)
    }

    /// The `ui.chain_analysis.query` args (`execute`): the subject trimmed or left out, the
    /// target only on a path query.
    pub fn run_args(&self) -> Value {
        let mut v = self.to_json();
        let m = v.as_object_mut().expect("object");
        match self
            .subject
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(s) => m.insert("subject".into(), json!(s)),
            None => m.remove("subject"),
        };
        match (self.mode == "path").then(|| self.target.as_deref().map(str::trim).unwrap_or("")) {
            Some(t) => m.insert("target".into(), json!(t)),
            None => m.remove("target"),
        };
        v
    }

    /// `analysisQueryKey`: what decides whether two queries ask the same thing.
    pub fn key(&self) -> String {
        let trimmed = |s: &Option<String>| {
            s.as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let nonempty =
            |s: &Option<String>| s.as_deref().filter(|s| !s.is_empty()).map(str::to_owned);
        json!([
            self.mode,
            trimmed(&self.subject),
            if self.mode == "path" {
                trimmed(&self.target)
            } else {
                None
            },
            nonempty(&self.chain),
            nonempty(&self.network),
            self.direction,
            self.depth,
            self.node_limit,
            self.edge_limit,
            self.include_relations,
            self.include_hypotheses,
            self.observer,
            nonempty(&self.min_amount_msat),
            nonempty(&self.start),
            nonempty(&self.end),
        ])
        .to_string()
    }

    /// `analysisEffectiveLayers`: the public observer never sees private custody relations.
    pub fn effective_layers(&self) -> (bool, bool) {
        (
            self.include_relations && self.observer != "public",
            self.include_hypotheses,
        )
    }

    /// The Advanced summary's count: collapsed controls that differ from the defaults
    /// (`activeFilterCount`, where `0`, `""` and `false` compare as unset).
    pub fn active_filter_count(&self) -> usize {
        let d = Query::default();
        let s = |v: &Option<String>| v.clone().filter(|s| !s.is_empty());
        let n = |v: i64| (v != 0).then_some(v);
        let b = |v: bool| v.then_some(true);
        [
            (Some(self.observer.clone()) != Some(d.observer.clone())),
            (Some(self.direction.clone()) != Some(d.direction.clone())),
            s(&self.chain) != s(&d.chain),
            s(&self.network) != s(&d.network),
            n(self.depth) != n(d.depth),
            n(self.node_limit) != n(d.node_limit),
            n(self.edge_limit) != n(d.edge_limit),
            s(&self.min_amount_msat) != s(&d.min_amount_msat),
            s(&self.start) != s(&d.start),
            s(&self.end) != s(&d.end),
            b(self.include_relations) != b(d.include_relations),
            b(self.include_hypotheses) != b(d.include_hypotheses),
        ]
        .iter()
        .filter(|x| **x)
        .count()
    }
}

/// `analysisNetworkInput`: Liquid's network names as the analysis inputs spell them.
pub fn network_input(network: Option<&str>) -> String {
    match network.unwrap_or("") {
        "liquidv1" => "main".into(),
        "liquidtestnet" => "test".into(),
        "elementsregtest" => "regtest".into(),
        "" => "main".into(),
        n => n.into(),
    }
}

/// `analysisUtcInput`: a `datetime-local` value as UTC.
pub fn utc_input(value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    Some(format!(
        "{}Z",
        if value.len() == 16 {
            format!("{value}:00")
        } else {
            value.to_owned()
        }
    ))
}

// ---- Navigation (search params) -----------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Workspace {
    #[default]
    Graph,
    Psbt,
    Datasets,
}

impl Workspace {
    pub fn code(self) -> &'static str {
        match self {
            Workspace::Graph => "graph",
            Workspace::Psbt => "psbt",
            Workspace::Datasets => "datasets",
        }
    }
    fn parse(s: &str) -> Option<Workspace> {
        Some(match s {
            "graph" => Workspace::Graph,
            "psbt" => Workspace::Psbt,
            "datasets" => Workspace::Datasets,
            _ => return None,
        })
    }
}

/// `AnalysisTab`, in `ANALYSIS_SECTIONS` order (the Results select lists them so).
pub const TABS: [&str; 9] = [
    "findings", "patterns", "clusters", "exposure", "paths", "coverage", "frontier", "entropy",
    "labels",
];

/// `AnalysisSearch`: the bounded local view a link selects (never an acquisition).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Search {
    pub fields: Map<String, Value>,
    pub workspace: Option<Workspace>,
    pub tab: Option<&'static str>,
}

/// Decodes `%xx` and `+` in a query component.
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
                {
                    Ok(v) => {
                        out.push(v);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Encodes a query component (unreserved characters stay).
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b':') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The router's search parsing (TanStack `parseSearchWith(JSON.parse)`): each value is JSON
/// when it parses as JSON, otherwise the raw string.
pub fn raw_search(route: &str) -> Map<String, Value> {
    let mut m = Map::new();
    let Some((_, q)) = route.split_once('?') else {
        return m;
    };
    let q = q.split('#').next().unwrap_or("");
    for part in q.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        let (k, v) = (percent_decode(k), percent_decode(v));
        let value = serde_json::from_str::<Value>(&v).unwrap_or(Value::String(v));
        m.insert(k, value);
    }
    m
}

/// `parseAnalysisSearch`: typed, bounded fields; anything else dropped.
pub fn parse_search(raw: &Map<String, Value>) -> Search {
    let mut f = Map::new();
    let one_of = |k: &str, choices: &[&str]| {
        raw.get(k)
            .and_then(Value::as_str)
            .filter(|s| choices.contains(s))
            .map(str::to_owned)
    };
    let bounded = |k: &str, limit: usize| {
        raw.get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && s.chars().count() <= limit)
            .map(str::to_owned)
    };
    let int = |k: &str, min: i64, max: i64| {
        raw.get(k)
            .and_then(|v| {
                v.as_i64()
                    .or_else(|| v.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64))
            })
            .filter(|n| (min..=max).contains(n))
    };
    if let Some(v) = one_of("mode", &["overview", "trace", "path"]) {
        f.insert("mode".into(), json!(v));
    }
    for k in ["subject", "target"] {
        if let Some(v) = bounded(k, 1024) {
            f.insert(k.into(), json!(v));
        }
    }
    if let Some(v) = one_of("observer", &["owner", "public", "disclosed"]) {
        f.insert("observer".into(), json!(v));
    }
    if let Some(v) = one_of("chain", &["bitcoin", "liquid"]) {
        f.insert("chain".into(), json!(v));
    }
    let network = raw.get("network").map(|v| match v.as_str() {
        Some("liquidv1") => json!("main"),
        Some("liquidtestnet") => json!("test"),
        Some("elementsregtest") => json!("regtest"),
        _ => v.clone(),
    });
    if let Some(n) = network
        .as_ref()
        .and_then(Value::as_str)
        .filter(|s| ["main", "test", "signet", "regtest"].contains(s))
    {
        f.insert("network".into(), json!(n));
    }
    if let Some(v) = one_of("direction", &["backward", "forward", "both"]) {
        f.insert("direction".into(), json!(v));
    }
    for (k, min, max) in [
        ("depth", 1, 50),
        ("node_limit", 25, 2000),
        ("edge_limit", 50, 6000),
    ] {
        if let Some(v) = int(k, min, max) {
            f.insert(k.into(), json!(v));
        }
    }
    for k in ["include_relations", "include_hypotheses"] {
        if let Some(b) = raw.get(k).and_then(Value::as_bool) {
            f.insert(k.into(), json!(b));
        }
    }
    if let Some(v) = raw
        .get("min_amount_msat")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 30 && s.bytes().all(|b| b.is_ascii_digit()))
    {
        f.insert("min_amount_msat".into(), json!(v));
    }
    for k in ["start", "end"] {
        if let Some(v) = bounded(k, 40) {
            f.insert(k.into(), json!(v));
        }
    }
    let workspace = raw
        .get("workspace")
        .and_then(Value::as_str)
        .and_then(Workspace::parse);
    let tab = raw
        .get("tab")
        .and_then(Value::as_str)
        .and_then(|t| TABS.iter().copied().find(|x| *x == t));
    Search {
        fields: f,
        workspace,
        tab,
    }
}

/// `analysisSearchQuery`: the defaults, `trace` when a subject comes without a mode, then the link.
pub fn search_query(s: &Search) -> Query {
    let mut v = Query::default().to_json();
    let m = v.as_object_mut().expect("object");
    if s.fields.contains_key("subject") && !s.fields.contains_key("mode") {
        m.insert("mode".into(), json!("trace"));
    }
    for (k, x) in &s.fields {
        m.insert(k.clone(), x.clone());
    }
    Query::from_json(&v)
}

/// A `/chain-analysis` link for an investigation (`analysisInvestigationSearch`), as the router
/// writes it: strings bare, numbers and booleans as JSON.
pub fn investigation_href(query: &Value, workspace: Workspace, tab: &str) -> String {
    let mut raw = query.as_object().cloned().unwrap_or_default();
    raw.insert("workspace".into(), json!(workspace.code()));
    raw.insert("tab".into(), json!(tab));
    let parsed = parse_search(&raw);
    let mut parts = vec![];
    for (k, v) in &parsed.fields {
        let s = match v {
            Value::String(s) => {
                // A string the router would read back as JSON is quoted, as TanStack does.
                if serde_json::from_str::<Value>(s).is_ok() {
                    serde_json::to_string(s).unwrap_or_default()
                } else {
                    s.clone()
                }
            }
            other => other.to_string(),
        };
        parts.push(format!("{k}={}", percent_encode(&s)));
    }
    if let Some(w) = parsed.workspace {
        parts.push(format!("workspace={}", w.code()));
    }
    if let Some(t) = parsed.tab {
        parts.push(format!("tab={t}"));
    }
    format!("/chain-analysis?{}", parts.join("&"))
}

// ---- Graph data ---------------------------------------------------------------------------

/// `AnalysisNode` (the fields the screen reads; the rest stays in `raw`).
#[derive(Clone, Debug)]
pub struct Node {
    pub id: String,
    pub kind: String,
    pub chain: String,
    pub network: String,
    pub label: String,
    pub txid: Option<String>,
    pub transaction_id: Option<String>,
    pub outpoint: Option<String>,
    pub wallet_ids: Vec<String>,
    pub amount_msat: Option<String>,
    pub asset: Option<String>,
    pub address: Option<String>,
    pub status: Option<String>,
    pub evidence: Value,
}

/// `AnalysisEdge`.
#[derive(Clone, Debug)]
pub struct Edge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub kind: String,
    pub evidence_level: String,
    pub amount_msat: Option<String>,
    pub asset: Option<String>,
    pub label: String,
    pub status: Option<String>,
    pub evidence: Value,
}

fn amount_text(v: &Value, k: &str) -> Option<String> {
    match v.get(k) {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

impl Node {
    pub fn parse(v: &Value) -> Node {
        Node {
            id: opt_str(v, "id").unwrap_or_default(),
            kind: opt_str(v, "kind").unwrap_or_default(),
            chain: opt_str(v, "chain").unwrap_or_default(),
            network: opt_str(v, "network").unwrap_or_default(),
            label: opt_str(v, "label").unwrap_or_default(),
            txid: opt_str(v, "txid"),
            transaction_id: opt_str(v, "transaction_id"),
            outpoint: opt_str(v, "outpoint"),
            wallet_ids: v
                .get("wallet_ids")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            amount_msat: amount_text(v, "amount_msat"),
            asset: opt_str(v, "asset"),
            address: opt_str(v, "address"),
            status: opt_str(v, "status"),
            evidence: v.get("evidence").cloned().unwrap_or(Value::Array(vec![])),
        }
    }
    /// The physical identity a person reads (`outpoint || txid || id`).
    pub fn identity(&self) -> &str {
        self.outpoint
            .as_deref()
            .or(self.txid.as_deref())
            .unwrap_or(&self.id)
    }
}

impl Edge {
    pub fn parse(v: &Value) -> Edge {
        Edge {
            id: opt_str(v, "id").unwrap_or_default(),
            source: opt_str(v, "source").unwrap_or_default(),
            target: opt_str(v, "target").unwrap_or_default(),
            kind: opt_str(v, "kind").unwrap_or_default(),
            evidence_level: opt_str(v, "evidence_level").unwrap_or_default(),
            amount_msat: amount_text(v, "amount_msat"),
            asset: opt_str(v, "asset"),
            label: opt_str(v, "label").unwrap_or_default(),
            status: opt_str(v, "status"),
            evidence: v.get("evidence").cloned().unwrap_or(Value::Array(vec![])),
        }
    }
}

/// An `AnalysisResult`, its graph parsed and laid out once.
#[derive(Debug)]
pub struct AResult {
    pub raw: Value,
    pub snapshot_id: String,
    pub query: Query,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub layout: Layout,
}

impl AResult {
    pub fn parse(raw: Value) -> AResult {
        let nodes: Vec<Node> = raw
            .get("nodes")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(Node::parse).collect())
            .unwrap_or_default();
        let edges: Vec<Edge> = raw
            .get("edges")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(Edge::parse).collect())
            .unwrap_or_default();
        let layout = layout(
            &nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
            &edges
                .iter()
                .filter(|e| e.kind != "hypothesis")
                .map(|e| (e.source.as_str(), e.target.as_str()))
                .collect::<Vec<_>>(),
        );
        AResult {
            snapshot_id: opt_str(&raw, "snapshot_id").unwrap_or_default(),
            query: Query::from_json(raw.get("query").unwrap_or(&Value::Null)),
            nodes,
            edges,
            layout,
            raw,
        }
    }
    pub fn list(&self, k: &str) -> &[Value] {
        self.raw
            .get(k)
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
    pub fn coverage(&self) -> &Value {
        self.raw.get("coverage").unwrap_or(&Value::Null)
    }
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }
    pub fn edge(&self, id: &str) -> Option<&Edge> {
        self.edges.iter().find(|e| e.id == id)
    }
    /// `analysisSubjectNodeId`: the node the executed subject names exactly, if exactly one.
    pub fn subject_node_id(&self) -> Option<String> {
        let subject = self
            .query
            .subject
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())?;
        let matches: Vec<&Node> = self
            .nodes
            .iter()
            .filter(|n| {
                n.id == subject
                    || n.outpoint.as_deref() == Some(subject)
                    || (n.kind == "transaction" && n.txid.as_deref() == Some(subject))
            })
            .collect();
        (matches.len() == 1).then(|| matches[0].id.clone())
    }
    /// The transaction features of a node (`transaction_features[].subject == node.id`).
    pub fn features(&self, node_id: &str) -> Option<&Value> {
        self.list("transaction_features")
            .iter()
            .find(|f| f.get("subject").and_then(Value::as_str) == Some(node_id))
            .and_then(|f| f.get("features"))
    }
    /// A tab's count in the Results select (`tabCount`).
    pub fn tab_count(&self, tab: &str) -> Option<usize> {
        match tab {
            "findings" | "frontier" | "clusters" | "paths" | "patterns" | "exposure" => {
                Some(self.list(tab).len())
            }
            _ => None,
        }
    }
}

/// `formatAnalysisAmount`: decimal integer msat text to BTC without floating point.
pub fn format_amount(amount: Option<&str>, asset: Option<&str>) -> String {
    let Some(a) = amount else { return "—".into() };
    let digits = a.strip_prefix('-').unwrap_or(a);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return "—".into();
    }
    let negative = a.starts_with('-') && digits.bytes().any(|b| b != b'0');
    let sign = if negative { "−" } else { "" };
    let magnitude = digits.trim_start_matches('0');
    let magnitude = if magnitude.is_empty() { "0" } else { magnitude };
    if let Some(asset) = asset
        && !["BTC", "LBTC", "L-BTC"].contains(&asset.to_uppercase().as_str())
    {
        return format!("{sign}{magnitude} msat · {asset}");
    }
    let padded = format!("{magnitude:0>12}");
    let (whole, fraction) = padded.split_at(padded.len() - 11);
    let whole = whole.trim_start_matches('0');
    let whole = if whole.is_empty() { "0" } else { whole };
    let fraction = fraction.trim_end_matches('0');
    let asset = asset.unwrap_or("BTC");
    if fraction.is_empty() {
        format!("{sign}{whole} {asset}")
    } else {
        format!("{sign}{whole}.{fraction} {asset}")
    }
}

/// `shortAnalysisId`.
pub fn short_id(value: &str, maximum: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() > maximum {
        let head: String = chars[..maximum / 2].iter().collect();
        let tail: String = chars[chars.len() - 8..].iter().collect();
        format!("{head}…{tail}")
    } else {
        value.to_owned()
    }
}

/// `analysisNodeCaption`: meaningful labels pass, identity-derived ones become head…tail.
pub fn node_caption(node: &Node, maximum: usize) -> String {
    let identity = node.identity();
    let label = node.label.trim();
    let stem = label.trim_end_matches(['…', '.']);
    let colon = node.outpoint.as_deref().and_then(|o| o.rfind(':'));
    let (core, index) = match (node.outpoint.as_deref(), colon) {
        (Some(o), Some(c)) if c > 0 => (&o[..c], Some(&o[c + 1..])),
        _ => (identity, None),
    };
    let abbreviated_outpoint = index.is_some_and(|index| {
        let looks = stem.rfind(':').is_some_and(|c| {
            c > 0
                && stem[..c].bytes().all(|b| b.is_ascii_hexdigit())
                && !stem[c + 1..].is_empty()
                && stem[c + 1..].bytes().all(|b| b.is_ascii_digit())
        });
        looks
            && stem.ends_with(&format!(":{index}"))
            && core.starts_with(&stem[..stem.rfind(':').unwrap_or(0)])
    });
    let derived = stem.is_empty()
        || abbreviated_outpoint
        || [
            Some(node.id.as_str()),
            node.txid.as_deref(),
            node.outpoint.as_deref(),
        ]
        .iter()
        .flatten()
        .any(|v| !v.is_empty() && (*v == label || v.starts_with(stem)));
    if !derived {
        return short_id(label, maximum);
    }
    let chars: Vec<char> = core.chars().collect();
    let abbreviated = if chars.len() > 20 {
        format!(
            "{}…{}",
            chars[..8].iter().collect::<String>(),
            chars[chars.len() - 8..].iter().collect::<String>()
        )
    } else {
        core.to_owned()
    };
    match index {
        Some(i) => format!("{abbreviated}:{i}"),
        None => abbreviated,
    }
}

/// `readableAnalysisValue`: `JSON.stringify(value, null, 2)` for structures.
pub fn readable(value: &Value) -> String {
    match value {
        Value::Null => "—".into(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

/// `pathNodeIds`.
pub fn path_node_ids(path: &Value) -> Vec<String> {
    let list = if path.is_array() {
        Some(path)
    } else {
        path.get("node_ids")
    };
    list.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// `analysisCsv`: nodes then edges, untrusted text guarded against spreadsheet formulas.
pub fn csv(r: &AResult) -> String {
    let mut rows: Vec<Vec<String>> = vec![
        [
            "snapshot_id",
            "row_type",
            "id",
            "kind",
            "chain",
            "network",
            "source",
            "target",
            "amount_msat",
            "asset",
            "label",
            "evidence_level",
            "evidence_json",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
    ];
    for n in &r.nodes {
        rows.push(vec![
            r.snapshot_id.clone(),
            "node".into(),
            n.id.clone(),
            n.kind.clone(),
            n.chain.clone(),
            n.network.clone(),
            String::new(),
            String::new(),
            n.amount_msat.clone().unwrap_or_default(),
            n.asset.clone().unwrap_or_default(),
            n.label.clone(),
            String::new(),
            n.evidence.to_string(),
        ]);
    }
    for e in &r.edges {
        rows.push(vec![
            r.snapshot_id.clone(),
            "edge".into(),
            e.id.clone(),
            e.kind.clone(),
            String::new(),
            String::new(),
            e.source.clone(),
            e.target.clone(),
            e.amount_msat.clone().unwrap_or_default(),
            e.asset.clone().unwrap_or_default(),
            e.label.clone(),
            e.evidence_level.clone(),
            e.evidence.to_string(),
        ]);
    }
    let cell = |c: &str| {
        let integer = !c.is_empty()
            && c.strip_prefix('-')
                .unwrap_or(c)
                .bytes()
                .all(|b| b.is_ascii_digit())
            && c != "-";
        let mut value = c.to_owned();
        if !integer && matches!(c.trim_start().chars().next(), Some('=' | '+' | '@' | '-')) {
            value = format!("'{value}");
        }
        format!("\"{}\"", value.replace('"', "\"\""))
    };
    rows.iter()
        .map(|r| r.iter().map(|c| cell(c)).collect::<Vec<_>>().join(","))
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n"
}

/// The export's file name (`exportChainAnalysis`).
pub fn export_name(snapshot_id: &str, format: &str) -> String {
    let head: String = snapshot_id
        .chars()
        .take(16)
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    format!("kassiber-chain-analysis-{head}.{format}")
}

// ---- Layout and camera (components/evidence-graph) ----------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pos {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub positions: HashMap<String, Pos>,
    pub order: Vec<String>,
    pub width: f32,
    pub height: f32,
}

/// One connected block of the layout: its nodes at block-local grid cells (column, row offset
/// in rows from the block's top), and its size in columns and rows.
struct Block<'a> {
    cells: Vec<(&'a str, usize, f32)>,
    cols: usize,
    rows: usize,
}

/// `layoutEvidenceGraph`'s levels for one connected set: breadth-first from its roots, each
/// level a column, the columns centred against the tallest.
fn levels<'a>(nodes: &[&'a str], edges: &[(&'a str, &'a str)]) -> Block<'a> {
    let ids: HashSet<&str> = nodes.iter().copied().collect();
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut indegree: HashMap<&str, usize> = nodes.iter().map(|n| (*n, 0)).collect();
    for (s, t) in edges {
        if !ids.contains(s) || !ids.contains(t) || s == t {
            continue;
        }
        adjacency.entry(s).or_default().push(t);
        *indegree.entry(t).or_default() += 1;
    }
    let roots: Vec<&str> = nodes
        .iter()
        .copied()
        .filter(|n| indegree.get(n) == Some(&0))
        .collect();
    let mut level: HashMap<&str, usize> = roots.iter().map(|r| (*r, 0)).collect();
    let mut queue = roots.clone();
    let mut i = 0;
    while i < queue.len() {
        let id = queue[i];
        i += 1;
        for next in adjacency.get(id).cloned().unwrap_or_default() {
            if !level.contains_key(next) {
                level.insert(next, (level.get(id).copied().unwrap_or(0) + 1).min(50));
                queue.push(next);
            }
        }
    }
    let mut columns: Vec<(usize, Vec<&str>)> = vec![];
    for n in nodes {
        let l = *level.entry(n).or_insert(0);
        match columns.iter_mut().find(|(c, _)| *c == l) {
            Some((_, c)) => c.push(n),
            None => columns.push((l, vec![n])),
        }
    }
    let rows = columns
        .iter()
        .map(|(_, c)| c.len())
        .max()
        .unwrap_or(1)
        .max(1);
    let cols = columns.iter().map(|(l, _)| *l).max().unwrap_or(0) + 1;
    let mut cells = vec![];
    for (l, entries) in &columns {
        for (index, n) in entries.iter().enumerate() {
            cells.push((*n, *l, (rows - entries.len()) as f32 / 2.0 + index as f32));
        }
    }
    Block { cells, cols, rows }
}

/// `layoutEvidenceGraph`: breadth-first levels into columns 265 apart, rows 90 apart. A graph
/// of several unconnected parts is laid out part by part (the web stacks every part into the
/// same columns, which turns a book overview into one column hundreds of rows tall with its
/// edges drawn over each other): the connected parts in shelves, the largest first, and the
/// nodes without any edge as a grid after them.
pub fn layout(nodes: &[&str], edges: &[(&str, &str)]) -> Layout {
    const SHELF_COLS: usize = 8;
    let ids: HashSet<&str> = nodes.iter().copied().collect();
    let index: HashMap<&str, usize> = nodes.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    let mut parent: Vec<usize> = (0..nodes.len()).collect();
    fn find(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    for (s, t) in edges {
        if !ids.contains(s) || !ids.contains(t) || s == t {
            continue;
        }
        let (a, b) = (find(&mut parent, index[s]), find(&mut parent, index[t]));
        if a != b {
            parent[a.max(b)] = a.min(b);
        }
    }
    let mut groups: Vec<(usize, Vec<&str>)> = vec![];
    for (i, n) in nodes.iter().enumerate() {
        let root = find(&mut parent, i);
        match groups.iter_mut().find(|(r, _)| *r == root) {
            Some((_, g)) => g.push(n),
            None => groups.push((root, vec![n])),
        }
    }
    let mut blocks: Vec<Block> = vec![];
    if groups.len() <= 1 {
        blocks.push(levels(nodes, edges));
    } else {
        let mut connected: Vec<&Vec<&str>> = groups
            .iter()
            .map(|(_, g)| g)
            .filter(|g| g.len() > 1)
            .collect();
        connected.sort_by_key(|g| std::cmp::Reverse(g.len()));
        for g in connected {
            let set: HashSet<&str> = g.iter().copied().collect();
            let own: Vec<(&str, &str)> = edges
                .iter()
                .copied()
                .filter(|(s, _)| set.contains(s))
                .collect();
            blocks.push(levels(g, &own));
        }
        let single: Vec<&str> = groups
            .iter()
            .filter(|(_, g)| g.len() == 1)
            .map(|(_, g)| g[0])
            .collect();
        if !single.is_empty() {
            let cols = single.len().min(SHELF_COLS);
            blocks.push(Block {
                cells: single
                    .iter()
                    .enumerate()
                    .map(|(i, n)| (*n, i % cols, (i / cols) as f32))
                    .collect(),
                cols,
                rows: single.len().div_ceil(cols),
            });
        }
    }
    // Shelves: blocks side by side while they fit, each shelf as tall as its tallest block.
    let shelf = blocks
        .iter()
        .map(|b| b.cols)
        .max()
        .unwrap_or(1)
        .max(SHELF_COLS);
    let mut positions = HashMap::new();
    let (mut col0, mut row0, mut shelf_rows, mut max_cols) = (0usize, 0.0f32, 0usize, 1usize);
    let mut placed: Vec<(usize, f32, &Block)> = vec![];
    let mut shelves: Vec<(usize, Vec<(usize, &Block)>)> = vec![];
    for b in &blocks {
        if col0 > 0 && col0 + b.cols > shelf {
            col0 = 0;
            shelves.push((shelf_rows, vec![]));
            shelf_rows = 0;
        }
        if shelves.is_empty() {
            shelves.push((0, vec![]));
        }
        let last = shelves.len() - 1;
        shelves[last].1.push((col0, b));
        col0 += b.cols;
        shelf_rows = shelf_rows.max(b.rows);
        shelves[last].0 = shelf_rows;
        max_cols = max_cols.max(col0);
    }
    for (rows, items) in &shelves {
        for (c, b) in items {
            placed.push((*c, row0 + (*rows - b.rows) as f32 / 2.0, b));
        }
        row0 += *rows as f32 + 0.5;
    }
    let total_rows = (row0 - 0.5).max(1.0);
    let height = (total_rows * 90.0 + 100.0).max(380.0);
    let top = (height - total_rows * 90.0) / 2.0 + 45.0;
    for (c, r, b) in placed {
        for (n, col, row) in &b.cells {
            positions.insert(
                (*n).to_owned(),
                Pos {
                    x: 145.0 + (c + *col) as f32 * 265.0,
                    y: top + (r + *row) * 90.0,
                },
            );
        }
    }
    Layout {
        positions,
        order: nodes.iter().map(|s| (*s).to_owned()).collect(),
        width: (max_cols as f32 * 265.0 + 40.0).max(700.0),
        height,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
}

pub fn zoom_camera(c: Camera, factor: f32, point: Pos) -> Camera {
    let scale = (c.scale * factor).clamp(0.001, 4.0);
    let ratio = scale / c.scale;
    Camera {
        scale,
        x: point.x - (point.x - c.x) * ratio,
        y: point.y - (point.y - c.y) * ratio,
    }
}

// `min` then `max`, not `clamp`: a NaN scale (an empty layout or viewport) becomes the upper
// bound here, where `clamp` would pass the NaN through to the camera.
#[allow(clippy::manual_clamp)]
pub fn fit_camera(l: &Layout, vp: [f32; 2]) -> Camera {
    let scale = (vp[0] / l.width * 0.9)
        .min(vp[1] / l.height * 0.9)
        .min(1.0)
        .max(0.001);
    Camera {
        scale,
        x: (vp[0] - l.width * scale) / 2.0,
        y: (vp[1] - l.height * scale) / 2.0,
    }
}

// `min` then `max`, not `clamp`: a NaN scale (an empty layout or viewport) becomes the upper
// bound here, where `clamp` would pass the NaN through to the camera.
#[allow(clippy::manual_clamp)]
pub fn focus_camera(p: Pos, vp: [f32; 2]) -> Camera {
    let scale = ((vp[0] - 40.0) / 198.0)
        .min((vp[1] - 100.0) / 58.0)
        .min(1.0)
        .max(0.025);
    Camera {
        scale,
        x: vp[0] / 2.0 - p.x * scale,
        y: vp[1] / 2.0 - p.y * scale,
    }
}

/// Where a selection sits: a node's position, an edge's midpoint.
pub fn selection_position(l: &Layout, edges: &[Edge], sel: Option<&Sel>) -> Option<Pos> {
    let sel = sel?;
    if sel.edge {
        let e = edges.iter().find(|e| e.id == sel.id)?;
        let (a, b) = (l.positions.get(&e.source)?, l.positions.get(&e.target)?);
        Some(Pos {
            x: (a.x + b.x) / 2.0,
            y: (a.y + b.y) / 2.0,
        })
    } else {
        l.positions.get(&sel.id).copied()
    }
}

/// `initialGraphCamera` ("readable"): the selection, a fit if it is readable, else the leftmost
/// preferred anchor (transactions) at native size, left of centre.
pub fn initial_camera(
    l: &Layout,
    vp: [f32; 2],
    nodes: &[Node],
    edges: &[Edge],
    sel: Option<&Sel>,
) -> Camera {
    if let Some(p) = selection_position(l, edges, sel) {
        return focus_camera(p, vp);
    }
    let fitted = fit_camera(l, vp);
    if fitted.scale >= 0.9 || nodes.is_empty() {
        return fitted;
    }
    let preferred = nodes.iter().any(|n| n.kind == "transaction");
    let mut positions: Vec<Pos> = nodes
        .iter()
        .filter(|n| !preferred || n.kind == "transaction")
        .filter_map(|n| l.positions.get(&n.id).copied())
        .collect();
    positions.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    let Some(anchor) = positions.first().copied() else {
        return fitted;
    };
    let cam = focus_camera(anchor, vp);
    // The anchor left of centre; and no empty band above the graph's first row (it starts
    // under the zoom controls instead of mid-height).
    let top = l
        .positions
        .values()
        .map(|p| p.y)
        .fold(f32::INFINITY, f32::min);
    let y = if top.is_finite() {
        cam.y.min(72.0 - (top - 29.0) * cam.scale)
    } else {
        cam.y
    };
    Camera {
        x: (vp[0] / 2.0).min(123.0 * cam.scale) - anchor.x * cam.scale,
        y,
        ..cam
    }
}

pub fn node_in_view(p: Pos, c: Camera, vp: [f32; 2]) -> bool {
    let (x, y) = (p.x * c.scale + c.x, p.y * c.scale + c.y);
    x + 99.0 * c.scale > 0.0
        && x - 99.0 * c.scale < vp[0]
        && y + 29.0 * c.scale > 0.0
        && y - 29.0 * c.scale < vp[1]
}

pub fn selection_in_view(p: Pos, c: Camera, vp: [f32; 2]) -> bool {
    let (x, y) = (p.x * c.scale + c.x, p.y * c.scale + c.y);
    c.scale >= 0.75
        && x - 99.0 * c.scale >= 12.0
        && x + 99.0 * c.scale <= vp[0] - 12.0
        && y - 29.0 * c.scale >= 60.0
        && y + 29.0 * c.scale <= vp[1] - 80.0
}

pub fn resize_camera(c: Camera, prev: [f32; 2], next: [f32; 2]) -> Camera {
    Camera {
        x: c.x + (next[0] - prev[0]) / 2.0,
        y: c.y + (next[1] - prev[1]) / 2.0,
        ..c
    }
}

/// `graphOverview`: the minimap's scale and offset, and the viewport rectangle in it.
pub struct Overview {
    pub scale: f32,
    pub x: f32,
    pub y: f32,
    pub rect: [f32; 4],
}

pub fn overview(l: &Layout, c: Camera, vp: [f32; 2], w: f32, h: f32) -> Overview {
    let scale = ((w - 12.0) / l.width).min((h - 12.0) / l.height);
    let x = (w - l.width * scale) / 2.0;
    let y = (h - l.height * scale) / 2.0;
    let left = (-c.x / c.scale).clamp(0.0, l.width);
    let top = (-c.y / c.scale).clamp(0.0, l.height);
    let right = ((vp[0] - c.x) / c.scale).clamp(0.0, l.width);
    let bottom = ((vp[1] - c.y) / c.scale).clamp(0.0, l.height);
    Overview {
        scale,
        x,
        y,
        rect: [
            x + left * scale,
            y + top * scale,
            ((right - left) * scale).max(2.0),
            ((bottom - top) * scale).max(4.0),
        ],
    }
}

/// A graph selection (`{ kind: "node" | "edge", id }`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sel {
    pub edge: bool,
    pub id: String,
}

impl Sel {
    pub fn node(id: impl Into<String>) -> Sel {
        Sel {
            edge: false,
            id: id.into(),
        }
    }
}

// ---- Watches ------------------------------------------------------------------------------

/// `watchQueryScope`: a generic saved investigation keeps every bound chain; a scoped one
/// takes its chain from the subject and its network from the binding.
pub fn watch_query_scope(query: &Query, binding: Option<&Value>) -> Query {
    let Some(b) = binding.filter(|b| b.get("state").and_then(Value::as_str) == Some("bound"))
    else {
        return query.clone();
    };
    let parts: Vec<&str> = query
        .subject
        .as_deref()
        .map(|s| s.split(':').collect())
        .unwrap_or_default();
    let subject_chain = (parts.len() >= 4
        && ["tx", "out"].contains(&parts[2])
        && ["bitcoin", "liquid"].contains(&parts[0]))
    .then(|| parts[0].to_owned());
    let Some(chain) = query.chain.clone().or(subject_chain.clone()) else {
        return query.clone();
    };
    let network = query
        .network
        .clone()
        .or_else(|| {
            subject_chain
                .as_ref()
                .map(|_| network_input(Some(parts[1])))
        })
        .or_else(|| {
            b.get("domains")
                .and_then(Value::as_array)
                .and_then(|d| {
                    d.iter()
                        .find(|d| d.get("chain").and_then(Value::as_str) == Some(chain.as_str()))
                })
                .and_then(|d| d.get("network"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    Query {
        chain: Some(chain),
        network: network
            .map(|n| network_input(Some(&n)))
            .or(query.network.clone()),
        ..query.clone()
    }
}

// ---- Entropy and workbench helpers --------------------------------------------------------

/// `entropySubjectDomain`: the one observed transaction a reference names, or why not.
#[derive(Clone, Debug, PartialEq)]
pub enum Domain {
    Resolved {
        subject: String,
        chain: String,
        network: String,
    },
    Unknown,
    Ambiguous,
}

pub fn entropy_subject_domain(
    subject: &str,
    nodes: &[Node],
    chain: Option<&str>,
    network: Option<&str>,
) -> Domain {
    let value = subject.trim().to_lowercase();
    if value.is_empty() {
        return Domain::Unknown;
    }
    let mut found: Vec<(String, String, String)> = vec![];
    for n in nodes {
        if n.kind != "transaction"
            || ![Some(&n.id), n.txid.as_ref(), n.transaction_id.as_ref()]
                .iter()
                .flatten()
                .any(|id| id.to_lowercase() == value)
        {
            continue;
        }
        if n.chain != "bitcoin" && n.chain != "liquid" || n.network.is_empty() {
            continue;
        }
        let normalized_network = network_input(Some(&n.network));
        if !["main", "test", "signet", "regtest"].contains(&normalized_network.as_str())
            || (n.chain == "liquid" && normalized_network == "signet")
        {
            continue;
        }
        if chain.is_some_and(|c| c != n.chain)
            || network.is_some_and(|w| network_input(Some(w)) != normalized_network)
        {
            continue;
        }
        let c = (n.id.clone(), n.chain.clone(), normalized_network);
        if !found.contains(&c) {
            found.push(c);
        }
    }
    match found.len() {
        1 => {
            let (subject, chain, network) = found.remove(0);
            Domain::Resolved {
                subject,
                chain,
                network,
            }
        }
        0 => Domain::Unknown,
        _ => Domain::Ambiguous,
    }
}

const MAX_MSAT: u128 = 2_100_000_000_000_000_000;

/// `exactSatoshiMsat`: whole satoshis written as msat, within the supply.
pub fn exact_satoshi_msat(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 19
        && value.bytes().all(|b| b.is_ascii_digit())
        && value
            .parse::<u128>()
            .is_ok_and(|v| v % 1000 == 0 && v <= MAX_MSAT)
}

/// `entropyScenario`.
pub fn entropy_scenario(kind: &str, protocol: &str, received: &str, paid: &str) -> Option<Value> {
    if kind == "independent" {
        return Some(json!({"kind": kind, "protocol": protocol}));
    }
    (exact_satoshi_msat(received) && exact_satoshi_msat(paid)).then(|| {
        json!({"kind": kind, "protocol": protocol, "max_received_fee_msat": received.parse::<u128>().unwrap_or(0).to_string(), "max_paid_fee_msat": paid.parse::<u128>().unwrap_or(0).to_string()})
    })
}

/// `entropyLinks`: the well-formed link counts.
pub fn entropy_links(value: Option<&Value>) -> Vec<(String, String, String)> {
    value
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    let (i, o, c) = (
                        r.get("input_id")?.as_str()?,
                        r.get("output_id")?.as_str()?,
                        r.get("interpretation_count")?.as_str()?,
                    );
                    (!c.is_empty() && c.bytes().all(|b| b.is_ascii_digit()))
                        .then(|| (i.to_owned(), o.to_owned(), c.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// One changed conditional link of `entropyLinkChanges`.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkChange {
    pub input_id: String,
    pub output_id: String,
    pub before: String,
    pub after: String,
}

fn big(s: &str) -> Option<u128> {
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse::<u128>().ok())
        .flatten()
}

/// `entropyLinkChanges`: exact rational comparison of two exact results of one subject,
/// observer and snapshot; None when they are not comparable.
pub fn entropy_link_changes(before: &Value, after: &Value) -> Option<Vec<LinkChange>> {
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    if s(before, "status").as_deref() != Some("exact")
        || s(after, "status").as_deref() != Some("exact")
    {
        return None;
    }
    let snap = s(before, "snapshot_id")?;
    if snap.is_empty()
        || Some(&snap) != s(after, "snapshot_id").as_ref()
        || before.get("subject") != after.get("subject")
        || before.get("observer") != after.get("observer")
    {
        return None;
    }
    let a = big(&s(before, "interpretation_count")?)?;
    let b = big(&s(after, "interpretation_count")?)?;
    if a == 0 || b == 0 {
        return None;
    }
    let previous: HashMap<(String, String), String> = entropy_links(before.get("link_counts"))
        .into_iter()
        .map(|(i, o, c)| ((i, o), c))
        .collect();
    let next = entropy_links(after.get("link_counts"));
    if previous.len() != next.len()
        || next
            .iter()
            .any(|(i, o, _)| !previous.contains_key(&(i.clone(), o.clone())))
    {
        return None;
    }
    let (sa, sb) = (
        s(before, "interpretation_count")?,
        s(after, "interpretation_count")?,
    );
    Some(
        next.into_iter()
            .filter_map(|(i, o, c)| {
                let old = previous.get(&(i.clone(), o.clone()))?.clone();
                let lhs = big(&c)?.checked_mul(a)?;
                let rhs = big(&old)?.checked_mul(b)?;
                (lhs != rhs).then(|| LinkChange {
                    before: format!("{old}/{sa}"),
                    after: format!("{c}/{sb}"),
                    input_id: i,
                    output_id: o,
                })
            })
            .collect(),
    )
}

/// `payjoinOptions`: the explicit BIP78 constraints, or None while a field is invalid.
pub fn payjoin_options(
    payment: &str,
    fee_output: &str,
    maximum: &str,
    minimum_rate: &str,
    substitute: bool,
) -> Option<Value> {
    let index = |v: &str| {
        !v.is_empty()
            && v.len() <= 4
            && v.bytes().all(|b| b.is_ascii_digit())
            && v.parse::<u32>().is_ok_and(|n| n <= 4095)
    };
    let rate_ok = |v: &str| {
        let (w, f) = v.split_once('.').unwrap_or((v, ""));
        !w.is_empty()
            && w.len() <= 12
            && w.bytes().all(|b| b.is_ascii_digit())
            && (!v.contains('.')
                || (!f.is_empty() && f.len() <= 12 && f.bytes().all(|b| b.is_ascii_digit())))
    };
    if !index(payment)
        || (!fee_output.is_empty()
            && (!index(fee_output)
                || fee_output.parse::<u32>().ok() == payment.parse::<u32>().ok()))
        || !exact_satoshi_msat(maximum)
        || (!minimum_rate.is_empty() && !rate_ok(minimum_rate))
    {
        return None;
    }
    let mut m = Map::new();
    m.insert(
        "payment_output_index".into(),
        json!(payment.parse::<u32>().unwrap_or(0)),
    );
    if !fee_output.is_empty() {
        m.insert(
            "additional_fee_output_index".into(),
            json!(fee_output.parse::<u32>().unwrap_or(0)),
        );
    }
    m.insert(
        "max_additional_fee_contribution_msat".into(),
        json!(maximum.parse::<u128>().unwrap_or(0).to_string()),
    );
    m.insert("allow_output_substitution".into(), json!(substitute));
    if !minimum_rate.is_empty() {
        m.insert("minimum_fee_rate_sat_vb".into(), json!(minimum_rate));
    }
    Some(Value::Object(m))
}

/// `datasetReplacementManifest`: the strict import fields of an active dataset, version cleared.
pub fn dataset_replacement_manifest(dataset: &Value) -> Map<String, Value> {
    let old = dataset.get("manifest").cloned().unwrap_or(Value::Null);
    let g = |k: &str| old.get(k).cloned().unwrap_or(json!(""));
    let network = old.get("network").and_then(Value::as_str).map(|n| match n {
        "liquidv1" => "main",
        "liquidtestnet" => "test",
        "elementsregtest" => "regtest",
        n => n,
    });
    let mut m = Map::new();
    for k in ["dataset_key", "name"] {
        m.insert(k.into(), g(k));
    }
    m.insert("version".into(), json!(""));
    m.insert("chain".into(), g("chain"));
    m.insert("network".into(), json!(network.unwrap_or("")));
    for k in ["source", "license", "attribution_method", "visibility"] {
        m.insert(k.into(), g(k));
    }
    m.insert(
        "expected_active_id".into(),
        dataset.get("id").cloned().unwrap_or(Value::Null),
    );
    m
}

/// `datasetPreviewKey`: the source token, the manifest's sorted fields, format and adapter.
pub fn dataset_preview_key(
    token: Option<&str>,
    manifest: &Map<String, Value>,
    format: &str,
    adapter: &str,
) -> String {
    let mut keys: Vec<&String> = manifest.keys().collect();
    keys.sort();
    let mut parts = vec![json!(token)];
    for k in keys {
        parts.push(json!([k, manifest[k]]));
    }
    parts.push(json!(format));
    parts.push(json!(adapter));
    Value::Array(parts).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, kind: &str, label: &str, txid: Option<&str>, outpoint: Option<&str>) -> Node {
        Node::parse(
            &json!({"id": id, "kind": kind, "label": label, "txid": txid, "outpoint": outpoint, "chain": "bitcoin", "network": "regtest"}),
        )
    }

    #[test]
    fn amounts_format_without_floats() {
        assert_eq!(
            format_amount(Some("20913532000"), Some("BTC")),
            "0.20913532 BTC"
        );
        assert_eq!(format_amount(Some("100000000000"), None), "1 BTC");
        assert_eq!(
            format_amount(Some("-150000000000"), Some("L-BTC")),
            "−1.5 L-BTC"
        );
        assert_eq!(format_amount(Some("12"), Some("USDT")), "12 msat · USDT");
        assert_eq!(format_amount(None, None), "—");
        assert_eq!(format_amount(Some("1.5"), None), "—");
        assert_eq!(format_amount(Some("0"), None), "0 BTC");
    }

    #[test]
    fn captions_abbreviate_derived_labels() {
        let tx = "006cf2c07b2cd172a33142e12c6e05afea3bc643b73e92768d17c1bdd9882c45";
        assert_eq!(
            node_caption(
                &node(
                    &format!("bitcoin:regtest:tx:{tx}"),
                    "transaction",
                    "006cf2c07b2c",
                    Some(tx),
                    None
                ),
                26
            ),
            "006cf2c0…d9882c45"
        );
        let op = format!("{tx}:3");
        assert_eq!(
            node_caption(
                &node("x", "output", "006cf2c07b:3", Some(tx), Some(&op)),
                26
            ),
            "006cf2c0…d9882c45:3"
        );
        assert_eq!(
            node_caption(&node("x", "record", "Cold storage", None, None), 26),
            "Cold storage"
        );
    }

    #[test]
    fn query_keys_ignore_draft_only_differences() {
        let mut a = Query::default();
        let mut b = Query {
            subject: Some("  ".into()),
            ..Query::default()
        };
        assert_eq!(a.key(), b.key());
        b.target = Some("t".into());
        assert_eq!(a.key(), b.key(), "a target only counts on a path query");
        a.mode = "path".into();
        b.mode = "path".into();
        assert_ne!(a.key(), b.key());
        assert_eq!(Query::default().active_filter_count(), 0);
        assert_eq!(
            Query {
                observer: "public".into(),
                depth: 6,
                ..Query::default()
            }
            .active_filter_count(),
            2
        );
    }

    #[test]
    fn run_args_trim_and_scope_the_target() {
        let q = Query {
            subject: Some(" abc ".into()),
            target: Some(" t ".into()),
            ..Query::default()
        };
        let a = q.run_args();
        assert_eq!(a["subject"], "abc");
        assert!(a.get("target").is_none());
        let a = Query {
            mode: "path".into(),
            ..q
        }
        .run_args();
        assert_eq!(a["target"], "t");
    }

    #[test]
    fn search_parses_bounded_fields_and_round_trips() {
        let raw = raw_search(
            "/chain-analysis?subject=abc&depth=6&network=liquidv1&tab=coverage&workspace=psbt&bogus=1&node_limit=5",
        );
        let s = parse_search(&raw);
        assert_eq!(s.tab, Some("coverage"));
        assert_eq!(s.workspace, Some(Workspace::Psbt));
        let q = search_query(&s);
        assert_eq!(q.mode, "trace");
        assert_eq!(q.depth, 6);
        assert_eq!(q.network.as_deref(), Some("main"));
        assert_eq!(q.node_limit, 400, "out-of-range limits are dropped");
        let href = investigation_href(
            &json!({"mode": "trace", "subject": "script:51", "chain": "bitcoin", "network": "regtest", "depth": 12, "include_relations": false, "observer": "public"}),
            Workspace::Graph,
            "patterns",
        );
        let back = parse_search(&raw_search(&href));
        assert_eq!(back.tab, Some("patterns"));
        let q = search_query(&back);
        assert_eq!(q.subject.as_deref(), Some("script:51"));
        assert_eq!(q.depth, 12);
        assert!(!q.include_relations);
        assert_eq!(q.observer, "public");
    }

    #[test]
    fn layout_levels_and_initial_camera() {
        let l = layout(&["a", "b", "c"], &[("a", "b"), ("a", "c")]);
        assert_eq!(l.positions["a"], Pos { x: 145.0, y: 190.0 });
        assert_eq!(l.positions["b"].x, 410.0);
        assert_eq!(l.width, 700.0);
        assert_eq!(l.height, 380.0);
        let cam = fit_camera(&l, [1000.0, 520.0]);
        assert!((cam.scale - 1.0).abs() < 1e-6);
        let f = focus_camera(Pos { x: 145.0, y: 190.0 }, [1000.0, 520.0]);
        assert_eq!(f.scale, 1.0);
        assert_eq!(f.x, 355.0);
    }

    #[test]
    fn layout_keeps_unconnected_parts_apart() {
        // A chain of three, a pair, and twenty nodes without an edge.
        let mut nodes = vec!["a", "b", "c", "p", "q"];
        let lone: Vec<String> = (0..20).map(|i| format!("s{i}")).collect();
        nodes.extend(lone.iter().map(String::as_str));
        let l = layout(&nodes, &[("a", "b"), ("b", "c"), ("p", "q")]);
        assert_eq!(l.positions.len(), 25);
        // The chain first, the pair beside it on the same shelf.
        assert_eq!((l.positions["a"].x, l.positions["c"].x), (145.0, 675.0));
        assert_eq!(l.positions["p"].x, 940.0);
        assert_eq!(l.positions["a"].y, l.positions["p"].y);
        // The lone nodes eight to a row below, never over a connected part.
        assert!(l.positions["s0"].y > l.positions["a"].y);
        assert_eq!(l.positions["s8"].x, l.positions["s0"].x);
        assert_eq!(l.positions["s8"].y - l.positions["s0"].y, 90.0);
        let mut seen = std::collections::HashSet::new();
        assert!(
            l.positions
                .values()
                .all(|p| seen.insert((p.x as i64, p.y as i64)))
        );
        assert!(
            l.positions
                .values()
                .all(|p| p.x + 99.0 <= l.width && p.y + 29.0 <= l.height)
        );
    }

    #[test]
    fn csv_guards_formulas_but_keeps_amounts() {
        let r = AResult::parse(
            json!({"snapshot_id": "s", "query": {}, "nodes": [{"id": "n", "kind": "record", "label": "=cmd", "amount_msat": "-5", "chain": "bitcoin", "network": "main", "evidence": []}], "edges": []}),
        );
        let out = csv(&r);
        assert!(out.contains("\"'=cmd\""));
        assert!(out.contains("\"-5\""));
        assert!(out.ends_with("\r\n"));
    }

    #[test]
    fn entropy_changes_compare_exact_fractions() {
        let before = json!({"status": "exact", "snapshot_id": "s", "subject": "t", "observer": "owner", "interpretation_count": "4", "link_counts": [{"input_id": "i", "output_id": "o", "interpretation_count": "2"}]});
        let after = json!({"status": "exact", "snapshot_id": "s", "subject": "t", "observer": "owner", "interpretation_count": "2", "link_counts": [{"input_id": "i", "output_id": "o", "interpretation_count": "2"}]});
        let c = entropy_link_changes(&before, &after).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].before, "2/4");
        assert_eq!(c[0].after, "2/2");
        assert!(entropy_link_changes(&before, &json!({"status": "timeout"})).is_none());
        assert!(entropy_scenario("coinjoin_intrafees", "generic", "1500", "0").is_none());
        assert!(entropy_scenario("coinjoin_intrafees", "generic", "1000", "0").is_some());
    }

    #[test]
    fn payjoin_constraints_validate() {
        assert!(payjoin_options("0", "", "0", "", false).is_some());
        assert!(payjoin_options("0", "0", "0", "", false).is_none());
        assert!(payjoin_options("0", "", "1500", "", false).is_none());
        assert!(payjoin_options("1", "2", "1000", "1.5", true).is_some());
        assert!(payjoin_options("1", "", "1000", "1.", true).is_none());
    }

    #[test]
    fn watch_scope_takes_the_subject_chain() {
        let b = json!({"state": "bound", "domains": [{"chain": "bitcoin", "network": "regtest"}]});
        let q = Query {
            subject: Some("liquid:liquidv1:tx:ab".into()),
            ..Query::default()
        };
        let s = watch_query_scope(&q, Some(&b));
        assert_eq!(s.chain.as_deref(), Some("liquid"));
        assert_eq!(s.network.as_deref(), Some("main"));
        let s = watch_query_scope(
            &Query {
                chain: Some("bitcoin".into()),
                ..Query::default()
            },
            Some(&b),
        );
        assert_eq!(s.network.as_deref(), Some("regtest"));
        assert_eq!(
            watch_query_scope(&Query::default(), Some(&b)),
            Query::default()
        );
    }
}
