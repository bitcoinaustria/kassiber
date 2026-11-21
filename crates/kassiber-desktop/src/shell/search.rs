//! The ⌘K palette (ShellSearch.tsx + search/appSearch.ts). An empty query shows the launcher
//! ("Go to", "Quick actions"); a typed one ranks pages, actions, settings, wallets, review
//! items and transactions from the snapshot, and a txid-like query also asks
//! `ui.transactions.resolve`. Arrows wrap, Enter runs, Escape closes.

use rune::{Color, El, Icon, Key, KeyEvent, Tx, col, row, text};
use rune_kit::TextInput;
use serde_json::{Value, json};

use super::sidebar::shortcut;
use super::{PAGES, Popup, go};
use crate::app::{App, Reply};
use crate::i18n::{t, tc};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

#[derive(Clone, Debug)]
pub enum Action {
    Nav(String),
    Refresh,
    AddWallet,
    ProcessJournals,
    Lock,
    Settings,
    FullRescan,
}

/// A palette group: the id selects `chrome:search.group.<id>` for its heading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    GoTo,
    QuickActions,
    Actions,
    Pages,
    Review,
    Settings,
    Transactions,
    Wallets,
}

impl Group {
    fn heading(self) -> String {
        t(match self {
            Group::GoTo => "chrome:search.group.goTo",
            Group::QuickActions => "chrome:search.group.quickActions",
            Group::Actions => "chrome:search.group.actions",
            Group::Pages => "chrome:search.group.pages",
            Group::Review => "chrome:search.group.review",
            Group::Settings => "chrome:search.group.settings",
            Group::Transactions => "chrome:search.group.transactions",
            Group::Wallets => "chrome:search.group.wallets",
        })
    }
    /// Result order of a typed query (alphabetical by id in the web's ranking ties).
    fn rank(self) -> usize {
        match self {
            Group::GoTo | Group::QuickActions | Group::Actions => 0,
            Group::Pages => 1,
            Group::Review => 3,
            Group::Settings => 4,
            Group::Transactions => 5,
            Group::Wallets => 6,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Item {
    pub group: Group,
    pub label: String,
    pub detail: Option<String>,
    pub shortcut: Option<String>,
    pub icon: &'static Icon,
    pub action: Action,
}

fn item(
    group: Group,
    label: &str,
    detail: Option<String>,
    shortcut: Option<String>,
    icon: &'static Icon,
    action: Action,
) -> Item {
    Item {
        group,
        label: label.to_owned(),
        detail,
        shortcut,
        icon,
        action,
    }
}

/// `SUGGESTED_RESULT_IDS`: page titles come from `nav:book.*`, actions from `search:action.*`.
fn launcher(app: &App) -> Vec<Item> {
    let go = |key: &str, icon: &'static Icon, href: &str, sc: &str| {
        item(
            Group::GoTo,
            &t(key),
            None,
            Some(shortcut(sc, false)),
            icon,
            if href == "/settings" {
                Action::Settings
            } else {
                Action::Nav(href.into())
            },
        )
    };
    let action = |id: &str, sc: Option<String>, icon: &'static Icon, a: Action| {
        item(
            Group::QuickActions,
            &t(&format!("search:action.{id}.title")),
            None,
            sc,
            icon,
            a,
        )
    };
    let mut out = vec![
        go("nav:book.overview", &I::GAUGE, "/overview", "1"),
        go(
            "nav:book.transactions",
            &I::ARROW_LEFT_RIGHT,
            "/transactions",
            "2",
        ),
        go("nav:book.wallets", &I::WALLET, "/connections", "3"),
        go("nav:book.review", &I::SHIELD_ALERT, "/review", "8"),
        go("nav:book.reports", &I::CHART_COLUMN, "/reports", "5"),
    ];
    if app.prefs.ai_features {
        out.push(go(
            "nav:book.assistant",
            &I::MESSAGE_SQUARE_TEXT,
            "/assistant",
            "9",
        ));
    }
    out.extend([
        go("nav:book.ledger", &I::CLIPBOARD_LIST, "/journals", "7"),
        go("nav:book.settings", &I::SETTINGS, "/settings", ","),
        action(
            "sync-wallets",
            Some(shortcut("R", false)),
            &I::ARROW_LEFT_RIGHT,
            Action::Refresh,
        ),
        action(
            "add-wallet",
            Some(shortcut("A", true)),
            &I::WALLET,
            Action::AddWallet,
        ),
        action(
            "process-journals",
            Some(shortcut("J", true)),
            &I::CLIPBOARD_LIST,
            Action::ProcessJournals,
        ),
        action(
            "lock-app",
            Some(shortcut("L", false)),
            &I::LOCK_KEYHOLE,
            Action::Lock,
        ),
    ]);
    out
}

/// Search subtitles of pages (`search:page.<id>.subtitle`) by route.
fn page_subtitle(href: &str) -> Option<String> {
    let id = match href {
        "/overview" => "overview",
        "/transactions" => "transactions",
        "/connections" => "connections",
        "/books" => "books",
        "/source-of-funds" => "source-of-funds",
        "/journals" => "journals",
        "/reports" => "reports",
        "/exit-tax" => "exit-tax",
        "/review" => "quarantine",
        "/transfers" => "swaps-transfers",
        "/logs" => "logs",
        "/assistant" => "assistant",
        "/chain-analysis" => "chain-analysis",
        _ => return None,
    };
    Some(t(&format!("search:page.{id}.subtitle")))
}

pub fn looks_like_txid(q: &str) -> bool {
    let q = q.trim();
    let hex = (12..=64).contains(&q.len()) && q.chars().all(|c| c.is_ascii_hexdigit());
    let uuid = q.len() == 36
        && q.chars().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        });
    let tx = q.strip_prefix("tx").is_some_and(|r| {
        !r.is_empty()
            && r.chars()
                .all(|c| c.is_ascii_alphanumeric() || ":_-".contains(c))
    });
    hex || uuid || tx
}

