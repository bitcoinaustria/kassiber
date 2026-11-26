//! The generated redaction corpus: the cases `tests/fixtures/secret_redaction_generator.json`
//! describes, built by the same LCG and shapes as the removed web app's
//! `redactionGeneration.test-support.ts` (itself a port of the Python generator), so the pinned
//! `output_sha256` still applies. Standard library and `serde_json` only: the desktop crate
//! compiles this file too.

use serde_json::Value;

pub struct Case {
    pub index: usize,
    pub input: String,
    /// Empty for the help-text shape, which plants no secret.
    pub sentinel: String,
    pub preserved: [String; 2],
}

/// `JSON.stringify` of a string.
pub fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `text.replaceAll("\\", "\\\\").replaceAll("'", "\\'")`.
fn single_quoted_body(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\'', "\\'")
}

/// `state = (Math.imul(1664525, state) + 1013904223) >>> 0`, then `(state >>> 8) % length`.
struct Lcg(u32);

impl Lcg {
    fn pick<'v>(&mut self, values: &[&'v str]) -> &'v str {
        self.0 = 1_664_525u32
            .wrapping_mul(self.0)
            .wrapping_add(1_013_904_223);
        values[(self.0 >> 8) as usize % values.len()]
    }
}

fn list<'s>(spec: &'s Value, name: &str) -> Vec<&'s str> {
    spec[name]
        .as_array()
        .unwrap_or_else(|| panic!("{name}"))
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect()
}

/// Every case of the spec, in order.
pub fn for_each_case(spec: &Value, mut visit: impl FnMut(Case)) {
    let schemes = list(spec, "schemes");
    let hosts = list(spec, "hosts");
    let ports = list(spec, "ports");
    let users = list(spec, "users");
    let userinfo_separators = list(spec, "userinfo_separators");
    let delimiters = list(spec, "delimiters");
    let keys = list(spec, "keys");
    let helps = list(spec, "helps");
    let nonsecrets = list(spec, "nonsecrets");
    let flag_prefixes = list(spec, "flag_prefixes");
    let malformed_separators = list(spec, "malformed_separators");
    let mut rng = Lcg(u32::try_from(spec["seed"].as_u64().unwrap()).unwrap());
    const ALPHABET: &[u8; 16] = b"BCDFGJKLMNQRVXYZ";
    let count = usize::try_from(spec["count"].as_u64().unwrap()).unwrap();
    for index in 0..count {
        let digits: String = format!("{index:08x}")
            .chars()
            .map(|digit| char::from(ALPHABET[digit.to_digit(16).unwrap() as usize]))
            .collect();
        let mut sentinel = format!("QZX{digits}ZVQJK");
        let (left, right) = (sentinel[..8].to_owned(), sentinel[8..].to_owned());
        let scheme = rng.pick(&schemes);
        let host = rng.pick(&hosts);
        let port = rng.pick(&ports);
        let user = rng.pick(&users);
        let separator = rng.pick(&userinfo_separators);
        let delimiter = rng.pick(&delimiters);
        let key = rng.pick(&keys);
        let help = rng.pick(&helps);
        let keep = rng.pick(&nonsecrets);
        let flag_prefix = rng.pick(&flag_prefixes);
        let malformed = rng.pick(&malformed_separators);
        let url = format!("{scheme}://{host}{port}");
        let block = index / 20;
        let secret = format!(
            "{left}@{right}{}",
            [r#"\"9"#, "'007", r"\\", "1234\n"][block % 4]
        );
        let q = quoted(&secret);
        let text = match index % 20 {
            0 => format!("{scheme}://{user}:{left}{separator}{right}@{host}{port}/{delimiter}"),
            1 => format!(
                "{{\"error\":{},\"{key}\":{q}}}",
                quoted(&format!("cannot reach ({url}{delimiter}"))
            ),
            2 => format!("{key}={q}"),
            3 => format!("{{\"{key}\":{q},\"url\":{}}}", quoted(&url)),
            4 => format!("{key}: {q}"),
            5 => format!("--{key}={q}"),
            6 => format!("cmd {flag_prefix}{key} {q}"),
            7 => format!(
                "Authorization: {} {sentinel}\n",
                ["Basic", "Bearer", "Token", "Digest", "Negotiate"][block % 5]
            ),
            shape @ (8 | 9 | 16) => {
                let mut text =
                    format!("{{\"backend\":{{\"url\":{},\"{key}\":{q}}}}}", quoted(&url));
                for _ in 0..block % 4 {
                    text = quoted(&text);
                }
                if shape == 9 {
                    text = format!("'{}'", single_quoted_body(&text));
                }
                text
            }
            10 => format!("{scheme}://u:{left}{malformed}{right}@{host}{port}/"),
            11 => format!("url={url}|{key}={q}|status=ready"),
            12 => {
                sentinel.clear();
                help.to_owned()
            }
            13 => format!(
                "cmd {flag_prefix}{key} {sentinel}{} --verbose",
                if block % 2 == 1 { "." } else { "" }
            ),
            14 => format!("{key}={left}@{right}{delimiter}"),
            15 => format!("url={url}&rpcpassword={left}@{right}"),
            17 => format!("{key}={}", quoted(&format!("{secret}\\"))),
            18 => format!("{scheme}://u:{left}@ss@{right}@{host}{port}/"),
            _ => format!("{{'{key}': '{}'}}", single_quoted_body(&secret)),
        };
        visit(Case {
            index,
            input: format!("{text}\n{keep}\n{help}"),
            sentinel,
            preserved: [keep.to_owned(), help.to_owned()],
        });
    }
}
