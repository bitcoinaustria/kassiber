//! argparse's `HelpFormatter` (usage and help text) and `textwrap`.
//!
//! Usage text is part of the machine contract: `--machine` usage errors
//! carry `parser.format_usage().strip()` in `details.usage`. Help text is
//! for people, but follows the same code so `-h` reads as it does today.

use std::sync::OnceLock;

use fancy_regex::Regex;
use kassiber_pytext::{is_space, parse_int};

use crate::spec::{parsers, Action, ActionKind, Help, Mutex, Nargs, ParserId};

/// `HelpFormatter`'s default width: `shutil.get_terminal_size().columns - 2`.
///
/// `COLUMNS` wins when it parses as a positive `int()`; otherwise Python
/// asks the terminal and falls back to 80 columns. This port does not query
/// the terminal, so on an interactive terminal without `COLUMNS` it wraps at
/// 80 columns where Python would use the terminal's width. Agents and the
/// desktop read pipes, where both fall back to 80.
pub fn width_from_columns(columns: Option<&str>) -> usize {
    let columns = columns
        .and_then(|text| parse_int(text).ok())
        .and_then(|n| i64::try_from(n).ok())
        .filter(|&n| n > 0)
        .unwrap_or(80);
    usize::try_from(columns - 2).unwrap_or(0)
}

fn regex(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| match Regex::new(pattern) {
        Ok(regex) => regex,
        Err(error) => panic!("argparse pattern {pattern:?} does not compile: {error}"),
    })
}

fn len(text: &str) -> usize {
    text.chars().count()
}

/// `_metavar_formatter(action, default)(1)[0]`.
fn metavar(action: &Action, default: &str) -> String {
    if let Some(metavar) = action.metavar {
        metavar.to_owned()
    } else if let Some(choices) = action.choices {
        format!("{{{}}}", choices.join(","))
    } else if let Some(sub) = action.subparsers {
        let names: Vec<&str> = sub.names.iter().map(|(name, _)| *name).collect();
        format!("{{{}}}", names.join(","))
    } else {
        default.to_owned()
    }
}

/// `_format_args(action, default_metavar)`.
fn format_args(action: &Action, default: &str) -> String {
    let m = metavar(action, default);
    match action.nargs {
        Nargs::Single => m,
        Nargs::Optional => format!("[{m}]"),
        Nargs::ZeroOrMore => format!("[{m} ...]"),
        Nargs::OneOrMore => format!("{m} [{m} ...]"),
        Nargs::Parser => format!("{m} ..."),
        Nargs::Zero => String::new(),
    }
}

fn default_optional_metavar(action: &Action) -> String {
    action.dest.unwrap_or("").to_uppercase()
}

fn default_positional_metavar(action: &Action) -> String {
    action.dest.unwrap_or("").to_owned()
}

/// `action.format_usage()` for a flag.
fn flag_usage(action: &Action) -> String {
    if action.kind == ActionKind::BooleanOptional {
        action.option_strings.join(" | ")
    } else {
        action.option_strings[0].to_owned()
    }
}

