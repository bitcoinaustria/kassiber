//! Desktop descriptor preview, with no provider lookup or network capability.
use crate::{descriptors as d, material};
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value as Wire;
use serde_json::{json, Value};

pub fn preview(args: &Value) -> Result<Wire, AppError> {
    let mut config = json!({});
    let mut descriptor = d::field(args, "descriptor");
    let mut change = d::field(args, "change_descriptor");
    let wallet_material = d::field(args, "wallet_material");
    if !wallet_material.is_empty() {
        let material = material::normalize(&wallet_material, args).map_err(d::Error::into_app)?;
        if let Some(network) = material.get("network") {
            config["network"] = network.clone();
        }
        if material.get("xpub").is_some() {
            config["xpub"] = material["xpub"].clone();
            config["script_types"] = material["script_types"].clone();
        } else {
            if descriptor.is_empty() {
                descriptor = d::field(&material, "descriptor");
            }
            if change.is_empty() {
                change = d::field(&material, "change_descriptor");
            }
            for key in ["descriptor_source", "synthesize_change"] {
                if let Some(value) = material.get(key) {
                    config[key] = value.clone();
                }
            }
        }
    }
    if config.get("xpub").is_none() {
        if descriptor.is_empty() {
            return Err(
                AppError::new("validation", "Descriptor or wallet material is required").with_hint(
                    "Paste a wallet export, descriptor, or supported extended public key.",
                ),
            );
        }
        config["descriptor"] = json!(descriptor);
        if !change.is_empty() {
            config["change_descriptor"] = json!(change);
        }
    }
    let chain = d::field(args, "chain");
    config["chain"] = json!(if chain.is_empty() { "bitcoin" } else { &chain });
    let network = d::field(args, "network");
    if !network.is_empty() {
        config["network"] = json!(network);
    }
    let plan = d::load(&config)
        .map_err(|error| match error {
            d::Error::App(e) if e.code == "wallet_spending_private_material" => e,
            d::Error::App(e) => AppError::new(
                "validation",
                format!("Could not parse descriptor: {}", e.message),
            ),
            d::Error::Value(message) => AppError::new(
                "validation",
                format!("Could not parse descriptor: {message}"),
            ),
        })?
        .ok_or_else(|| {
            AppError::new(
                "validation",
                "Descriptor preview requires a parseable descriptor",
            )
        })?;
    let raw_count = if args["count"] == Value::Bool(true) {
        Some(1)
    } else {
        args["count"].as_u64().or_else(|| {
            args["count"]
                .as_number()
                .filter(|n| !n.is_f64() && !n.to_string().starts_with('-'))
                .map(|_| 20)
        })
    };
    let count = raw_count.filter(|n| *n > 0).unwrap_or(5).min(20) as u32;
    let mut addresses = vec![];
    for branch in &plan.branches {
        let end = if branch.label.ends_with("change") {
            1
        } else {
            count
        };
        for target in plan
            .derive(Some(branch.index), 0, end)
            .map_err(d::Error::into_app)?
        {
            addresses.push(dict([
                ("branch", s(branch.label.as_str())),
                (
                    "index",
                    Wire::from(target["address_index"].as_u64().expect("derived index")),
                ),
                (
                    "address",
                    s(target["address"].as_str().expect("derived address")),
                ),
                (
                    "derivation_path",
                    target["derivation_path"].as_str().map_or(Wire::Null, s),
                ),
            ]));
        }
    }
    Ok(dict([
        ("chain", s(plan.chain)),
        ("network", s(plan.network)),
        ("addresses", Wire::List(addresses)),
        (
            "has_change_branch",
            Wire::Bool(plan.branches.iter().any(|b| b.label.ends_with("change"))),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn python_preview_vectors() {
        let vectors: Value =
            serde_json::from_str(include_str!("../tests/vectors/python_descriptors.json")).unwrap();
        let mut failures = vec![];
        for case in vectors["previews"].as_array().unwrap() {
            let actual = match preview(&case["args"]) {
                Ok(value) => {
                    json!({"ok":serde_json::from_str::<Value>(&kassiber_pyjson::dumps(&value, &Default::default()).unwrap()).unwrap()})
                }
                Err(error) => d::Error::App(error).projection(),
            };
            if actual != case["expected"] {
                failures.push(case["name"].as_str().unwrap());
            }
        }
        assert!(failures.is_empty(), "Preview cases differ: {failures:?}");
    }
}
