//! CPython 3.11's built-in MIME table, followed by its local Unix mime.types
//! files. This is filename inference only; no file contents are indexed.
use std::{collections::BTreeMap, fs, sync::OnceLock};

pub fn guess(name: &str) -> String {
    static TYPES: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    let types = TYPES.get_or_init(|| {
        let value =
            kassiber_pytext::loads(include_str!("mime-types.json")).expect("CPython MIME fixture");
        let mut types = BTreeMap::new();
        if let kassiber_pyjson::Value::Dict(pairs) = value {
            for (key, value) in pairs {
                if let (kassiber_pyjson::Key::Str(key), kassiber_pyjson::Value::Str(value)) =
                    (key, value)
                {
                    if let (Some(key), Some(value)) = (key.as_str(), value.as_str()) {
                        types.insert(key.to_owned(), value.to_owned());
                    }
                }
            }
        }
        for path in [
            "/etc/mime.types",
            "/etc/httpd/mime.types",
            "/etc/httpd/conf/mime.types",
            "/etc/apache/mime.types",
            "/etc/apache2/mime.types",
            "/usr/local/etc/httpd/conf/mime.types",
            "/usr/local/lib/netscape/mime.types",
            "/usr/local/etc/httpd/conf/mime.types",
            "/usr/local/etc/mime.types",
        ] {
            if let Ok(content) = fs::read_to_string(path) {
                for line in content.lines() {
                    let mut words = line
                        .split('#')
                        .next()
                        .unwrap_or_default()
                        .split_whitespace();
                    if let Some(kind) = words.next() {
                        for ext in words {
                            types.insert(format!(".{ext}"), kind.into());
                        }
                    }
                }
            }
        }
        types
    });
    let mut name = name.to_owned();
    for (suffix, replacement) in [
        (".svgz", ".svg.gz"),
        (".tgz", ".tar.gz"),
        (".taz", ".tar.gz"),
        (".tz", ".tar.gz"),
        (".tbz2", ".tar.bz2"),
        (".txz", ".tar.xz"),
    ] {
        if name.to_ascii_lowercase().ends_with(suffix) {
            name.truncate(name.len() - suffix.len());
            name.push_str(replacement);
            break;
        }
    }
    for suffix in [".gz", ".Z", ".bz2", ".xz", ".br"] {
        if name.ends_with(suffix) {
            name.truncate(name.len() - suffix.len());
            break;
        }
    }
    name.rsplit_once('.')
        .and_then(|(_, ext)| {
            types
                .get(&format!(".{ext}"))
                .or_else(|| types.get(&format!(".{}", ext.to_ascii_lowercase())))
        })
        .cloned()
        .unwrap_or_else(|| "application/octet-stream".into())
}
