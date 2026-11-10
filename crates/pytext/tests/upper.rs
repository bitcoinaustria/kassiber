use kassiber_pytext::upper;
use std::collections::BTreeMap;

#[test]
fn cpython_uppercase_for_every_scalar_including_future_case_pairs() {
    let changes: BTreeMap<_, _> = include_str!("data/upper.txt")
        .lines()
        .map(|line| {
            let (point, chars) = line.split_once(';').unwrap();
            (
                u32::from_str_radix(point, 16).unwrap(),
                chars
                    .split_whitespace()
                    .map(|c| char::from_u32(u32::from_str_radix(c, 16).unwrap()).unwrap())
                    .collect::<String>(),
            )
        })
        .collect();
    for point in 0..=0x10ffff {
        let Some(c) = char::from_u32(point) else {
            continue;
        };
        let text = c.to_string();
        assert_eq!(
            upper(&text),
            *changes.get(&point).unwrap_or(&text),
            "U+{point:X}"
        );
    }
}
