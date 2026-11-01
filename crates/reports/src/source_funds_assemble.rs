use super::*;
use std::collections::BTreeMap;

pub fn handles(command: &str) -> bool {
    matches!(
        command,
        "source-funds.assemble" | "ui.source_funds.assemble"
    )
}

pub fn execute(
    conn: &Connection,
    command: &str,
    raw: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    let desktop = command.starts_with("ui.");
    let empty = dict::<&str>([]);
    let raw = if desktop && matches!(raw, Value::Null) {
        &empty
    } else {
        raw
    };
    if !matches!(raw, Value::Dict(_)) {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(kassiber_envelope::type_name(raw)))])));
    }
    let mut args = if desktop { empty.clone() } else { raw.clone() };
    if desktop {
        let target = value(raw, "target_transaction");
        let Value::Str(target) = target else {
            return Err(AppError::new(
                "validation",
                "ui.source_funds.assemble requires args.target_transaction",
            ));
        };
        let target = kassiber_pytext::strip_py(target);
        if target.code_points().next().is_none() {
            return Err(AppError::new(
                "validation",
                "ui.source_funds.assemble requires args.target_transaction",
            ));
        }
        replace(&mut args, "target_transaction", Value::Str(target));
        replace(
            &mut args,
            "include_broad_hints",
            Value::Bool(truthy(value(raw, "include_broad_hints"))),
        );
    }
    replace(&mut args, "max_suggestions", Value::Int(500.into()));
    let max = get(raw, "max_passes")
        .filter(|v| !desktop || truthy(v))
        .cloned()
        .unwrap_or(Value::Int(8.into()));
    let max = source_funds_assembly::integer(&max)?;
    if max <= 0.into() {
        return Err(AppError::new("validation", "--max-passes must be positive"));
    }
    let max = usize::try_from(max.min(16.into())).expect("bounded passes");
    let (_, profile) = kassiber_setup::references::scope(conn, &args)?;
    let profile_id = text(&profile, "id");
    let inputs = source_funds_suggest::load_inputs(conn, &profile)?;
    let mut passes = 0;
    let mut inserted = 0;
    let mut reviewed = 0;
    let mut methods = BTreeMap::<String, usize>::new();
    let mut result = Value::Null;
    for _ in 0..max {
        passes += 1;
        let suggested = source_funds_suggest::committed(conn, || {
            source_funds_suggest::run(conn, &args, sources, Some(&inputs))
        })?;
        result = source_funds_batch::execute_with_inputs(
            conn,
            "source-funds.links.bulk-review",
            &args,
            sources,
            Some(&inputs),
        )?;
        inserted += int(&suggested, "inserted");
        reviewed += int(&result, "reviewed");
        if let Value::List(links) = value(&result, "links") {
            for link in links {
                *methods.entry(text(link, "method")).or_default() += 1;
            }
        }
        if int(&suggested, "inserted") == 0 && int(&result, "reviewed") == 0 {
            break;
        }
    }
    let scope =
        source_funds_suggest::scope(conn, &profile_id, &text(&result, "target_transaction_id"))?;
    let mut missing = std::collections::BTreeSet::new();
    for row in &inputs.report.active {
        let raw = source_funds_assembly::mapping(value(row, "raw_json"));
        let authoritative =
            kassiber_custody::evidence::authority(&kassiber_custody::codec::to_json(row))
                .is_ok_and(|v| v["authoritative"] == true);
        if text(row, "direction") == "outbound"
            && authoritative
            && get(&raw, "observer_owned_scripts").is_none()
            && matches!(value(&raw,"vin"),Value::List(v) if !v.is_empty())
            && scope.contains(&text(row, "id"))
        {
            missing.insert(text(row, "id"));
        }
    }
    Ok(dict([
        ("target_transaction_id",value(&result,"target_transaction_id").clone()),
        ("passes",Value::Int(passes.into())),("inserted",Value::Int(inserted.into())),
        ("auto_reviewed",Value::Int(reviewed.into())),("awaiting_manual_review",value(&result,"skipped").clone()),
        ("methods",dict(methods.into_iter().map(|(k,v)|(k,Value::Int(v.into()))))),
        ("chain_attestation_missing",Value::List(missing.into_iter().map(s).collect())),
        ("policy",s("Assembly derives exact edges from unambiguous synced transaction inputs/outputs and source-qualified Lightning payment hashes, plus canonical transaction scope, reviewed-pair matches, and effective custody-component allocations. Unreviewed N:M pro-rata allocations, provider ids, weak hints, chain observations, privacy boundaries, and root-source evidence remain manual review items.")),
    ]))
}
