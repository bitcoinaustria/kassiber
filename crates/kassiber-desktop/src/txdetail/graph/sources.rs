//! Where the graph's data comes from and what each network path would contact, made explicit.
//!
//! An improvement over the web requested by the owner ("I don't know when it works and when it
//! does not"): the web shows "Look up on-chain" only when it could add something and says
//! nothing otherwise. Here every public-lookup and explorer path states why it is unavailable,
//! what it would contact before the user consents, and what it returned afterwards. Nothing here
//! starts traffic: the lookup still goes out only from the button (`approve_lookup`), explorer
//! links only open from a click, and every fact below comes from reads the panel already makes
//! (`ui.transactions.graph`, `ui.backends.settings.list`, `ui.network.offline`).
//!
//! Gating mirrors TransactionGraphLookup.ts (`hasPublicGraphLookupReference`,
//! `publicLookupCanAddToGraph`), lib/explorer.ts (configured explorer, public mainnet fallbacks,
//! the setup's and the user's fallback switches) and the daemon's lookup
//! (crates/journals/src/transaction_graph{,/public}.rs: local and cached references first, the
//! wallet's own backend, then the other configured backends of that chain and network).

use rune::{El, col, row, text};
use serde_json::Value;

use super::auto::AutoState;
use super::data::Payload;
use super::{Ctx, Entry, Shown, model};
use crate::app::App;
use crate::i18n::{Lang, lang, t};
use crate::icons as I;
use crate::settings::model as sm;
use crate::ui::ic;

// ---- Strings ----------------------------------------------------------------------------------
// The web has no copy for these states, so they live here in both languages.

