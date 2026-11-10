//! Local label matching. The profile's ownership index (`build_owned_index`,
//! derivation included) lives in `kassiber-wallets`, which registers it here;
//! without it, descriptor derivation and graph evidence refuse explicitly.
//! This never writes ownership, custody or accounting facts.
use super::*;
use kassiber_pytext::parse_int;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

/// One owner of a matched reference (`OwnedMatch`).
pub struct OwnedMatch {
    pub wallet_id: String,
    pub wallet_label: String,
    pub source: String,
}

/// What `_match_bip329_record` asks the profile's `OwnedIndex`.
pub trait Ownership {
    /// `build_owned_index`'s non-fatal warnings.
    fn warnings(&self) -> Vec<String>;
    /// `classify_address({"input": reference})`: its matches and
    /// `ownership_ambiguous`.
    fn address(&self, reference: &str) -> (Vec<OwnedMatch>, bool);
    /// `lookup_outpoint(outpoint)` over every scope.
    fn outpoint(&self, outpoint: &str) -> Vec<OwnedMatch>;
    /// The owners of a stored transaction's input: `load_local_tx_legs`,
    /// then the leg's outpoint, else its script.
    fn input(
        &self,
        conn: &Connection,
        profile: &str,
        txid: &str,
        position: usize,
    ) -> Result<Vec<OwnedMatch>>;
}

/// Builds the profile's ownership index for one BIP329 operation.
pub type Builder = fn(&Connection, &str) -> Result<Box<dyn Ownership>>;
static BUILDER: OnceLock<Builder> = OnceLock::new();

/// Install the ownership index builder; the first registration wins.
pub fn register(builder: Builder) {
    let _ = BUILDER.set(builder);
}

fn local(matches: Vec<OwnedMatch>) -> Vec<Match> {
    matches
        .into_iter()
        .map(|m| Match {
            id: m.wallet_id,
            label: m.wallet_label,
            source: m.source,
        })
        .collect()
}