/// How well `hay` matches every term of `q` (0 = no match); prefix matches rank higher.
fn score(hay: &str, terms: &[String]) -> u32 {
    let h = hay.to_lowercase();
    let mut total = 0;
    for t in terms {
        if h.starts_with(t.as_str()) {
            total += 3;
        } else if h
            .split(|c: char| !c.is_alphanumeric())
            .any(|w| w.starts_with(t.as_str()))
        {
            total += 2;
        } else if h.contains(t.as_str()) {
            total += 1;
        } else {
            return 0;
        }
    }
    total
}

/// The best `score` over separate fields (`bestTextMatch`): every term has to match within one
/// field, so a phrase like "open overview" matches its alias, not words spread over fields.
fn best_score<S: AsRef<str>>(fields: &[S], terms: &[String]) -> u32 {
    fields
        .iter()
        .map(|f| score(f.as_ref(), terms))
        .max()
        .unwrap_or(0)
}

/// `uniqueKeywords`: the lists in order, each phrase once (trimmed, case-insensitive), blanks dropped.
fn unique(lists: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    lists
        .into_iter()
        .filter(|k| !k.trim().is_empty() && seen.insert(k.trim().to_lowercase()))
        .collect()
}

// The English alias templates appSearch.ts renders before the language's own
// (`DEFAULT_*_ALIAS_TEMPLATES`), so English phrases keep matching in German.
const PAGE_ALIASES: [&str; 5] = [
    "{{title}} page",
    "{{title}} screen",
    "{{title}} view",
    "open {{title}}",
    "go to {{title}}",
];
const LABEL_ALIASES: [&str; 7] = [
    "{{label}} wallet",
    "{{label}} connection",
    "{{label}} detail",
    "{{label}} wallet detail",
    "open {{label}}",
    "open {{label}} wallet",
    "open {{label}} detail",
];
const KIND_ALIASES: [&str; 2] = ["{{kind}} wallet", "{{kind}} connection"];
const STATUS_ALIASES: [&str; 1] = ["{{status}} wallet"];
const NETWORK_ALIASES: [&str; 1] = ["{{network}} wallet"];

/// `localizedAliasKeywords`: the English templates, then `search:aliases.<list>` in the current
/// language, with `{{var}}` filled in.
fn aliases(list: &str, templates: &[&str], var: &str, value: &str) -> Vec<String> {
    let mut out: Vec<String> = templates
        .iter()
        .map(|tpl| tpl.replace(&format!("{{{{{var}}}}}"), value))
        .collect();
    let mut i = 0;
    loop {
        let key = format!("search:aliases.{list}.{i}");
        if !crate::i18n::has(&key) {
            break;
        }
        out.push(crate::i18n::tr(&key, &[(var, value)]));
        i += 1;
    }
    unique(out)
}