fn s(key: &str) -> &'static str {
    let (en, de) = match key {
        "title" => ("Network & data sources", "Netzwerk & Datenquellen"),
        "row.data" => ("Graph data", "Graphdaten"),
        "row.lookup" => ("On-chain lookup", "On-Chain-Abfrage"),
        "row.explorer" => ("Explorer links", "Explorer-Links"),
        "row.inputs" => ("Spent inputs", "Ausgegebene Inputs"),
        "row.fee" => ("Fee & size", "Gebühr & Größe"),
        "row.confirm" => ("Confirmation", "Bestätigung"),
        "row.exposure" => ("Public exposure", "Öffentliche Sichtbarkeit"),

        "data.book" => (
            "Stored in this book: the wallet's own sync recorded these inputs and outputs. Reading them sends nothing.",
            "In diesem Buch gespeichert: Die Synchronisierung der Wallet hat diese Inputs und Outputs erfasst. Das Lesen sendet nichts.",
        ),
        "data.bookRefs" => (
            "Built from transaction references already stored in this book (another row of the same transaction). References do not establish ownership.",
            "Aus Transaktionsreferenzen, die schon in diesem Buch liegen (eine andere Zeile derselben Transaktion). Referenzen begründen kein Eigentum.",
        ),
        "data.cache" => (
            "From public references fetched earlier and kept in this book's cache. Reading them sends nothing.",
            "Aus früher abgerufenen öffentlichen Referenzen im Cache dieses Buchs. Das Lesen sendet nichts.",
        ),
        "data.public" => (
            "From the on-chain lookup you approved for this transaction.",
            "Aus der On-Chain-Abfrage, die du für diese Transaktion freigegeben hast.",
        ),
        "data.none" => (
            "No inputs or outputs are stored for this row.",
            "Für diese Zeile sind keine Inputs oder Outputs gespeichert.",
        ),
        "data.loading" => ("Reading the local graph…", "Lokaler Graph wird gelesen …"),
        "data.error" => (
            "The local graph could not be read: {{error}}",
            "Der lokale Graph konnte nicht gelesen werden: {{error}}",
        ),
        "short.data.book" => ("Data: this book", "Daten: dieses Buch"),
        "short.data.bookRefs" => ("Data: book references", "Daten: Buchreferenzen"),
        "short.data.cache" => ("Data: cache", "Daten: Cache"),
        "short.data.public" => ("Data: lookup", "Daten: Abfrage"),
        "short.data.none" => ("Data: none stored", "Daten: nichts gespeichert"),
        "short.data.loading" => ("Data: loading", "Daten: wird geladen"),

        "lookup.notOnChain" => (
            "Not available: this is not an on-chain Bitcoin or Liquid transaction (Lightning, exchange or manual entry), so there is nothing to look up.",
            "Nicht verfügbar: keine On-Chain-Transaktion auf Bitcoin oder Liquid (Lightning, Börse oder manueller Eintrag), daher gibt es nichts abzufragen.",
        ),
        "lookup.noTxid" => (
            "Not available: the row has no transaction id to look up.",
            "Nicht verfügbar: Die Zeile hat keine Transaktions-ID, die sich abfragen ließe.",
        ),
        "lookup.loading" => (
            "Waiting for the local graph.",
            "Wartet auf den lokalen Graphen.",
        ),
        "lookup.notNeeded" => (
            "Not needed: every input and output amount is already known locally. Nothing is sent.",
            "Nicht nötig: Alle Beträge der Inputs und Outputs sind lokal bekannt. Es wird nichts gesendet.",
        ),
        "lookup.unsupported" => (
            "Not offered: this graph shape is not supported, and a lookup would not change it.",
            "Nicht angeboten: Diese Graphform wird nicht unterstützt; eine Abfrage würde daran nichts ändern.",
        ),
        "lookup.liquidLocal" => (
            "Not offered: a Liquid graph stored locally is never replaced, and no backend can reveal confidential amounts.",
            "Nicht angeboten: Ein lokal gespeicherter Liquid-Graph wird nie ersetzt, und kein Backend kann vertrauliche Beträge offenlegen.",
        ),
        "lookup.offer" => (
            "Available on request. It would add {{adds}}",
            "Auf Anfrage verfügbar. Sie würde {{adds}} ergänzen.",
        ),
        "lookup.addsGraph" => (
            "the inputs and outputs of this transaction, which are not stored locally.",
            "die Inputs und Outputs dieser Transaktion, die lokal nicht gespeichert sind,",
        ),
        "lookup.addsInputs_one" => (
            "the amount of {{count}} spent input, and with it the fee and fee rate.",
            "den Betrag von {{count}} ausgegebenen Input und damit Gebühr und Gebührensatz",
        ),
        "lookup.addsInputs_other" => (
            "the amounts of {{count}} spent inputs, and with them the fee and fee rate.",
            "die Beträge von {{count}} ausgegebenen Inputs und damit Gebühr und Gebührensatz",
        ),
        "lookup.sendsTo" => (
            "“{{button}}” sends the transaction id {{txid}}, and for missing input amounts the ids of the transactions it spends, to the first of these that answers:",
            "„{{button}}“ sendet die Transaktions-ID {{txid}} und für fehlende Input-Beträge die IDs der ausgegebenen Transaktionen an den ersten dieser Server, der antwortet:",
        ),
        "lookup.asks" => (
            "“{{button}}” sends the transaction id {{txid}} to {{first}} first{{rest}}. That server sees your IP address; the approval covers this transaction only.",
            "„{{button}}“ sendet die Transaktions-ID {{txid}} zuerst an {{first}}{{rest}}. Dieser Server sieht deine IP-Adresse; die Freigabe gilt nur für diese Transaktion.",
        ),
        "lookup.asksMore_one" => (
            ", then if needed {{count}} other configured backend (Network & data sources lists it)",
            ", bei Bedarf danach an {{count}} weiteres konfiguriertes Backend (Netzwerk & Datenquellen nennt es)",
        ),
        "lookup.asksMore_other" => (
            ", then if needed {{count}} other configured backends (Network & data sources lists them)",
            ", bei Bedarf danach an {{count}} weitere konfigurierte Backends (Netzwerk & Datenquellen nennt sie)",
        ),
        "lookup.footnote" => (
            "That server sees your IP address and when you asked. The approval covers this transaction only; the next one asks again.",
            "Dieser Server sieht deine IP-Adresse und den Zeitpunkt der Anfrage. Die Freigabe gilt nur für diese Transaktion; die nächste fragt erneut.",
        ),
        "lookup.contactsNone" => (
            "No configured {{chain}} backend matches this transaction's network ({{network}}). “{{button}}” would report that without contacting anything.",
            "Kein konfiguriertes {{chain}}-Backend passt zum Netzwerk dieser Transaktion ({{network}}). „{{button}}“ würde das melden, ohne etwas zu kontaktieren.",
        ),
        "lookup.offline" => (
            "Offline mode is on, so the lookup is switched off and nothing is sent. Turn on “{{online}}” in the title bar's connection panel to use it.",
            "Der Offline-Modus ist an, daher ist die Abfrage ausgeschaltet und es wird nichts gesendet. Schalte im Verbindungsbereich der Titelleiste „{{online}}“ ein, um sie zu nutzen.",
        ),
        "lookup.running" => (
            "Looking up: asking the configured backends for this transaction's public references…",
            "Abfrage läuft: Die konfigurierten Backends werden nach den öffentlichen Referenzen dieser Transaktion gefragt …",
        ),
        "lookup.doneGraph" => (
            "Lookup done: the graph now shows {{inputs}} inputs and {{outputs}} outputs from public references, kept in this book's cache for next time.",
            "Abfrage abgeschlossen: Der Graph zeigt jetzt {{inputs}} Inputs und {{outputs}} Outputs aus öffentlichen Referenzen, die für das nächste Mal im Cache dieses Buchs bleiben.",
        ),
        "lookup.doneInputs_one" => (
            "Lookup done: added the amount of {{count}} spent input. The references stay in this book's cache for next time.",
            "Abfrage abgeschlossen: Der Betrag von {{count}} ausgegebenen Input wurde ergänzt. Die Referenzen bleiben für das nächste Mal im Cache dieses Buchs.",
        ),
        "lookup.doneInputs_other" => (
            "Lookup done: added the amounts of {{count}} spent inputs. The references stay in this book's cache for next time.",
            "Abfrage abgeschlossen: Die Beträge von {{count}} ausgegebenen Inputs wurden ergänzt. Die Referenzen bleiben für das nächste Mal im Cache dieses Buchs.",
        ),
        "lookup.doneNothing" => (
            "Lookup done: the backend returned nothing this graph did not already have.",
            "Abfrage abgeschlossen: Das Backend lieferte nichts, was dieser Graph nicht schon hatte.",
        ),
        "lookup.failed" => (
            "Lookup failed: {{reason}} The graph stored in this book is shown unchanged.",
            "Abfrage fehlgeschlagen: {{reason}} Der in diesem Buch gespeicherte Graph wird unverändert gezeigt.",
        ),
        "lookup.refusedOffline" => (
            "Offline mode refused it before anything was sent.",
            "Der Offline-Modus hat sie abgelehnt, bevor etwas gesendet wurde.",
        ),
        "lookup.filling" => (
            "Asking this wallet's own backend for the missing amounts. No other server is asked.",
            "Das Backend dieser Wallet wird nach den fehlenden Beträgen gefragt. Kein anderer Server wird gefragt.",
        ),
        "lookup.filled" => (
            "Filled in from this wallet's own backend when the transaction opened (Settings → Privacy). The references stay in this book's cache.",
            "Beim Öffnen der Transaktion vom Backend dieser Wallet ergänzt (Einstellungen → Datenschutz). Die Referenzen bleiben im Cache dieses Buchs.",
        ),
        "lookup.walletOffline" => (
            "Offline mode is on, so this wallet's backend was not asked. Nothing was sent.",
            "Der Offline-Modus ist an, daher wurde das Backend dieser Wallet nicht gefragt. Es wurde nichts gesendet.",
        ),
        "lookup.walletFailed" => (
            "This wallet's backend did not answer: {{reason}} No other server was asked.",
            "Das Backend dieser Wallet hat nicht geantwortet: {{reason}} Kein anderer Server wurde gefragt.",
        ),
        "quiet.loading" => ("Loading input amounts…", "Input-Beträge werden geladen …"),
        "quiet.offline" => (
            "Offline — input amounts not loaded",
            "Offline – Input-Beträge nicht geladen",
        ),
        "quiet.failed" => (
            "Input amounts unavailable: backend unreachable",
            "Input-Beträge nicht verfügbar: Backend nicht erreichbar",
        ),
        "data.wallet" => (
            "Filled in from this wallet's own backend, kept in this book's cache.",
            "Vom Backend dieser Wallet ergänzt, im Cache dieses Buchs gespeichert.",
        ),
        "short.data.wallet" => ("Data: wallet backend", "Daten: Wallet-Backend"),
        "short.lookup.notNeeded" => ("Lookup: not needed", "Abfrage: nicht nötig"),
        "short.lookup.available" => ("Lookup: available", "Abfrage: verfügbar"),
        "short.lookup.offline" => ("Lookup: off (offline)", "Abfrage: aus (offline)"),
        "short.lookup.running" => ("Lookup: running", "Abfrage: läuft"),
        "short.lookup.done" => ("Lookup: done", "Abfrage: abgeschlossen"),
        "short.lookup.failed" => ("Lookup: failed", "Abfrage: fehlgeschlagen"),
        "short.lookup.unavailable" => ("Lookup: not available", "Abfrage: nicht verfügbar"),
        "short.lookup.notOffered" => ("Lookup: not offered", "Abfrage: nicht angeboten"),
        "target.wallet" => ("this wallet's backend", "Backend dieser Wallet"),
        "target.default" => ("default", "Standard"),

        "explorer.configured" => (
            "Links open {{host}}, the Explorer API in your backend settings, in the system browser. It sees your IP address and the transaction or address you open.",
            "Links öffnen {{host}}, die Explorer-API aus deinen Backend-Einstellungen, im Systembrowser. Sie sieht deine IP-Adresse und die geöffnete Transaktion oder Adresse.",
        ),
        "explorer.public" => (
            "Links open the public explorer {{host}} in the system browser. It learns your IP address and the transaction or address you open. Turn this off in Settings → Privacy.",
            "Links öffnen den öffentlichen Explorer {{host}} im Systembrowser. Er erfährt deine IP-Adresse und die geöffnete Transaktion oder Adresse. Ausschalten unter Einstellungen → Datenschutz.",
        ),
        "explorer.userOff" => (
            "No links: public explorer links are turned off in Settings → Privacy, and no explorer is configured for this layer.",
            "Keine Links: Öffentliche Explorer-Links sind unter Einstellungen → Datenschutz ausgeschaltet, und für diese Ebene ist kein Explorer konfiguriert.",
        ),
        "explorer.setupOff" => (
            "No links: this book's setup (your own server, offline or regtest) does not use public explorers, and no explorer is configured for this layer.",
            "Keine Links: Die Einrichtung dieses Buchs (eigener Server, offline oder Regtest) nutzt keine öffentlichen Explorer, und für diese Ebene ist kein Explorer konfiguriert.",
        ),
        "explorer.notMainnet" => (
            "No links: public explorers serve mainnet only, there is none for {{network}}, and no explorer is configured for this layer.",
            "Keine Links: Öffentliche Explorer bedienen nur Mainnet, für {{network}} gibt es keinen, und für diese Ebene ist kein Explorer konfiguriert.",
        ),
        "explorer.hidden" => (
            "Hidden while amounts and references are hidden (privacy mode).",
            "Ausgeblendet, solange Beträge und Referenzen verborgen sind (Privatsphäre-Modus).",
        ),
        "explorer.notOnChain" => (
            "No links: not an on-chain transaction.",
            "Keine Links: keine On-Chain-Transaktion.",
        ),
        "short.explorer" => ("Explorer: {{host}}", "Explorer: {{host}}"),
        "short.explorer.none" => ("Explorer: none", "Explorer: keiner"),
        "short.explorer.hidden" => ("Explorer: hidden", "Explorer: ausgeblendet"),

        "inputs.known" => (
            "{{known}} of {{count}} spent input amounts known locally.",
            "{{known}} von {{count}} Beträgen ausgegebener Inputs lokal bekannt.",
        ),
        "inputs.confidential" => (
            "Liquid amounts are confidential; only the references are known.",
            "Liquid-Beträge sind vertraulich; nur die Referenzen sind bekannt.",
        ),
        "inputs.none" => (
            "No spent inputs are stored.",
            "Es sind keine ausgegebenen Inputs gespeichert.",
        ),
        "fee.known" => (
            "{{facts}}, computed from the stored transaction.",
            "{{facts}}, aus der gespeicherten Transaktion berechnet.",
        ),
        "fee.missing" => (
            "Unknown until every spent input amount is known.",
            "Unbekannt, bis alle Beträge der ausgegebenen Inputs bekannt sind.",
        ),
        "fee.none" => ("Not known locally.", "Lokal nicht bekannt."),
        "confirm.block" => (
            "Confirmed in block {{height}}, as the wallet last synced it.",
            "Bestätigt in Block {{height}}, laut letzter Synchronisierung der Wallet.",
        ),
        "confirm.noHeight" => (
            "Confirmed; the block height is not stored for this row.",
            "Bestätigt; die Blockhöhe ist für diese Zeile nicht gespeichert.",
        ),
        "confirm.pending" => (
            "Unconfirmed when the wallet last synced. Refreshing the book updates it.",
            "Bei der letzten Synchronisierung unbestätigt. Ein Aktualisieren des Buchs bringt das auf den neuesten Stand.",
        ),
        "exposure.local" => (
            "“{{label}}” opens Chain Analysis on this device. Opening it sends nothing; a public acquisition there asks first.",
            "„{{label}}“ öffnet die Chain-Analyse auf diesem Gerät. Das Öffnen sendet nichts; eine öffentliche Abfrage dort fragt vorher nach.",
        ),
        "exposure.none" => (
            "Not available: it needs a transaction id with its chain and network.",
            "Nicht verfügbar: Dafür braucht es eine Transaktions-ID mit Chain und Netzwerk.",
        ),
        _ => ("", ""),
    };
    if lang() == Lang::De { de } else { en }
}