/// `_format_actions_usage(actions, groups)`; `actions` index into
/// `all_actions`.
fn format_actions_usage(all_actions: &[Action], actions: &[usize], groups: &[Mutex]) -> String {
    let mut group_actions: Vec<usize> = Vec::new();
    let mut inserts: std::collections::BTreeMap<usize, String> = std::collections::BTreeMap::new();
    for group in groups {
        let Some(start) = actions.iter().position(|&a| a == group.actions[0]) else {
            continue;
        };
        let end = start + group.actions.len();
        if end > actions.len() || actions[start..end] != *group.actions {
            continue;
        }
        let mut suppressed = 0;
        for &action in group.actions {
            group_actions.push(action);
            if all_actions[action].help == Help::Suppressed {
                suppressed += 1;
            }
        }
        let exposed = group.actions.len() - suppressed;
        if exposed == 0 {
            continue;
        }
        let mut add = |at: usize, text: &str| {
            inserts
                .entry(at)
                .and_modify(|existing| {
                    existing.push(' ');
                    existing.push_str(text);
                })
                .or_insert_with(|| text.to_owned());
        };
        if !group.required {
            add(start, "[");
            inserts
                .entry(end)
                .and_modify(|e| e.push(']'))
                .or_insert_with(|| "]".to_owned());
        } else if exposed > 1 {
            add(start, "(");
            inserts
                .entry(end)
                .and_modify(|e| e.push(')'))
                .or_insert_with(|| ")".to_owned());
        }
        for i in start + 1..end {
            inserts.insert(i, "|".to_owned());
        }
    }

    let mut parts: Vec<Option<String>> = Vec::new();
    for (i, &index) in actions.iter().enumerate() {
        let action = &all_actions[index];
        if action.help == Help::Suppressed {
            parts.push(None);
            if inserts.get(&i).map(String::as_str) == Some("|") {
                inserts.remove(&i);
            } else if inserts.get(&(i + 1)).map(String::as_str) == Some("|") {
                inserts.remove(&(i + 1));
            }
        } else if action.is_positional() {
            let mut part = format_args(action, &default_positional_metavar(action));
            if group_actions.contains(&index) && part.starts_with('[') && part.ends_with(']') {
                part = part[1..part.len() - 1].to_owned();
            }
            parts.push(Some(part));
        } else {
            let mut part = if action.nargs == Nargs::Zero {
                flag_usage(action)
            } else {
                format!(
                    "{} {}",
                    action.option_strings[0],
                    format_args(action, &default_optional_metavar(action))
                )
            };
            if !action.required && !group_actions.contains(&index) {
                part = format!("[{part}]");
            }
            parts.push(Some(part));
        }
    }
    for (&at, text) in inserts.iter().rev() {
        parts.insert(at, Some(text.clone()));
    }
    let text = parts.into_iter().flatten().collect::<Vec<_>>().join(" ");

    static OPEN: OnceLock<Regex> = OnceLock::new();
    static CLOSE: OnceLock<Regex> = OnceLock::new();
    static EMPTY: OnceLock<Regex> = OnceLock::new();
    let text = replace(regex(&OPEN, r"([\[(]) "), &text, "$1");
    let text = replace(regex(&CLOSE, r" ([\])])"), &text, "$1");
    let text = replace(regex(&EMPTY, r"[\[(] *[\])]"), &text, "");
    kassiber_pytext::strip(&text).to_owned()
}

fn replace(regex: &Regex, text: &str, replacement: &str) -> String {
    regex
        .try_replacen(text, 0, replacement)
        .map(|t| t.into_owned())
        .unwrap_or_else(|_| text.to_owned())
}

