//! The picker's data rules, ported from lib/aiCapabilities.ts, providerModelSearch.ts,
//! modelPickerSearch.ts, modelPickerKeys.ts and providerIdentity.ts. Pure functions, unit-tested.

use serde_json::Value;

use crate::assistant::model::Provider;

/// A model row (`AiModelRow`) as a cached listing or the selection names it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelRow {
    pub id: String,
    pub display_name: Option<String>,
    pub owned_by: Option<String>,
    pub source_provider: Option<String>,
    /// `local` | `remote` | `tee`.
    pub privacy_posture: Option<String>,
    pub privacy_reason: Option<String>,
    pub supports_reasoning_effort: Option<bool>,
    pub reasoning_efforts: Vec<String>,
    pub supports_tools: Option<bool>,
    pub output_modalities: Vec<String>,
    pub unsupported_reason: Option<String>,
    pub prefix: Option<String>,
    pub base_model: Option<String>,
    pub supports_fast_mode: Option<bool>,
    pub fast_mode_description: Option<String>,
}

impl ModelRow {
    pub fn bare(id: &str) -> ModelRow {
        ModelRow {
            id: id.to_owned(),
            ..Default::default()
        }
    }

    pub fn parse(v: &Value) -> Option<ModelRow> {
        let s = |k: &str| v[k].as_str().map(str::to_owned).filter(|s| !s.is_empty());
        let list = |k: &str| {
            v[k].as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        Some(ModelRow {
            id: v["id"].as_str()?.to_owned(),
            display_name: s("display_name"),
            owned_by: s("owned_by"),
            source_provider: s("source_provider"),
            privacy_posture: s("privacy_posture"),
            privacy_reason: s("privacy_reason"),
            supports_reasoning_effort: v["supports_reasoning_effort"].as_bool(),
            reasoning_efforts: list("reasoning_efforts"),
            supports_tools: v["supports_tools"].as_bool(),
            output_modalities: list("output_modalities"),
            unsupported_reason: s("unsupported_reason"),
            prefix: s("prefix").filter(|p| !p.trim().is_empty()),
            base_model: s("base_model"),
            supports_fast_mode: v["supports_fast_mode"].as_bool(),
            fast_mode_description: s("fast_mode_description")
                .map(|d| d.trim().to_owned())
                .filter(|d| !d.is_empty()),
        })
    }

    pub fn parse_list(v: &Value) -> Vec<ModelRow> {
        v.as_array()
            .map(|a| a.iter().filter_map(ModelRow::parse).collect())
            .unwrap_or_default()
    }
}

/// What a cached check left for one provider (`AiModelsListData`).
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub models: Vec<ModelRow>,
    pub stale: bool,
    pub error: Option<String>,
    pub checked_at: Option<String>,
}

/// `NATIVE_AI_PROVIDER_BY_LOCATOR`.
pub fn native_runtime(url: &str) -> Option<&'static str> {
    match url.trim().to_ascii_lowercase().as_str() {
        "claude-cli://default" => Some("claude"),
        "codex-cli://default" => Some("codex"),
        "opencode-cli://default" => Some("opencode"),
        "copilot-cli://default" => Some("copilot"),
        _ => None,
    }
}

pub fn is_cli(p: &Provider) -> bool {
    native_runtime(&p.base_url).is_some()
}

/// `providerPosture`: a CLI proxy sends prompts off the device.
pub fn posture(kind: &str) -> &'static str {
    match kind {
        "local" => "local",
        "tee" => "tee",
        _ => "remote",
    }
}

pub fn model_posture(p: &Provider, m: &ModelRow) -> &'static str {
    m.privacy_posture
        .as_deref()
        .map(posture)
        .unwrap_or_else(|| posture(&p.kind))
}

fn posture_rank(p: &str) -> u8 {
    match p {
        "local" => 0,
        "tee" => 1,
        _ => 2,
    }
}

/// Where a provider's prompts go, for the quiet disclosure line under its name (the picker no
/// longer shows LOCAL / REMOTE pills; the disclosure itself is a product rule and stays).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Where {
    /// A local endpoint on loopback.
    ThisDevice,
    /// A local endpoint on a private address or a LAN name.
    Network,
    /// A local endpoint the user named by a public host.
    Host(String),
    /// An off-device provider: its vendor's name, else its host.
    Vendor(String),
    /// OpenCode routes to whichever model services it is configured with.
    OpenCode,
    Tee,
    /// Off the device, nothing more specific known (a CLI proxy, a model-level posture).
    Remote,
}