fn sr(key: &str, args: &[(&str, &str)]) -> String {
    let mut out = s(key).to_owned();
    for (k, v) in args {
        out = out.replace(&format!("{{{{{k}}}}}"), v);
    }
    out
}

fn sc(key: &str, count: usize, args: &[(&str, &str)]) -> String {
    let n = count.to_string();
    let mut all = vec![("count", n.as_str())];
    all.extend_from_slice(args);
    sr(
        &format!("{key}_{}", if count == 1 { "one" } else { "other" }),
        &all,
    )
}

/// Every string of this module in both languages (for the completeness test).
#[cfg(test)]
const KEYS: &[&str] = &[
    "title",
    "row.data",
    "row.lookup",
    "row.explorer",
    "row.inputs",
    "row.fee",
    "row.confirm",
    "row.exposure",
    "data.book",
    "data.bookRefs",
    "data.cache",
    "data.public",
    "data.none",
    "data.loading",
    "data.error",
    "short.data.book",
    "short.data.bookRefs",
    "short.data.cache",
    "short.data.public",
    "short.data.none",
    "short.data.loading",
    "lookup.notOnChain",
    "lookup.noTxid",
    "lookup.loading",
    "lookup.notNeeded",
    "lookup.unsupported",
    "lookup.liquidLocal",
    "lookup.offer",
    "lookup.addsGraph",
    "lookup.addsInputs_one",
    "lookup.addsInputs_other",
    "lookup.sendsTo",
    "lookup.asks",
    "lookup.asksMore_one",
    "lookup.asksMore_other",
    "lookup.footnote",
    "lookup.contactsNone",
    "lookup.offline",
    "lookup.running",
    "lookup.doneGraph",
    "lookup.doneInputs_one",
    "lookup.doneInputs_other",
    "lookup.doneNothing",
    "lookup.failed",
    "lookup.refusedOffline",
    "short.lookup.notNeeded",
    "short.lookup.available",
    "short.lookup.offline",
    "short.lookup.running",
    "short.lookup.done",
    "short.lookup.failed",
    "short.lookup.unavailable",
    "short.lookup.notOffered",
    "target.wallet",
    "target.default",
    "explorer.configured",
    "explorer.public",
    "explorer.userOff",
    "explorer.setupOff",
    "explorer.notMainnet",
    "explorer.hidden",
    "explorer.notOnChain",
    "short.explorer",
    "short.explorer.none",
    "short.explorer.hidden",
    "inputs.known",
    "inputs.confidential",
    "inputs.none",
    "fee.known",
    "fee.missing",
    "fee.none",
    "confirm.block",
    "confirm.noHeight",
    "confirm.pending",
    "exposure.local",
    "exposure.none",
    "lookup.filling",
    "lookup.filled",
    "lookup.walletOffline",
    "lookup.walletFailed",
    "quiet.loading",
    "quiet.offline",
    "quiet.failed",
    "data.wallet",
    "short.data.wallet",
];

