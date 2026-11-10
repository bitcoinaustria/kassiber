//! Sanitized Wasabi RPC bundles. Source data never confers tax authority.
use super::*;
use kassiber_envelope::{get, py_str, set, truthy};
use kassiber_pyjson::{Key, Options};

pub(super) struct Bundle {
    pub records: Vec<Py>,
    pub(super) coins: Vec<Py>,
    pub(super) metadata: Py,
    pub(super) coin_sections: bool,
    pub(super) wallet_json: bool,
    pub(super) keys: usize,
    payments: usize,
    #[cfg(test)]
    pub(super) payment_rows: Vec<Py>,
}
fn field<'a>(v: &'a Py, key: &str) -> &'a Py {
    get(v, key).unwrap_or(&Py::Null)
}
fn blank(v: &Py) -> bool {
    *v == Py::Null || matches!(v, Py::Str(s) if s.as_str()==Some(""))
}
fn text(v: &Py) -> Py {
    if *v == Py::Null {
        return Py::Null;
    }
    if let Py::Str(s) = v {
        if s.has_surrogates() {
            let points = s.code_points().collect::<Vec<_>>();
            let first = points
                .iter()
                .position(|p| !kassiber_pytext::is_space(*p))
                .unwrap_or(points.len());
            let last = points
                .iter()
                .rposition(|p| !kassiber_pytext::is_space(*p))
                .map_or(first, |i| i + 1);
            return Py::Str(
                kassiber_pyjson::PyStr::from_code_points(points[first..last].iter().copied())
                    .expect("existing Python code points"),
            );
        }
    }
    let raw = py_str(v);
    let raw = kassiber_pytext::strip(&raw);
    if raw.is_empty() {
        Py::Null
    } else {
        s(raw)
    }
}
fn first(v: &Py, names: &[&str]) -> Py {
    names
        .iter()
        .find_map(|k| get(v, k).filter(|v| truthy(v)))
        .cloned()
        .unwrap_or_else(|| names.last().map_or(Py::Null, |key| field(v, key).clone()))
}
fn pick(v: &Py, names: &[&str]) -> Py {
    names
        .iter()
        .find_map(|k| get(v, k).filter(|v| !blank(v)))
        .cloned()
        .unwrap_or(Py::Null)
}
fn key(v: &str) -> String {
    kassiber_pytext::casefold(&v.replace(['-', '_', '.'], ""))
}
fn section(v: &Py, names: &[&str]) -> Py {
    let Py::Dict(items) = v else {
        return Py::Null;
    };
    for (k, v) in items {
        if let Key::Str(k) = k {
            if names.iter().any(|n| {
                key(n)
                    == key(&k
                        .code_points()
                        .map(|p| char::from_u32(p).unwrap_or(char::REPLACEMENT_CHARACTER))
                        .collect::<String>())
            }) {
                return get(v, "result").unwrap_or(v).clone();
            }
        }
    }
    for container in ["rpc", "responses", "exports", "wasabi"] {
        let found = section(field(v, container), names);
        if found != Py::Null {
            return found;
        }
    }
    Py::Null
}
fn present(v: &Py, names: &[&str]) -> bool {
    let Py::Dict(items) = v else {
        return false;
    };
    items.iter().any(|(k,_)| matches!(k,Key::Str(k) if names.iter().any(|n| key(n)==key(&k.code_points().map(|p|char::from_u32(p).unwrap_or(char::REPLACEMENT_CHARACTER)).collect::<String>())))) || ["rpc","responses","exports","wasabi"].iter().any(|k| present(field(v,k),names))
}
fn list(v: &Py, names: &[&str]) -> Result<Vec<Py>, AppError> {
    let mut value = section(v, names);
    if value == Py::Null {
        return Ok(vec![]);
    }
    if matches!(value, Py::Dict(_)) {
        for key in [
            "items",
            "records",
            "transactions",
            "history",
            "coins",
            "keys",
            "payments",
        ] {
            if let Some(Py::List(items)) = get(&value, key) {
                value = Py::List(items.clone());
                break;
            }
        }
    }
    let Py::List(items) = value else {
        return Err(AppError::new(
            "validation",
            format!("Wasabi bundle section '{}' must be a list", names[0]),
        ));
    };
    Ok(items
        .into_iter()
        .filter(|v| matches!(v, Py::Dict(_)))
        .collect())
}
fn boolean(v: &Py) -> bool {
    if let Py::Bool(b) = v {
        return *b;
    }
    if matches!(v, Py::Str(s) if s.has_surrogates()) {
        return false;
    }
    matches!(
        kassiber_pytext::casefold(kassiber_pytext::strip(&py_str(v))).as_str(),
        "1" | "true" | "yes" | "on"
    )
}
fn integral(v: &Py, name: &str, amount: bool) -> Result<Py, AppError> {
    if blank(v) {
        return if amount {
            Err(AppError::new(
                "validation",
                "Wasabi transaction is missing amount",
            ))
        } else {
            Ok(Py::Null)
        };
    }
    if amount {
        // A JSON integer token is satoshis; any other number (bool, `1.0`,
        // `1e3`) is refused rather than reinterpreted as BTC.
        match v {
            Py::Int(_) => return Ok(v.clone()),
            Py::Bool(_) | Py::Float(_) => {
                return Err(AppError::new(
                    "validation",
                    "Wasabi numeric amounts must be integer satoshis",
                ))
            }
            _ => {}
        }
    }
    if let Py::Float(value) = v {
        if !amount && value.is_finite() {
            return Ok(Py::Int(
                kassiber_pytext::parse_int(&format!("{:.0}", value.trunc()))
                    .expect("finite integral float"),
            ));
        }
    }
    let raw = py_str(v);
    let raw = kassiber_pytext::strip(&raw);
    let number = if matches!(v, Py::Str(s) if s.has_surrogates()) {
        None
    } else if matches!(v, Py::Float(_)) && !amount || raw.contains('.') {
        let parse = || -> Result<String, AppError> {
            let mut n = dec(raw)?;
            if amount {
                n = n.mul(&dec("100000000")?, &CTX).map_err(decimal_error)?;
            }
            n.to_fixed_places(
                0,
                if amount {
                    Rounding::HalfEven
                } else {
                    Rounding::Down
                },
            )
            .map_err(decimal_error)
        };
        parse()
            .ok()
            .and_then(|v| kassiber_pytext::parse_int(&v).ok())
    } else {
        kassiber_pytext::parse_int(raw).ok()
    };
    number.map(Py::Int).ok_or_else(|| {
        let error = AppError::new(
            "validation",
            if amount {
                format!("Wasabi bundle has invalid amount '{}'", py_str(v))
            } else {
                format!("Wasabi bundle has invalid {name}")
            },
        );
        if amount {
            if let Py::Str(value) = v {
                if value.has_surrogates() {
                    return error.with_python_message(
                        kassiber_pyjson::PyStr::from_code_points(
                            "Wasabi bundle has invalid amount '"
                                .chars()
                                .map(u32::from)
                                .chain(value.code_points())
                                .chain(std::iter::once(u32::from('\''))),
                        )
                        .expect("existing Python code points"),
                    );
                }
            }
        }
        error
    })
}
fn txid(v: &Py) -> Py {
    let value = text(v);
    if let Py::Str(identifier) = &value {
        if identifier.has_surrogates() {
            let lower = |plain: &str| {
                kassiber_pytext::lower(plain)
                    .chars()
                    .map(u32::from)
                    .collect::<Vec<_>>()
            };
            let mut points = vec![];
            let mut plain = String::new();
            for point in identifier.code_points() {
                if let Some(c) = char::from_u32(point) {
                    plain.push(c);
                } else {
                    points.extend(lower(&plain));
                    plain.clear();
                    points.push(point);
                }
            }
            points.extend(lower(&plain));
            return Py::Str(
                kassiber_pyjson::PyStr::from_code_points(points)
                    .expect("existing Python code points"),
            );
        }
    }
    if blank(&value) {
        value
    } else {
        s(kassiber_pytext::lower(&py_str(&value)))
    }
}
pub(super) fn dump(v: &Py) -> Result<String, AppError> {
    kassiber_pyjson::dumps(v, &Options::new().sort_keys(true))
        .map_err(|e| AppError::new("internal_error", e.to_string()))
}
fn compact(mut v: Py, arrays: bool) -> Py {
    if let Py::Dict(items) = &mut v {
        items.retain(|(_, v)| {
            !blank(v) && !(arrays && matches!(v,Py::List(items) if items.is_empty()))
        });
    }
    v
}
fn sanitize(v: &Py) -> Py {
    match v {
        Py::Dict(items)=>Py::Dict(items.iter().filter(|(k,_)| !matches!(k,Key::Str(k) if ["accountkeypath","chaincode","encryptedsecret","encryptedsecretdecryptioninfos","extpubkey","fullkeypath","keypath","masterkeyfingerprint","password","passphrase","privatekey","pubkey","publickey","walletfile","xpub"].contains(&key(&k.code_points().map(|p|char::from_u32(p).unwrap_or(char::REPLACEMENT_CHARACTER)).collect::<String>()).as_str()))).map(|(k,v)|(k.clone(),sanitize(v))).collect()),
        Py::List(items)=>Py::List(items.iter().map(sanitize).collect()),
        _=>v.clone()
    }
}
fn parts(v: &Py) -> Vec<kassiber_pyjson::PyStr> {
    let Py::Str(value) = text(v) else {
        return vec![];
    };
    let points = value.code_points().collect::<Vec<_>>();
    let mut parts = points
        .split(|p| *p == u32::from('/'))
        .map(|p| {
            kassiber_pytext::strip_py(
                &kassiber_pyjson::PyStr::from_code_points(p.iter().copied())
                    .expect("existing Python code points"),
            )
        })
        .filter(|p| p.code_points().next().is_some())
        .collect::<Vec<_>>();
    if parts
        .first()
        .and_then(kassiber_pyjson::PyStr::as_str)
        .is_some_and(|s| s.eq_ignore_ascii_case("m"))
    {
        parts.remove(0);
    }
    parts
}
fn account_prefix(p: &[kassiber_pyjson::PyStr]) -> Py {
    let suffix = "/*'";
    Py::Str(
        kassiber_pyjson::PyStr::from_code_points(
            p[0].code_points()
                .chain(std::iter::once(u32::from('/')))
                .chain(p[1].code_points())
                .chain(suffix.chars().map(u32::from)),
        )
        .expect("existing Python code points"),
    )
}
fn path_hint(v: &Py) -> Py {
    let p = parts(v);
    match p.len() {
        0 => Py::Null,
        1 => s("account_path_present"),
        _ => account_prefix(&p),
    }
}
fn history(v: &Py) -> Result<Py, AppError> {
    let tx = txid(&first(v, &["tx", "txid", "transactionId", "transactionid"]));
    let amount = integral(&pick(v, &["amount", "amountSats"]), "amount", true)?;
    let n = dec(&match &amount {
        Py::Bool(b) => i64::from(*b).to_string(),
        _ => py_str(&amount),
    })?;
    let cj = boolean(
        get(v, "islikelycoinjoin")
            .or_else(|| get(v, "isLikelyCoinJoin"))
            .unwrap_or(field(v, "isLikelyCoinjoin")),
    );
    let label = text(&first(v, &["label", "labels"]));
    let height = integral(&first(v, &["height", "blockHeight"]), "height", false)?;
    let outbound = n.is_negative() || n.is_zero() && cj;
    let kind = if cj {
        "coinjoin"
    } else if outbound {
        "withdrawal"
    } else {
        "deposit"
    };
    let raw = compact(
        dict([
            ("source", s("wasabi_gethistory")),
            ("tx", tx.clone()),
            ("height", height.clone()),
            ("amount_sats", amount),
            ("label", label.clone()),
            ("islikelycoinjoin", Py::Bool(cj)),
            ("privacy_hop", if cj { s("coinjoin") } else { Py::Null }),
        ]),
        false,
    );
    Ok(dict([
        ("txid", tx),
        ("occurred_at", first(v, &["datetime", "date", "time"])),
        (
            "confirmed_at",
            if truthy(&height) {
                field(v, "datetime").clone()
            } else {
                Py::Null
            },
        ),
        (
            "direction",
            s(if outbound { "outbound" } else { "inbound" }),
        ),
        ("asset", s("BTC")),
        (
            "amount",
            s(n.copy_abs()
                .div(&dec("100000000")?, &CTX)
                .map_err(decimal_error)?
                .to_string()),
        ),
        ("fee", s("0")),
        ("kind", s(kind)),
        (
            "description",
            if truthy(&label) {
                label.clone()
            } else {
                s(if cj {
                    "CoinJoin transaction"
                } else {
                    "Wasabi transaction"
                })
            },
        ),
        ("counterparty", Py::Null),
        ("_wasabi_label", label),
        ("_wasabi_islikelycoinjoin", Py::Bool(cj)),
        ("raw_json", s(dump(&raw)?)),
    ]))
}
fn coin(v: &Py) -> Result<Py, AppError> {
    let vout = integral(
        get(v, "vout")
            .or_else(|| get(v, "index"))
            .unwrap_or(field(v, "outputIndex")),
        "vout",
        false,
    )?;
    let amount = integral(&pick(v, &["amount", "amountSats"]), "amount", true)?;
    let p = parts(&first(v, &["keyPath", "fullKeyPath", "key_path"]));
    let tail = if p.len() >= 2 {
        let integer = |part: &kassiber_pyjson::PyStr| {
            part.as_str().and_then(|part| {
                kassiber_pytext::parse_int(part.trim_end_matches(['\'', 'h', 'H'])).ok()
            })
        };
        integer(&p[p.len() - 2]).zip(integer(&p[p.len() - 1]))
    } else {
        None
    };
    let (branch, index, label) = tail.map_or((Py::Null, Py::Null, Py::Null), |(b, i)| {
        let label = match b.to_string().as_str() {
            "0" => "receive".into(),
            "1" => "change".into(),
            _ => format!("branch-{b}"),
        };
        (Py::Int(b), Py::Int(i), s(label))
    });
    let anon = first(
        v,
        &[
            "anonHistory",
            "anonymityHistory",
            "anonScoreHistory",
            "clusterHistory",
        ],
    );
    let anon = if matches!(anon, Py::List(_)) {
        sanitize(&anon)
    } else {
        Py::List(vec![])
    };
    let spent = txid(&first(v, &["spentBy", "spent_by"]));
    let confirmations = integral(field(v, "confirmations"), "confirmations", false)?;
    Ok(dict([
        ("txid", txid(&first(v, &["txid", "tx", "transactionId"]))),
        ("vout", vout),
        ("amount_sats", amount),
        ("asset", s("BTC")),
        ("chain", s("bitcoin")),
        ("network", s("mainnet")),
        (
            "confirmation_status",
            s(if boolean(field(v, "confirmed")) {
                "confirmed"
            } else {
                "mempool"
            }),
        ),
        (
            "confirmations",
            if blank(&confirmations) {
                Py::from(0)
            } else {
                confirmations
            },
        ),
        (
            "block_height",
            integral(&first(v, &["height", "blockHeight"]), "height", false)?,
        ),
        ("address", text(field(v, "address"))),
        ("address_label", text(field(v, "label"))),
        ("branch_label", label),
        ("branch_index", branch),
        ("address_index", index),
        (
            "anonymity_score",
            integral(field(v, "anonymityScore"), "anonymityScore", false)?,
        ),
        ("spent_by", spent.clone()),
        ("spent", Py::Bool(truthy(&spent))),
        (
            "excluded_from_coinjoin",
            Py::Bool(boolean(field(v, "excludedFromCoinjoin"))),
        ),
        ("key_state", text(field(v, "keyState"))),
        ("anon_history", anon.clone()),
        (
            "raw",
            dict([
                ("source", s("wasabi_coin")),
                ("anonymityScore", field(v, "anonymityScore").clone()),
                (
                    "excludedFromCoinjoin",
                    field(v, "excludedFromCoinjoin").clone(),
                ),
                ("keyState", field(v, "keyState").clone()),
                ("has_key_path", Py::Bool(!p.is_empty())),
                ("anon_history", anon),
            ]),
        ),
    ]))
}
fn payment(v: &Py) -> Result<Py, AppError> {
    let states = first(v, &["state", "states"]);
    let states = match states {
        Py::Dict(_) => vec![states],
        Py::List(v) => v,
        _ => vec![],
    };
    let states = states
        .iter()
        .filter(|s| matches!(s, Py::Dict(_)))
        .map(|v| {
            compact(
                dict([
                    ("status", text(field(v, "status"))),
                    ("round_id", text(&first(v, &["roundId", "round_id"]))),
                    ("payment_id", text(&first(v, &["paymentId", "payment_id"]))),
                ]),
                false,
            )
        })
        .collect();
    Ok(compact(
        dict([
            ("id", text(&first(v, &["id", "paymentId", "payment_id"]))),
            ("round_id", text(&first(v, &["roundId", "round_id"]))),
            (
                "amount_sats",
                integral(&pick(v, &["amount", "amountSats"]), "payment amount", false)?,
            ),
            ("states", Py::List(states)),
        ]),
        true,
    ))
}
fn digit(c: char) -> bool {
    const EXTRA: &[(u32, u32)] = &[
        (0xb2, 0xb3),
        (0xb9, 0xb9),
        (0x1369, 0x1371),
        (0x19da, 0x19da),
        (0x2070, 0x2070),
        (0x2074, 0x2079),
        (0x2080, 0x2089),
        (0x2460, 0x2468),
        (0x2474, 0x247c),
        (0x2488, 0x2490),
        (0x24ea, 0x24ea),
        (0x24f5, 0x24fd),
        (0x24ff, 0x24ff),
        (0x2776, 0x277e),
        (0x2780, 0x2788),
        (0x278a, 0x2792),
        (0x10a40, 0x10a43),
        (0x10e60, 0x10e68),
        (0x11052, 0x1105a),
        (0x1f100, 0x1f10a),
    ];
    let point = u32::from(c);
    kassiber_pytext::decimal_value(point).is_some()
        || EXTRA.iter().any(|(a, b)| *a <= point && point <= *b)
}
fn metadata(info: &Py, wallet: &Py) -> Result<Py, AppError> {
    let mut merged = if matches!(wallet, Py::Dict(_)) {
        wallet.clone()
    } else {
        dict::<&str>([])
    };
    if let Py::Dict(items) = info {
        for (k, v) in items {
            if let Key::Str(k) = k {
                if let Some(k) = k.as_str() {
                    set(&mut merged, k, v.clone());
                }
            }
        }
    }
    if !truthy(&merged) {
        return Ok(merged);
    }
    let choose = |names: &[&str]| -> Py {
        for n in names {
            if let Some(v) = get(&merged, n) {
                return v.clone();
            }
        }
        if let Py::Dict(items) = &merged {
            for n in names {
                if let Some((_, v)) = items.iter().rev().find(
                    |(k, _)| matches!(k,Key::Str(k) if key(&k.code_points().map(|p|char::from_u32(p).unwrap_or(char::REPLACEMENT_CHARACTER)).collect::<String>())==key(n)),
                ) {
                    return v.clone();
                }
            }
        }
        Py::Null
    };
    let accounts = match choose(&["accounts", "Accounts"]) {
        Py::List(items) => items,
        _ => vec![],
    };
    let mut hints = vec![];
    for a in accounts.iter().filter(|a| matches!(a, Py::Dict(_))) {
        let name = text(&first(a, &["name", "Name"]));
        let mut hint = dict([("name", if blank(&name) { s("account") } else { name })]);
        let p = parts(&first(a, &["keyPath", "KeyPath", "fullKeyPath"]));
        for (i, k) in [(0, "purpose"), (1, "coin_type")] {
            if let Some(p) = p.get(i).and_then(kassiber_pyjson::PyStr::as_str) {
                let p = p.trim_end_matches(['\'', 'h', 'H']);
                if !p.is_empty() && p.chars().all(digit) {
                    let n = kassiber_pytext::parse_int(p).map_err(|_| {
                        AppError::new(
                            "internal_error",
                            format!(
                                "invalid literal for int() with base 10: {}",
                                kassiber_pytext::repr_str(p)
                            ),
                        )
                    })?;
                    set(&mut hint, k, Py::Int(n));
                }
            }
        }
        if p.len() >= 3 {
            set(&mut hint, "account_path_hint", account_prefix(&p));
        }
        hints.push(hint);
    }
    let mut result = dict::<&str>([]);
    for (name, names) in [
        ("walletName", &["walletName", "Name"][..]),
        ("state", &["State", "state"]),
        ("coinjoinStatus", &["coinjoinStatus", "CoinjoinStatus"]),
    ] {
        set(&mut result, name, text(&choose(names)));
    }
    for (name, names, field) in [
        (
            "anonScoreTarget",
            &["anonScoreTarget", "AnonScoreTarget"][..],
            "anonScoreTarget",
        ),
        (
            "minGapLimit",
            &["MinGapLimit", "minGapLimit"],
            "MinGapLimit",
        ),
    ] {
        set(&mut result, name, integral(&choose(names), field, false)?);
    }
    for (name, names) in [
        ("autoCoinJoin", &["isAutoCoinjoin", "AutoCoinJoin"][..]),
        (
            "redCoinIsolation",
            &["RedCoinIsolation", "isNonPrivateCoinIsolation"],
        ),
        ("isWatchOnly", &["isWatchOnly", "IsWatchOnly"]),
        (
            "isHardwareWallet",
            &["isHardwareWallet", "IsHardwareWallet"],
        ),
    ] {
        set(&mut result, name, Py::Bool(boolean(&choose(names))));
    }
    for (name, names) in [
        (
            "segwitAccountPathHint",
            &["SegWitAccountKeyPath", "segwitAccountKeyPath"][..],
        ),
        (
            "taprootAccountPathHint",
            &["TaprootAccountKeyPath", "taprootAccountKeyPath"],
        ),
        (
            "silentPaymentAccountPathHint",
            &["SilentPaymentAccountKeyPath", "silentPaymentAccountKeyPath"],
        ),
    ] {
        set(&mut result, name, path_hint(&choose(names)));
    }
    set(&mut result, "accounts", Py::List(hints));
    set(&mut result, "walletJsonImported", Py::Bool(truthy(wallet)));
    Ok(compact(result, true))
}
pub(super) fn rows(records: &[Py]) -> Result<Vec<csv::Row>, AppError> {
    Ok(records
        .iter()
        .map(|r| csv::Row {
            headers: vec![],
            data: generic_json::projection(r),
            rest: None,
            original: Some(r.clone()),
        })
        .collect())
}
pub(super) fn load(file: &str) -> Result<Bundle, AppError> {
    load_payload(csv::read_json(file, "Wasabi bundle", false)?)
}