/// `PAGE_RESULTS`' English title and keywords by route: the synonyms a page also answers to.
fn page_synonyms(href: &str) -> (&'static str, &'static [&'static str]) {
    match href {
        "/overview" => ("Overview", &["dashboard", "home", "balance", "portfolio"]),
        "/transactions" => (
            "Transactions",
            &["tx", "counterparty", "account", "amount", "import"],
        ),
        "/connections" => (
            "Wallets",
            &["connections", "wallets", "xpub", "backend", "sync"],
        ),
        "/books" => (
            "Books",
            &[
                "book",
                "books",
                "book set",
                "bird's eye",
                "overview",
                "tax",
                "country",
            ],
        ),
        "/source-of-funds" => (
            "Source of Funds",
            &["source", "funds", "wallet", "balance", "provenance"],
        ),
        "/journals" => (
            "Ledger",
            &["journal", "process", "entries", "fees", "basis", "ledger"],
        ),
        "/reports" => ("Reports", &["csv", "pdf", "xlsx", "tax", "austria", "e1kv"]),
        "/exit-tax" => (
            "Exit Tax",
            &[
                "exit",
                "exit-tax",
                "wegzug",
                "wegzugsbesteuerung",
                "departure",
                "leaving",
                "emigration",
                "relocation",
                "deemed disposal",
            ],
        ),
        "/review" => (
            "Review",
            &[
                "review",
                "quarantine",
                "issues",
                "missing",
                "price",
                "transfer",
                "pair",
            ],
        ),
        "/egress" => (
            "Egress",
            &["egress", "network", "privacy", "telemetry", "hosts"],
        ),
        "/transfers" => (
            "Transfers",
            &[
                "swap",
                "swaps",
                "transfer",
                "transfers",
                "review",
                "pair",
                "custody",
                "components",
            ],
        ),
        "/custody-gaps" => (
            "Missing wallets",
            &[
                "custody",
                "missing wallet",
                "wallet roll",
                "whirlpool",
                "coinjoin",
            ],
        ),
        "/settings" => (
            "Settings",
            &["preferences", "backends", "providers", "privacy", "lock"],
        ),
        "/logs" => ("Logs", &["log", "logs", "error", "daemon", "download"]),
        "/assistant" => ("Assistant", &["chat", "ai", "tools"]),
        "/chain-analysis" => (
            "Chain Analysis",
            &[
                "chain",
                "graph",
                "trace",
                "investigation",
                "entropy",
                "outpoint",
                "analyse",
                "herkunft",
            ],
        ),
        _ => ("", &[]),
    }
}

/// The fields a page matches on (`localizePageResult`): its title and subtitle, the English
/// title and keywords, and the page phrases ("Overview page", "open Overview", "Übersicht öffnen")
/// for both titles.
fn page_fields(href: &str, title: &str, subtitle: Option<&str>) -> Vec<String> {
    let (default_title, keywords) = page_synonyms(href);
    let mut fields = vec![title.to_owned(), subtitle.unwrap_or("").to_owned()];
    fields.extend(keywords.iter().map(|k| (*k).to_owned()));
    fields.push(default_title.to_owned());
    if !default_title.is_empty() && !default_title.eq_ignore_ascii_case(title.trim()) {
        fields.extend(aliases(
            "pagePhrases",
            &PAGE_ALIASES,
            "title",
            default_title,
        ));
    }
    fields.extend(aliases("pagePhrases", &PAGE_ALIASES, "title", title));
    unique(fields)
}

/// The fields a wallet matches on (`connectionResult`): its label, kind, status and chain
/// network, the wallet keywords, and the wallet phrases ("Cold wallet", "open Cold",
/// "descriptor wallet", "regtest wallet").
fn wallet_fields(c: &crate::model::Connection) -> Vec<String> {
    let chain_network = [c.chain.as_str(), c.network.as_deref().unwrap_or("")]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    let mut fields = vec![
        c.label.clone(),
        c.kind.clone(),
        c.status.clone(),
        chain_network.clone(),
    ];
    fields.extend(aliases(
        "connectionLabelPhrases",
        &LABEL_ALIASES,
        "label",
        &c.label,
    ));
    fields.extend(aliases(
        "connectionKindPhrases",
        &KIND_ALIASES,
        "kind",
        &c.kind,
    ));
    fields.extend(aliases(
        "connectionStatusPhrases",
        &STATUS_ALIASES,
        "status",
        &c.status,
    ));
    if !chain_network.is_empty() {
        fields.extend(aliases(
            "connectionNetworkPhrases",
            &NETWORK_ALIASES,
            "network",
            &chain_network,
        ));
    }
    fields.extend(
        [
            "connection",
            "connections",
            "wallet",
            "wallets",
            "source",
            "sync",
        ]
        .map(str::to_owned),
    );
    fields.push(c.chain.clone());
    fields.push(c.network.clone().unwrap_or_default());
    unique(fields)
}

pub fn results(app: &App) -> Vec<Item> {
    let q = app.shell.search.text.trim().to_owned();
    if q.is_empty() {
        return launcher(app);
    }
    let terms: Vec<String> = q
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let mut scored: Vec<(u32, Item)> = Vec::new();
    let mut push = |s: u32, it: Item| {
        if s > 0 {
            scored.push((s, it));
        }
    };
    // Actions (`search:action.*`): title and subtitle both match.
    let actions: [(&str, Option<String>, &'static Icon, Action); 6] = [
        (
            "sync-wallets",
            Some(shortcut("R", false)),
            &I::ARROW_LEFT_RIGHT,
            Action::Refresh,
        ),
        (
            "add-wallet",
            Some(shortcut("A", true)),
            &I::WALLET,
            Action::AddWallet,
        ),
        (
            "process-journals",
            Some(shortcut("J", true)),
            &I::CLIPBOARD_LIST,
            Action::ProcessJournals,
        ),
        (
            "lock-app",
            Some(shortcut("L", false)),
            &I::LOCK_KEYHOLE,
            Action::Lock,
        ),
        (
            "export-report",
            None,
            &I::CHART_COLUMN,
            Action::Nav("/reports".into()),
        ),
        (
            "change-passphrase",
            None,
            &I::LOCK_KEYHOLE,
            Action::Nav("/settings/security".into()),
        ),
    ];
    for (id, sc, icon, action) in actions {
        let title = t(&format!("search:action.{id}.title"));
        let subtitle = t(&format!("search:action.{id}.subtitle"));
        push(
            score(&format!("{title} {subtitle}"), &terms),
            item(Group::Actions, &title, Some(subtitle), sc, icon, action),
        );
    }
    let rescan = t("chrome:shell.refreshMenu.fullRescan");
    push(
        score(&rescan, &terms),
        item(
            Group::Actions,
            &rescan,
            None,
            Some(shortcut("R", true)),
            &I::ROTATE_CCW,
            Action::FullRescan,
        ),
    );
    for (href, key) in PAGES {
        let gated = (super::DEV_LOCKED.contains(href) || super::DEV_HIDDEN.contains(href))
            && !app.prefs.developer_tools;
        if gated || *href == "/diagnostics" || (*href == "/assistant" && !app.prefs.ai_features) {
            continue;
        }
        let title = t(key);
        let sub = page_subtitle(href);
        push(
            best_score(&page_fields(href, &title, sub.as_deref()), &terms),
            item(
                Group::Pages,
                &title,
                sub,
                None,
                page_icon(href),
                Action::Nav((*href).into()),
            ),
        );
    }
    if let Some(b) = &app.badges {
        if b.total() > 0.0 {
            let n = b.total() as usize;
            let title = t("search:review.queue.title");
            let sub = tc("search:review.queue.subtitle", n, &[]);
            push(
                score(&format!("{title} {sub}"), &terms),
                item(
                    Group::Review,
                    &title,
                    Some(sub),
                    None,
                    &I::SHIELD_ALERT,
                    Action::Nav("/review".into()),
                ),
            );
        }
        if b.journals_needs_processing {
            let title = t("search:review.journals.title");
            let sub = t("search:review.journals.subtitle");
            push(
                score(&format!("{title} {sub}"), &terms),
                item(
                    Group::Review,
                    &title,
                    Some(sub),
                    None,
                    &I::CLIPBOARD_LIST,
                    Action::ProcessJournals,
                ),
            );
        }
    }
    // Sections and one result per setting (`settingsResults`); a setting opens its row.
    for r in crate::settings::search::results(app) {
        push(
            score(
                &format!("{} {} {}", r.title, r.subtitle, r.keywords),
                &terms,
            ),
            item(
                Group::Settings,
                &r.title,
                Some(r.subtitle),
                None,
                &I::SETTINGS,
                Action::Nav(r.href),
            ),
        );
    }
    if let Some(snap) = app.snapshot() {
        for c in &snap.connections {
            let detail = format!("{} · {}", c.kind, crate::format::btc(c.balance, 8, false));
            push(
                best_score(&wallet_fields(c), &terms),
                item(
                    Group::Wallets,
                    &c.label,
                    Some(detail),
                    None,
                    &I::WALLET,
                    Action::Nav(format!("/connections/{}", c.id)),
                ),
            );
        }
        let mut seen = std::collections::HashSet::new();
        let mut tx_items = Vec::new();
        for tx in snap.txs.iter().chain(snap.activity_txs.iter()) {
            if !seen.insert(tx.id.clone()) {
                continue;
            }
            let hay = format!(
                "{} {} {} {} {} {}",
                tx.counter, tx.account, tx.tag, tx.tx_type, tx.external_id, tx.note
            );
            let s = score(&hay, &terms);
            if s > 0 {
                let label = if !tx.counter.is_empty() {
                    tx.counter.clone()
                } else if !tx.account.is_empty() {
                    tx.account.clone()
                } else {
                    crate::format::compact_id(&tx.external_id)
                };
                let detail = format!(
                    "{} · {} · {}",
                    crate::format::tx_date(&tx.date, tx.conf > 0.0),
                    tx.tx_type,
                    crate::format::btc(tx.amount_sat / 1e8, 8, true)
                );
                tx_items.push((
                    s,
                    item(
                        Group::Transactions,
                        &label,
                        Some(detail),
                        None,
                        &I::ARROW_LEFT_RIGHT,
                        Action::Nav(format!("/transactions?tx={}", tx.id)),
                    ),
                ));
            }
        }
        tx_items.sort_by_key(|x| std::cmp::Reverse(x.0));
        scored.extend(tx_items.into_iter().take(8));
    }
    if let Some((rq, tx)) = &app.shell.resolved
        && rq == &q
        && let Some(id) = tx["id"].as_str()
    {
        // appSearch.ts `resolvedTransactionResults`: these two strings are hard-coded
        // English in the web, not in the bundles.
        let date = crate::format::tx_date(
            tx["date"].as_str().unwrap_or(""),
            tx["conf"].as_f64().unwrap_or(0.0) > 0.0,
        );
        let detail = [
            "Exact txid match",
            tx["account"].as_str().unwrap_or(""),
            tx["type"].as_str().unwrap_or(""),
            &date,
        ]
        .iter()
        .filter(|p| !p.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" · ");
        scored.push((
            100,
            item(
                Group::Transactions,
                "Open exact transaction",
                Some(detail),
                None,
                &I::ARROW_LEFT_RIGHT,
                Action::Nav(format!("/transactions?tx={id}")),
            ),
        ));
    }
    scored.sort_by(|a, b| a.1.group.rank().cmp(&b.1.group.rank()).then(b.0.cmp(&a.0)));
    scored.into_iter().map(|(_, i)| i).collect()
}

/// `chrome:routeMeta.<id>` for the palette's placeholder and label on each route: AppShell's
/// `ROUTE_META`, the first prefix the path starts with.
pub(crate) fn route_meta(path: &str) -> Option<&'static str> {
    const META: [(&str, &str); 22] = [
        ("/activity", "activity"),
        ("/connections/", "walletDetail"),
        ("/connections", "wallets"),
        ("/books/", "booksOverview"),
        ("/books", "books"),
        ("/journals", "ledger"),
        ("/imports", "imports"),
        ("/egress", "egress"),
        ("/logs", "logs"),
        ("/settings", "settings"),
        ("/reports", "reports"),
        ("/chain-analysis", "chainAnalysis"),
        ("/privacy-mirror", "privacyMirror"),
        ("/exit-tax", "exitTax"),
        ("/source-of-funds", "sourceOfFunds"),
        ("/review", "review"),
        ("/reconcile", "reconcile"),
        ("/custody-gaps", "custodyGaps"),
        ("/transfers", "swaps"),
        ("/transactions", "transactions"),
        ("/assistant", "assistant"),
        ("/overview", "overview"),
    ];
    META.iter()
        .find(|(prefix, _)| path.starts_with(prefix))
        .map(|(_, id)| *id)
}

pub fn run(app: &mut App, action: Action) {
    app.shell.popup = None;
    match action {
        Action::Nav(href) => go(app, &href),
        Action::Refresh => app.refresh_book(false),
        Action::FullRescan => app.refresh_book(true),
        Action::AddWallet => go(app, "/connections"),
        Action::ProcessJournals => app.process_journals(),
        Action::Lock => super::overlays::lock(app),
        Action::Settings => app.shell.settings_open = true,
    }
}

/// How long typing must pause before a txid-shaped query is looked up (a typed txid would
/// otherwise send one lookup per character from the 12th on).
pub const RESOLVE_DEBOUNCE: f64 = 0.25;

fn on_change(s: &mut App, _: &mut Tx) {
    s.shell.search_sel = 0;
    let q = s.shell.search.text.trim().to_owned();
    let known = s.shell.resolving.as_ref().is_some_and(|(r, _)| *r == q)
        || s.shell.resolved.as_ref().is_some_and(|r| r.0 == q);
    s.shell.resolve_due =
        (looks_like_txid(&q) && !known).then_some((q, s.clock + RESOLVE_DEBOUNCE));
}

/// Every frame: sends the lookup once typing paused (`ui.transactions.resolve`, a local read).
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let Some((q, at)) = app.shell.resolve_due.clone() else {
        return false;
    };
    if app.clock < at {
        tx.wake_after(std::time::Duration::from_secs_f64(
            (at - app.clock).max(0.01),
        ));
        return false;
    }
    app.shell.resolve_due = None;
    if app.shell.search.text.trim() != q {
        return false;
    }
    if let Some(id) = app.send(
        "ui.transactions.resolve",
        json!({"query": q}),
        "shell.resolve",
    ) {
        app.shell.resolving = Some((q, id));
    }
    false
}