/// The host of a base URL, without scheme, credentials, port or path (never shows userinfo).
pub fn url_host(url: &str) -> Option<String> {
    let rest = url.trim().split_once("://").map_or(url.trim(), |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(v6) = host_port.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host.ends_with(".localhost")
        || host.starts_with("127.")
        || host == "::1"
        || host == "0.0.0.0"
}

fn is_private(host: &str) -> bool {
    let octets: Vec<u8> = host.split('.').filter_map(|o| o.parse().ok()).collect();
    if octets.len() == 4 && host.split('.').count() == 4 {
        return matches!(octets[..], [10, ..] | [192, 168, ..] | [169, 254, ..])
            || (octets[0] == 172 && (16..=31).contains(&octets[1]))
            || (octets[0] == 100 && (64..=127).contains(&octets[1]));
    }
    if host.contains(':') {
        return host.starts_with("fe80") || host.starts_with("fc") || host.starts_with("fd");
    }
    !host.contains('.')
        || [
            ".local",
            ".lan",
            ".home",
            ".internal",
            ".home.arpa",
            ".ts.net",
        ]
        .iter()
        .any(|s| host.ends_with(s))
}

/// The vendor an off-device host belongs to.
fn vendor(host: &str) -> Option<&'static str> {
    const KNOWN: [(&str, &str); 12] = [
        ("openai.com", "OpenAI"),
        ("anthropic.com", "Anthropic"),
        ("openrouter.ai", "OpenRouter"),
        ("googleapis.com", "Google"),
        ("mistral.ai", "Mistral"),
        ("groq.com", "Groq"),
        ("together.xyz", "Together AI"),
        ("together.ai", "Together AI"),
        ("deepseek.com", "DeepSeek"),
        ("x.ai", "xAI"),
        ("ppq.ai", "PayPerQ"),
        ("githubcopilot.com", "GitHub"),
    ];
    KNOWN
        .iter()
        .find(|(d, _)| host == *d || host.ends_with(&format!(".{d}")))
        .map(|(_, v)| *v)
}

/// `Where` a provider sends prompts.
pub fn where_prompts_go(p: &Provider) -> Where {
    match native_runtime(&p.base_url) {
        Some("claude") => return Where::Vendor("Anthropic".into()),
        Some("codex") => return Where::Vendor("OpenAI".into()),
        Some("copilot") => return Where::Vendor("GitHub".into()),
        Some("opencode") => return Where::OpenCode,
        _ => {}
    }
    let host = url_host(&p.base_url);
    match (posture(&p.kind), host) {
        ("tee", _) => Where::Tee,
        ("local", Some(h)) if is_loopback(&h) => Where::ThisDevice,
        ("local", Some(h)) if is_private(&h) => Where::Network,
        ("local", Some(h)) => Where::Host(h),
        ("local", None) => Where::ThisDevice,
        (_, Some(h)) if p.kind == "remote" && !is_loopback(&h) && !is_private(&h) => {
            Where::Vendor(vendor(&h).map(str::to_owned).unwrap_or(h))
        }
        _ => Where::Remote,
    }
}

/// `Where` a model's prompts go when its listing says otherwise than its provider (an Ollama
/// cloud model, a proxy route); `None` when it goes where the provider's do.
pub fn model_where(p: &Provider, m: &ModelRow) -> Option<Where> {
    let mp = model_posture(p, m);
    if mp == posture(&p.kind) {
        return None;
    }
    Some(match mp {
        "local" => Where::ThisDevice,
        "tee" => Where::Tee,
        _ => Where::Remote,
    })
}

/// Stable privacy-first sort.
pub fn sort_by_posture<T>(items: &mut [T], posture_of: impl Fn(&T) -> &'static str) {
    items.sort_by_key(|i| posture_rank(posture_of(i)));
}

/// `compareModelRoutePrefix`.
pub fn compare_prefix(a: &ModelRow, b: &ModelRow) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    match (&a.prefix, &b.prefix) {
        (x, y) if x == y => Equal,
        (None, _) => Less,
        (_, None) => Greater,
        (Some(x), Some(y)) => x.cmp(y),
    }
}

