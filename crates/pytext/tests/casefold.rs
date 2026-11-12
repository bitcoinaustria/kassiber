use kassiber_pytext::casefold;
#[test]
fn casefold_matches_python_unicode_14() {
    for line in include_str!("data/casefold.txt").lines() {
        let (input, expected) = line.split_once(';').unwrap();
        let input = char::from_u32(u32::from_str_radix(input, 16).unwrap()).unwrap();
        let expected: String = expected
            .split(',')
            .map(|c| char::from_u32(u32::from_str_radix(c, 16).unwrap()).unwrap())
            .collect();
        assert_eq!(
            casefold(&input.to_string()),
            expected,
            "U+{:04X}",
            input as u32
        );
    }
    assert_eq!(
        casefold(" BTC ß ẞ ı İ ﬃ 𐐀 Ꭰ ꭰ ς 🚀 \u{1c8a}"),
        " btc ss ss ı i\u{307} ffi 𐐨 Ꭰ Ꭰ σ 🚀 \u{1c8a}"
    );
}
