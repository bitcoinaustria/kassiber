//! CPython 3.11's argparse parsing algorithm over the generated tree.
//!
//! `ArgumentParser.parse_args` decides Kassiber's argv grammar: which long
//! option prefixes resolve, how `=` and `--` split arguments, which parser
//! reports an error, and in which order errors surface. This module follows
//! `_parse_known_args`, `consume_optional`, `consume_positionals`, the nargs
//! patterns, and the subparser recursion step for step, so the Rust CLI
//! accepts the same command lines and fails on the same first error with
//! the same message. Help and version actions print and stop parsing, as in
//! Python, wherever they appear.

use std::collections::{BTreeMap, HashMap, HashSet};

use kassiber_envelope::py_repr;
use kassiber_pyjson::{BigInt, PyStr, Value};
use kassiber_pytext::{decimal_value, parse_float, parse_int, repr_str};

use crate::help::{format_help, format_version};
use crate::spec::{parsers, Action, ActionKind, Lit, Nargs, Parser, ParserId, ValueType};

/// The parsed namespace: destinations in the order argparse set them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Namespace {
    values: Vec<(String, Value)>,
    /// `_UNRECOGNIZED_ARGS_ATTR`: arguments a subparser left over.
    unrecognized: Option<Vec<String>>,
}

impl Namespace {
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.values
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }

    /// The value as text, when it is a non-empty string.
    pub fn text(&self, name: &str) -> Option<&str> {
        match self.get(name) {
            Some(Value::Str(text)) => text.as_str().filter(|t| !t.is_empty()),
            _ => None,
        }
    }

    pub fn flag(&self, name: &str) -> bool {
        matches!(self.get(name), Some(Value::Bool(true)))
    }

    pub fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    pub fn set(&mut self, name: &str, value: Value) {
        match self.values.iter_mut().find(|(key, _)| key == name) {
            Some((_, slot)) => *slot = value,
            None => self.values.push((name.to_owned(), value)),
        }
    }

    pub fn items(&self) -> &[(String, Value)] {
        &self.values
    }
}

/// Why parsing stopped before producing a namespace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    /// `parser.error(message)`: argparse prints `parser`'s usage and exits 2.
    Error { parser: ParserId, message: String },
    /// `-h`/`--help` or `--version` printed this text to stdout; exit 0.
    Exit { stdout: String },
}

/// Formatting inputs that come from the process, not the grammar.
#[derive(Clone, Copy, Debug)]
pub struct Context<'a> {
    /// `HelpFormatter`'s width: the terminal width minus 2.
    pub width: usize,
    /// `version_text()`, which `--version` prints.
    pub version: &'a str,
}

/// An `ArgumentError` raised inside `_parse_known_args`; the parser
/// reports it as `argument <name>: <message>`.
enum Fail {
    Arg { action: usize, message: String },
    Stop(Stop),
}

impl From<Stop> for Fail {
    fn from(stop: Stop) -> Self {
        Fail::Stop(stop)
    }
}

/// `parser.parse_args(argv)` on the root parser.
pub fn parse_args(argv: &[String], cx: Context<'_>) -> Result<Namespace, Stop> {
    let mut namespace = Namespace::default();
    let extras = parse_known_args(0, argv.to_vec(), &mut namespace, cx)?;
    if !extras.is_empty() {
        return Err(Stop::Error {
            parser: 0,
            message: format!("unrecognized arguments: {}", extras.join(" ")),
        });
    }
    Ok(namespace)
}

/// `_get_action_name(action)`.
pub fn action_name(action: &Action) -> Option<String> {
    if !action.option_strings.is_empty() {
        Some(action.option_strings.join("/"))
    } else if let Some(metavar) = action.metavar {
        Some(metavar.to_owned())
    } else if let Some(dest) = action.dest {
        Some(dest.to_owned())
    } else {
        action
            .choices
            .map(|choices| format!("{{{}}}", choices.join(",")))
    }
}