/// `providerInitials`: up to two initials, ignoring generic words.
pub fn initials(label: &str) -> String {
    let words: Vec<&str> = label
        .split(|c: char| c.is_whitespace() || c == '_' || c == '-')
        .filter(|w| !w.is_empty())
        .collect();
    let distinct: Vec<&str> = words
        .iter()
        .copied()
        .filter(|w| !["cli", "ai", "acp", "agent"].contains(&w.to_lowercase().as_str()))
        .collect();
    let words = if distinct.is_empty() { words } else { distinct };
    match words.len() {
        0 => "?".into(),
        1 => words[0].chars().take(2).collect::<String>().to_uppercase(),
        _ => words
            .iter()
            .take(2)
            .filter_map(|w| w.chars().next())
            .collect::<String>()
            .to_uppercase(),
    }
}

/// `providerIconKey`: the CLI runtime, or `ollama` / `omlx` by name.
pub fn icon_key(p: &Provider) -> Option<&'static str> {
    if let Some(r) = native_runtime(&p.base_url) {
        return Some(r);
    }
    for label in [p.name.as_str(), p.display_name.as_deref().unwrap_or("")] {
        let l = label.trim().to_ascii_lowercase();
        let word = |w: &str| {
            l.starts_with(w) && !l[w.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
        };
        if word("ollama") {
            return Some("ollama");
        }
        if word("omlx") {
            return Some("omlx");
        }
    }
    None
}

/// `dedupeProviderRows`: one row per native CLI locator (the picked one, else the runtime's own).
pub fn dedupe(providers: &[&Provider], preferred: Option<&str>) -> Vec<Provider> {
    let loc = |p: &Provider| p.base_url.trim().to_lowercase();
    let mut keep: Vec<(String, String)> = vec![];
    for p in providers {
        let l = loc(p);
        let Some(runtime) = native_runtime(&l) else {
            continue;
        };
        if keep.iter().any(|(k, _)| *k == l) {
            continue;
        }
        let same: Vec<&&Provider> = providers.iter().filter(|c| loc(c) == l).collect();
        let pick = same
            .iter()
            .find(|c| Some(c.name.as_str()) == preferred)
            .or_else(|| same.iter().find(|c| c.name == runtime))
            .map(|c| c.name.clone())
            .unwrap_or_else(|| p.name.clone());
        keep.push((l, pick));
    }
    providers
        .iter()
        .filter(|p| match keep.iter().find(|(k, _)| *k == loc(p)) {
            Some((_, name)) => *name == p.name,
            None => true,
        })
        .map(|p| (*p).clone())
        .collect()
}

const NON_CHAT: [&str; 6] = [
    "embed",
    "whisper",
    "dall-e",
    "gpt-image",
    "moderation",
    "transcribe",
];

/// `modelIdNamesNonChatModel`.
pub fn names_non_chat(id: &str) -> bool {
    let l = id.to_lowercase();
    let word = |w: &str| {
        l.match_indices(w).any(|(i, _)| {
            let before = l[..i].chars().last();
            let after = l[i + w.len()..].chars().next();
            !before.is_some_and(|c| c.is_ascii_lowercase())
                && !after.is_some_and(|c| c.is_ascii_lowercase())
        })
    };
    NON_CHAT.iter().any(|p| l.contains(p)) || l.contains("realtime") || word("tts") || word("audio")
}

/// `httpModelRunsAssistant`: judged from the cached listing alone.
pub fn runs_assistant(m: &ModelRow) -> bool {
    if m.unsupported_reason.is_some() || m.supports_tools == Some(false) {
        return false;
    }
    if !m.output_modalities.is_empty() {
        return m.output_modalities.iter().any(|o| o == "text");
    }
    if m.supports_tools == Some(true) {
        return true;
    }
    !names_non_chat(&m.id)
}

/// `modelPickerModelKey`: the provider name length-prefixed, so ids with `:` never collide.
pub fn model_key(provider: &str, model: &str) -> String {
    format!("model:{}:{provider}{model}", provider.len())
}

