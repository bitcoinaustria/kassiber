//! Ordered exact allocation for native-route reconciliation. Callers construct
//! unique string ids and nonnegative integer capacities, rather than admitting
//! arbitrary Python buckets at this internal boundary.
use kassiber_pyjson::BigInt;

pub(super) struct Cell {
    pub source: String,
    pub sink: String,
    pub amount: BigInt,
}
pub(super) struct Allocation {
    pub cells: Vec<Cell>,
    pub remaining: Vec<(String, BigInt)>,
}

pub(super) fn allocate(
    sources: &[(String, BigInt)],
    sinks: &[(String, BigInt)],
) -> Result<Allocation, String> {
    let requested: BigInt = sinks.iter().map(|(_, n)| n).sum();
    let capacity: BigInt = sources.iter().map(|(_, n)| n).sum();
    if requested > capacity {
        return Err("custody_allocation_insufficient_capacity".into());
    }
    let mut source_used = vec![BigInt::from(0); sources.len()];
    let mut sink_used = vec![BigInt::from(0); sinks.len()];
    let (mut source, mut sink) = (0, 0);
    let mut remaining = requested;
    let mut cells = Vec::new();
    while remaining > BigInt::from(0) {
        while source_used[source] == sources[source].1 {
            source += 1;
        }
        while sink_used[sink] == sinks[sink].1 {
            sink += 1;
        }
        let amount = remaining
            .clone()
            .min(&sources[source].1 - &source_used[source])
            .min(&sinks[sink].1 - &sink_used[sink]);
        cells.push(Cell {
            source: sources[source].0.clone(),
            sink: sinks[sink].0.clone(),
            amount: amount.clone(),
        });
        source_used[source] += &amount;
        sink_used[sink] += &amount;
        remaining -= amount;
    }
    Ok(Allocation {
        cells,
        remaining: sources
            .iter()
            .zip(source_used)
            .map(|((id, n), used)| (id.clone(), n - used))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    #[test]
    fn reconciliation_fifo_vectors() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_reconciliation_helpers.json"
        ))
        .unwrap();
        for v in vectors.iter().filter(|v| v["op"] == "fifo") {
            let buckets = |key: &str| {
                super::super::array(&v["value"][key])
                    .iter()
                    .map(|b| {
                        (
                            crate::codec::pystr(&b[0]),
                            super::super::reviewed::amount(&b[1]).unwrap(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let actual = allocate(&buckets("sources"), &buckets("sinks"));
            if let Some(error) = v.get("error") {
                assert_eq!(
                    actual.err(),
                    error.as_str().map(str::to_owned),
                    "{}",
                    v["case"]
                );
            } else {
                let actual = actual.unwrap();
                assert_eq!(
                    json!({"cells":actual.cells.into_iter().map(|c|json!({"source":c.source,"sink":c.sink,"amount":super::super::reviewed::number(&c.amount)})).collect::<Vec<_>>(),"remaining":actual.remaining.iter().map(|(id,n)|json!([id,super::super::reviewed::number(n)])).collect::<Vec<_>>()}),
                    v["expected"],
                    "{}",
                    v["case"]
                );
            }
        }
    }
}