// ---- Explorer links (lib/explorer.ts) ----------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Explorer {
    pub label: String,
    pub url: String,
    pub configured: bool,
}

/// Why a reference gets no explorer link.
#[derive(Clone, Debug, PartialEq)]
pub enum NoExplorer {
    NotOnChain,
    NoReference,
    /// The user turned public explorer links off (Settings → Privacy).
    UserOff,
    /// The setup derives no public fallbacks (manual bootstrap or a regtest default).
    SetupOff,
    /// Public fallbacks serve mainnet only.
    NotMainnet(String),
    BadUrl,
}

/// `ExplorerSettings`, derived as SettingsModel.ts `deriveExplorerSettings` does from the
/// backend list the shell reads on mount, plus the user's `publicFallbacksAllowed`.
struct Settings {
    bitcoin: String,
    liquid: String,
    setup_allows: bool,
    user_allows: bool,
}

fn settings(app: &App) -> Settings {
    let list = app.read("ui.backends.settings.list");
    let backends = sm::backends(list, None);
    let base = |net_kind: sm::Net| {
        backends
            .iter()
            .filter(|b| b.net_kind == net_kind)
            .find_map(sm::explorer_base)
            .unwrap_or_default()
    };
    Settings {
        bitcoin: base(sm::Net::Btc),
        liquid: base(sm::Net::Liquid),
        setup_allows: sm::public_fallbacks(&backends, sm::bootstrap_mode(list)),
        user_allows: app.settings.prefs.explorer_fallbacks_allowed,
    }
}

const PUBLIC_BITCOIN: (&str, &str) = (
    "mempool.bitcoin-austria.at",
    "https://mempool.bitcoin-austria.at",
);
const PUBLIC_LIQUID: (&str, &str) = ("Liquid Network", "https://liquid.network");

fn is_mainnet(network: &str, name: &str) -> bool {
    let n = name.trim().to_lowercase();
    match network {
        "liquid" => ["", "main", "mainnet", "liquid", "liquidv1"].contains(&n.as_str()),
        _ => ["", "main", "mainnet", "bitcoin"].contains(&n.as_str()),
    }
}

fn encode(v: &str) -> String {
    v.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// `explorerUrl`: `{txid}` / `{address}` placeholders, else `<base>/<segment>/<value>`; http(s) only.
fn explorer_url(base: &str, segment: &str, value: &str) -> Option<String> {
    let enc = encode(value);
    let placeholder = if segment == "tx" {
        "{txid}"
    } else {
        "{address}"
    };
    let url = if base.contains(placeholder) {
        base.replace(placeholder, &enc)
    } else {
        let mut b = base.trim().trim_end_matches('/').to_owned();
        if b.to_lowercase().ends_with("/api") {
            b.truncate(b.len() - 4);
        }
        format!("{b}/{segment}/{enc}")
    };
    let lower = url.to_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://")).then_some(url)
}

/// `targetForExplorerValue`: the explorer link for a transaction or address, or why there is none.
pub fn explorer(
    app: &App,
    network: &str,
    network_name: &str,
    segment: &str,
    value: &str,
) -> Result<Explorer, NoExplorer> {
    let value = value.trim();
    if value.is_empty() {
        return Err(NoExplorer::NoReference);
    }
    let st = settings(app);
    let configured = if network == "liquid" {
        st.liquid.trim()
    } else {
        st.bitcoin.trim()
    }
    .to_owned();
    if configured.is_empty() {
        if !st.user_allows {
            return Err(NoExplorer::UserOff);
        }
        if !st.setup_allows {
            return Err(NoExplorer::SetupOff);
        }
        if !is_mainnet(network, network_name) {
            return Err(NoExplorer::NotMainnet(network_name.trim().to_owned()));
        }
    }
    let (label, base) = if network == "liquid" {
        PUBLIC_LIQUID
    } else {
        PUBLIC_BITCOIN
    };
    let url = explorer_url(
        if configured.is_empty() {
            base
        } else {
            &configured
        },
        segment,
        value,
    )
    .ok_or(NoExplorer::BadUrl)?;
    let label = if configured.is_empty() {
        label.to_owned()
    } else {
        sm::host_label(&url)
    };
    Ok(Explorer {
        label,
        url,
        configured: !configured.is_empty(),
    })
}

/// `explorerForTransaction` on a transaction row.
pub fn explorer_for_record(app: &App, r: &Value) -> Result<Explorer, NoExplorer> {
    let chain = r["chain"].as_str().unwrap_or("").trim().to_lowercase();
    let method = r["paymentMethod"].as_str().unwrap_or("");
    let network = if chain == "liquid" || (chain.is_empty() && method == "Liquid") {
        "liquid"
    } else if chain == "bitcoin" || (chain.is_empty() && method == "On-chain") {
        "bitcoin"
    } else {
        return Err(NoExplorer::NotOnChain);
    };
    explorer(
        app,
        network,
        r["network"].as_str().unwrap_or(""),
        "tx",
        r["explorerId"].as_str().unwrap_or(""),
    )
}

/// `explorerTargetForGraphNode`: a peg-out's destination resolves against Bitcoin.
pub fn explorer_for_node(app: &App, g: &Payload, n: &super::data::Node) -> Option<Explorer> {
    let peg_out = n.role.as_deref() == Some("peg_out");
    let network = if peg_out || !g.is_liquid() {
        "bitcoin"
    } else {
        "liquid"
    };
    let chain_network = g
        .transaction
        .as_ref()
        .and_then(|t| t.network.clone())
        .unwrap_or_default()
        .to_lowercase();
    let chain_network = if peg_out {
        match chain_network.as_str() {
            "" | "liquidv1" => "main".to_owned(),
            "elementsregtest" => "regtest".into(),
            _ => "test".into(),
        }
    } else {
        chain_network
    };
    let (segment, value) = match &n.address {
        Some(a) if !a.is_empty() => ("address", a.clone()),
        _ => (
            "tx",
            n.txid.clone().or_else(|| {
                n.outpoint
                    .as_ref()
                    .and_then(|o| o.split(':').next().map(str::to_owned))
            })?,
        ),
    };
    explorer(app, network, &chain_network, segment, &value).ok()
}

// ---- The on-chain lookup -----------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Lookup {
    NotOnChain,
    NoTxid,
    Loading,
    NotNeeded,
    Unsupported,
    LiquidLocal,
    /// The button is offered; `adds` says what it could add.
    Offer {
        adds: String,
    },
    /// It could add something, but offline mode switches it off.
    Offline {
        adds: String,
    },
    Running,
    Done(String),
    Failed(String),
    /// The automatic lookup from the wallet's own backend (`super::auto`): no notice, no button.
    Filling,
    Filled,
    WalletOffline,
    WalletFailed(String),
}

