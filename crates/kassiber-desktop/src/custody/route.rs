//! Navigation state of the Transfers screen (`routes/transfers-custody/routeSearch.ts`).
//!
//! `/transfers` is the screen; `/swaps` and `/custody-gaps` are its names from before the
//! redesign and redirect with their navigation state (`routeTree.tsx`). Only this bounded state
//! enters the screen: a `link` (open "Link manually" on a transaction, or `new`), a saved view
//! of the suggestions, or the developer-gated advanced tools (`section=advanced`, a tab, a gap).

use crate::app::App;

/// `link=new`: "Link manually" with nothing picked yet.
pub const LINK_NEW: &str = "new";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Gaps,
    Components,
    Coverage,
}

impl Tab {
    pub fn parse(s: &str) -> Option<Tab> {
        match s {
            "gaps" => Some(Tab::Gaps),
            "components" => Some(Tab::Components),
            "coverage" => Some(Tab::Coverage),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Tab::Gaps => "gaps",
            Tab::Components => "components",
            Tab::Coverage => "coverage",
        }
    }
}

/// `TransfersCustodySearch`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Search {
    pub advanced: bool,
    pub tab: Option<Tab>,
    pub gap: Option<String>,
    pub link: Option<String>,
    pub saved_view: Option<String>,
}

/// `TransfersCustodyView`: what the screen shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum View {
    Overview {
        link: Option<String>,
        saved_view: Option<String>,
    },
    Advanced {
        tab: Tab,
        gap: Option<String>,
    },
}

/// The query's `key=value` pairs, percent-decoded.
pub fn params(query: &str) -> Vec<(String, String)> {
    query
        .trim_start_matches('?')
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (decode(k), decode(v))
        })
        .collect()
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(b);
                    i += 2;
                } else {
                    out.push(b'%');
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~:@".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `SAFE_ENTITY_ID`: `^[A-Za-z0-9][A-Za-z0-9._:@+-]{0,255}$` after trimming.
pub fn entity_id(value: Option<&str>) -> Option<String> {
    let v = value?.trim();
    let mut chars = v.chars();
    let first = chars.next()?;
    let ok = first.is_ascii_alphanumeric()
        && v.len() <= 256
        && chars.all(|c| c.is_ascii_alphanumeric() || "._:@+-".contains(c));
    ok.then(|| v.to_owned())
}

fn get<'a>(raw: &'a [(String, String)], key: &str) -> Option<&'a str> {
    raw.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// `parseTransfersCustodySearch`.
pub fn parse(raw: &[(String, String)]) -> Search {
    if let Some(link) = ["link", "focus", "tx", "transaction"]
        .iter()
        .find_map(|k| entity_id(get(raw, k)))
    {
        return Search {
            link: Some(link),
            ..Search::default()
        };
    }
    if let Some(gap) = ["gap", "gap_id"]
        .iter()
        .find_map(|k| entity_id(get(raw, k)))
    {
        return Search {
            advanced: true,
            tab: Some(Tab::Gaps),
            gap: Some(gap),
            ..Search::default()
        };
    }
    let tab = get(raw, "tab").and_then(Tab::parse);
    if get(raw, "section") == Some("advanced") || tab.is_some() {
        return Search {
            advanced: true,
            tab,
            ..Search::default()
        };
    }
    let saved = entity_id(get(raw, "savedView").or_else(|| get(raw, "saved_view")));
    Search {
        saved_view: saved,
        ..Search::default()
    }
}

/// `custodyGapRedirectSearch`.
pub fn gap_redirect(raw: &[(String, String)]) -> Search {
    let gap = ["gap", "gap_id"]
        .iter()
        .find_map(|k| entity_id(get(raw, k)));
    Search {
        advanced: true,
        tab: Some(Tab::Gaps),
        gap,
        ..Search::default()
    }
}

