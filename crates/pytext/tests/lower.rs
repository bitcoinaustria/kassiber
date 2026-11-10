use kassiber_pytext::lower;

#[test]
fn cpython_lowercase_and_contextual_sigma_vectors() {
    let decode = |text: &str| {
        text.split_whitespace()
            .map(|point| char::from_u32(u32::from_str_radix(point, 16).unwrap()).unwrap())
            .collect::<String>()
    };
    for line in include_str!("data/lower.txt").lines() {
        let (input, expected) = line.split_once(';').unwrap();
        let input = decode(input);
        assert_eq!(lower(&input), decode(expected), "{input:?}");
    }
}