/// All levels Kassiber can label, lowest first (`KNOWN_EFFORTS`).
pub const KNOWN_EFFORTS: [&str; 6] = ["low", "medium", "high", "xhigh", "max", "ultra"];
const HTTP_EFFORTS: [&str; 3] = ["low", "medium", "high"];
const CLI_PROXY_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
/// The fixed levels of the native runtimes whose models list none
/// (`NATIVE_RUNTIME_REASONING_EFFORTS`): the provider broker's `CLAUDE_EFFORTS` and
/// `COPILOT_EFFORTS`, whose tests keep these lines equal to them.
const NATIVE_RUNTIME_EFFORTS: &[(&str, &[&str])] = &[
    ("claude", &["low", "medium", "high", "xhigh", "max"]),
    ("copilot", &["low", "medium", "high", "xhigh", "max"]),
];

fn raw_efforts(v: &Value) -> Vec<String> {
    v["reasoning_efforts"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(str::to_lowercase)
                .collect()
        })
        .unwrap_or_default()
}

fn capability_supports_effort(v: &Value) -> bool {
    let truthy =
        |x: &Value| x.as_bool() == Some(true) || matches!(x.as_str(), Some("true" | "supported"));
    v.is_object()
        && (truthy(&v["reasoning_effort"])
            || v["supported_parameters"]
                .as_array()
                .is_some_and(|a| a.iter().any(|x| x == "reasoning_effort"))
            || !raw_efforts(v).is_empty())
}

/// `providerSupportsReasoningEffort`.
pub fn provider_supports_effort(p: &Provider) -> bool {
    p.supports_reasoning_effort || capability_supports_effort(&p.raw["capabilities"])
}

/// `selectedModelSupportsReasoningEffort`.
pub fn supports_effort(p: Option<&Provider>, model: Option<&ModelRow>) -> bool {
    if model.and_then(|m| m.supports_reasoning_effort) == Some(false) {
        return false;
    }
    if p.is_some_and(provider_supports_effort) {
        return true;
    }
    if p.is_some_and(|p| !is_cli(p)) {
        return model.and_then(|m| m.supports_reasoning_effort) == Some(true);
    }
    model.is_some_and(|m| {
        m.supports_reasoning_effort == Some(true) || !m.reasoning_efforts.is_empty()
    })
}

/// `selectedModelReasoningEfforts`: only what the model (or its runtime) advertises.
pub fn advertised_efforts(p: Option<&Provider>, model: Option<&ModelRow>) -> Vec<String> {
    let dedup = |v: Vec<String>| {
        let mut out: Vec<String> = vec![];
        for e in v {
            if !out.contains(&e) {
                out.push(e);
            }
        }
        out
    };
    if let Some(p) = p {
        if p.kind == "cli_proxy" {
            if model.and_then(|m| m.supports_reasoning_effort) != Some(true) {
                return vec![];
            }
            let listed: Vec<String> = model
                .map(|m| {
                    m.reasoning_efforts
                        .iter()
                        .map(|e| e.to_lowercase())
                        .filter(|e| CLI_PROXY_EFFORTS.contains(&e.as_str()))
                        .collect()
                })
                .unwrap_or_default();
            return if listed.is_empty() {
                CLI_PROXY_EFFORTS.iter().map(|s| s.to_string()).collect()
            } else {
                dedup(listed)
            };
        }
        if !is_cli(p) {
            if model.and_then(|m| m.supports_reasoning_effort) != Some(true) {
                return vec![];
            }
            let listed: Vec<String> = model
                .map(|m| {
                    m.reasoning_efforts
                        .iter()
                        .map(|e| e.to_lowercase())
                        .filter(|e| HTTP_EFFORTS.contains(&e.as_str()))
                        .collect()
                })
                .unwrap_or_default();
            return if listed.is_empty() {
                HTTP_EFFORTS.iter().map(|s| s.to_string()).collect()
            } else {
                dedup(listed)
            };
        }
    }
    let mut adv: Vec<String> = model
        .map(|m| {
            m.reasoning_efforts
                .iter()
                .map(|e| e.to_lowercase())
                .collect()
        })
        .unwrap_or_default();
    if let Some(p) = p {
        adv.extend(raw_efforts(&p.raw["capabilities"]));
    }
    if adv.is_empty()
        && model.is_none_or(|m| m.reasoning_efforts.is_empty())
        && let Some(r) = p.and_then(|p| native_runtime(&p.base_url))
        && let Some((_, fixed)) = NATIVE_RUNTIME_EFFORTS.iter().find(|(n, _)| *n == r)
    {
        return fixed.iter().map(|s| s.to_string()).collect();
    }
    dedup(adv)
}