/// `_format_usage(None, actions, groups, None)` at `width`, without the
/// trailing blank line.
fn format_usage_text(pid: ParserId, width: usize) -> String {
    let parser = &parsers()[pid];
    let prefix = "usage: ";
    let prog = parser.prog;
    if parser.actions.is_empty() {
        return format!("{prefix}{prog}");
    }
    let optionals: Vec<usize> = (0..parser.actions.len())
        .filter(|&i| !parser.actions[i].is_positional())
        .collect();
    let positionals: Vec<usize> = (0..parser.actions.len())
        .filter(|&i| parser.actions[i].is_positional())
        .collect();
    let all: Vec<usize> = optionals.iter().chain(&positionals).copied().collect();
    let action_usage = format_actions_usage(parser.actions, &all, parser.mutex_groups);
    let usage = [prog, action_usage.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let text_width = width;
    if len(prefix) + len(&usage) <= text_width {
        return format!("{prefix}{usage}");
    }
    static PART: OnceLock<Regex> = OnceLock::new();
    let part = regex(&PART, r"\(.*?\)+(?=\s|$)|\[.*?\]+(?=\s|$)|\S+");
    let split = |text: &str| -> Vec<String> {
        part.find_iter(text)
            .filter_map(Result::ok)
            .map(|m| m.as_str().to_owned())
            .collect()
    };
    let opt_parts = split(&format_actions_usage(
        parser.actions,
        &optionals,
        parser.mutex_groups,
    ));
    let pos_parts = split(&format_actions_usage(
        parser.actions,
        &positionals,
        parser.mutex_groups,
    ));
    let get_lines = |parts: &[String], indent: &str, prefix: Option<&str>| -> Vec<String> {
        let mut lines = Vec::new();
        let mut line: Vec<&str> = Vec::new();
        let mut line_len = match prefix {
            Some(prefix) => len(prefix) as isize - 1,
            None => len(indent) as isize - 1,
        };
        for part in parts {
            if line_len + 1 + len(part) as isize > text_width as isize && !line.is_empty() {
                lines.push(format!("{indent}{}", line.join(" ")));
                line.clear();
                line_len = len(indent) as isize - 1;
            }
            line.push(part);
            line_len += len(part) as isize + 1;
        }
        if !line.is_empty() {
            lines.push(format!("{indent}{}", line.join(" ")));
        }
        if prefix.is_some() {
            if let Some(first) = lines.first_mut() {
                *first = first.chars().skip(len(indent)).collect();
            }
        }
        lines
    };
    let lines: Vec<String> = if (len(prefix) + len(prog)) as f64 <= 0.75 * text_width as f64 {
        let indent = " ".repeat(len(prefix) + len(prog) + 1);
        let with_prog = |parts: &[String]| {
            let mut all = vec![prog.to_owned()];
            all.extend(parts.iter().cloned());
            all
        };
        if !opt_parts.is_empty() {
            let mut lines = get_lines(&with_prog(&opt_parts), &indent, Some(prefix));
            lines.extend(get_lines(&pos_parts, &indent, None));
            lines
        } else if !pos_parts.is_empty() {
            get_lines(&with_prog(&pos_parts), &indent, Some(prefix))
        } else {
            vec![prog.to_owned()]
        }
    } else {
        let indent = " ".repeat(len(prefix));
        let parts: Vec<String> = opt_parts.iter().chain(&pos_parts).cloned().collect();
        let mut lines = get_lines(&parts, &indent, None);
        if lines.len() > 1 {
            lines = get_lines(&opt_parts, &indent, None);
            lines.extend(get_lines(&pos_parts, &indent, None));
        }
        let mut all = vec![prog.to_owned()];
        all.extend(lines);
        all
    };
    format!("{prefix}{}", lines.join("\n"))
}

/// `parser.format_usage()`: the usage line(s) and a newline.
pub fn format_usage(pid: ParserId, width: usize) -> String {
    format!("{}\n", format_usage_text(pid, width))
}

/// `textwrap.TextWrapper(width, initial_indent, subsequent_indent).wrap`.
fn wrap(text: &str, width: usize, initial: &str, subsequent: &str) -> Vec<String> {
    static WORDSEP: OnceLock<Regex> = OnceLock::new();
    let ws = r"[\t\n\x0b\x0c\r ]";
    let nws = r"[^\t\n\x0b\x0c\r ]";
    let wp = r#"[\w!"'&.,?]"#;
    let lt = r"[^\d\W]";
    let wordsep = regex(
        &WORDSEP,
        &format!(
            r"({ws}+|(?<={wp})-{{2,}}(?=\w)|{nws}+?(?:-(?:(?<={lt}{{2}}-)|(?<={lt}-{lt}-))(?={lt}-?{lt})|(?={ws}|\z)|(?<={wp})(?=-{{2,}}\w)))"
        ),
    );
    let munged: String = text
        .chars()
        .map(|c| if "\t\n\x0b\x0c\r".contains(c) { ' ' } else { c })
        .collect();
    let mut chunks: Vec<String> = Vec::new();
    let mut last = 0;
    for found in wordsep.find_iter(&munged).filter_map(Result::ok) {
        chunks.push(munged[last..found.start()].to_owned());
        chunks.push(found.as_str().to_owned());
        last = found.end();
    }
    chunks.push(munged[last..].to_owned());
    chunks.retain(|c| !c.is_empty());
    chunks.reverse();

    let blank = |chunk: &str| chunk.chars().all(|c| is_space(u32::from(c)));
    let mut lines: Vec<String> = Vec::new();
    while !chunks.is_empty() {
        let mut current: Vec<String> = Vec::new();
        let mut current_len = 0usize;
        let indent = if lines.is_empty() {
            initial
        } else {
            subsequent
        };
        let line_width = width.saturating_sub(len(indent));
        if !lines.is_empty() && chunks.last().is_some_and(|c| blank(c)) {
            chunks.pop();
        }
        while let Some(chunk) = chunks.last() {
            let l = len(chunk);
            if current_len + l <= line_width {
                current_len += l;
                current.push(chunks.pop().unwrap_or_default());
            } else {
                break;
            }
        }
        if chunks.last().is_some_and(|c| len(c) > line_width) {
            let space_left = if line_width < 1 {
                1
            } else {
                line_width - current_len
            };
            if let Some(chunk) = chunks.pop() {
                let chars: Vec<char> = chunk.chars().collect();
                let mut end = space_left.min(chars.len());
                if chars.len() > space_left {
                    let hyphen = chars[..space_left.min(chars.len())]
                        .iter()
                        .rposition(|&c| c == '-');
                    if let Some(hyphen) = hyphen {
                        if hyphen > 0 && chars[..hyphen].iter().any(|&c| c != '-') {
                            end = hyphen + 1;
                        }
                    }
                }
                current.push(chars[..end].iter().collect());
                let rest: String = chars[end..].iter().collect();
                chunks.push(rest);
            }
            current_len = current.iter().map(|c| len(c)).sum();
        }
        if current.last().is_some_and(|c| blank(c)) {
            current_len -= current.last().map_or(0, |c| len(c));
            current.pop();
        }
        let _ = current_len;
        if !current.is_empty() {
            lines.push(format!("{indent}{}", current.concat()));
        }
    }
    lines
}

/// `_whitespace_matcher.sub(' ', text).strip()`.
fn collapse(text: &str) -> String {
    let mut out = String::new();
    let mut in_space = false;
    for c in text.chars() {
        if "\t\n\x0b\x0c\r ".contains(c) {
            in_space = true;
        } else {
            if in_space && !out.is_empty() {
                out.push(' ');
            }
            in_space = false;
            out.push(c);
        }
    }
    kassiber_pytext::strip(&out).to_owned()
}

struct Formatter {
    width: usize,
    max_help_position: usize,
    indent: usize,
    action_max_length: usize,
}

impl Formatter {
    fn new(width: usize) -> Self {
        Formatter {
            width,
            max_help_position: 24.min((width as isize - 20).max(4) as usize),
            indent: 0,
            action_max_length: 0,
        }
    }

    fn text(&self, text: &str) -> String {
        let width = (self.width as isize - self.indent as isize).max(11) as usize;
        let indent = " ".repeat(self.indent);
        let lines = wrap(&collapse(text), width, &indent, &indent);
        format!("{}\n\n", lines.join("\n"))
    }

    fn invocation(action: &Action) -> String {
        if action.is_positional() {
            return metavar(action, &default_positional_metavar(action));
        }
        if action.nargs == Nargs::Zero {
            return action.option_strings.join(", ");
        }
        let args = format_args(action, &default_optional_metavar(action));
        action
            .option_strings
            .iter()
            .map(|option| format!("{option} {args}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// `add_argument`: track the widest invocation.
    fn measure(&mut self, action: &Action) {
        if action.help == Help::Suppressed {
            return;
        }
        let mut widest = len(&Self::invocation(action));
        if let Some(sub) = action.subparsers {
            for choice in sub.choice_help {
                widest = widest.max(len(choice.metavar));
            }
        }
        self.action_max_length = self.action_max_length.max(widest + self.indent);
    }

    fn entry(&self, header: String, help: Help, indent: usize) -> String {
        let help_position = (self.action_max_length + 2).min(self.max_help_position);
        let help_width = (self.width as isize - help_position as isize).max(11) as usize;
        let action_width = help_position as isize - indent as isize - 2;
        let pad = " ".repeat(indent);
        // Python formats a suppressed subcommand's help literally.
        let help_text = match help {
            Help::Suppressed => Some("==SUPPRESS=="),
            other => other.text().filter(|h| !h.is_empty()),
        };
        let (mut out, indent_first) = match help_text {
            None => (format!("{pad}{header}\n"), 0),
            Some(_) if len(&header) as isize <= action_width => (
                format!(
                    "{pad}{header:<width$}  ",
                    width = action_width.max(0) as usize
                ),
                0,
            ),
            Some(_) => (format!("{pad}{header}\n"), help_position),
        };
        match help_text {
            Some(text) if !kassiber_pytext::strip(text).is_empty() => {
                let lines = wrap(&collapse(text), help_width, "", "");
                if let Some((first, rest)) = lines.split_first() {
                    out.push_str(&format!("{}{first}\n", " ".repeat(indent_first)));
                    for line in rest {
                        out.push_str(&format!("{}{line}\n", " ".repeat(help_position)));
                    }
                }
            }
            _ => {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
        }
        out
    }

    fn action(&self, action: &Action) -> String {
        let mut out = self.entry(Self::invocation(action), action.help, self.indent);
        if let Some(sub) = action.subparsers {
            for choice in sub.choice_help {
                out.push_str(&self.entry(choice.metavar.to_owned(), choice.help, self.indent + 2));
            }
        }
        out
    }
}

/// `parser.format_help()`.
pub fn format_help(pid: ParserId, width: usize) -> String {
    let parser = &parsers()[pid];
    let mut formatter = Formatter::new(width);
    formatter.indent = 2;
    for section in parser.sections {
        for &action in section.actions {
            formatter.measure(&parser.actions[action]);
        }
    }
    formatter.indent = 0;
    let mut items = vec![format!("{}\n\n", format_usage_text(pid, width))];
    if let Some(description) = parser.description {
        items.push(formatter.text(description));
    }
    for section in parser.sections {
        formatter.indent = 2;
        let mut body = String::new();
        if let Some(description) = section.description {
            body.push_str(&formatter.text(description));
        }
        for &action in section.actions {
            let action = &parser.actions[action];
            if action.help != Help::Suppressed {
                body.push_str(&formatter.action(action));
            }
        }
        formatter.indent = 0;
        if !body.is_empty() {
            items.push(format!("\n{}:\n{body}\n", section.title));
        }
    }
    finish(&format!("\n{}\n", items.concat()))
}

/// `formatter.format_help()`'s final pass.
fn finish(help: &str) -> String {
    static LONG_BREAK: OnceLock<Regex> = OnceLock::new();
    let collapsed = replace(regex(&LONG_BREAK, r"\n\n\n+"), help, "\n\n");
    format!("{}\n", collapsed.trim_matches('\n'))
}

/// `--version`: the version text as `add_text` formats it.
pub fn format_version(version: &str, width: usize) -> String {
    let formatter = Formatter::new(width);
    finish(&format!("\n{}\n", formatter.text(version)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_follows_columns() {
        assert_eq!(width_from_columns(None), 78);
        assert_eq!(width_from_columns(Some(" 120 ")), 118);
        assert_eq!(width_from_columns(Some("0")), 78);
        assert_eq!(width_from_columns(Some("abc")), 78);
    }

    #[test]
    fn wrap_breaks_hyphenated_words_like_textwrap() {
        assert_eq!(
            wrap("read-only commands stay read-only", 12, "", ""),
            vec!["read-only", "commands", "stay read-", "only"]
        );
        assert_eq!(
            wrap("abcdefghijkl", 5, "", ""),
            vec!["abcde", "fghij", "kl"]
        );
    }

    #[test]
    fn version_text_is_one_line() {
        assert_eq!(format_version("Kassiber 0.22.78", 78), "Kassiber 0.22.78\n");
    }
}