#[derive(Clone)]
struct Match {
    id: String,
    label: String,
    source: String,
}
#[derive(Default)]
pub(super) struct Index {
    addresses: BTreeMap<String, Vec<Match>>,
    scripts: BTreeMap<String, Vec<Match>>,
    outpoints: BTreeMap<String, Vec<Match>>,
    pub warnings: Vec<Value>,
    /// The registered ownership index, when one is installed.
    owned: Option<Box<dyn Ownership>>,
}
fn unsupported() -> AppError {
    AppError::new(
        "not_implemented",
        "BIP329 descriptor derivation and complex local transaction graphs are not implemented",
    )
    .with_details(dict([("component", s("metadata.bip329.ownership"))]))
}
fn address_key(raw: &str) -> String {
    let text = strip(raw);
    let lowered = lower(text);
    if [
        "bc1", "tb1", "bcrt1", "lq1", "tlq1", "ex1", "tex1", "el1", "ert1",
    ]
    .iter()
    .any(|p| lowered.starts_with(p))
    {
        lowered
    } else {
        text.into()
    }
}
fn script(raw: &str) -> Option<String> {
    if lower(raw).starts_with("bc1")
        || lower(raw).starts_with("tb1")
        || lower(raw).starts_with("bcrt1")
    {
        if raw != lower(raw) && raw != raw.to_uppercase() {
            return None;
        }
        let text = lower(raw);
        let split = text.rfind('1')?;
        if split < 1 || split + 7 > text.len() {
            return None;
        }
        let hrp = &text.as_bytes()[..split];
        let charset = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";
        let data = text.as_bytes()[split + 1..]
            .iter()
            .map(|b| charset.iter().position(|x| x == b).map(|n| n as u8))
            .collect::<Option<Vec<_>>>()?;
        let mut checksum = 1u32;
        for n in hrp
            .iter()
            .map(|b| b >> 5)
            .chain([0])
            .chain(hrp.iter().map(|b| b & 31))
            .chain(data.iter().copied())
        {
            let top = checksum >> 25;
            checksum = ((checksum & 0x1ffffff) << 5) ^ u32::from(n);
            for (i, generator) in [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3]
                .iter()
                .enumerate()
            {
                if (top >> i) & 1 != 0 {
                    checksum ^= generator;
                }
            }
        }
        let data = &data[..data.len() - 6];
        let version = *data.first()?;
        if version > 16 || checksum != if version == 0 { 1 } else { 0x2bc830a3 } {
            return None;
        }
        let mut bytes = Vec::new();
        let mut accumulator = 0u32;
        let mut bits = 0;
        for n in &data[1..] {
            accumulator = ((accumulator << 5) | u32::from(*n)) & 0xfff;
            bits += 5;
            if bits >= 8 {
                bits -= 8;
                bytes.push((accumulator >> bits) as u8);
            }
        }
        if bits >= 5
            || ((accumulator << (8 - bits)) & 255) != 0
            || !(2..=40).contains(&bytes.len())
            || (version == 0 && ![20, 32].contains(&bytes.len()))
        {
            return None;
        }
        let mut output = vec![
            if version == 0 { 0 } else { 0x50 + version },
            bytes.len() as u8,
        ];
        output.extend(bytes);
        return Some(hex(&output));
    }
    let alphabet = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut decoded = vec![0u8];
    for c in raw.bytes() {
        let mut carry = alphabet.iter().position(|v| *v == c)? as u32;
        for byte in decoded.iter_mut().rev() {
            carry += u32::from(*byte) * 58;
            *byte = carry as u8;
            carry >>= 8;
        }
        while carry > 0 {
            decoded.insert(0, carry as u8);
            carry >>= 8;
        }
    }
    if decoded == [0] {
        decoded.clear();
    }
    let mut bytes = vec![0; raw.bytes().take_while(|b| *b == b'1').count()];
    bytes.extend(decoded);
    if bytes.len() != 25 {
        return None;
    }
    let digest = Sha256::digest(Sha256::digest(&bytes[..21]));
    if bytes[21..] != digest[..4] {
        return None;
    }
    let (prefix, suffix) = match bytes[0] {
        0 | 111 => ("76a914", "88ac"),
        5 | 196 => ("a914", "87"),
        _ => return None,
    };
    Some(format!("{prefix}{}{suffix}", hex(&bytes[1..21])))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn normalized_script(raw: &str) -> Option<String> {
    let text = lower(strip(raw));
    if text.is_empty()
        || !text.len().is_multiple_of(2)
        || !text.bytes().all(|c| c.is_ascii_hexdigit())
    {
        None
    } else {
        Some(text)
    }
}
fn outpoint(raw: &str) -> Option<String> {
    let (txid, index) = strip(raw).rsplit_once(':')?;
    let txid = strip(txid);
    let index = parse_int(index).ok()?;
    if txid.is_empty() || index < 0.into() {
        None
    } else {
        Some(format!("{}:{index}", lower(txid)))
    }
}
fn integer(value: &Value) -> Option<kassiber_pyjson::BigInt> {
    match value {
        Value::Int(n) => Some(n.clone()),
        Value::Bool(b) => Some(i64::from(*b).into()),
        Value::Float(n) if n.is_finite() => parse_int(&format!("{:.0}", n.trunc())).ok(),
        Value::Str(t) => t.as_str().and_then(|s| parse_int(s).ok()),
        _ => None,
    }
}
fn has_derivation(config: &Value) -> bool {
    truthy(&optional_string(&value(config, "descriptor")))
        || (truthy(&optional_string(&value(config, "xpub")))
            && truthy(&value(config, "script_types")))
}
fn report(matches: &[Match], source: &str, tx_count: Option<usize>, preserved: bool) -> Value {
    let unique = matches
        .iter()
        .map(|m| (m.id.clone(), m.label.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut wallets = unique.into_iter().collect::<Vec<_>>();
    wallets.sort_by(|a, b| (&a.1, &a.0).cmp(&(&b.1, &b.0)));
    let status = if preserved {
        "preserved"
    } else if tx_count == Some(0) || tx_count.is_none() && wallets.is_empty() {
        "unmatched"
    } else if wallets.len() == 1 && tx_count.is_none_or(|n| n == 1) {
        "exact"
    } else {
        "ambiguous"
    };
    let sources = matches
        .iter()
        .map(|m| m.source.as_str())
        .filter(|s| !s.is_empty())
        .collect::<BTreeSet<_>>();
    let source = if sources.is_empty() {
        source.to_owned()
    } else {
        sources.into_iter().collect::<Vec<_>>().join(",")
    };
    dict([
        ("status", s(status)),
        (
            "confidence",
            s(if status == "preserved" || status == "unmatched" {
                "none"
            } else {
                "deterministic"
            }),
        ),
        (
            "wallet_ids",
            Value::List(wallets.iter().map(|(id, _)| s(id.as_str())).collect()),
        ),
        (
            "wallets",
            Value::List(wallets.iter().map(|(_, label)| s(label.as_str())).collect()),
        ),
        ("match_source", s(source)),
    ])
}
impl Index {
    fn add_address(&mut self, address: &str, matched: Match) {
        if !address_key(address).is_empty() {
            self.addresses
                .entry(address_key(address))
                .or_default()
                .push(matched.clone());
        }
        if let Some(script) = script(address) {
            self.scripts.entry(script).or_default().push(matched);
        }
    }
    fn address(&self, address: &str) -> Vec<Match> {
        self.addresses
            .get(&address_key(address))
            .cloned()
            .filter(|v| !v.is_empty())
            .or_else(|| script(address).and_then(|s| self.scripts.get(&s).cloned()))
            .unwrap_or_default()
    }
    pub fn build(conn: &Connection, profile: &str) -> Result<Self> {
        if let Some(builder) = BUILDER.get() {
            let owned = builder(conn, profile)?;
            return Ok(Self {
                warnings: owned.warnings().into_iter().map(s).collect(),
                owned: Some(owned),
                ..Self::default()
            });
        }
        let mut index = Self::default();
        let wallets = rows(
            conn,
            "SELECT id,label,config_json FROM wallets WHERE profile_id=? ORDER BY label ASC",
            [profile],
        )?;
        for wallet in &wallets {
            let config = decoded(text(wallet, "config_json"));
            let mut configs = vec![(config.clone(), false)];
            for epoch in rows(conn,"SELECT private_material_json FROM wallet_policy_epochs WHERE wallet_id=? AND status='retired' ORDER BY created_at ASC,id ASC",[text(wallet,"id")])? {configs.push((decoded(text(&epoch,"private_material_json")),true));}
            if let Some(scan) = get(&config, "ownership_scan_to_index").filter(|v| truthy(v)) {
                if integer(scan).is_none() {
                    index.warnings.push(s(format!(
                        "Wallet '{}': ownership scan depth is invalid",
                        text(wallet, "label")
                    )));
                }
            }
            for (config, historic) in configs {
                if has_derivation(&config) {
                    return Err(unsupported());
                }
                let addresses = match value(&config, "addresses") {
                    Value::Str(t) => t
                        .as_str()
                        .unwrap_or_default()
                        .split(',')
                        .map(|s| s.to_owned())
                        .collect(),
                    Value::List(items) => items.iter().map(py_str).collect(),
                    _ => Vec::new(),
                };
                for address in addresses {
                    index.add_address(
                        strip(&address),
                        Match {
                            id: text(wallet, "id").into(),
                            label: text(wallet, "label").into(),
                            source: if historic {
                                "address_history"
                            } else {
                                "address_list"
                            }
                            .into(),
                        },
                    );
                }
            }
        }
        for output in rows(conn,"SELECT u.*,w.label AS wallet_label FROM wallet_utxos u LEFT JOIN wallets w ON w.id=u.wallet_id WHERE u.profile_id=?",[profile])? {
            if !wallets.iter().any(|w|text(w,"id")==text(&output,"wallet_id")){continue;}
            let matched=Match{id:text(&output,"wallet_id").into(),label:if text(&output,"wallet_label").is_empty(){text(&output,"wallet_id")}else{text(&output,"wallet_label")}.into(),source:"inventory".into()};
            index.add_address(text(&output,"address"),matched.clone());
            if script(text(&output,"address")).is_none(){if let Some(script)=normalized_script(text(&output,"script_pubkey")){index.scripts.entry(script).or_default().push(matched.clone());}}
            if !text(&output,"txid").is_empty()&&value(&output,"vout")!=Value::Null {index.outpoints.entry(format!("{}:{}",lower(text(&output,"txid")),int(&output,"vout"))).or_default().push(matched);}
        }
        for tx in rows(conn,"SELECT external_id,wallet_id,raw_json FROM transactions WHERE profile_id=? AND external_id IS NOT NULL AND raw_json IS NOT NULL",[profile])? {
            let txid=lower(strip(text(&tx,"external_id")));if txid.len()!=64||!txid.bytes().all(|b|b.is_ascii_hexdigit()){continue;}
            let Ok(Value::List(observations))=loads(text(&tx,"raw_json")) else {continue};
            for observation in observations {
                if lower(strip(text(&observation,"txid")))!=txid{continue;}
                let address=strip(text(&observation,"address"));let vout=value(&observation,"vout");
                if address.is_empty()||matches!(vout,Value::Bool(_)|Value::Null){continue;}
                let n=integer(&vout);
                let Some(n)=n.filter(|n|n>=&0.into())else{continue};
                let matches=index.address(address);let ids=matches.iter().map(|m|m.id.as_str()).collect::<BTreeSet<_>>();
                if ids==BTreeSet::from([text(&tx,"wallet_id")]){index.outpoints.entry(format!("{txid}:{n}")).or_default().extend(matches);}
            }
        }
        Ok(index)
    }
    pub fn matches(
        &self,
        conn: &Connection,
        profile: &str,
        record: &Value,
    ) -> Result<(Value, Vec<Value>)> {
        let kind = text(record, "type");
        let reference = text(record, "ref");
        if kind == "tx" {
            let txs=rows(conn,"SELECT t.id,t.wallet_id,w.label AS wallet_label FROM transactions t LEFT JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND lower(t.external_id)=lower(?) ORDER BY w.label ASC,t.occurred_at ASC,t.created_at ASC,t.id ASC",[profile,reference])?;
            let matches = txs
                .iter()
                .filter(|t| !text(t, "wallet_id").is_empty())
                .map(|t| Match {
                    id: text(t, "wallet_id").into(),
                    label: if text(t, "wallet_label").is_empty() {
                        text(t, "wallet_id")
                    } else {
                        text(t, "wallet_label")
                    }
                    .into(),
                    source: String::new(),
                })
                .collect::<Vec<_>>();
            return Ok((
                report(&matches, "transactions.external_id", Some(txs.len()), false),
                txs,
            ));
        }
        if let Some(owned) = &self.owned {
            return self.owned_matches(owned.as_ref(), conn, profile, kind, reference);
        }
        let (matches, source, preserved) = match kind {
            "addr" => (self.address(reference), "address", false),
            "output" => (
                outpoint(reference)
                    .and_then(|p| self.outpoints.get(&p).cloned())
                    .unwrap_or_default(),
                "wallet_utxos.outpoint",
                false,
            ),
            "input" => {
                let Some((txid, _position)) =
                    strip(reference).rsplit_once(':').and_then(|(txid, n)| {
                        parse_int(n)
                            .ok()
                            .filter(|n| n >= &0.into())
                            .map(|n| (strip(txid), n))
                    })
                else {
                    return Ok((report(&[], "input_ref", None, false), vec![]));
                };
                if txid.is_empty() {
                    return Ok((report(&[], "input_ref", None, false), vec![]));
                }
                let raw = rows(
                    conn,
                    "SELECT raw_json FROM transactions WHERE profile_id=? AND external_id=?",
                    [profile, txid],
                )?;
                let matches = Vec::new();
                for row in raw {
                    let parsed = loads(text(&row, "raw_json")).unwrap_or(Value::Null);
                    // A complete graph normalizer is owned by the observation port. Refuse
                    // complex graph evidence rather than pick a subset and guess a match.
                    if ["vin", "vout", "tx", "ownership_graph", "raw_hex"]
                        .iter()
                        .any(|k| get(&parsed, k).is_some())
                    {
                        return Err(unsupported());
                    }
                }
                (matches, "local_tx.input", false)
            }
            "pubkey" | "xpub" | "spscan" => (vec![], kind, true),
            _ => (vec![], "unsupported", false),
        };
        let mut matched = report(&matches, source, None, preserved);
        if kind == "addr" {
            put(&mut matched, "match_source", s("address"));
        }
        Ok((matched, vec![]))
    }

    /// `_match_bip329_record` for `addr`, `output` and `input` references over
    /// the registered index; preserved and unsupported types answer as above.
    fn owned_matches(
        &self,
        index: &dyn Ownership,
        conn: &Connection,
        profile: &str,
        kind: &str,
        reference: &str,
    ) -> Result<(Value, Vec<Value>)> {
        let matched = match kind {
            "addr" => {
                let (matches, ambiguous) = index.address(reference);
                let mut matched = report(&local(matches), "address", None, false);
                put(&mut matched, "match_source", s("address"));
                if get(&matched, "status") != Some(&s("unmatched")) {
                    put(
                        &mut matched,
                        "status",
                        s(if ambiguous { "ambiguous" } else { "exact" }),
                    );
                }
                matched
            }
            "output" => {
                let matches = outpoint(reference)
                    .map(|p| index.outpoint(&p))
                    .unwrap_or_default();
                report(&local(matches), "wallet_utxos.outpoint", None, false)
            }
            "input" => {
                let parsed = strip(reference).rsplit_once(':').and_then(|(txid, n)| {
                    parse_int(n)
                        .ok()
                        .filter(|n| n >= &0.into())
                        .map(|n| (strip(txid).to_owned(), n))
                });
                let matches = match parsed {
                    Some((txid, position)) if !txid.is_empty() => match usize::try_from(position) {
                        Ok(position) => index.input(conn, profile, &txid, position)?,
                        Err(_) => vec![],
                    },
                    _ => return Ok((report(&[], "input_ref", None, false), vec![])),
                };
                report(&local(matches), "local_tx.input", None, false)
            }
            "pubkey" | "xpub" | "spscan" => report(&[], kind, None, true),
            _ => report(&[], "unsupported", None, false),
        };
        Ok((matched, vec![]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn address_scripts_preserve_network_aliases_and_reject_bad_checksums() {
        assert_eq!(
            script("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"),
            Some("76a91462e907b15cbf27d5425399ebf6f0fb50ebb88f1888ac".into())
        );
        assert_eq!(script("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNb"), None);
        let address = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";
        assert_eq!(
            script(address),
            Some("0014751e76e8199196d454941c45d1b3a323f1433bd6".into())
        );
        assert_eq!(script(&address.to_uppercase()), script(address));
    }
}