fn lit_value(lit: Lit) -> Value {
    match lit {
        Lit::Suppress | Lit::None => Value::Null,
        Lit::Bool(b) => Value::Bool(b),
        Lit::Int(n) => Value::from(n),
        Lit::Float(f) => Value::Float(f),
        Lit::Str(s) => Value::Str(PyStr::from(s)),
        Lit::List(items) => Value::List(items.iter().copied().map(lit_value).collect()),
    }
}

/// `parser.parse_known_args(args, namespace)`.
fn parse_known_args(
    pid: ParserId,
    args: Vec<String>,
    namespace: &mut Namespace,
    cx: Context<'_>,
) -> Result<Vec<String>, Stop> {
    let parser = &parsers()[pid];
    for action in parser.actions {
        if let Some(dest) = action.dest {
            if !namespace.has(dest) && action.default != Lit::Suppress {
                namespace.set(dest, lit_value(action.default));
            }
        }
    }
    for &(dest, value) in parser.defaults {
        if !namespace.has(dest) {
            namespace.set(dest, lit_value(value));
        }
    }
    let mut run = Run {
        pid,
        parser,
        cx,
        args,
        pattern: Vec::new(),
        options: BTreeMap::new(),
        conflicts: HashMap::new(),
        seen: HashSet::new(),
        seen_non_default: HashSet::new(),
        extras: Vec::new(),
        positionals: Vec::new(),
    };
    let mut extras = match run.parse(namespace) {
        Ok(()) => run.extras,
        Err(Fail::Arg { action, message }) => {
            let message = match action_name(&parser.actions[action]) {
                Some(name) => format!("argument {name}: {message}"),
                None => message,
            };
            return Err(Stop::Error {
                parser: pid,
                message,
            });
        }
        Err(Fail::Stop(stop)) => return Err(stop),
    };
    if let Some(unrecognized) = namespace.unrecognized.take() {
        extras.extend(unrecognized);
    }
    Ok(extras)
}

/// `_parse_optional`'s 4-tuple.
#[derive(Clone, Debug)]
struct OptionTuple {
    action: Option<usize>,
    option_string: String,
    /// `None`, `""`, or `"="`.
    sep: Option<String>,
    explicit_arg: Option<String>,
}

struct Run<'a> {
    pid: ParserId,
    parser: &'static Parser,
    cx: Context<'a>,
    args: Vec<String>,
    /// `arg_strings_pattern`: `O` option, `A` argument, `-` for `--`.
    pattern: Vec<u8>,
    options: BTreeMap<usize, OptionTuple>,
    conflicts: HashMap<usize, Vec<usize>>,
    seen: HashSet<usize>,
    seen_non_default: HashSet<usize>,
    extras: Vec<String>,
    positionals: Vec<usize>,
}