impl Lookup {
    fn short(&self) -> &'static str {
        match self {
            Lookup::NotOnChain | Lookup::NoTxid => s("short.lookup.unavailable"),
            Lookup::Loading => s("short.data.loading"),
            Lookup::NotNeeded => s("short.lookup.notNeeded"),
            Lookup::Unsupported | Lookup::LiquidLocal => s("short.lookup.notOffered"),
            Lookup::Offer { .. } => s("short.lookup.available"),
            Lookup::Offline { .. } => s("short.lookup.offline"),
            Lookup::Running => s("short.lookup.running"),
            Lookup::Done(_) | Lookup::Filled => s("short.lookup.done"),
            Lookup::Failed(_) | Lookup::WalletFailed(_) => s("short.lookup.failed"),
            Lookup::Filling => s("short.lookup.running"),
            Lookup::WalletOffline => s("short.lookup.offline"),
        }
    }
}

/// Offline mode (the title bar's switch) or `KASSIBER_NO_EGRESS`: the daemon refuses every
/// lookup, so the button is disabled.
pub fn offline(app: &App) -> bool {
    app.read("ui.network.offline")
        .is_some_and(|v| v["offline"] == true || v["environment_blocked"] == true)
}

/// `graphLookupChain`: the chain a lookup for this row would ask (None for Lightning, exchange
/// and manual rows).
fn lookup_chain(r: &Value) -> Option<&'static str> {
    match r["chain"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_lowercase()
        .as_str()
    {
        "bitcoin" => Some("bitcoin"),
        "liquid" => Some("liquid"),
        _ => match r["paymentMethod"].as_str() {
            Some("On-chain") => Some("bitcoin"),
            Some("Liquid") => Some("liquid"),
            _ => None,
        },
    }
}

fn adds(p: &Payload) -> String {
    if p.support_level == "graphless" {
        s("lookup.addsGraph").to_owned()
    } else {
        sc("lookup.addsInputs", p.missing_inputs(), &[])
    }
}

/// The text for a lookup warning code: the web's own copy where it has one.
fn warning_text(code: &str, message: &str) -> String {
    let key = format!("transactions:graph.warnings.{code}");
    if crate::i18n::has(&key) {
        t(&key)
    } else {
        message.to_owned()
    }
}

/// Codes the daemon returns when a lookup (or its scope check) stopped.
fn stop_warning(p: &Payload) -> Option<String> {
    p.warnings
        .iter()
        .find(|w| {
            w.code.contains("reference_lookup_")
                || matches!(
                    w.code.as_str(),
                    "invalid_reference_scope"
                        | "book_network_mismatch"
                        | "local_reference_conflict"
                        | "graph_lookup_failed"
                        | "graph_lookup_warning"
                )
        })
        .map(|w| warning_text(&w.code, &w.message))
}

/// The lookup's state for the row and the graph shown.
pub fn lookup(app: &App, record: &Value, sh: &Shown) -> Lookup {
    if lookup_chain(record).is_none() {
        return Lookup::NotOnChain;
    }
    if !model::has_lookup_reference(record) {
        return Lookup::NoTxid;
    }
    let local = match &sh.local {
        Some(Entry::Ready(p)) => Some(p.clone()),
        _ => None,
    };
    match &sh.public {
        Some(Entry::Loading) => return Lookup::Running,
        Some(Entry::Failed(e)) => {
            let reason =
                if e.contains("network_egress_disabled") || e.to_lowercase().contains("offline") {
                    s("lookup.refusedOffline").to_owned()
                } else {
                    e.trim_end_matches('.').to_owned() + "."
                };
            return Lookup::Failed(sr("lookup.failed", &[("reason", &reason)]));
        }
        Some(Entry::Ready(p)) => {
            if let Some(reason) = stop_warning(p) {
                return Lookup::Failed(sr("lookup.failed", &[("reason", &reason)]));
            }
            let before = local.as_deref();
            let drawn_before = before.is_some_and(|b| b.drawable());
            if !drawn_before && p.drawable() {
                return Lookup::Done(sr(
                    "lookup.doneGraph",
                    &[
                        ("inputs", &p.inputs.len().to_string()),
                        ("outputs", &p.outputs.len().to_string()),
                    ],
                ));
            }
            let added = before
                .map(|b| b.missing_inputs())
                .unwrap_or(0)
                .saturating_sub(p.missing_inputs());
            return Lookup::Done(if added > 0 {
                sc("lookup.doneInputs", added, &[])
            } else {
                s("lookup.doneNothing").to_owned()
            });
        }
        None => {}
    }
    match sh.auto {
        AutoState::Running => return Lookup::Filling,
        AutoState::Filled => return Lookup::Filled,
        AutoState::Offline => return Lookup::WalletOffline,
        AutoState::Unreachable => {
            let reason = match sh.entry_auto_payload() {
                Some(p) => stop_warning(&p).unwrap_or_default(),
                None => String::new(),
            };
            return Lookup::WalletFailed(sr("lookup.walletFailed", &[("reason", reason.trim())]));
        }
        AutoState::NotAsked | AutoState::Unavailable => {}
    }
    let Some(p) = sh.entry.as_ref().and_then(|e| match e {
        Entry::Ready(p) => Some(p.clone()),
        _ => None,
    }) else {
        return Lookup::Loading;
    };
    if model::lookup_can_add(&p) {
        let a = adds(&p);
        return if offline(app) {
            Lookup::Offline { adds: a }
        } else {
            Lookup::Offer { adds: a }
        };
    }
    if p.is_liquid() {
        Lookup::LiquidLocal
    } else if p.support_level == "unsupported" {
        Lookup::Unsupported
    } else {
        Lookup::NotNeeded
    }
}

/// `chain`/`network` normalised as the daemon compares them (`normalize_network`).
fn norm_network(chain: &str, network: &str) -> String {
    let n = network.trim().to_lowercase();
    match chain {
        "liquid" => match n.as_str() {
            "" | "liquid" | "liquidv1" | "main" | "mainnet" => "liquidv1".into(),
            _ => n,
        },
        _ => match n.as_str() {
            "" | "main" | "mainnet" | "bitcoin" => "main".into(),
            "test" | "testnet" | "testnet3" => "testnet".into(),
            _ => n,
        },
    }
}

