use super::*;

const TOKEN_HINT: &str = "Rename the wallet, profile, or asset label to remove commas, newlines, brackets, comments, or key delimiters before processing reports.";
const CURRENCIES: &str = "AED AFN ALL AMD AOA ARS AUD AWG AZN BAM BBD BDT BHD BIF BMD BND BOB BOV BRL BSD BTN BWP BYN BZD CAD CDF CHE CHF CHW CLF CLP CNY COP COU CRC CUP CVE CZK DJF DKK DOP DZD EGP ERN ETB EUR FJD FKP GBP GEL GHS GIP GMD GNF GTQ GYD HKD HNL HTG HUF IDR ILS INR IQD IRR ISK JMD JOD JPY KES KGS KHR KMF KPW KRW KWD KYD KZT LAK LBP LKR LRD LSL LYD MAD MDL MGA MKD MMK MNT MOP MRU MUR MVR MWK MXN MXV MYR MZN NAD NGN NIO NOK NPR NZD OMR PAB PEN PGK PHP PKR PLN PYG QAR RON RSD RUB RWF SAR SBD SCR SDG SEK SGD SHP SLE SOS SRD SSP STN SVC SYP SZL THB TJS TMT TND TOP TRY TTD TWD TZS UAH UGX USD USN UYI UYU UYW UZS VED VES VND VUV WST XAD XAF XAG XAU XBA XBB XBC XBD XCD XCG XDR XOF XPD XPF XPT XSU XTS XUA XXX YER ZAR ZMW ZWG";
fn app(stage: &'static str, message: impl Into<String>, validation: bool) -> Refusal {
    let mut error = Refusal::new(stage, "AppError", message);
    error.code = Some(if validation {
        "validation"
    } else {
        "app_error"
    });
    error
}
fn token(value: &Value, field: &str) -> Result<String, Refusal> {
    let raw = or_text(value);
    let value = raw.py_strip();
    if value.is_empty() {
        let mut error = app("configuration", "RP2 configuration value is required", true);
        error.details = Box::new(json!({"field":field}));
        return Err(error);
    }
    let chars: Vec<String> = ['\n', '\r', '#', ',', ':', ';', '=', '[', ']']
        .into_iter()
        .filter(|c| value.contains(*c))
        .map(|c| c.to_string())
        .collect();
    if !chars.is_empty() {
        let mut error = app(
            "configuration",
            format!("RP2 configuration {field} contains unsupported characters"),
            true,
        );
        error.hint = Some(TOKEN_HINT.into());
        error.details = Box::new(json!({"field":field,"value":value,"characters":chars}));
        error.details_order = Some(&(&["field", "value", "characters"] as &[&str]));
        return Err(error);
    }
    Ok(value.into())
}
fn policy(profile: &Value) -> Result<Value, Refusal> {
    let raw = &profile["tax_country"];
    let country = if codec::truth(raw) {
        string(raw).py_strip().py_lower()
    } else {
        "generic".into()
    };
    let country = if country.is_empty() {
        "generic"
    } else {
        &country
    };
    if !["generic", "at"].contains(&country) {
        return Err(app(
            "configuration",
            format!("Unsupported tax country '{}'", string(raw)),
            false,
        ));
    }
    let country = if country == "at" {
        json!({"kind":"at"})
    } else {
        let days = profile
            .get("tax_long_term_days")
            .filter(|v| codec::truth(v))
            .cloned()
            .unwrap_or(json!(365));
        let days=codec::integer(&days).map_err(|_|{
            if days.is_null()||days.is_array()||days.is_object() {
                let kind=if days.is_null(){"NoneType"}else if days.is_array(){"list"}else{"dict"};
                Refusal::new("configuration","TypeError",format!("int() argument must be a string, a bytes-like object or a real number, not '{kind}'"))
            } else if days.as_f64().is_some_and(|v|!v.is_finite()) {
                Refusal::new("configuration","OverflowError","cannot convert float infinity to integer")
            } else {app("configuration",format!("invalid literal for int() with base 10: {}",repr(&days)),false)}
        })?;
        let n = integer(&days).unwrap();
        if n < BigInt::from(0) {
            return Err(app(
                "configuration",
                "tax_long_term_days cannot be negative",
                false,
            ));
        }
        let currency = string(&profile["fiat_currency"]).py_strip().py_upper();
        if !CURRENCIES.split_whitespace().any(|c| c == currency) {
            return Err(Refusal::new(
                "configuration",
                "EngineValueError",
                format!("Parameter 'currency_iso_code' has non-ISO-4217 format value {currency}."),
            ));
        }
        json!({"kind":"generic","long_term_days":if n>BigInt::from(i64::MAX){json!(i64::MAX)}else{days}})
    };
    let label = string(&profile["gains_algorithm"]);
    let method = label.py_strip().py_lower();
    let supported = matches!(
        method.as_str(),
        "fifo" | "lifo" | "hifo" | "lofo" | "moving_average"
    ) || (country["kind"] == "at" && method == "moving_average_at");
    let coarse = &profile["require_coarse_review"];
    let coarse = if let Some(s) = coarse.as_str() {
        matches!(
            s.py_strip().py_lower().as_str(),
            "1" | "true" | "yes" | "on"
        )
    } else {
        codec::truth(coarse)
    };
    Ok(
        json!({"country":country,"method":if supported{json!(method)}else{Value::Null},"method_label":label,"require_coarse_review":coarse}),
    )
}
// ConfigParser BasicInterpolation, including its distinct syntax/missing/depth
// exceptions. The original field's complete raw value appears in errors.
fn interpolate(
    field: &str,
    raw: &str,
    values: &BTreeMap<&str, String>,
    depth: usize,
) -> Result<String, Refusal> {
    let fail = |family, message| Refusal::new("configuration", family, message);
    if depth > 10 {
        return Err(fail("InterpolationDepthError",format!("Recursion limit exceeded in value substitution: option '{field}' in section 'general' contains an interpolation key which cannot be substituted in 10 steps. Raw value: {}",repr(&json!(values[field])))));
    }
    let mut rest = raw;
    let mut result = String::new();
    while let Some(at) = rest.find('%') {
        result.push_str(&rest[..at]);
        rest = &rest[at..];
        if let Some(tail) = rest.strip_prefix("%%") {
            result.push('%');
            rest = tail;
            continue;
        }
        if rest.starts_with("%(") {
            if let Some(end) = rest.find(")s") {
                let name = &rest[2..end];
                if !name.contains(')') {
                    let key = name.py_lower();
                    let Some(value) = values.get(key.as_str()) else {
                        return Err(fail("InterpolationMissingOptionError",format!("Bad value substitution: option '{field}' in section 'general' contains an interpolation key '{key}' which is not a valid option name. Raw value: {}",repr(&json!(values[field])))));
                    };
                    result.push_str(&interpolate(field, value, values, depth + 1)?);
                    rest = &rest[end + 2..];
                    continue;
                }
            }
            return Err(fail(
                "InterpolationSyntaxError",
                format!(
                    "bad interpolation variable reference {}",
                    repr(&json!(rest))
                ),
            ));
        }
        return Err(fail(
            "InterpolationSyntaxError",
            format!(
                "'%' must be followed by '%' or '(', found: {}",
                repr(&json!(rest))
            ),
        ));
    }
    result.push_str(rest);
    Ok(result)
}
pub(super) fn check(
    profile: &Value,
    wallets: &[(Value, Value)],
    rows: &[impl super::RequestRow],
) -> Result<(Value, Value), Refusal> {
    let mut labels = Vec::new();
    for (_, wallet) in wallets {
        if !empty(&wallet["label"]) {
            labels.push(token(&json!(string(&wallet["label"])), "wallet_label")?);
        }
    }
    labels.sort();
    let assets: BTreeSet<_> = rows.iter().map(|r| string(r.at("asset"))).collect();
    let mut assets = assets
        .iter()
        .map(|a| token(&json!(a), "asset"))
        .collect::<Result<Vec<_>, _>>()?;
    assets.sort();
    let holder = token(
        &json!(if profile["label"].is_null() {
            String::new()
        } else {
            string(&profile["label"]).py_strip().into()
        }),
        "profile_label",
    )?;
    if labels.is_empty() {
        return Err(app(
            "configuration",
            "RP2 configuration requires at least one wallet",
            false,
        ));
    }
    if assets.is_empty() {
        return Err(app(
            "configuration",
            "RP2 configuration requires at least one asset",
            false,
        ));
    }
    let policy = policy(profile)?;
    let values = BTreeMap::from([
        ("assets", assets.join(", ")),
        ("exchanges", labels.join(", ")),
        ("holders", holder),
    ]);
    let mut names = json!({});
    for field in ["assets", "exchanges", "holders"] {
        let parsed = interpolate(field, &values[field], &values, 1)?;
        if parsed.py_strip().is_empty() {
            return Err(Refusal::new(
                "configuration",
                "EngineValueError",
                format!("field '{field}' in section 'general' cannot be empty"),
            ));
        }
        let mut unique = BTreeSet::new();
        for element in parsed.split(',').map(|s| s.py_strip()) {
            if element.is_empty() {
                return Err(Refusal::new(
                    "configuration",
                    "EngineValueError",
                    format!("field '{field}' in section 'general' cannot contain empty elements"),
                ));
            }
            if !unique.insert(element) {
                return Err(Refusal::new("configuration","EngineValueError",format!("field '{field}' in section 'general' contains duplicate elements: {element}")));
            }
        }
        names[field] = json!(unique);
    }
    Ok((policy, names))
}
pub(super) fn pool_scope(profile: &Value) -> Result<(), Refusal> {
    let raw = profile
        .get("cost_basis_pool_scope")
        .cloned()
        .unwrap_or(json!("global"));
    let scope = if codec::truth(&raw) {
        string(&raw)
    } else {
        "global".into()
    };
    if scope.py_strip().py_lower() != "global" {
        let mut error = app(
            "pool_scope",
            format!("Unsupported cost-basis pool scope '{}'", string(&raw)),
            true,
        );
        error.hint = Some("Choose one of: global".into());
        return Err(error);
    }
    Ok(())
}