impl Run<'_> {
    fn parse(&mut self, namespace: &mut Namespace) -> Result<(), Fail> {
        for group in self.parser.mutex_groups {
            for (i, &action) in group.actions.iter().enumerate() {
                let conflicts = self.conflicts.entry(action).or_default();
                conflicts.extend(&group.actions[..i]);
                conflicts.extend(&group.actions[i + 1..]);
            }
        }
        let mut after_separator = false;
        for i in 0..self.args.len() {
            if after_separator {
                self.pattern.push(b'A');
            } else if self.args[i] == "--" {
                self.pattern.push(b'-');
                after_separator = true;
            } else {
                match self.parse_optional(&self.args[i].clone())? {
                    None => self.pattern.push(b'A'),
                    Some(tuple) => {
                        self.options.insert(i, tuple);
                        self.pattern.push(b'O');
                    }
                }
            }
        }
        self.positionals = (0..self.parser.actions.len())
            .filter(|&i| self.parser.actions[i].is_positional())
            .collect();

        let mut start = 0usize;
        let max_option = self.options.keys().next_back().copied();
        while max_option.is_some_and(|max| start <= max) {
            let next_option = self
                .options
                .range(start..)
                .next()
                .map(|(&index, _)| index)
                .unwrap_or(start);
            if start != next_option {
                let end = self.consume_positionals(start, namespace)?;
                if end > start {
                    start = end;
                    continue;
                }
                start = end;
            }
            if !self.options.contains_key(&start) {
                self.extras
                    .extend(self.args[start..next_option].iter().cloned());
                start = next_option;
            }
            start = self.consume_optional(start, namespace)?;
        }
        let stop = self.consume_positionals(start, namespace)?;
        self.extras.extend(self.args[stop..].iter().cloned());

        let missing: Vec<String> = (0..self.parser.actions.len())
            .filter(|i| !self.seen.contains(i) && self.parser.actions[*i].required)
            .filter_map(|i| action_name(&self.parser.actions[i]))
            .collect();
        if !missing.is_empty() {
            return Err(Fail::Stop(Stop::Error {
                parser: self.pid,
                message: format!(
                    "the following arguments are required: {}",
                    missing.join(", ")
                ),
            }));
        }
        for group in self.parser.mutex_groups {
            if group.required
                && !group
                    .actions
                    .iter()
                    .any(|a| self.seen_non_default.contains(a))
            {
                let names: Vec<String> = group
                    .actions
                    .iter()
                    .map(|&a| &self.parser.actions[a])
                    .filter(|a| a.help != crate::spec::Help::Suppressed)
                    .filter_map(action_name)
                    .collect();
                return Err(Fail::Stop(Stop::Error {
                    parser: self.pid,
                    message: format!("one of the arguments {} is required", names.join(" ")),
                }));
            }
        }
        Ok(())
    }

    fn error(&self, message: String) -> Fail {
        Fail::Stop(Stop::Error {
            parser: self.pid,
            message,
        })
    }

    fn parse_optional(&self, arg: &str) -> Result<Option<OptionTuple>, Fail> {
        if !arg.starts_with('-') {
            return Ok(None);
        }
        if let Some(action) = self.parser.option(arg) {
            return Ok(Some(OptionTuple {
                action: Some(action),
                option_string: arg.to_owned(),
                sep: None,
                explicit_arg: None,
            }));
        }
        if arg.chars().count() == 1 {
            return Ok(None);
        }
        if let Some((option_string, explicit_arg)) = arg.split_once('=') {
            if let Some(action) = self.parser.option(option_string) {
                return Ok(Some(OptionTuple {
                    action: Some(action),
                    option_string: option_string.to_owned(),
                    sep: Some("=".to_owned()),
                    explicit_arg: Some(explicit_arg.to_owned()),
                }));
            }
        }
        let mut tuples = self.option_tuples(arg);
        if tuples.len() > 1 {
            let matches: Vec<&str> = tuples.iter().map(|t| t.option_string.as_str()).collect();
            return Err(self.error(format!(
                "ambiguous option: {arg} could match {}",
                matches.join(", ")
            )));
        }
        if let Some(tuple) = tuples.pop() {
            return Ok(Some(tuple));
        }
        // No option of this tree looks like a negative number, so a
        // negative number is always positional.
        if looks_like_negative_number(arg) {
            return Ok(None);
        }
        if arg.contains(' ') {
            return Ok(None);
        }
        Ok(Some(OptionTuple {
            action: None,
            option_string: arg.to_owned(),
            sep: None,
            explicit_arg: None,
        }))
    }

    /// `_get_option_tuples`: prefix matches (`allow_abbrev`).
    fn option_tuples(&self, arg: &str) -> Vec<OptionTuple> {
        let mut result = Vec::new();
        if arg.starts_with("--") {
            let (prefix, sep, explicit_arg) = match arg.split_once('=') {
                Some((prefix, explicit)) => (prefix, Some("="), Some(explicit)),
                None => (arg, None, None),
            };
            for &(option_string, action) in self.parser.options {
                if option_string.starts_with(prefix) {
                    result.push(OptionTuple {
                        action: Some(action),
                        option_string: option_string.to_owned(),
                        sep: sep.map(str::to_owned),
                        explicit_arg: explicit_arg.map(str::to_owned),
                    });
                }
            }
        } else {
            let split = arg
                .char_indices()
                .nth(2)
                .map_or(arg.len(), |(index, _)| index);
            let (short_prefix, short_explicit) = arg.split_at(split);
            for &(option_string, action) in self.parser.options {
                if option_string == short_prefix {
                    result.push(OptionTuple {
                        action: Some(action),
                        option_string: option_string.to_owned(),
                        sep: Some(String::new()),
                        explicit_arg: Some(short_explicit.to_owned()),
                    });
                } else if option_string.starts_with(arg) {
                    result.push(OptionTuple {
                        action: Some(action),
                        option_string: option_string.to_owned(),
                        sep: None,
                        explicit_arg: None,
                    });
                }
            }
        }
        result
    }

    fn consume_optional(&mut self, start: usize, namespace: &mut Namespace) -> Result<usize, Fail> {
        let tuple = self.options[&start].clone();
        let mut action = tuple.action;
        let mut option_string = tuple.option_string;
        let mut sep = tuple.sep;
        let mut explicit_arg = tuple.explicit_arg;
        let mut action_tuples: Vec<(usize, Vec<String>, String)> = Vec::new();
        let stop;
        loop {
            let Some(current) = action else {
                self.extras.push(self.args[start].clone());
                return Ok(start + 1);
            };
            if let Some(explicit) = explicit_arg.clone() {
                let arg_count = self.match_argument(current, b"A")?;
                let second_is_prefix = option_string.chars().nth(1) == Some('-');
                if arg_count == 0 && !second_is_prefix && !explicit.is_empty() {
                    if sep.as_deref() == Some("=") || explicit.starts_with('-') {
                        return Err(Fail::Arg {
                            action: current,
                            message: format!("ignored explicit argument {}", repr_str(&explicit)),
                        });
                    }
                    action_tuples.push((current, Vec::new(), option_string.clone()));
                    let prefix_char = option_string.chars().next().unwrap_or('-');
                    let first = explicit.chars().next().unwrap_or('-');
                    option_string = format!("{prefix_char}{first}");
                    if let Some(next) = self.parser.option(&option_string) {
                        action = Some(next);
                        let rest = &explicit[first.len_utf8()..];
                        if rest.is_empty() {
                            sep = None;
                            explicit_arg = None;
                        } else if let Some(value) = rest.strip_prefix('=') {
                            sep = Some("=".to_owned());
                            explicit_arg = Some(value.to_owned());
                        } else {
                            sep = Some(String::new());
                            explicit_arg = Some(rest.to_owned());
                        }
                    } else {
                        self.extras.push(format!("{prefix_char}{explicit}"));
                        stop = start + 1;
                        break;
                    }
                } else if arg_count == 1 {
                    stop = start + 1;
                    action_tuples.push((current, vec![explicit], option_string));
                    break;
                } else {
                    return Err(Fail::Arg {
                        action: current,
                        message: format!("ignored explicit argument {}", repr_str(&explicit)),
                    });
                }
            } else {
                let begin = start + 1;
                let selected = self.pattern[begin..].to_vec();
                let arg_count = self.match_argument(current, &selected)?;
                stop = begin + arg_count;
                action_tuples.push((current, self.args[begin..stop].to_vec(), option_string));
                break;
            }
        }
        for (action, args, option_string) in action_tuples {
            self.take_action(action, args, Some(option_string), namespace)?;
        }
        Ok(stop)
    }

    fn consume_positionals(
        &mut self,
        start: usize,
        namespace: &mut Namespace,
    ) -> Result<usize, Fail> {
        let selected = self.pattern[start..].to_vec();
        let actions: Vec<&Action> = self
            .positionals
            .iter()
            .map(|&i| &self.parser.actions[i])
            .collect();
        let counts = match_partial(&actions, &selected);
        let mut index = start;
        let taken: Vec<usize> = self.positionals[..counts.len()].to_vec();
        for (action, count) in taken.iter().zip(&counts) {
            let args = self.args[index..index + count].to_vec();
            index += count;
            self.take_action(*action, args, None, namespace)?;
        }
        self.positionals.drain(..counts.len());
        Ok(index)
    }

    fn match_argument(&self, action: usize, pattern: &[u8]) -> Result<usize, Fail> {
        let current = &self.parser.actions[action];
        match match_counts(&[nargs_pattern(current)], pattern) {
            Some(counts) => Ok(counts[0]),
            None => Err(Fail::Arg {
                action,
                message: match current.nargs {
                    Nargs::Single => "expected one argument".to_owned(),
                    Nargs::Optional => "expected at most one argument".to_owned(),
                    Nargs::OneOrMore => "expected at least one argument".to_owned(),
                    Nargs::Zero => "expected 0 arguments".to_owned(),
                    Nargs::ZeroOrMore | Nargs::Parser => "expected arguments".to_owned(),
                },
            }),
        }
    }

    fn take_action(
        &mut self,
        index: usize,
        args: Vec<String>,
        option_string: Option<String>,
        namespace: &mut Namespace,
    ) -> Result<(), Fail> {
        let action = &self.parser.actions[index];
        self.seen.insert(index);
        let (values, is_default) = self.values(index, args)?;
        if !is_default {
            self.seen_non_default.insert(index);
            for conflict in self.conflicts.get(&index).cloned().unwrap_or_default() {
                if self.seen_non_default.contains(&conflict) {
                    let name = action_name(&self.parser.actions[conflict]).unwrap_or_default();
                    return Err(Fail::Arg {
                        action: index,
                        message: format!("not allowed with argument {name}"),
                    });
                }
            }
        }
        let dest = action.dest.unwrap_or("");
        match action.kind {
            ActionKind::Store => namespace.set(dest, values),
            ActionKind::StoreConst | ActionKind::StoreTrue | ActionKind::StoreFalse => {
                namespace.set(dest, lit_value(action.constant));
            }
            ActionKind::Append => {
                let mut items = match namespace.get(dest) {
                    Some(Value::List(items)) => items.clone(),
                    Some(Value::Null) | None => Vec::new(),
                    Some(other) => vec![other.clone()],
                };
                items.push(values);
                namespace.set(dest, Value::List(items));
            }
            ActionKind::BooleanOptional => {
                if let Some(option_string) = option_string {
                    if action.option_strings.contains(&option_string.as_str()) {
                        namespace.set(dest, Value::Bool(!option_string.starts_with("--no-")));
                    }
                }
            }
            ActionKind::Help => {
                return Err(Fail::Stop(Stop::Exit {
                    stdout: format_help(self.pid, self.cx.width),
                }));
            }
            ActionKind::Version => {
                return Err(Fail::Stop(Stop::Exit {
                    stdout: format_version(self.cx.version, self.cx.width),
                }));
            }
            ActionKind::SubParsers => self.call_subparser(index, values, namespace)?,
        }
        Ok(())
    }

    fn call_subparser(
        &mut self,
        index: usize,
        values: Value,
        namespace: &mut Namespace,
    ) -> Result<(), Fail> {
        let action = &self.parser.actions[index];
        let Value::List(values) = values else {
            return Ok(());
        };
        let strings: Vec<String> = values
            .iter()
            .map(|v| match v {
                Value::Str(s) => s.as_str().unwrap_or_default().to_owned(),
                _ => String::new(),
            })
            .collect();
        let Some((name, rest)) = strings.split_first() else {
            return Ok(());
        };
        if let Some(dest) = action.dest {
            namespace.set(dest, Value::Str(PyStr::from(name.as_str())));
        }
        let Some(child) = action
            .subparsers
            .and_then(|sub| sub.names.iter().find(|(n, _)| n == name))
            .map(|&(_, child)| usize::from(child))
        else {
            // _check_value already refused an unknown name.
            return Ok(());
        };
        let mut child_namespace = Namespace::default();
        let leftover = parse_known_args(child, rest.to_vec(), &mut child_namespace, self.cx)?;
        for (key, value) in child_namespace.values {
            namespace.set(&key, value);
        }
        if !leftover.is_empty() {
            namespace
                .unrecognized
                .get_or_insert_with(Vec::new)
                .extend(leftover);
        }
        Ok(())
    }

    /// `_get_values`, and whether the result `is action.default`.
    fn values(&self, index: usize, mut args: Vec<String>) -> Result<(Value, bool), Fail> {
        let action = &self.parser.actions[index];
        if action.is_positional() && action.nargs != Nargs::Parser {
            if let Some(at) = args.iter().position(|a| a == "--") {
                args.remove(at);
            }
        }
        if args.is_empty() && action.nargs == Nargs::Optional {
            let (lit, is_default) = if action.is_positional() {
                (action.default, true)
            } else {
                (
                    action.constant,
                    lit_identical(action.constant, action.default),
                )
            };
            let mut value = lit_value(lit);
            if let Lit::Str(text) = lit {
                value = self.value(index, text)?;
                self.check_value(index, &value)?;
            }
            return Ok((value, is_default));
        }
        if args.is_empty() && action.nargs == Nargs::ZeroOrMore && action.is_positional() {
            let (value, is_default) = if action.default != Lit::None {
                (lit_value(action.default), true)
            } else {
                (Value::List(Vec::new()), false)
            };
            self.check_value(index, &value)?;
            return Ok((value, is_default));
        }
        if args.len() == 1 && matches!(action.nargs, Nargs::Single | Nargs::Optional) {
            let value = self.value(index, &args[0])?;
            self.check_value(index, &value)?;
            let is_default = is_cached_small_int(&value, action.default);
            return Ok((value, is_default));
        }
        if action.nargs == Nargs::Parser {
            let values: Vec<Value> = args
                .iter()
                .map(|a| self.value(index, a))
                .collect::<Result<_, _>>()?;
            if let Some(first) = values.first() {
                self.check_value(index, first)?;
            }
            return Ok((Value::List(values), false));
        }
        let values: Vec<Value> = args
            .iter()
            .map(|a| self.value(index, a))
            .collect::<Result<_, _>>()?;
        for value in &values {
            self.check_value(index, value)?;
        }
        Ok((Value::List(values), false))
    }

    /// `_get_value`: the `type=` conversion.
    fn value(&self, index: usize, text: &str) -> Result<Value, Fail> {
        let action = &self.parser.actions[index];
        let invalid = |name: &str| Fail::Arg {
            action: index,
            message: format!("invalid {name} value: {}", repr_str(text)),
        };
        match action.value_type {
            ValueType::Str => Ok(Value::Str(PyStr::from(text))),
            ValueType::Int => parse_int(text).map(Value::Int).map_err(|_| invalid("int")),
            ValueType::Float => parse_float(text)
                .map(Value::Float)
                .map_err(|_| invalid("float")),
            ValueType::LightningWindowDays => {
                let Ok(days) = parse_int(text) else {
                    return Err(Fail::Arg {
                        action: index,
                        message: format!(
                            "--window-days must be an integer (got {})",
                            repr_str(text)
                        ),
                    });
                };
                if days < BigInt::from(1) || days > BigInt::from(365) {
                    return Err(Fail::Arg {
                        action: index,
                        message: format!("--window-days must be between 1 and 365 (got {days})"),
                    });
                }
                Ok(Value::Int(days))
            }
        }
    }

    /// `_check_value`: membership in `choices`.
    fn check_value(&self, index: usize, value: &Value) -> Result<(), Fail> {
        let action = &self.parser.actions[index];
        let choices: Vec<&str> = match (action.choices, action.subparsers) {
            (Some(choices), _) => choices.to_vec(),
            (None, Some(sub)) => sub.names.iter().map(|(name, _)| *name).collect(),
            (None, None) => return Ok(()),
        };
        let member = match value {
            Value::Str(text) => text.as_str().is_some_and(|text| {
                choices.contains(&text)
                    // A method only a Rust registry country offers (Japan's
                    // averages) is accepted beside Python's listed choices;
                    // the profile then checks it against the book's country
                    // (`docs/reference/rust-shell.md#tax-countries`).
                    || (action.dest == Some("gains_algorithm")
                        && kassiber_tax_countries::COUNTRIES
                            .iter()
                            .any(|country| country.allows_method(text)))
            }),
            _ => false,
        };
        if member {
            return Ok(());
        }
        let listed: Vec<String> = choices.iter().map(|c| repr_str(c)).collect();
        Err(Fail::Arg {
            action: index,
            message: format!(
                "invalid choice: {} (choose from {})",
                py_repr(value),
                listed.join(", ")
            ),
        })
    }
}