/// What a lookup would contact, in the daemon's order: the wallet's own backend, the default,
/// then every other configured backend of the row's chain and network (explorer APIs before
/// Electrum before Bitcoin Core). Lightning nodes and rate providers never take part.
pub fn targets(app: &App, record: &Value, p: Option<&Payload>) -> Vec<(String, String)> {
    let Some(chain) = lookup_chain(record) else {
        return vec![];
    };
    let network = norm_network(chain, record["network"].as_str().unwrap_or(""));
    let list = app.read("ui.backends.settings.list");
    let rows: Vec<&Value> = list
        .and_then(|v| v["backends"].as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    let wallet = p
        .and_then(|p| p.transaction.as_ref())
        .and_then(|t| t.wallet.label.clone())
        .or_else(|| record["account"].as_str().map(str::to_owned))
        .unwrap_or_default();
    let scope = p
        .map(|p| {
            format!(
                "{}/{}/",
                p.context.workspace.as_deref().unwrap_or(""),
                p.context.profile.as_deref().unwrap_or("")
            )
        })
        .unwrap_or_default();
    let mut found: Vec<(u8, (String, String))> = vec![];
    for b in rows {
        let kind = b["kind"].as_str().unwrap_or("").to_lowercase();
        let rank = match kind.as_str() {
            "esplora" | "mempool" | "liquid-esplora" => 2,
            "electrum" => 3,
            "bitcoinrpc" if chain == "bitcoin" => 4,
            _ => continue,
        };
        let b_chain = b["chain"].as_str().unwrap_or("bitcoin").to_lowercase();
        if b_chain != chain || norm_network(chain, b["network"].as_str().unwrap_or("")) != network {
            continue;
        }
        let own = !wallet.is_empty()
            && b["wallet_refs"].as_array().is_some_and(|refs| {
                refs.iter().filter_map(Value::as_str).any(|r| {
                    r.ends_with(&format!("/{wallet}"))
                        && (scope.len() <= 2 || r.starts_with(&scope))
                })
            });
        let default = b["is_default"].as_bool() == Some(true);
        // Shipped presets take part only once chosen (`user_provided`): the wallet's or the default.
        let preset = ["mempool", "fulcrum", "liquid", "liquid-blockstream"]
            .contains(&b["name"].as_str().unwrap_or("").to_lowercase().as_str());
        if preset && !own && !default {
            continue;
        }
        let name = b["display_name"]
            .as_str()
            .filter(|n| !n.trim().is_empty())
            .or(b["name"].as_str())
            .unwrap_or("backend");
        let url = b["url"].as_str().unwrap_or("");
        let protocol = match kind.as_str() {
            "esplora" | "mempool" | "liquid-esplora" => "Explorer API",
            "electrum" => "Electrum / Fulcrum",
            _ => "Bitcoin Core RPC",
        };
        let mut facts = vec![sm::host_label(url), protocol.to_owned()];
        if own {
            facts.push(s("target.wallet").to_owned());
        } else if default {
            facts.push(s("target.default").to_owned());
        }
        found.push((
            if own {
                0
            } else if default {
                1
            } else {
                rank
            },
            (name.to_owned(), facts.join(" · ")),
        ));
    }
    found.sort_by_key(|(r, _)| *r);
    found.into_iter().map(|(_, s)| s).collect()
}

fn short_txid(record: &Value) -> String {
    model::short(record["explorerId"].as_str().unwrap_or(""))
}

/// The txid as the notice names it: "Hidden" in privacy mode.
fn txid_shown(c: &Ctx, record: &Value) -> String {
    if c.hide {
        t("transactions:graph.hidden")
    } else {
        short_txid(record)
    }
}

fn chain_label(record: &Value) -> &'static str {
    if lookup_chain(record) == Some("liquid") {
        "Liquid"
    } else {
        "Bitcoin"
    }
}

/// The state as one sentence (the summary of the structured body below).
fn headline(l: &Lookup) -> String {
    match l {
        Lookup::NotOnChain => s("lookup.notOnChain").into(),
        Lookup::NoTxid => s("lookup.noTxid").into(),
        Lookup::Loading => s("lookup.loading").into(),
        Lookup::NotNeeded => s("lookup.notNeeded").into(),
        Lookup::Unsupported => s("lookup.unsupported").into(),
        Lookup::LiquidLocal => s("lookup.liquidLocal").into(),
        Lookup::Offer { adds } | Lookup::Offline { adds } => sr("lookup.offer", &[("adds", adds)]),
        Lookup::Running => s("lookup.running").into(),
        Lookup::Done(m) | Lookup::Failed(m) | Lookup::WalletFailed(m) => m.clone(),
        Lookup::Filling => s("lookup.filling").into(),
        Lookup::Filled => s("lookup.filled").into(),
        Lookup::WalletOffline => s("lookup.walletOffline").into(),
    }
}

/// The lookup's state: the headline, then before consent what goes where (or why it is off).
fn body(c: &Ctx, record: &Value, p: Option<&Payload>, l: &Lookup) -> El {
    let tk = c.tk;
    let mut out = col()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.375))
        .child(text(headline(l)).font(tk.txs(400.0)).color(tk.fg).wrap());
    let muted = |s: String| text(s).font(tk.txs(400.0)).color(tk.muted_fg).wrap();
    match l {
        Lookup::Offer { .. } => {
            let list = targets(c.app, record, p);
            let button = t("transactions:graph.lookupOnChain");
            if list.is_empty() {
                return out.child(muted(sr(
                    "lookup.contactsNone",
                    &[
                        ("chain", chain_label(record)),
                        ("network", record["network"].as_str().unwrap_or("main")),
                        ("button", &button),
                    ],
                )));
            }
            out = out.child(muted(sr(
                "lookup.sendsTo",
                &[("button", &button), ("txid", &txid_shown(c, record))],
            )));
            let mut items = col()
                .key("graph-lookup-targets")
                .w_full()
                .gap(2.0)
                .role("list");
            for (i, (name, facts)) in list.into_iter().enumerate() {
                let label = format!("{name} · {facts}");
                items = items.child(
                    row()
                        .w_full()
                        .gap(tk.r(0.375))
                        .items_start()
                        .role("listitem")
                        .label(label.clone())
                        .child(
                            text(format!("{}.", i + 1))
                                .font(tk.txs(500.0).features("'tnum' 1"))
                                .color(tk.muted_fg)
                                .w(tk.r(1.0))
                                .shrink(),
                        )
                        .child(
                            row()
                                .grow(1.0)
                                .min_w(0.0)
                                .flex_wrap()
                                .gap(tk.r(0.375))
                                .child(text(name).font(tk.txs(500.0)).color(tk.fg).wrap().shrink())
                                .child(
                                    text(facts)
                                        .font(tk.txs(400.0))
                                        .color(tk.muted_fg)
                                        .wrap()
                                        .shrink(),
                                ),
                        ),
                );
            }
            out.child(items).child(muted(s("lookup.footnote").into()))
        }
        Lookup::Offline { .. } => out.child(muted(sr(
            "lookup.offline",
            &[("online", &t("chrome:network.offlineMode.label"))],
        ))),
        _ => out,
    }
}