/// `load_wasabi_bundle_payload`: a bundle the desktop sent inline.
pub(super) fn load_payload(mut payload: Py) -> Result<Bundle, AppError> {
    if let Py::List(items) = payload {
        payload = dict([(
            "gethistory",
            Py::List(
                items
                    .into_iter()
                    .filter(|v| matches!(v, Py::Dict(_)))
                    .collect(),
            ),
        )]);
    }
    if !matches!(payload, Py::Dict(_)) {
        return Err(AppError::new(
            "validation",
            "Wasabi bundle must be a JSON object",
        ));
    }
    let history_rows = list(&payload, &["gethistory", "history", "transactions"])?;
    let coin_sections = present(&payload, &["listcoins", "coins"])
        || present(&payload, &["listunspentcoins", "unspentcoins", "unspent"]);
    let mut coins: Vec<Py> = vec![];
    for row in list(&payload, &["listcoins", "coins"])?
        .iter()
        .chain(list(&payload, &["listunspentcoins", "unspentcoins", "unspent"])?.iter())
    {
        let row = coin(row)?;
        if let Some(i) = coins.iter().position(|c| {
            field(c, "txid") == field(&row, "txid") && field(c, "vout") == field(&row, "vout")
        }) {
            coins[i] = row;
        } else {
            coins.push(row);
        }
    }
    let info = section(&payload, &["getwalletinfo", "walletinfo"]);
    let wallet = section(&payload, &["wallet_json", "walletjson", "wallet"]);
    let wallet = if matches!(wallet, Py::Dict(_)) {
        wallet
    } else {
        dict::<&str>([])
    };
    let payments = list(&payload, &["listpaymentsincoinjoin", "paymentsincoinjoin"])?
        .iter()
        .map(payment)
        .collect::<Result<Vec<_>, _>>()?;
    let keys = list(&payload, &["listkeys", "keys"])?;
    let mut metadata = metadata(&info, &wallet)?;
    if !keys.is_empty() {
        let mut counts: Vec<(Key, Py)> = vec![];
        for k in &keys {
            let state = text(field(k, "keyState"));
            if let Py::Str(state) = state {
                let key = Key::Str(state);
                if let Some((_, value)) = counts.iter_mut().find(|(k, _)| *k == key) {
                    let n = py_str(value).parse::<i64>().unwrap();
                    *value = Py::from(n + 1);
                } else {
                    counts.push((key, Py::from(1)));
                }
            }
        }
        set(&mut metadata, "keyStateCounts", Py::Dict(counts));
    }
    if !payments.is_empty() {
        set(
            &mut metadata,
            "paymentsInCoinJoin",
            Py::List(payments.clone()),
        );
    }
    let records = history_rows
        .iter()
        .map(history)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Bundle {
        records,
        coins,
        metadata,
        coin_sections,
        wallet_json: truthy(&wallet),
        keys: keys.len(),
        payments: payments.len(),
        #[cfg(test)]
        payment_rows: payments,
    })
}

