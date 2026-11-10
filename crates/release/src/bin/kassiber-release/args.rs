//! A small `argparse`-shaped option parser: `--name value`, `--name=value`,
//! flags, and repeatable options, all in any order. Usage errors exit 2.
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Value,
    Flag,
    Append,
}

pub struct Spec {
    pub name: &'static str,
    pub kind: Kind,
    pub required: bool,
    pub choices: &'static [&'static str],
}

pub const fn value(name: &'static str) -> Spec {
    Spec {
        name,
        kind: Kind::Value,
        required: false,
        choices: &[],
    }
}

pub const fn required(name: &'static str) -> Spec {
    Spec {
        name,
        kind: Kind::Value,
        required: true,
        choices: &[],
    }
}

pub const fn flag(name: &'static str) -> Spec {
    Spec {
        name,
        kind: Kind::Flag,
        required: false,
        choices: &[],
    }
}

pub const fn append(name: &'static str) -> Spec {
    Spec {
        name,
        kind: Kind::Append,
        required: false,
        choices: &[],
    }
}

/// A usage error: printed by `main` as `kassiber-release: error: ...`, exit 2.
pub struct Usage(pub String);

#[derive(Default)]
pub struct Parsed {
    values: BTreeMap<&'static str, Vec<String>>,
}

impl Parsed {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values
            .get(name)
            .and_then(|values| values.last())
            .map(String::as_str)
    }

    pub fn req(&self, name: &str) -> &str {
        self.get(name).expect("required option checked by parse")
    }

    pub fn flag(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    pub fn all(&self, name: &str) -> Vec<String> {
        self.values.get(name).cloned().unwrap_or_default()
    }
}

pub fn parse(args: &[String], specs: &[Spec]) -> Result<Parsed, Usage> {
    let mut parsed = Parsed::default();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        index += 1;
        let Some(option) = arg.strip_prefix("--") else {
            return Err(Usage(format!("unrecognized arguments: {arg}")));
        };
        let (option, inline) = match option.split_once('=') {
            Some((name, value)) => (name, Some(value.to_owned())),
            None => (option, None),
        };
        let Some(spec) = specs.iter().find(|spec| spec.name == option) else {
            return Err(Usage(format!("unrecognized arguments: {arg}")));
        };
        if spec.kind == Kind::Flag {
            if let Some(inline) = inline {
                return Err(Usage(format!(
                    "argument --{}: ignored explicit argument '{inline}'",
                    spec.name
                )));
            }
            parsed
                .values
                .entry(spec.name)
                .or_default()
                .push(String::new());
            continue;
        }
        let value = match inline {
            Some(value) => value,
            None => {
                // argparse refuses an option-looking value ("--x --y"),
                // but accepts negative numbers and plain text.
                let next = args.get(index).filter(|next| !next.starts_with("--"));
                let Some(next) = next else {
                    return Err(Usage(format!(
                        "argument --{}: expected one argument",
                        spec.name
                    )));
                };
                index += 1;
                next.clone()
            }
        };
        if !spec.choices.is_empty() && !spec.choices.contains(&value.as_str()) {
            let choices: Vec<String> = spec.choices.iter().map(|c| format!("'{c}'")).collect();
            return Err(Usage(format!(
                "argument --{}: invalid choice: '{value}' (choose from {})",
                spec.name,
                choices.join(", ")
            )));
        }
        let values = parsed.values.entry(spec.name).or_default();
        if spec.kind == Kind::Value {
            values.clear();
        }
        values.push(value);
    }
    let missing: Vec<String> = specs
        .iter()
        .filter(|spec| spec.required && !parsed.values.contains_key(spec.name))
        .map(|spec| format!("--{}", spec.name))
        .collect();
    if !missing.is_empty() {
        return Err(Usage(format!(
            "the following arguments are required: {}",
            missing.join(", ")
        )));
    }
    Ok(parsed)
}