/// The notice's short form: the state, and before consent the first server asked (the sources
/// panel has the whole list).
fn brief(c: &Ctx, record: &Value, p: Option<&Payload>, l: &Lookup) -> El {
    let tk = c.tk;
    let line = |s: String, color| text(s).font(tk.txs(400.0)).color(color).wrap();
    let button = t("transactions:graph.lookupOnChain");
    let out = col().w_full().min_w(0.0).gap(2.0);
    match l {
        Lookup::Offer { .. } => {
            let list = targets(c.app, record, p);
            let second = match list.split_first() {
                None => sr(
                    "lookup.contactsNone",
                    &[
                        ("chain", chain_label(record)),
                        ("network", record["network"].as_str().unwrap_or("main")),
                        ("button", &button),
                    ],
                ),
                Some(((name, facts), rest)) => {
                    let host = facts.split(" · ").next().unwrap_or("");
                    let first = if host.is_empty() {
                        name.clone()
                    } else {
                        format!("{name} ({host})")
                    };
                    let more = if rest.is_empty() {
                        String::new()
                    } else {
                        sc("lookup.asksMore", rest.len(), &[])
                    };
                    sr(
                        "lookup.asks",
                        &[
                            ("button", &button),
                            ("txid", &txid_shown(c, record)),
                            ("first", &first),
                            ("rest", &more),
                        ],
                    )
                }
            };
            out.child(line(headline(l), tk.fg))
                .child(line(second, tk.muted_fg))
        }
        Lookup::Offline { .. } => out.child(line(
            sr(
                "lookup.offline",
                &[("online", &t("chrome:network.offlineMode.label"))],
            ),
            tk.fg,
        )),
        _ => out.child(line(headline(l), tk.fg)),
    }
}

// ---- Rendering -------------------------------------------------------------------------------

/// The lookup notice under the graph's header, for the states that need the user's attention
/// before or after they press "Look up on-chain".
pub fn notice(c: &Ctx, record: &Value, sh: &Shown, l: &Lookup) -> Option<El> {
    let tk = c.tk;
    let p = match &sh.local {
        Some(Entry::Ready(p)) => Some(p.clone()),
        _ => None,
    };
    let (glyph, tone): (&'static rune::Icon, rune::Color) = match l {
        Lookup::Offer { .. } => (&I::GLOBE, tk.muted_fg),
        Lookup::Offline { .. } => (&I::WIFI_OFF, tk.muted_fg),
        Lookup::Running => (&I::LOADER_CIRCLE, tk.muted_fg),
        Lookup::Done(_) => (&I::CIRCLE_CHECK, tk.good.solid),
        Lookup::Failed(_) => (&I::TRIANGLE_ALERT, tk.warn.solid),
        _ => return None,
    };
    let mut icon = ic(glyph, tk.r(0.875), tone);
    if matches!(l, Lookup::Running) {
        icon = icon.looping(rune::LoopSpec::rotation(0.9));
    }
    Some(
        row()
            .key("graph-lookup-notice")
            .w_full()
            .gap(tk.r(0.5))
            .px(tk.r(0.75))
            .py(tk.r(0.5))
            .radius(tk.r(0.375))
            .border(1.0, tk.border)
            .fill(tk.muted.alpha(0.35))
            .role("status")
            .busy(matches!(l, Lookup::Running))
            .child(row().mt(tk.r(0.125)).child(icon))
            .child(
                row()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(brief(c, record, p.as_deref(), l)),
            ),
    )
}

/// The wallet-backend lookup's one quiet line under the graph's header: a subtle loading state,
/// and only when the amounts could not be loaded, why. Nothing when it filled them in.
pub fn quiet(c: &Ctx, l: &Lookup) -> Option<El> {
    let tk = c.tk;
    let (key, busy) = match l {
        Lookup::Filling => ("quiet.loading", true),
        Lookup::WalletOffline => ("quiet.offline", false),
        Lookup::WalletFailed(_) => ("quiet.failed", false),
        _ => return None,
    };
    let mut line = row()
        .key("graph-auto-lookup")
        .w_full()
        .gap(tk.r(0.375))
        .center_y()
        .role("status")
        .busy(busy)
        .label(s(key));
    if busy {
        line = line.child(
            ic(&I::LOADER_CIRCLE, tk.r(0.75), tk.muted_fg).looping(rune::LoopSpec::rotation(0.9)),
        );
    }
    Some(
        line.child(
            text(s(key))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap()
                .shrink(),
        ),
    )
}

/// Where the shown graph's references came from.
fn data_source(sh: &Shown) -> (&'static str, String) {
    match &sh.entry {
        None | Some(Entry::Loading) => ("loading", s("data.loading").into()),
        Some(Entry::Failed(e)) => ("none", sr("data.error", &[("error", e)])),
        Some(Entry::Ready(p)) => {
            let from_public = matches!(&sh.public, Some(Entry::Ready(q)) if std::rc::Rc::ptr_eq(p, q))
                && stop_warning(p).is_none();
            let code = if from_public {
                "public"
            } else if sh.auto == AutoState::Filled {
                "wallet"
            } else if p.has_warning("cached_reference_graph") {
                "cache"
            } else if p.has_warning("local_transaction_reference") {
                "bookRefs"
            } else if p.support_level == "graphless" {
                "none"
            } else {
                "book"
            };
            (code, s(&format!("data.{code}")).into())
        }
    }
}

fn explorer_line(app: &App, record: &Value, hide: bool) -> (String, String) {
    if hide {
        return (
            s("short.explorer.hidden").into(),
            s("explorer.hidden").into(),
        );
    }
    match explorer_for_record(app, record) {
        Ok(e) if e.configured => (
            sr("short.explorer", &[("host", &e.label)]),
            sr("explorer.configured", &[("host", &e.label)]),
        ),
        Ok(e) => (
            sr("short.explorer", &[("host", &e.label)]),
            sr("explorer.public", &[("host", &e.label)]),
        ),
        Err(why) => (
            s("short.explorer.none").into(),
            match why {
                NoExplorer::NotOnChain | NoExplorer::NoReference => s("explorer.notOnChain").into(),
                NoExplorer::UserOff => s("explorer.userOff").into(),
                NoExplorer::SetupOff | NoExplorer::BadUrl => s("explorer.setupOff").into(),
                NoExplorer::NotMainnet(n) => sr("explorer.notMainnet", &[("network", &n)]),
            },
        ),
    }
}

fn line(c: &Ctx, key: &str, label: &str, value: String) -> El {
    let v = text(value)
        .font(c.tk.txs(400.0))
        .color(c.tk.fg)
        .wrap()
        .grow(1.0)
        .min_w(0.0);
    line_el(c, key, label, v)
}

