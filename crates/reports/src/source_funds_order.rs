use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

fn hash(raw: &str) -> u64 {
    if raw.is_empty() {
        return 0;
    }
    let max = raw.chars().map(|c| c as u32).max().unwrap();
    let size = if max <= 255 {
        1
    } else if max <= 65535 {
        2
    } else {
        4
    };
    let bytes: Vec<_> = raw
        .chars()
        .flat_map(|c| (c as u32).to_le_bytes()[..size].to_vec())
        .collect();
    let mut hasher = DefaultHasher::new();
    hasher.write(&bytes);
    let hash = hasher.finish();
    if hash == u64::MAX {
        u64::MAX - 1
    } else {
        hash
    }
}
fn insert(table: &mut [Option<(u64, String)>], hash: u64, key: String) -> bool {
    let mask = table.len() - 1;
    let mut i = hash as usize & mask;
    let mut perturb = hash;
    loop {
        let probes = if i + 9 <= mask { 9 } else { 0 };
        for slot in &mut table[i..=i + probes] {
            match slot {
                Some((h, k)) if *h == hash && *k == key => return false,
                None => {
                    *slot = Some((hash, key));
                    return true;
                }
                _ => {}
            }
        }
        perturb >>= 5;
        i = (i
            .wrapping_mul(5)
            .wrapping_add(1)
            .wrapping_add(perturb as usize))
            & mask;
    }
}
pub(super) fn zero_seed_set(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut table = vec![None; 8];
    let mut used = 0;
    for key in items {
        if !insert(&mut table, hash(&key), key) {
            continue;
        }
        used += 1;
        if used * 5 >= (table.len() - 1) * 3 {
            let minimum = used * if used > 50_000 { 2 } else { 4 };
            let mut size = 8;
            while size <= minimum {
                size *= 2;
            }
            let old = std::mem::replace(&mut table, vec![None; size]);
            for (hash, key) in old.into_iter().flatten() {
                insert(&mut table, hash, key);
            }
        }
    }
    table.into_iter().flatten().map(|(_, key)| key).collect()
}