/// `selectedModelFastMode`: (supported, description, runtime).
pub fn fast_mode(
    p: Option<&Provider>,
    model: Option<&ModelRow>,
) -> (bool, Option<String>, Option<&'static str>) {
    let Some(runtime) = p.and_then(|p| native_runtime(&p.base_url)) else {
        return (false, None, None);
    };
    match model.and_then(|m| {
        m.supports_fast_mode
            .map(|s| (s, m.fast_mode_description.clone()))
    }) {
        Some((true, d)) => (true, d, Some(runtime)),
        _ => (false, None, Some(runtime)),
    }
}

// ---- Search (modelPickerSearch.ts) ------------------------------------------------------------

/// Lower is better; None when `query` is not a subsequence of `value`.
pub fn score_subsequence(value: &str, query: &str) -> Option<i64> {
    let v: Vec<char> = value.chars().collect();
    let q: Vec<char> = query.chars().collect();
    if q.is_empty() {
        return Some(0);
    }
    let (mut qi, mut first, mut prev, mut gap) = (0usize, -1i64, -1i64, 0i64);
    for (i, c) in v.iter().enumerate() {
        if *c != q[qi] {
            continue;
        }
        let i = i as i64;
        if first == -1 {
            first = i;
        }
        if prev != -1 {
            gap += i - prev - 1;
        }
        prev = i;
        qi += 1;
        if qi == q.len() {
            let span = i - first + 1 - q.len() as i64;
            let len = (v.len() as i64 - q.len() as i64).min(64);
            return Some(first * 2 + gap * 3 + span + len);
        }
    }
    None
}

fn len_penalty(value: &str, query: &str) -> i64 {
    (value.chars().count() as i64 - query.chars().count() as i64).clamp(0, 64)
}

fn char_index(s: &str, byte: usize) -> i64 {
    s[..byte].chars().count() as i64
}

/// Tiered match: exact, prefix, word boundary, substring, then fuzzy for 3+ character tokens.
pub fn score_query(value: &str, query: &str, base: i64, fuzzy: bool) -> Option<i64> {
    if value.is_empty() || query.is_empty() {
        return None;
    }
    if value == query {
        return Some(base);
    }
    if value.starts_with(query) {
        return Some(base + 2 + len_penalty(value, query));
    }
    let mut boundary: Option<usize> = None;
    for m in [' ', '-', '_', '/', ':', '.'] {
        if let Some(i) = value.find(&format!("{m}{query}")) {
            let at = i + m.len_utf8();
            boundary = Some(boundary.map_or(at, |b| b.min(at)));
        }
    }
    if let Some(b) = boundary {
        return Some(base + 4 + char_index(value, b) * 2 + len_penalty(value, query));
    }
    if let Some(i) = value.find(query) {
        return Some(base + 6 + char_index(value, i) * 2 + len_penalty(value, query));
    }
    if fuzzy {
        return score_subsequence(value, query).map(|s| base + 100 + s);
    }
    None
}

/// What a row is searched by (`ModelPickerSearchable`).
pub struct Searchable<'a> {
    pub id: &'a str,
    pub display_name: Option<&'a str>,
    pub owned_by: Option<&'a str>,
    pub source_provider: Option<&'a str>,
    pub provider_name: &'a str,
    pub provider_display_name: &'a str,
    pub favorite: bool,
}