/// `transfersCustodyView`: the advanced tools fall back to the overview while gated.
pub fn view(raw: &[(String, String)], dev: bool) -> View {
    let s = parse(raw);
    if s.advanced {
        if !dev {
            return View::Overview {
                link: None,
                saved_view: None,
            };
        }
        return View::Advanced {
            tab: s.tab.unwrap_or_default(),
            gap: s.gap,
        };
    }
    View::Overview {
        link: s.link,
        saved_view: s.saved_view,
    }
}

/// The search as a query string (`?…`, or empty), in TanStack's key order.
pub fn query(s: &Search) -> String {
    let mut parts = vec![];
    if s.advanced {
        parts.push("section=advanced".to_owned());
    }
    if let Some(t) = s.tab {
        parts.push(format!("tab={}", t.as_str()));
    }
    if let Some(g) = &s.gap {
        parts.push(format!("gap={}", encode(g)));
    }
    if let Some(l) = &s.link {
        parts.push(format!("link={}", encode(l)));
    }
    if let Some(v) = &s.saved_view {
        parts.push(format!("savedView={}", encode(v)));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}

/// `/transfers` with this search.
pub fn href(s: &Search) -> String {
    format!("/transfers{}", query(s))
}

/// The router's handling of `/transfers`, `/swaps` and `/custody-gaps` (`routeTree.tsx`): the
/// canonical target, or None when `to` already is it.
pub fn redirect(app: &App, path: &str, query_str: &str) -> Option<String> {
    let raw = params(query_str);
    let dev = app.prefs.developer_tools;
    let target = match path {
        // `requireDeveloperTools` runs first.
        "/custody-gaps" if !dev => "/overview".to_owned(),
        "/custody-gaps" => href(&gap_redirect(&raw)),
        _ => {
            let s = parse(&raw);
            if s.advanced && !dev {
                "/transfers".to_owned()
            } else {
                href(&s)
            }
        }
    };
    (target != format!("{path}{query_str}")).then_some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(q: &str) -> Vec<(String, String)> {
        params(q)
    }

    #[test]
    fn parses_like_the_web() {
        assert_eq!(
            parse(&p("?focus=tx-1&mode=swaps")),
            Search {
                link: Some("tx-1".into()),
                ..Default::default()
            }
        );
        assert_eq!(
            parse(&p("?tab=review&focus=abc")).link.as_deref(),
            Some("abc")
        );
        assert_eq!(
            parse(&p("?gap_id=g1")),
            Search {
                advanced: true,
                tab: Some(Tab::Gaps),
                gap: Some("g1".into()),
                ..Default::default()
            }
        );
        assert_eq!(
            parse(&p("?tab=components")),
            Search {
                advanced: true,
                tab: Some(Tab::Components),
                ..Default::default()
            }
        );
        assert_eq!(
            parse(&p("?section=advanced")),
            Search {
                advanced: true,
                ..Default::default()
            }
        );
        assert_eq!(parse(&p("?tab=review")), Search::default());
        assert_eq!(
            parse(&p("?saved_view=v1")).saved_view.as_deref(),
            Some("v1")
        );
        assert_eq!(parse(&p("?link=../etc")), Search::default());
        assert_eq!(
            parse(&p("?link=%20tx%3A1%20")).link.as_deref(),
            Some("tx:1")
        );
    }

    #[test]
    fn gated_view_falls_back() {
        assert_eq!(
            view(&p("?section=advanced&tab=coverage"), false),
            View::Overview {
                link: None,
                saved_view: None
            }
        );
        assert_eq!(
            view(&p("?section=advanced&tab=coverage"), true),
            View::Advanced {
                tab: Tab::Coverage,
                gap: None
            }
        );
        assert_eq!(
            view(&p("?section=advanced"), true),
            View::Advanced {
                tab: Tab::Gaps,
                gap: None
            }
        );
    }

    #[test]
    fn gap_redirect_keeps_the_gap() {
        assert_eq!(
            href(&gap_redirect(&p("?gap=g-2"))),
            "/transfers?section=advanced&tab=gaps&gap=g-2"
        );
        assert_eq!(
            href(&gap_redirect(&p(""))),
            "/transfers?section=advanced&tab=gaps"
        );
    }
}