fn line_el(c: &Ctx, key: &str, label: &str, value: El) -> El {
    let tk = c.tk;
    // Label above value below 34 rem of panel, side by side above (the facts grid's rhythm).
    row()
        .key(format!("graph-src-{key}"))
        .w_full()
        .flex_wrap()
        .gaps(tk.r(0.75), 2.0)
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .child(
            text(label.to_owned())
                .font(tk.txs(500.0))
                .color(tk.muted_fg)
                .wrap()
                .w(tk.r(8.5))
                .shrink(),
        )
        .child(row().grow(1.0).min_w(tk.r(14.0)).child(value))
}

/// "Network & data sources": a folded inset with one summary line, listing every path.
pub fn panel(c: &Ctx, record: &Value, sh: &Shown, l: &Lookup) -> El {
    let tk = c.tk;
    let app = c.app;
    let p = match &sh.entry {
        Some(Entry::Ready(p)) => Some(p.clone()),
        _ => None,
    };
    let local = match &sh.local {
        Some(Entry::Ready(p)) => Some(p.clone()),
        _ => None,
    };
    let (code, data) = data_source(sh);
    let (explorer_short, explorer_long) = explorer_line(app, record, c.hide);
    let summary = [
        s(&format!("short.data.{code}")).to_owned(),
        l.short().to_owned(),
        explorer_short,
    ]
    .join(" · ");
    let open = app.graph.sources_open;
    let head = row()
        .key("graph-src-toggle")
        .w_full()
        .gap(tk.r(0.5))
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .center_y()
        .role("button")
        .expanded(open)
        .label(s("title"))
        .focusable(true)
        .on_press(|a: &mut App| a.graph.sources_open = !a.graph.sources_open)
        .child(ic(
            if open {
                &I::CHEVRON_DOWN
            } else {
                &I::CHEVRON_RIGHT
            },
            tk.r(0.875),
            tk.muted_fg,
        ))
        .child(text(s("title")).font(tk.txs(500.0)).color(tk.fg).shrink())
        .child(
            text(summary.clone())
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .truncate()
                .grow(1.0)
                .min_w(0.0)
                .help_when_truncated(summary),
        );
    let mut out = col()
        .key("graph-src")
        .w_full()
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .fill(tk.muted.alpha(0.2))
        .clip_rounded(tk.r(0.5))
        .child(head);
    if !open {
        return out;
    }
    let tx = p.as_ref().and_then(|p| p.transaction.clone());
    let inputs = match &p {
        Some(p) if p.is_liquid() && p.inputs.iter().any(|n| n.state() == "confidential") => {
            s("inputs.confidential").into()
        }
        Some(p) if !p.inputs.is_empty() => {
            let known = p.inputs.iter().filter(|n| n.state() != "missing").count();
            sr(
                "inputs.known",
                &[
                    ("known", &known.to_string()),
                    ("count", &p.inputs.len().to_string()),
                ],
            )
        }
        _ => s("inputs.none").into(),
    };
    let fee = match (&p, &tx) {
        _ if c.hide => t("transactions:graph.hidden"),
        (Some(p), Some(tx)) if tx.fee_rate_sat_vb.is_some() || tx.vsize.is_some() => {
            let mut facts = vec![];
            if let Some(r) = tx.fee_rate_sat_vb {
                facts.push(format!("{r} sat/vB"));
            }
            if let Some(v) = tx.vsize {
                facts.push(format!("{} vB", v as i64));
            }
            if let Some(f) = p
                .fee
                .as_ref()
                .and_then(|f| f.value_btc.or(f.value_sats.map(|s| s / 1e8)))
            {
                facts.insert(0, crate::txdetail::draft::btc_amount(f));
            }
            sr("fee.known", &[("facts", &facts.join(" · "))])
        }
        (Some(p), _) if p.missing_inputs() > 0 => s("fee.missing").into(),
        _ => s("fee.none").into(),
    };
    let confirm = match (
        tx.as_ref().and_then(|t| t.block_height),
        record["conf"].as_f64(),
    ) {
        (Some(h), _) => sr("confirm.block", &[("height", &format!("{}", h as i64))]),
        (None, Some(c)) if c > 0.0 => s("confirm.noHeight").into(),
        _ if tx.as_ref().is_some_and(|t| t.confirmed_at.is_some()) => s("confirm.noHeight").into(),
        _ => s("confirm.pending").into(),
    };
    let investigate = tx.as_ref().is_some_and(|t| {
        t.chain.is_some()
            && t.network.is_some()
            && [&t.txid, &t.external_id]
                .into_iter()
                .flatten()
                .any(|v| v.len() == 64)
    });
    let exposure = if investigate {
        sr(
            "exposure.local",
            &[("label", &t("privacyMirror:investigateTransaction"))],
        )
    } else {
        s("exposure.none").into()
    };
    let on_chain = lookup_chain(record).is_some();
    out = out
        .child(line(c, "data", s("row.data"), data))
        .child(line_el(
            c,
            "lookup",
            s("row.lookup"),
            body(c, record, local.as_deref().or(p.as_deref()), l),
        ))
        .child(line(c, "explorer", s("row.explorer"), explorer_long));
    if on_chain {
        out = out
            .child(line(c, "inputs", s("row.inputs"), inputs))
            .child(line(c, "fee", s("row.fee"), fee))
            .child(line(c, "confirm", s("row.confirm"), confirm));
    }
    out.child(line(c, "exposure", s("row.exposure"), exposure))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_string_has_both_languages() {
        crate::i18n::set_lang(Lang::En);
        let en: Vec<_> = KEYS.iter().map(|k| s(k)).collect();
        crate::i18n::set_lang(Lang::De);
        let de: Vec<_> = KEYS.iter().map(|k| s(k)).collect();
        crate::i18n::set_lang(Lang::En);
        for (i, k) in KEYS.iter().enumerate() {
            assert!(!en[i].is_empty() && !de[i].is_empty(), "{k}");
            // Identical only where German uses the same word ("Explorer: {{host}}").
            if *k != "short.explorer" {
                assert_ne!(en[i], de[i], "{k} is not translated");
            }
        }
    }

    #[test]
    fn explorer_urls_follow_the_web() {
        assert_eq!(
            explorer_url("https://ex.example/api/", "tx", "ab"),
            Some("https://ex.example/tx/ab".into())
        );
        assert_eq!(
            explorer_url("https://ex.example/t/{txid}", "tx", "ab"),
            Some("https://ex.example/t/ab".into())
        );
        assert_eq!(explorer_url("ftp://ex.example", "tx", "ab"), None);
        assert!(
            is_mainnet("bitcoin", "main")
                && !is_mainnet("bitcoin", "regtest")
                && is_mainnet("liquid", "liquidv1")
        );
    }

    #[test]
    fn networks_normalise_like_the_daemon() {
        assert_eq!(norm_network("bitcoin", ""), "main");
        assert_eq!(norm_network("bitcoin", "regtest"), "regtest");
        assert_eq!(norm_network("liquid", "liquid"), "liquidv1");
    }
}