pub(super) fn apply(
    conn: &Connection,
    request: &Request,
    wallet: &crate::Wallet,
    bundle: &Bundle,
    ids: &mut Ids<'_>,
    mut result: Py,
) -> Result<Py, AppError> {
    let workspace: String = conn
        .query_row(
            "SELECT workspace_id FROM profiles WHERE id=?",
            [&request.profile_id],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    let original: String = conn
        .query_row(
            "SELECT config_json FROM wallets WHERE id=?",
            [&wallet.id],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    let mut tags = vec![];
    let mut created = 0;
    for (code, label) in [
        ("wasabi", "Wasabi"),
        ("coinjoin", "CoinJoin"),
        ("privacy-hop-review", "Privacy hop review"),
    ] {
        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM tags WHERE profile_id=? AND code=?",
                [&request.profile_id, code],
                |r| r.get(0),
            )
            .optional()
            .map_err(database_error)?;
        let tag = match existing {
            Some(id) => id,
            None => {
                let id = ids.next(conn)?;
                conn.execute("INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES(?,?,?,?,?,?)",params![id,workspace,request.profile_id,code,label,ids.time(conn)?]).map_err(database_error)?;
                created += 1;
                id
            }
        };
        tags.push(tag);
    }
    let (mut notes, mut added, mut marked, mut cleared) = (0i64, 0i64, 0i64, 0i64);
    for row in &bundle.records {
        let external = text(field(row, "txid"));
        if blank(&external) {
            continue;
        }
        type MetadataRow = (String, Option<String>, Option<String>, Option<String>);
        let tx:Option<MetadataRow>=conn.query_row("SELECT id,note,review_status,privacy_boundary FROM transactions WHERE profile_id=? AND wallet_id=? AND external_id=? ORDER BY created_at DESC LIMIT 1",params![request.profile_id,wallet.id,py_str(&external)],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(database_error)?;
        let Some((id, note, review, boundary)) = tx else {
            continue;
        };
        let label = text(field(row, "_wasabi_label"));
        if !blank(&label) && note.as_deref().is_none_or(str::is_empty) {
            conn.execute(
                "UPDATE transactions SET note=? WHERE id=?",
                params![py_str(&label), id],
            )
            .map_err(database_error)?;
            notes += 1;
        }
        let cj = truthy(field(row, "_wasabi_islikelycoinjoin"));
        if cj {
            if !matches!(review.as_deref(), Some("review" | "completed")) {
                conn.execute(
                    "UPDATE transactions SET review_status='review' WHERE id=?",
                    [&id],
                )
                .map_err(database_error)?;
                marked += 1;
            }
        } else if boundary.as_deref().is_none_or(str::is_empty) {
            let had:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM transaction_tags WHERE transaction_id=? AND tag_id=?)",[&id,&tags[2]],|r|r.get(0)).map_err(database_error)?;
            conn.execute(
                "DELETE FROM transaction_tags WHERE transaction_id=? AND tag_id IN (?,?)",
                params![id, tags[1], tags[2]],
            )
            .map_err(database_error)?;
            if had && review.as_deref() == Some("review") {
                conn.execute(
                    "UPDATE transactions SET review_status=NULL WHERE id=?",
                    [&id],
                )
                .map_err(database_error)?;
                cleared += 1;
            }
        }
        for tag in tags.iter().take(if cj { 3 } else { 1 }) {
            added += conn
                .execute(
                    "INSERT OR IGNORE INTO transaction_tags(transaction_id,tag_id) VALUES(?,?)",
                    [&id, tag],
                )
                .map_err(database_error)? as i64;
        }
    }
    let mut config = kassiber_pytext::loads(&original).unwrap_or_else(|_| dict::<&str>([]));
    let before = config.clone();
    if truthy(&bundle.metadata) {
        set(&mut config, "wasabi_metadata", bundle.metadata.clone());
    }
    for (key, value) in [
        ("chain", "bitcoin"),
        ("network", "mainnet"),
        ("source_format", "wasabi_bundle"),
    ] {
        if get(&config, key).is_none() {
            set(&mut config, key, s(value));
        }
    }
    if truthy(&bundle.metadata) || config != before {
        conn.execute(
            "UPDATE wallets SET config_json=? WHERE id=?",
            [dump(&config)?, wallet.id.clone()],
        )
        .map_err(database_error)?;
    }
    // The bundle's own keys end the reply; a file's batch follows them.
    if let Py::Dict(items) = &mut result {
        items.retain(
            |(k, _)| !matches!(k,Key::Str(k) if matches!(k.as_str(),Some("file"|"input_format"))),
        );
    }
    for (key, n) in [
        ("wasabi_notes_set", notes),
        ("wasabi_tags_added", added),
        ("wasabi_tags_created", created),
        ("wasabi_review_marked", marked),
        ("wasabi_review_cleared", cleared),
    ] {
        set(&mut result, key, Py::from(n));
    }
    if bundle.coin_sections {
        let original = kassiber_pytext::loads(&original)
            .map_err(|e| AppError::new("internal_error", e.to_string()))?;
        let chain = first(&original, &["chain"]);
        let chain = if truthy(&chain) {
            py_str(&chain)
        } else {
            "bitcoin".into()
        };
        let network = first(&original, &["network"]);
        let network = if truthy(&network) {
            py_str(&network)
        } else {
            "mainnet".into()
        };
        let inventory = crate::publication::inventory::update(
            conn,
            &crate::publication::inventory::Source {
                workspace_id: &workspace,
                profile_id: &request.profile_id,
                wallet_id: &wallet.id,
                wallet_config_json: &original,
                backend_name: "wasabi",
                backend_kind: "wasabi_bundle",
                chain: &chain,
                network: &network,
            },
            &bundle.coins,
            &ids.time(conn)?,
        )?;
        for (key, source) in [
            ("wasabi_coins_observed", "observed"),
            ("wasabi_coins_active", "active"),
            ("wasabi_coins_marked_spent", "spent"),
        ] {
            set(&mut result, key, field(&inventory, source).clone());
        }
    }
    for (key, v) in [
        ("wasabi_transactions", Py::from(bundle.records.len() as i64)),
        (
            "wasabi_payments_in_coinjoin",
            Py::from(bundle.payments as i64),
        ),
        ("wasabi_wallet_json_present", Py::Bool(bundle.wallet_json)),
        ("wasabi_listkeys_count", Py::from(bundle.keys as i64)),
        ("input_format", s("wasabi_bundle")),
    ] {
        set(&mut result, key, v);
    }
    if !truthy(field(&result, "journal_invalidated")) && notes + added + marked + cleared > 0 {
        invalidate_journals(conn, &request.profile_id)?;
        set(&mut result, "journal_invalidated", Py::Bool(true));
    }
    let path = kassiber_paths::abspath(std::path::Path::new(&request.file))
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    set(&mut result, "file", s(path.to_string_lossy().into_owned()));
    Ok(result)
}

/// `import_batches.record_batch` for a bundle file (06ddfc545): the rows this
/// run created, linked before commit so the run can be rolled back. An inline
/// bundle records none.
pub(super) fn record_batch(
    conn: &Connection,
    request: &Request,
    wallet: &crate::Wallet,
    ids: &mut Ids<'_>,
    mut result: Py,
) -> Result<Py, AppError> {
    let inserted: Vec<String> = match get(&result, "inserted_records") {
        Some(Py::List(records)) => records
            .iter()
            .filter_map(|record| get(record, "transaction_id").filter(|id| truthy(id)))
            .map(py_str)
            .collect(),
        _ => Vec::new(),
    };
    if inserted.is_empty() {
        return Ok(result);
    }
    let count = |key| match get(&result, key) {
        Some(Py::Int(n)) => i64::try_from(n.clone()).unwrap_or(0),
        Some(Py::Bool(flag)) => i64::from(*flag),
        _ => 0,
    };
    let (updated, skipped) = (count("updated"), count("skipped"));
    let batch = ids.next(conn)?;
    let workspace: String = conn
        .query_row(
            "SELECT workspace_id FROM profiles WHERE id=?",
            [&request.profile_id],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    let filename = std::path::Path::new(&request.file)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    conn.execute(
        "INSERT INTO import_batches(id,workspace_id,profile_id,wallet_id,source_format,source_filename,column_map_json,imported_at,rows_inserted,rows_updated,rows_skipped) VALUES(?,?,?,?,?,?,NULL,?,?,?,?)",
        kassiber_store::rusqlite::params![
            batch,
            workspace,
            request.profile_id,
            wallet.id,
            request.format.name(),
            filename,
            ids.time(conn)?,
            inserted.len() as i64,
            updated,
            skipped,
        ],
    )
    .map_err(database_error)?;
    for (position, id) in (1_i64..).zip(&inserted) {
        conn.execute(
            "INSERT OR IGNORE INTO import_batch_transactions(batch_id,transaction_id,source_position) VALUES(?,?,?)",
            params![batch, id, position],
        )
        .map_err(database_error)?;
    }
    set(&mut result, "import_batch_id", s(batch));
    Ok(result)
}
