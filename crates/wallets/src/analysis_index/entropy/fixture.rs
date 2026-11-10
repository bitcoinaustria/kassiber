use super::*;
use crate::analysis_index::AnalysisIndex;

pub fn evaluate(case: &Value, snapshots: &Value) -> Value {
    let snapshot = field(snapshots, field(case, "snapshot").as_str().unwrap());
    let index = AnalysisIndex::from_snapshot(snapshot.clone()).unwrap();
    let facts = get(case, "facts").unwrap_or_else(|| {
        index
            .lookup(
                "transaction_facts",
                field(case, "transaction").as_str().unwrap(),
            )
            .unwrap()
    });
    let mut progress = vec![];
    let mut report = |v| progress.push(v);
    let mut calls = 0;
    let cancel_after = get(case, "cancel_after")
        .and_then(integer)
        .map(|n| n.to_string().parse::<usize>().unwrap());
    let mut cancelled = || {
        calls += 1;
        cancel_after.is_some_and(|n| calls >= n)
    };
    let clock = list(field(case, "elapsed_ms"));
    let mut clock_calls = 0;
    let mut elapsed = || {
        let v = clock.get(clock_calls).or_else(|| clock.last());
        clock_calls += 1;
        v.and_then(integer)
            .map_or(0.0, |n| n.to_string().parse().unwrap())
    };
    let result = analyze_transaction_entropy(
        facts,
        get(case, "chain")
            .and_then(Text::as_str)
            .unwrap_or("bitcoin"),
        get(case, "max_states").unwrap_or(&num(200_000)),
        get(case, "max_duration_ms").unwrap_or(&num(1000)),
        field(case, "scenario"),
        Hooks {
            progress: Some(&mut report),
            cancelled: Some(&mut cancelled),
            elapsed_ms: Some(&mut elapsed),
            max_memo_cells: get(case, "max_memo_cells")
                .and_then(integer)
                .map(|n| n.to_string().parse().unwrap()),
        },
    );
    dict([("result", result), ("progress", Value::List(progress))])
}