/// `scoreModelPickerSearch`: every token must match some field; favorites get a boost.
pub fn score_search(m: &Searchable, query: &str) -> Option<i64> {
    let q = query.trim().to_lowercase();
    let tokens: Vec<&str> = q.split_whitespace().collect();
    if tokens.is_empty() {
        return Some(0);
    }
    let raw: Vec<&str> = [
        m.display_name,
        Some(m.id),
        m.source_provider,
        m.owned_by,
        Some(m.provider_display_name),
        Some(m.provider_name),
    ]
    .into_iter()
    .flatten()
    .filter(|s| !s.is_empty())
    .collect();
    let mut fields: Vec<String> = raw.iter().map(|s| s.trim().to_lowercase()).collect();
    let all: Vec<&str> = [
        m.display_name,
        Some(m.id),
        m.owned_by,
        m.source_provider,
        Some(m.provider_display_name),
        Some(m.provider_name),
    ]
    .into_iter()
    .flatten()
    .filter(|s| !s.is_empty())
    .collect();
    fields.push(all.join(" ").trim().to_lowercase());
    let mut score = 0;
    for t in tokens {
        let best = fields
            .iter()
            .enumerate()
            .filter_map(|(i, f)| score_query(f, t, i as i64 * 10, t.chars().count() >= 3))
            .min()?;
        score += best;
    }
    Some(if m.favorite { score - 24 } else { score })
}

