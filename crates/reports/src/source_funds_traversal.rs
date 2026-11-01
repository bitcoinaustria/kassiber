use super::*;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

#[derive(Debug)]
pub struct ReportTraversal {
    pub rank: HashMap<String, usize>,
    pub links: HashMap<String, Vec<Value>>,
    pub cycle_link_id: Option<String>,
    pub truncated: bool,
}

pub fn load(
    conn: &Connection,
    profile_id: &str,
    target_id: &str,
    max_depth: usize,
    node_limit: usize,
    edge_limit: usize,
) -> Result<ReportTraversal> {
    let mut ids = vec![target_id.to_owned()];
    let mut index = HashMap::from([(target_id.to_owned(), 0)]);
    let mut dependencies = vec![BTreeSet::new()];
    let mut links = Vec::new();
    let mut nodes = HashSet::from([format!("tx:{target_id}")]);
    let mut queue = VecDeque::from([(target_id.to_owned(), 0)]);
    let mut seen = HashSet::from([target_id.to_owned()]);
    let mut edge_count = 0;
    let mut truncated = false;
    while !queue.is_empty() && !truncated {
        let (tx_id, depth) = queue.pop_front().unwrap();
        if depth >= max_depth {
            continue;
        }
        let query_limit =
            i64::try_from((edge_limit - edge_count).saturating_add(1)).map_err(|_| {
                AppError::new(
                    "internal_error",
                    "Python int too large to convert to SQLite INTEGER",
                )
            })?;
        let records = rows(
            conn,
            "SELECT * FROM source_funds_links WHERE profile_id=? AND to_transaction_id=? AND state != 'rejected' ORDER BY state DESC,created_at ASC,id ASC LIMIT ?",
            kassiber_store::rusqlite::params![profile_id, tx_id, query_limit],
        )?;
        let mut retained = Vec::new();
        for row in records {
            if edge_count >= edge_limit {
                truncated = true;
                break;
            }
            let parent_id = value(&row, "from_transaction_id");
            if text(&row, "state") == "reviewed" {
                let endpoint = if truthy(parent_id) {
                    format!("tx:{}", py_str(parent_id))
                } else {
                    format!("source:{}", text(&row, "from_source_id"))
                };
                if !nodes.contains(&endpoint) && nodes.len() >= node_limit {
                    truncated = true;
                    break;
                }
                nodes.insert(endpoint);
                if truthy(parent_id) {
                    let parent_id = py_str(parent_id);
                    let parent = *index.entry(parent_id.clone()).or_insert_with(|| {
                        let ordinal = ids.len();
                        ids.push(parent_id.clone());
                        dependencies.push(BTreeSet::new());
                        ordinal
                    });
                    dependencies[parent].insert(index[&tx_id]);
                    if seen.insert(parent_id.clone()) {
                        queue.push_back((parent_id, depth + 1));
                    }
                }
            }
            retained.push(row);
            edge_count += 1;
        }
        links.push((tx_id, retained));
    }

    let mut successors = vec![Vec::new(); ids.len()];
    for (parent, children) in dependencies.iter().enumerate() {
        for &child in children {
            successors[child].push(parent);
        }
    }
    let cycle = find_cycle(&successors);
    let cycle_link_id = cycle.as_ref().map(|cycle| {
        let cycle_edges: HashSet<_> = cycle
            .windows(2)
            .map(|pair| (ids[pair[0]].as_str(), ids[pair[1]].as_str()))
            .collect();
        links
            .iter()
            .flat_map(|(_, records)| records)
            .find(|row| {
                cycle_edges.contains(&(
                    text(row, "from_transaction_id").as_str(),
                    text(row, "to_transaction_id").as_str(),
                ))
            })
            .map(|row| text(row, "id"))
            .unwrap_or_else(|| target_id.to_owned())
    });
    let mut remaining: Vec<_> = dependencies.iter().map(BTreeSet::len).collect();
    let mut ready: Vec<_> = (0..ids.len()).filter(|&i| remaining[i] == 0).collect();
    let mut order = Vec::new();
    while !ready.is_empty() {
        ready.sort_by(|&a, &b| ids[a].cmp(&ids[b]));
        let batch = std::mem::take(&mut ready);
        for &child in &batch {
            for &parent in &successors[child] {
                remaining[parent] -= 1;
                if remaining[parent] == 0 {
                    ready.push(parent);
                }
            }
        }
        order.extend(batch);
    }
    let ordered: HashSet<_> = order.iter().copied().collect();
    order.extend((0..ids.len()).filter(|i| !ordered.contains(i)));
    Ok(ReportTraversal {
        rank: order
            .into_iter()
            .enumerate()
            .map(|(rank, node)| (ids[node].clone(), rank))
            .collect(),
        links: links.into_iter().collect(),
        cycle_link_id,
        truncated,
    })
}

fn find_cycle(successors: &[Vec<usize>]) -> Option<Vec<usize>> {
    let mut seen = HashSet::new();
    for node in 0..successors.len() {
        if seen.contains(&node) {
            continue;
        }
        let mut stack = vec![(node, 0)];
        let mut positions = HashMap::from([(node, 0)]);
        seen.insert(node);
        while let Some((current, next)) = stack.last_mut() {
            if let Some(&successor) = successors[*current].get(*next) {
                *next += 1;
                if let Some(&start) = positions.get(&successor) {
                    let mut cycle: Vec<_> = stack[start..].iter().map(|&(n, _)| n).collect();
                    cycle.push(successor);
                    return Some(cycle);
                }
                if seen.insert(successor) {
                    positions.insert(successor, stack.len());
                    stack.push((successor, 0));
                }
            } else {
                let (completed, _) = stack.pop().unwrap();
                positions.remove(&completed);
            }
        }
    }
    None
}