/// Whether two literals are one Python object: the singletons, cached
/// small ints, and (as compiled constants of one function) equal strings.
fn lit_identical(a: Lit, b: Lit) -> bool {
    match (a, b) {
        (Lit::None, Lit::None) => true,
        (Lit::Bool(x), Lit::Bool(y)) => x == y,
        (Lit::Int(x), Lit::Int(y)) => x == y && (-5..=256).contains(&x),
        (Lit::Str(x), Lit::Str(y)) => x == y,
        _ => false,
    }
}

/// `int(text) is default`: CPython caches the ints from -5 to 256.
fn is_cached_small_int(value: &Value, default: Lit) -> bool {
    match (value, default) {
        (Value::Int(n), Lit::Int(d)) => *n == BigInt::from(d) && (-5..=256).contains(&d),
        _ => false,
    }
}

/// `_negative_number_matcher`: `^-\d+$|^-\d*\.\d+$`, where `\d` is any
/// Unicode decimal digit and `$` also matches before a final newline.
fn looks_like_negative_number(arg: &str) -> bool {
    let body = arg.strip_suffix('\n').unwrap_or(arg);
    let Some(digits) = body.strip_prefix('-') else {
        return false;
    };
    let is_digit = |c: char| decimal_value(u32::from(c)).is_some();
    if !digits.is_empty() && digits.chars().all(is_digit) {
        return true;
    }
    match digits.split_once('.') {
        Some((whole, fraction)) => {
            whole.chars().all(is_digit) && !fraction.is_empty() && fraction.chars().all(is_digit)
        }
        None => false,
    }
}

