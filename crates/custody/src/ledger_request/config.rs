use super::*;
use kassiber_tax_countries::{Fiat, Rules};

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
/// `build_tax_policy(profile).fiat_currency`: the country's currency, else
/// the profile's, stripped and upper-cased. `None` for a country the
/// registry does not know; `check` refuses such a book first.
pub fn book_currency(profile: &Value) -> Option<String> {
    let raw = &profile["tax_country"];
    let country = if codec::truth(raw) {
        string(raw).py_strip().py_lower()
    } else {
        kassiber_tax_countries::DEFAULT_CODE.into()
    };
    let country = if country.is_empty() {
        kassiber_tax_countries::DEFAULT_CODE
    } else {
        &country
    };
    Some(match kassiber_tax_countries::lookup(country)?.fiat {
        Fiat::Fixed(code) => code.to_owned(),
        Fiat::Profile => string(&profile["fiat_currency"]).py_strip().py_upper(),
    })
}
fn policy(profile: &Value) -> Result<Value, Refusal> {
    let raw = &profile["tax_country"];
    let country = if codec::truth(raw) {
        string(raw).py_strip().py_lower()
    } else {
        kassiber_tax_countries::DEFAULT_CODE.into()
    };
    let country = if country.is_empty() {
        kassiber_tax_countries::DEFAULT_CODE
    } else {
        &country
    };
    let Some(entry) = kassiber_tax_countries::lookup(country) else {
        return Err(app(
            "configuration",
            format!("Unsupported tax country '{}'", string(raw)),
            false,
        ));
    };
    let country = engine_country(entry, profile)?;
    let label = string(&profile["gains_algorithm"]);
    let method = label.py_strip().py_lower();
    let supported = entry
        .methods
        .iter()
        .any(|allowed| allowed.to_lowercase() == method);
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
/// The profile keys a book's own tax calendar arrives in (the journals
/// loader adds them from the book's settings only when the book has chosen
/// one, so every other profile keeps its bytes).
pub const BOOK_TAX_YEAR_START: &str = "tax_year_start";
pub const BOOK_TIMEZONE: &str = "tax_timezone";
/// The profile key a `global_then_wallet` book's wallet order arrives in: a
/// JSON array of its wallet ids by when they were added (the journals
/// loader adds it for those books only).
pub const BOOK_WALLET_ORDER: &str = "wallet_allocation_order";

/// The wallet ids the United States' 2025 allocation fills in order, when
/// the profile carries them; a value that is not a list of ids fails closed.
pub(super) fn wallet_order(profile: &Value) -> Result<Option<Vec<String>>, Refusal> {
    let Some(raw) = profile.get(BOOK_WALLET_ORDER).filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let invalid = || {
        app(
            "configuration",
            "The book's wallet order is not a list of wallet ids",
            false,
        )
    };
    let parsed: Value = match raw {
        Value::String(text) => serde_json::from_str(text).map_err(|_| invalid())?,
        other => other.clone(),
    };
    let ids = parsed.as_array().ok_or_else(invalid)?;
    ids.iter()
        .map(|id| id.as_str().map(str::to_owned).ok_or_else(invalid))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// The book's tax calendar: its country's, or the start and zone it chose
/// where the country lets it. A stored choice the country no longer allows
/// (its region changed under it) fails closed.
pub fn book_calendar(
    entry: &kassiber_tax_countries::Country,
    profile: &Value,
) -> Result<kassiber_tax_countries::BookCalendar, Refusal> {
    let text = |key: &str| {
        profile
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
    };
    entry
        .book_calendar(text(BOOK_TAX_YEAR_START), text(BOOK_TIMEZONE))
        .map_err(|error| {
            let mut refusal = app("configuration", error.to_string(), true);
            refusal.hint = Some(error.hint().into());
            refusal.details = Box::new(json!({"reason": error.code()}));
            refusal
        })
}

/// The engine's country of a registry entry: its shared engine rules when
/// it has them, else its family's (Python's `build_tax_policy` for generic
/// and Austrian books).
fn engine_country(
    entry: &kassiber_tax_countries::Country,
    profile: &Value,
) -> Result<Value, Refusal> {
    let calendar = book_calendar(entry, profile)?;
    // A country that spans zones computes in the book's.
    let zoned = kassiber_tax_countries::Country {
        timezone: calendar.timezone,
        ..*entry
    };
    Ok(match (entry.engine, entry.rules) {
        (Some(rules), _) => rules_country(&zoned, rules)?,
        (None, Rules::Austria) => json!({"kind":"at"}),
        // The holding period is the engine's US rule; the stored days are
        // not read.
        (None, Rules::UnitedStates) => json!({"kind":"us"}),
        // The registry gives every such country its engine rules.
        (
            None,
            Rules::Germany
            | Rules::Switzerland
            | Rules::Ireland
            | Rules::UnitedKingdom
            | Rules::Spain
            | Rules::Japan
            | Rules::Australia
            | Rules::NewZealand
            | Rules::India
            | Rules::SouthAfrica,
        ) => {
            return Err(app(
                "configuration",
                format!("Tax country '{}' has no engine rules", entry.code),
                false,
            ))
        }
        (None, Rules::Generic) => {
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
            let currency = match entry.fiat {
                Fiat::Fixed(code) => code.to_owned(),
                Fiat::Profile => string(&profile["fiat_currency"]).py_strip().py_upper(),
            };
            if !CURRENCIES.split_whitespace().any(|c| c == currency) {
                return Err(Refusal::new(
                    "configuration",
                    "EngineValueError",
                    format!(
                        "Parameter 'currency_iso_code' has non-ISO-4217 format value {currency}."
                    ),
                ));
            }
            let mut country = json!({"kind":"generic","long_term_days":if n>BigInt::from(i64::MAX){json!(i64::MAX)}else{days}});
            // A generic book's own tax year; absent for the UTC calendar
            // year, so those requests keep their bytes.
            if calendar.tax_year_start != (1, 1) {
                country["tax_year_start"] = json!(calendar.start_text());
            }
            if calendar.timezone != kassiber_tax_countries::UTC {
                country["timezone"] = json!(calendar.timezone);
            }
            country
        }
    })
}
/// The engine's `rules` country of a registry entry with shared engine
/// rules: its zone, its holding period, and each feature it switches on.
/// A holding period counted in the profile's days has no calendar form and
/// is refused rather than approximated.
fn rules_country(
    entry: &kassiber_tax_countries::Country,
    rules: kassiber_tax_countries::EngineRules,
) -> Result<Value, Refusal> {
    let entry = kassiber_tax_countries::Country {
        engine: Some(rules),
        ..*entry
    };
    match entry.engine_country() {
        Some(Ok(country)) => Ok(country),
        _ => Err(app(
            "configuration",
            format!(
                "Tax country '{}' counts its holding period in days, which its engine rules cannot express",
                entry.code
            ),
            false,
        )),
    }
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
pub(super) fn pool_scope(profile: &Value) -> Result<String, Refusal> {
    let raw = profile
        .get("cost_basis_pool_scope")
        .cloned()
        .unwrap_or(json!("global"));
    let scope = if codec::truth(&raw) {
        string(&raw)
    } else {
        "global".into()
    };
    let scope = scope.py_strip().py_lower();
    // `policy` already refused an unknown country.
    let country = &profile["tax_country"];
    let code = if codec::truth(country) {
        string(country).py_strip().py_lower()
    } else {
        String::new()
    };
    let allowed = kassiber_tax_countries::lookup(if code.is_empty() {
        kassiber_tax_countries::DEFAULT_CODE
    } else {
        &code
    })
    .map_or(&["global"][..], |entry| entry.pool_scopes);
    if !allowed.contains(&scope.as_str()) {
        let mut error = app(
            "pool_scope",
            format!("Unsupported cost-basis pool scope '{}'", string(&raw)),
            true,
        );
        error.hint = Some(format!("Choose one of: {}", allowed.join(", ")).into());
        return Err(error);
    }
    // A method the engine has no form of in this scope fails here, before
    // the engine, with the profile refusal's code (`policy` left a method
    // the country does not offer to the method gate).
    let method = string(&profile["gains_algorithm"]).py_strip().py_upper();
    if kassiber_tax_countries::COUNTRIES
        .iter()
        .any(|entry| entry.allows_method(&method))
        && !kassiber_tax_countries::method_allows_pool_scope(&method, &scope)
    {
        let mut error = Refusal::new(
            "pool_scope",
            "AppError",
            format!(
                "Accounting method '{method}' has no form for the cost-basis pool scope '{scope}'"
            ),
        );
        error.code = Some("method_scope_unsupported");
        return Err(error);
    }
    Ok(scope)
}
#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_tax_countries::{
        lookup, Country, EngineRules, FeeUnits, Gifts, LongTerm, SwapValues,
    };

    /// A registry entry computed with shared engine rules: the US entry's
    /// fields with Germany's calendar.
    fn ruled(long_term: LongTerm) -> Country {
        Country {
            code: "zz",
            timezone: "Europe/Berlin",
            long_term,
            engine: Some(EngineRules::default()),
            ..*lookup("us").unwrap()
        }
    }

    #[test]
    fn engine_rules_give_a_rules_country_in_the_registrys_zone() {
        let entry = ruled(LongTerm::MoreThanOneYear);
        assert_eq!(
            engine_country(&entry, &json!({})).unwrap(),
            json!({"kind":"rules","timezone":"Europe/Berlin","holding":"more_than_one_year"})
        );
        let mut entry = ruled(LongTerm::Never);
        entry.tax_year_start = (4, 6);
        entry.pool_scopes = &["wallet"];
        entry.engine = Some(EngineRules {
            fee_units: FeeUnits::DisposalBeforeSale,
            swap_values: SwapValues::ReceivedGiven,
            gifts: Gifts {
                received_refused: true,
                ..Gifts::default()
            },
            ..EngineRules::default()
        });
        assert_eq!(
            engine_country(&entry, &json!({})).unwrap()["fee_units"],
            "disposal_before_sale"
        );
        assert_eq!(
            engine_country(&entry, &json!({})).unwrap()["swap_values"],
            "received_given"
        );
        assert_eq!(
            engine_country(&entry, &json!({})).unwrap()["gifts_received"],
            "quarantine"
        );
        assert_eq!(
            engine_country(&entry, &json!({})).unwrap()["tax_year_start"],
            "04-06"
        );
        assert_eq!(
            engine_country(&entry, &json!({})).unwrap()["pool_scope"],
            "wallet"
        );
        // A country allowing both scopes requires neither.
        let either = ruled(LongTerm::MoreThanOneYear);
        assert!(engine_country(&either, &json!({}))
            .unwrap()
            .get("pool_scope")
            .is_none());
        assert_eq!(
            rules_country(&entry, entry.engine.unwrap()).unwrap()["holding"],
            "never"
        );
        let entry = ruled(LongTerm::ProfileDays { default: 365 });
        let refusal = rules_country(&entry, entry.engine.unwrap()).unwrap_err();
        assert_eq!(refusal.code, Some("app_error"));
        // Germany's rules come from its entry, refused receipts included.
        let germany = engine_country(lookup("de").unwrap(), &json!({})).unwrap();
        assert_eq!(germany["kind"], "rules");
        assert_eq!(germany["pool_scope"], "wallet");
        assert_eq!(
            germany["refused_receipts"][0]["code"],
            "airdrop_classification_required"
        );
        // Generic, Austrian and US books keep their engine countries.
        for (code, kind) in [("generic", "generic"), ("at", "at"), ("us", "us")] {
            let entry = lookup(code).unwrap();
            assert_eq!(entry.engine, None, "{code}");
            let profile = json!({"fiat_currency": "EUR"});
            assert_eq!(engine_country(entry, &profile).unwrap()["kind"], kind);
        }
    }

    #[test]
    fn a_books_own_calendar_reaches_the_engine_only_when_chosen() {
        let generic = lookup("generic").unwrap();
        // No choice, or the calendar year in UTC: the request's bytes.
        for profile in [
            json!({"fiat_currency": "EUR"}),
            json!({"fiat_currency": "EUR", "tax_year_start": "01-01", "tax_timezone": "UTC"}),
        ] {
            assert_eq!(
                engine_country(generic, &profile).unwrap(),
                json!({"kind": "generic", "long_term_days": 365})
            );
        }
        let profile = json!({"fiat_currency": "AUD", "tax_year_start": "july", "tax_timezone": "Australia/Sydney"});
        assert_eq!(
            engine_country(generic, &profile).unwrap(),
            json!({"kind": "generic", "long_term_days": 365, "tax_year_start": "07-01", "timezone": "Australia/Sydney"})
        );
        // A country with a legal year refuses another one, fail closed.
        let refusal =
            engine_country(lookup("at").unwrap(), &json!({"tax_year_start": "07-01"})).unwrap_err();
        assert_eq!(refusal.code, Some("validation"));
        assert_eq!(refusal.details["reason"], "tax_year_start_locked");
        // Australia computes in the state zone the book chose.
        let perth = engine_country(
            lookup("au").unwrap(),
            &json!({"tax_timezone": "Australia/Perth"}),
        )
        .unwrap();
        assert_eq!(
            (&perth["timezone"], &perth["tax_year_start"]),
            (&json!("Australia/Perth"), &json!("07-01"))
        );
    }

    #[test]
    fn a_transition_books_wallet_order_is_read_or_refused() {
        assert_eq!(wallet_order(&json!({})).unwrap(), None);
        let profile = json!({BOOK_WALLET_ORDER: "[\"w-2\",\"w-1\"]"});
        assert_eq!(
            wallet_order(&profile).unwrap(),
            Some(vec!["w-2".to_owned(), "w-1".to_owned()])
        );
        for bad in [json!("not json"), json!("{}"), json!("[1]")] {
            let refusal = wallet_order(&json!({BOOK_WALLET_ORDER: bad})).unwrap_err();
            assert_eq!(refusal.code, Some("app_error"));
            assert_eq!(refusal.stage, "configuration");
        }
    }

    #[test]
    fn a_method_without_a_form_in_the_scope_is_refused_before_the_engine() {
        let profile = |method: &str, scope: &str, country: &str| json!({"tax_country": country, "gains_algorithm": method, "cost_basis_pool_scope": scope});
        // A generic book has the global pool only (yet).
        assert!(pool_scope(&profile("MOVING_AVERAGE", "wallet", "generic")).is_err());
        assert_eq!(
            pool_scope(&profile("FIFO", "global_then_wallet", "us")).unwrap(),
            "global_then_wallet"
        );
        let refusal = pool_scope(&profile("FIFO", "global_then_wallet", "generic")).unwrap_err();
        assert_eq!(refusal.code, Some("validation"));
        // Germany's wallet-only scope with a method that averages globally.
        let refusal = pool_scope(&profile("JP_TOTAL_AVERAGE", "wallet", "de")).unwrap_err();
        assert_eq!(refusal.code, Some("method_scope_unsupported"));
        // An unknown method is the method gate's, as before.
        assert!(pool_scope(&profile("NOPE", "wallet", "de")).is_ok());
    }
}