/// `rankModelPickerItems`: drops non-matches; ties keep favorites, then the given order.
pub fn rank<T>(
    items: Vec<T>,
    query: &str,
    searchable: impl for<'b> Fn(&'b T) -> Searchable<'b>,
) -> Vec<T> {
    let mut scored: Vec<(i64, bool, usize, T)> = items
        .into_iter()
        .enumerate()
        .filter_map(|(i, it)| {
            let s = searchable(&it);
            let fav = s.favorite;
            score_search(&s, query).map(|sc| (sc, fav, i, it))
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
    scored.into_iter().map(|e| e.3).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s<'a>(id: &'a str, provider: &'a str, fav: bool) -> Searchable<'a> {
        Searchable {
            id,
            display_name: None,
            owned_by: None,
            source_provider: None,
            provider_name: provider,
            provider_display_name: provider,
            favorite: fav,
        }
    }

    #[test]
    fn keys_never_collide() {
        assert_ne!(model_key("a:b", "c"), model_key("a", "b:c"));
        assert_eq!(
            model_key("ollama", "qwen3.6:35b"),
            "model:6:ollamaqwen3.6:35b"
        );
    }

    #[test]
    fn search_tiers_rank_exact_prefix_boundary_substring_fuzzy() {
        assert_eq!(score_query("qwen", "qwen", 0, true), Some(0));
        assert!(
            score_query("qwen3", "qwen", 0, true).unwrap()
                < score_query("big-qwen", "qwen", 0, true).unwrap()
        );
        // The web's formula: base + index * 2 + length penalty (boundary 4, substring 6).
        assert_eq!(score_query("big-qwen", "qwen", 0, true), Some(16));
        assert_eq!(score_query("bigqwen", "qwen", 0, true), Some(15));
        assert!(score_query("q-w-e-n", "qwen", 0, true).unwrap() >= 100);
        assert_eq!(score_query("abc", "qwen", 0, true), None);
        assert_eq!(score_query("q-w-e", "qwe", 0, false), None);
    }

    #[test]
    fn every_token_must_match_and_favorites_lead_ties() {
        assert!(score_search(&s("claude-opus", "claude", false), "claude opus").is_some());
        assert_eq!(
            score_search(&s("claude-opus", "claude", false), "claude gemini"),
            None
        );
        let ranked = rank(vec![("a", false), ("a", true)], "qwen", |(_, f)| {
            s("qwen3", "ollama", *f)
        });
        assert!(ranked[0].1);
        // A model-name hit outranks a hit on the serving provider.
        let ranked = rank(vec!["byprovider", "byname"], "olla", |x| {
            if *x == "byname" {
                s("ollama-mix", "local", false)
            } else {
                s("mix", "ollama", false)
            }
        });
        assert_eq!(ranked[0], "byname");
    }

    #[test]
    fn non_chat_models_are_hidden_by_id_only_when_the_listing_is_silent() {
        assert!(!runs_assistant(&ModelRow::bare("nomic-embed-text")));
        assert!(!runs_assistant(&ModelRow::bare("gpt-4o-mini-tts")));
        assert!(!runs_assistant(&ModelRow::bare("gpt-audio")));
        assert!(runs_assistant(&ModelRow::bare("qwen3.6:35b")));
        assert!(runs_assistant(&ModelRow::bare("gpt-4o-audiovisual")));
        let mut m = ModelRow::bare("nomic-embed-text");
        m.supports_tools = Some(true);
        assert!(runs_assistant(&m));
        m.output_modalities = vec!["image".into()];
        assert!(!runs_assistant(&m));
    }

    #[test]
    fn initials_skip_generic_words() {
        assert_eq!(initials("Gemini CLI"), "GE");
        assert_eq!(initials("GitHub Copilot"), "GC");
        assert_eq!(initials("my-local server"), "ML");
        assert_eq!(initials(""), "?");
    }

    fn provider(name: &str, url: &str, kind: &str) -> Provider {
        Provider::parse_list(
            &serde_json::json!({"providers": [{"name": name, "base_url": url, "kind": kind}]}),
        )
        .remove(0)
    }

    #[test]
    fn efforts_follow_the_runtime_and_the_listing() {
        let claude = provider("claude", "claude-cli://default", "remote");
        assert_eq!(advertised_efforts(Some(&claude), None).len(), 5);
        let codex = provider("codex", "codex-cli://default", "remote");
        assert!(advertised_efforts(Some(&codex), None).is_empty());
        let ollama = provider("ollama", "http://localhost:11434/v1", "local");
        assert!(advertised_efforts(Some(&ollama), Some(&ModelRow::bare("qwen"))).is_empty());
        let mut m = ModelRow::bare("qwen");
        m.supports_reasoning_effort = Some(true);
        assert_eq!(
            advertised_efforts(Some(&ollama), Some(&m)),
            vec!["low", "medium", "high"]
        );
        assert!(supports_effort(Some(&ollama), Some(&m)));
        assert!(!supports_effort(Some(&ollama), None));
    }

    #[test]
    fn disclosure_names_where_prompts_go_without_credentials() {
        assert_eq!(
            where_prompts_go(&provider("ollama", "http://localhost:11434/v1", "local")),
            Where::ThisDevice
        );
        assert_eq!(
            where_prompts_go(&provider("box", "http://192.168.1.20:8080/v1", "local")),
            Where::Network
        );
        assert_eq!(
            where_prompts_go(&provider("box", "http://studio.local:8000/v1", "local")),
            Where::Network
        );
        assert_eq!(
            where_prompts_go(&provider(
                "box",
                "https://user:pw@llm.example.org/v1",
                "local"
            )),
            Where::Host("llm.example.org".into())
        );
        assert_eq!(
            where_prompts_go(&provider("oa", "https://api.openai.com/v1", "remote")),
            Where::Vendor("OpenAI".into())
        );
        assert_eq!(
            where_prompts_go(&provider(
                "x",
                "https://user:pw@api.example.com/v1",
                "remote"
            )),
            Where::Vendor("api.example.com".into())
        );
        assert_eq!(
            where_prompts_go(&provider("claude", "claude-cli://default", "remote")),
            Where::Vendor("Anthropic".into())
        );
        assert_eq!(
            where_prompts_go(&provider("oc", "opencode-cli://default", "remote")),
            Where::OpenCode
        );
        assert_eq!(
            where_prompts_go(&provider("proxy", "http://127.0.0.1:8317/v1", "cli_proxy")),
            Where::Remote
        );
        assert_eq!(url_host("http://[::1]:11434/v1").as_deref(), Some("::1"));
        let ollama = provider("ollama", "http://localhost:11434/v1", "local");
        let mut cloud = ModelRow::bare("gpt-oss:120b-cloud");
        assert_eq!(model_where(&ollama, &cloud), None);
        cloud.privacy_posture = Some("remote".into());
        assert_eq!(model_where(&ollama, &cloud), Some(Where::Remote));
    }

    #[test]
    fn dedupe_keeps_one_row_per_cli_locator() {
        let a = provider("claude", "claude-cli://default", "remote");
        let b = provider("claude-work", "claude-cli://default", "remote");
        let c = provider("ollama", "http://localhost:11434/v1", "local");
        let kept = dedupe(&[&b, &a, &c], None);
        assert_eq!(
            kept.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["claude", "ollama"]
        );
        let kept = dedupe(&[&b, &a, &c], Some("claude-work"));
        assert_eq!(
            kept.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["claude-work", "ollama"]
        );
        assert_eq!(icon_key(&c), Some("ollama"));
    }
}