/// One element of an nargs pattern.
#[derive(Clone, Copy, Debug)]
enum Token {
    /// `X`: exactly this character.
    One(u8),
    /// `X?`: greedy.
    Maybe(u8),
    /// `[...]*`: greedy.
    Many(&'static [u8]),
}

/// `_get_nargs_pattern(action)` as tokens of one capture group. Options
/// may not take `--` (`-`) as arguments, so their patterns drop it.
fn nargs_pattern(action: &Action) -> Vec<Token> {
    use Token::*;
    let positional = action.is_positional();
    let dashes = Many(b"-");
    let mut tokens = match action.nargs {
        Nargs::Single => vec![dashes, One(b'A'), dashes],
        Nargs::Optional => vec![dashes, Maybe(b'A'), dashes],
        Nargs::ZeroOrMore => vec![dashes, Many(b"A-")],
        Nargs::OneOrMore => vec![dashes, One(b'A'), Many(b"A-")],
        Nargs::Parser => vec![dashes, One(b'A'), Many(b"-AO")],
        Nargs::Zero => vec![dashes, dashes],
    };
    if !positional {
        tokens = tokens
            .into_iter()
            .filter_map(|token| match token {
                Many(b"-") => None,
                Many(b"A-") => Some(Many(b"A")),
                Many(b"-AO") => Some(Many(b"AO")),
                other => Some(other),
            })
            .collect();
    }
    tokens
}

/// `re.match` of concatenated groups at the start of `text`: the group
/// lengths of the first match a backtracking engine finds, or `None`.
fn match_counts(groups: &[Vec<Token>], text: &[u8]) -> Option<Vec<usize>> {
    fn walk(
        groups: &[Vec<Token>],
        group: usize,
        token: usize,
        pos: usize,
        text: &[u8],
        lengths: &mut Vec<usize>,
        group_start: usize,
    ) -> bool {
        if group == groups.len() {
            return true;
        }
        if token == groups[group].len() {
            lengths.push(pos - group_start);
            if walk(groups, group + 1, 0, pos, text, lengths, pos) {
                return true;
            }
            lengths.pop();
            return false;
        }
        let at = |p: usize| text.get(p).copied();
        match groups[group][token] {
            Token::One(c) => {
                at(pos) == Some(c)
                    && walk(
                        groups,
                        group,
                        token + 1,
                        pos + 1,
                        text,
                        lengths,
                        group_start,
                    )
            }
            Token::Maybe(c) => {
                (at(pos) == Some(c)
                    && walk(
                        groups,
                        group,
                        token + 1,
                        pos + 1,
                        text,
                        lengths,
                        group_start,
                    ))
                    || walk(groups, group, token + 1, pos, text, lengths, group_start)
            }
            Token::Many(set) => {
                let mut end = pos;
                while at(end).is_some_and(|c| set.contains(&c)) {
                    end += 1;
                }
                (pos..=end)
                    .rev()
                    .any(|stop| walk(groups, group, token + 1, stop, text, lengths, group_start))
            }
        }
    }
    let mut lengths = Vec::new();
    walk(groups, 0, 0, 0, text, &mut lengths, 0).then_some(lengths)
}

/// `_match_arguments_partial`: the longest prefix of `actions` whose
/// patterns match, with each action's argument count.
fn match_partial(actions: &[&Action], text: &[u8]) -> Vec<usize> {
    for count in (1..=actions.len()).rev() {
        let groups: Vec<Vec<Token>> = actions[..count].iter().map(|a| nargs_pattern(a)).collect();
        if let Some(lengths) = match_counts(&groups, text) {
            return lengths;
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nargs_patterns_backtrack_like_python() {
        let single = vec![Token::Many(b"-"), Token::One(b'A'), Token::Many(b"-")];
        let star = vec![Token::Many(b"-"), Token::Many(b"A-")];
        assert_eq!(
            match_counts(std::slice::from_ref(&single), b"AA"),
            Some(vec![1])
        );
        assert_eq!(
            match_counts(&[single.clone(), single.clone()], b"A-A"),
            Some(vec![2, 1])
        );
        assert_eq!(
            match_counts(&[star.clone(), single.clone()], b"AAA"),
            Some(vec![2, 1])
        );
        assert_eq!(match_counts(&[single], b"O"), None);
        assert_eq!(match_counts(&[star], b"O"), Some(vec![0]));
    }

    #[test]
    fn negative_numbers_are_positional() {
        assert!(looks_like_negative_number("-5"));
        assert!(looks_like_negative_number("-.5"));
        assert!(looks_like_negative_number("-1.5\n"));
        assert!(looks_like_negative_number("-\u{663}"));
        assert!(!looks_like_negative_number("-5."));
        assert!(!looks_like_negative_number("--5"));
        assert!(!looks_like_negative_number("-x"));
    }
}