/// A lookup's answer, taken only when it answers the newest lookup: an answer for a superseded
/// query (a shorter prefix, say) is dropped, whatever order the daemon answers in.
pub fn on_resolve(app: &mut App, r: &Reply) {
    let current = app
        .shell
        .resolving
        .as_ref()
        .is_some_and(|(_, id)| Some(id.as_str()) == app.reply_id());
    if !current {
        return;
    }
    let Some((q, _)) = app.shell.resolving.take() else {
        return;
    };
    let tx = if r.error.is_some() {
        Value::Null
    } else {
        r.data.get("transaction").cloned().unwrap_or(Value::Null)
    };
    app.shell.resolved = Some((q, tx));
}

fn on_key(s: &mut App, ev: &KeyEvent, _: &mut Tx) -> bool {
    let n = results(s).len();
    match ev.key {
        Key::Down if n > 0 => s.shell.search_sel = (s.shell.search_sel + 1) % n,
        Key::Up if n > 0 => s.shell.search_sel = (s.shell.search_sel + n - 1) % n,
        Key::Enter => {
            if let Some(it) = results(s).into_iter().nth(s.shell.search_sel) {
                run(s, it.action);
            }
        }
        Key::Escape => s.shell.popup = None,
        _ => return false,
    }
    true
}

