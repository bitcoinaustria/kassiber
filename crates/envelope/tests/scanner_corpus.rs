//! The secret floor against Python's output (`scripts/gen_scanner_vectors.py`).
//!
//! `fixture_corpora_match_python` checks every case of the shared fixture
//! corpora. `generator_corpora_match_python` checks the six reviewer
//! generators' cases, written outside the tree because they are large, and
//! hashes the output as the Python tests do so pinned seeds reproduce the
//! pinned digests.

use kassiber_envelope::get;
use kassiber_envelope::redaction::{redact_secret_py, redact_secret_text, redact_secret_value};
use kassiber_pyjson::{Key, PyStr, Value};
use kassiber_pytext::loads;

fn py_text(value: &Value) -> PyStr {
    match value {
        Value::Str(s) => s.clone(),
        other => panic!("expected a string, got {other:?}"),
    }
}

#[test]
fn fixture_corpora_match_python() {
    let mut counts = std::collections::BTreeMap::new();
    let mut mismatches = Vec::new();
    for (number, line) in include_str!("data/secret_floor.txt").lines().enumerate() {
        let case = loads(line).unwrap();
        let set = py_text(get(&case, "set").unwrap())
            .as_str()
            .unwrap()
            .to_owned();
        let input = py_text(get(&case, "in").unwrap());
        let expected = py_text(get(&case, "out").unwrap());
        *counts.entry(set.clone()).or_insert(0usize) += 1;
        let output = redact_secret_py(&input);
        if output != expected {
            mismatches.push(format!(
                "line {} ({set}): {input:?}\n  python {expected:?}\n  rust   {output:?}",
                number + 1
            ));
        }
        let wrapped = Value::Dict(vec![(Key::from("message"), Value::Str(input.clone()))]);
        let expected_value = Value::Dict(vec![(Key::from("message"), Value::Str(expected))]);
        assert_eq!(
            redact_secret_value(&wrapped),
            expected_value,
            "line {}",
            number + 1
        );
    }
    assert!(
        mismatches.is_empty(),
        "{} mismatches:\n{}",
        mismatches.len(),
        mismatches[..mismatches.len().min(20)].join("\n")
    );
    let expected: std::collections::BTreeMap<String, usize> = [
        ("cases", 444),
        ("5e_cases", 215),
        ("5f_cases", 135),
        ("5g_cases", 76),
        ("5h_cases", 84),
        ("5i_cases", 133),
    ]
    .into_iter()
    .map(|(set, count)| (set.to_owned(), count))
    .collect();
    assert_eq!(counts, expected);
}

#[test]
fn oversized_records_are_masked_whole() {
    let limit = kassiber_envelope::redaction::scanner::REDACTION_LIMIT;
    assert_eq!(redact_secret_text(&"x".repeat(limit)), "x".repeat(limit));
    let long = format!("password=Pw9Leak {}", "x".repeat(limit));
    assert_eq!(redact_secret_text(&long), "[redacted-oversized]");
}

#[test]
#[ignore = "needs KASSIBER_REDACTION_CORPUS from scripts/gen_scanner_vectors.py generators"]
fn generator_corpora_match_python() {
    let directory = std::path::PathBuf::from(
        std::env::var("KASSIBER_REDACTION_CORPUS")
            .expect("KASSIBER_REDACTION_CORPUS names the corpus directory"),
    );
    let manifest =
        loads(&std::fs::read_to_string(directory.join("manifest.json")).unwrap()).unwrap();
    let Value::Dict(files) = manifest else {
        panic!("manifest is not an object");
    };
    assert_eq!(
        files.len(),
        6 * 3 + 9,
        "every generator's pinned, extra and added seeds"
    );
    let mut report = Vec::new();
    for (name, meta) in &files {
        let Key::Str(name) = name else {
            panic!("file names are strings");
        };
        let name = name.as_str().unwrap().to_owned();
        let pinned = match get(meta, "pinned_sha256") {
            Some(Value::Str(digest)) => Some(digest.as_str().unwrap().to_owned()),
            _ => None,
        };
        let python = py_text(get(meta, "python_sha256").unwrap())
            .as_str()
            .unwrap()
            .to_owned();
        let text = std::fs::read_to_string(directory.join(&name)).unwrap();
        let mut digest = Sha256::new();
        let mut cases = 0usize;
        let mut mismatches = Vec::new();
        for line in text.lines() {
            let case = loads(line).unwrap();
            let input = py_text(get(&case, "in").unwrap());
            let expected = py_text(get(&case, "out").unwrap());
            let output = redact_secret_py(&input);
            if output != expected && mismatches.len() < 10 {
                mismatches.push(format!(
                    "{input:?}\n  python {expected:?}\n  rust   {output:?}"
                ));
            }
            digest.update(
                output
                    .as_str()
                    .expect("outputs hold no surrogates")
                    .as_bytes(),
            );
            digest.update(b"\n");
            cases += 1;
        }
        let rust = digest.hex();
        assert!(
            mismatches.is_empty(),
            "{name}: mismatches\n{}",
            mismatches.join("\n")
        );
        assert_eq!(rust, python, "{name}: digest differs from Python's");
        if let Some(pinned) = &pinned {
            assert_eq!(&rust, pinned, "{name}: digest differs from the pinned one");
        }
        report.push(format!(
            "{name}: {cases} cases identical{}",
            if pinned.is_some() {
                ", pinned digest"
            } else {
                ""
            }
        ));
    }
    println!("{}", report.join("\n"));
}

/// SHA-256 (FIPS 180-4), enough to compare output digests with Python's.
struct Sha256 {
    state: [u32; 8],
    buffer: Vec<u8>,
    length: u64,
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buffer: Vec::with_capacity(64),
            length: 0,
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.length += data.len() as u64;
        while !data.is_empty() {
            let take = (64 - self.buffer.len()).min(data.len());
            self.buffer.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.buffer.len() == 64 {
                let block: [u8; 64] = self.buffer[..].try_into().unwrap();
                self.compress(&block);
                self.buffer.clear();
            }
        }
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes(word.try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(choice)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);
            (h, g, f, e, d, c, b, a) = (g, f, e, d.wrapping_add(t1), c, b, a, t1.wrapping_add(t2));
        }
        for (state, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *state = state.wrapping_add(value);
        }
    }

    fn hex(mut self) -> String {
        let bits = self.length * 8;
        let mut padding = vec![0x80u8];
        while (self.buffer.len() + padding.len()) % 64 != 56 {
            padding.push(0);
        }
        padding.extend_from_slice(&bits.to_be_bytes());
        let length = self.length;
        self.update(&padding);
        self.length = length;
        self.state
            .iter()
            .map(|word| format!("{word:08x}"))
            .collect()
    }
}

#[test]
fn sha256_matches_known_digests() {
    let digest = |data: &[u8]| {
        let mut sha = Sha256::new();
        sha.update(data);
        sha.hex()
    };
    assert_eq!(
        digest(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        digest(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        digest(&[b'a'; 1000]),
        "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
    );
}