pub fn mount(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    if app.shell.popup != Some(Popup::Search) {
        return;
    }
    let k = tk.kit();
    let (placeholder, label) = match route_meta(app.route.path()) {
        Some(id) => (
            t(&format!("chrome:routeMeta.{id}.placeholder")),
            t(&format!("chrome:routeMeta.{id}.label")),
        ),
        None => (
            t("chrome:shell.fallback.placeholder"),
            t("chrome:shell.fallback.label"),
        ),
    };
    let input = TextInput::new(&app.shell.search)
        .placeholder(placeholder)
        .label(label.clone())
        .plain()
        .height(tk.r(3.0))
        .text_style(tk.tbase(400.0))
        .on_edit(|s: &mut App| &mut s.shell.search)
        .on_change(on_change)
        .on_key(on_key)
        .build(&k)
        .key("sh-search-input")
        .grow(1.0);
    // `flex h-12 items-center gap-2.5 px-4`: the icon and a `text-base` input, no rule below.
    let head = row()
        .w_full()
        .h(tk.r(3.0))
        .px(tk.r(1.0))
        .gap(tk.r(0.625))
        .center_y()
        .child(ic(&I::SEARCH, tk.r(1.0), tk.muted_fg))
        .child(input);
    let items = results(app);
    let sel = app.shell.search_sel.min(items.len().saturating_sub(1));
    // `min-h-0 flex-1 overflow-y-auto px-2 pb-2`; groups `not-first:mt-2`.
    let mut list = col()
        .key("sh-search-list")
        .w_full()
        .px(tk.r(0.5))
        .pb(tk.r(0.5))
        .role("listbox");
    if items.is_empty() {
        list = list.child(
            col()
                .w_full()
                .py(tk.r(2.5))
                .items_center()
                .gap(tk.r(0.25))
                .child(
                    text(t("chrome:shell.searchNoMatches"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg),
                )
                .child(
                    text(t("chrome:shell.searchPrompt"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg),
                ),
        );
    }
    let mut group: Option<Group> = None;
    for (i, it) in items.into_iter().enumerate() {
        if group != Some(it.group) {
            let first = group.is_none();
            group = Some(it.group);
            // `px-2.5 pt-1.5 pb-1 text-xs font-medium text-muted-foreground`
            list = list.child(
                row()
                    .w_full()
                    .mt(if first { 0.0 } else { tk.r(0.5) })
                    .px(tk.r(0.625))
                    .pt(tk.r(0.375))
                    .pb(tk.r(0.25))
                    .child(
                        text(it.group.heading())
                            .font(tk.txs(500.0))
                            .color(tk.muted_fg),
                    ),
            );
        }
        let action = it.action.clone();
        let active = i == sel;
        // `flex min-h-8 items-center gap-2.5 rounded-lg px-2.5 py-1 text-sm`: icon, title, the
        // detail inline (`max-w-[45%] text-xs`), then the shortcut `Kbd`.
        let title_attr = match &it.detail {
            Some(d) => format!("{} — {d}", it.label),
            None => it.label.clone(),
        };
        let mut r = row()
            .key(format!("sh-search-{i}"))
            .w_full()
            .min_h(tk.r(2.0))
            .px(tk.r(0.625))
            .py(tk.r(0.25))
            .gap(tk.r(0.625))
            .center_y()
            .radius(tk.r(0.5))
            .fill(if active {
                tk.accent
            } else {
                Color::TRANSPARENT
            })
            .role("option")
            .checked(active)
            .label(it.label.clone())
            .help(title_attr)
            .on_press(move |s: &mut App| run(s, action.clone()))
            .on_hover(move |s: &mut App| s.shell.search_sel = i)
            .child(ic(it.icon, tk.r(1.0), tk.muted_fg))
            .child(
                text(it.label.clone())
                    .font(tk.tsm(400.0))
                    .color(tk.fg)
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0),
            );
        if let Some(d) = &it.detail {
            r = r.child(
                text(d.clone())
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .truncate()
                    .max_w(tk.r(36.0) * 0.45),
            );
        }
        if let Some(sc) = &it.shortcut {
            r = r.child(
                row()
                    .h(tk.r(1.25))
                    .min_w(tk.r(1.25))
                    .px(tk.r(0.25))
                    .center()
                    .radius(tk.r(0.25))
                    .fill(tk.fg.alpha(0.06))
                    .child(
                        text(sc.clone())
                            .font(tk.tkbd().tracking(tk.r(0.75) * 0.025))
                            .color(tk.muted_fg),
                    ),
            );
        }
        list = list.child(r);
    }
    // `--kb-window-top-inset` (the macOS title bar) + `--kb-dialog-top` = max(1 rem, 8 vh);
    // `--kb-dialog-max-h` is what the window leaves.
    let win_h = if app.shell.window_h > 0.0 {
        app.shell.window_h
    } else {
        900.0
    };
    let top = super::dialog_top(tk, win_h);
    let dialog_max = (win_h - top - tk.r(1.0)).max(tk.r(10.0));
    // `w-full max-w-[36rem]` in the `px-4` column: what the window leaves, at most 36 rem.
    let win_w = if app.shell.window_w > 0.0 {
        app.shell.window_w
    } else {
        1440.0
    };
    let width = (win_w - tk.r(2.0)).min(tk.r(36.0)).max(0.0);
    let dialog = super::glass_dialog(
        tk,
        col()
            .key("sh-search-dialog")
            .w(width)
            .role("dialog")
            .label(label)
            .child(head)
            // An auto-height dialog: the list's own cap is what the window leaves under the head.
            .child(
                col()
                    .w_full()
                    .max_h(dialog_max - tk.r(3.0) - 2.0)
                    .scroll(rune::ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(list),
            ),
    )
    .clip_rounded(tk.r(1.0))
    .modal(true, |s: &mut App| s.shell.popup = None);
    layer.push(
        super::window_backdrop("sh-search-backdrop", tk.glass_backdrop)
            .on_press(|s: &mut App| s.shell.popup = None),
    );
    layer.push(super::centered(dialog, win_w, width, top));
}

/// A page result's icon (`iconKey` in search/appSearch.ts through ShellSearch's icon map).
fn page_icon(href: &str) -> &'static Icon {
    match href {
        "/overview" => &I::GAUGE,
        "/transactions" | "/transfers" | "/chain-analysis" | "/custody-gaps" => {
            &I::ARROW_LEFT_RIGHT
        }
        "/connections" => &I::WALLET,
        "/books" => &I::BOOK_OPEN,
        "/source-of-funds" | "/review" | "/egress" => &I::SHIELD_ALERT,
        "/journals" => &I::CLIPBOARD_LIST,
        "/reports" | "/exit-tax" => &I::CHART_COLUMN,
        "/settings" => &I::SETTINGS,
        "/logs" | "/diagnostics" => &I::SQUARE_TERMINAL,
        "/assistant" => &I::MESSAGE_SQUARE_TEXT,
        _ => &I::SEARCH,
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_lookup_answer_counts_only_for_the_newest_query() {
        let mut app = crate::app::tests::quiet_app();
        let short = crate::app::tests::inject(&mut app, "ui.transactions.resolve", "shell.resolve");
        let long = crate::app::tests::inject(&mut app, "ui.transactions.resolve", "shell.resolve");
        app.shell.resolving = Some(("abcdef0123456789".into(), long.clone()));
        // The prefix's answer comes first (or last): it never labels itself with the newest query.
        crate::app::tests::answer(
            &mut app,
            &short,
            "ui.transactions.resolve",
            serde_json::json!({"transaction": {"id": "prefix-match"}}),
        );
        app.pump();
        assert!(app.shell.resolved.is_none());
        crate::app::tests::answer(
            &mut app,
            &long,
            "ui.transactions.resolve",
            serde_json::json!({"transaction": {"id": "exact"}}),
        );
        app.pump();
        let (q, tx) = app.shell.resolved.clone().expect("resolved");
        assert_eq!(q, "abcdef0123456789");
        assert_eq!(tx["id"], "exact");
        assert!(app.shell.resolving.is_none());
    }

    use super::*;
    use crate::i18n::{Lang, set_lang};

    fn terms(q: &str) -> Vec<String> {
        q.to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn page_phrases_and_synonyms_match_like_the_web() {
        set_lang(Lang::En);
        let f = page_fields(
            "/overview",
            "Overview",
            Some("Portfolio, balance, activity"),
        );
        assert!(best_score(&f, &terms("open overview")) > 0);
        assert!(best_score(&f, &terms("overview page")) > 0);
        assert!(best_score(&f, &terms("go to overview")) > 0);
        assert!(best_score(&f, &terms("dashboard")) > 0);
        // Words from different fields don't combine into a match.
        assert_eq!(best_score(&f, &terms("dashboard portfolio")), 0);
        set_lang(Lang::De);
        let de = page_fields("/overview", &t("nav:book.overview"), None);
        let localized = t("nav:book.overview").to_lowercase();
        assert!(best_score(&de, &terms(&format!("{localized} öffnen"))) > 0);
        assert!(best_score(&de, &terms(&format!("gehe zu {localized}"))) > 0);
        // The English phrases keep working in German.
        assert!(best_score(&de, &terms("open overview")) > 0);
        set_lang(Lang::En);
    }

    #[test]
    fn wallet_phrases_cover_kind_status_and_network() {
        set_lang(Lang::En);
        let c = crate::model::Connection {
            id: "w1".into(),
            kind: "descriptor".into(),
            label: "Cold Storage".into(),
            status: "synced".into(),
            chain: "bitcoin".into(),
            network: Some("regtest".into()),
            ..Default::default()
        };
        let f = wallet_fields(&c);
        assert!(best_score(&f, &terms("cold storage wallet")) > 0);
        assert!(best_score(&f, &terms("open cold storage")) > 0);
        assert!(best_score(&f, &terms("descriptor wallet")) > 0);
        assert!(best_score(&f, &terms("synced wallet")) > 0);
        assert!(best_score(&f, &terms("bitcoin regtest wallet")) > 0);
        assert_eq!(best_score(&f, &terms("lightning")), 0);
        let mut u = unique(["A".into(), "a ".into(), "".into(), "b".into()]);
        u.sort();
        assert_eq!(u, vec!["A".to_owned(), "b".to_owned()]);
    }
}
