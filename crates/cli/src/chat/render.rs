//! Terminal rendering for `kassiber chat` (`kassiber/cli/termrender.py`):
//! the streaming markdown-lite renderer, and deterministic tables drawn
//! from tool-result envelopes so the numbers shown come from the daemon,
//! never from the model retyping them. Callers decide when ANSI is
//! appropriate (a TTY without `--plain`).

use kassiber_envelope::get;
use kassiber_pyjson::{Key, Value};

const BOLD: &str = "\x1b[1m";
const BOLD_OFF: &str = "\x1b[22m";
const DIM: &str = "\x1b[2m";
const DIM_OFF: &str = "\x1b[22m";
const CODE: &str = "\x1b[36m";
const CODE_OFF: &str = "\x1b[39m";

const MAX_CELL_CHARS: usize = 28;
const DEFAULT_TERMINAL_WIDTH: usize = 100;
const HR_CHARS: [char; 3] = ['-', '*', '_'];

fn count(text: &str) -> usize {
    text.chars().count()
}

/// `re.sub(r"\*\*(.+?)\*\*", ...)`: the shortest non-empty span between
/// two `**`, left to right.
fn sub_bold(text: &str, open: &str, close: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '*' && chars.get(index + 1) == Some(&'*') {
            // `.+?` needs at least one character before the closing `**`
            // and never crosses a newline.
            let mut end = index + 3;
            let mut found = None;
            while end < chars.len() {
                if chars[end - 1] == '\n' {
                    break;
                }
                if chars[end] == '*' && chars.get(end + 1) == Some(&'*') {
                    found = Some(end);
                    break;
                }
                end += 1;
            }
            if let Some(end) = found {
                out.push_str(open);
                out.extend(&chars[index + 2..end]);
                out.push_str(close);
                index = end + 2;
                continue;
            }
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

/// ``re.sub(r"`([^`]+)`", ...)``.
fn sub_code(text: &str, open: &str, close: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '`' {
            if let Some(offset) = chars[index + 1..].iter().position(|c| *c == '`') {
                if offset > 0 {
                    out.push_str(open);
                    out.extend(&chars[index + 1..index + 1 + offset]);
                    out.push_str(close);
                    index += offset + 2;
                    continue;
                }
            }
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

/// `_render_inline_cell(text)`: the ANSI text and its visible width.
fn inline_cell(text: &str) -> (String, usize) {
    let visible = sub_code(&sub_bold(text, "", ""), "", "");
    let rendered = sub_code(&sub_bold(text, BOLD, BOLD_OFF), CODE, CODE_OFF);
    (rendered, count(&visible))
}

/// `_box_table(header, rows)`.
fn box_table(header: Vec<(String, usize)>, rows: Vec<Vec<(String, usize)>>) -> String {
    let columns = std::iter::once(header.len())
        .chain(rows.iter().map(Vec::len))
        .max()
        .unwrap_or(0);
    if columns == 0 {
        return String::new();
    }
    let pad = |mut cells: Vec<(String, usize)>| {
        cells.resize(columns, (String::new(), 0));
        cells
    };
    let header = pad(header);
    let rows: Vec<_> = rows.into_iter().map(pad).collect();
    let mut widths = vec![0; columns];
    for cells in std::iter::once(&header).chain(rows.iter()) {
        for (index, (_, visible)) in cells.iter().enumerate() {
            widths[index] = widths[index].max(*visible);
        }
    }
    let border = format!("{DIM}│{DIM_OFF}");
    let row = |cells: &[(String, usize)], bold: bool| {
        let parts: Vec<String> = cells
            .iter()
            .enumerate()
            .map(|(index, (rendered, visible))| {
                let text = if bold {
                    format!("{BOLD}{rendered}{BOLD_OFF}")
                } else {
                    rendered.clone()
                };
                format!(" {text}{}", " ".repeat(widths[index] - visible + 1))
            })
            .collect();
        format!("{border}{}{border}", parts.join(&border))
    };
    let mut lines = vec![row(&header, true)];
    let rule: Vec<String> = widths.iter().map(|width| "─".repeat(width + 2)).collect();
    lines.push(format!("{DIM}├{}┤{DIM_OFF}", rule.join("┼")));
    for cells in &rows {
        lines.push(row(cells, false));
    }
    lines.join("\n")
}

/// `^:?-+:?$`.
fn separator_cell(cell: &str) -> bool {
    let inner = cell.strip_prefix(':').unwrap_or(cell);
    let inner = inner.strip_suffix(':').unwrap_or(inner);
    !inner.is_empty() && inner.chars().all(|c| c == '-')
}

/// `render_markdown_table(lines)`.
pub fn render_markdown_table(lines: &[String]) -> String {
    let mut parsed: Vec<Vec<String>> = Vec::new();
    for line in lines {
        let stripped = kassiber_pytext::strip(line);
        if stripped.is_empty() {
            continue;
        }
        let cells: Vec<String> = stripped
            .trim_matches('|')
            .split('|')
            .map(|cell| kassiber_pytext::strip(cell).to_owned())
            .collect();
        if !cells.is_empty()
            && cells
                .iter()
                .filter(|cell| !cell.is_empty())
                .all(|cell| separator_cell(cell))
        {
            continue;
        }
        parsed.push(cells);
    }
    let Some((head, body)) = parsed.split_first() else {
        return String::new();
    };
    box_table(
        head.iter().map(|cell| inline_cell(cell)).collect(),
        body.iter()
            .map(|row| row.iter().map(|cell| inline_cell(cell)).collect())
            .collect(),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    LineStart,
    Inline,
    Fence,
    Table,
}

/// `MarkdownStreamRenderer`: `feed` returns text safe to print at once;
/// `flush` drains what is still buffered. Any chunking of a document yields
/// the same bytes.
pub struct MarkdownStream {
    state: State,
    hold: String,
    indent: String,
    bold: bool,
    code: bool,
    header: bool,
    pending_stars: usize,
    fence_info_pending: bool,
    table_lines: Vec<String>,
    table_current: String,
}

impl Default for MarkdownStream {
    fn default() -> Self {
        MarkdownStream {
            state: State::LineStart,
            hold: String::new(),
            indent: String::new(),
            bold: false,
            code: false,
            header: false,
            pending_stars: 0,
            fence_info_pending: false,
            table_lines: Vec::new(),
            table_current: String::new(),
        }
    }
}

impl MarkdownStream {
    pub fn feed(&mut self, chunk: &str) -> String {
        chunk.chars().map(|c| self.feed_char(c)).collect()
    }

    pub fn flush(&mut self) -> String {
        let mut out = String::new();
        if self.state == State::LineStart && !self.hold.is_empty() {
            out.push_str(&self.resolve_line_start_hold());
        }
        if !self.table_current.is_empty() {
            let current = std::mem::take(&mut self.table_current);
            self.table_lines.push(current);
        }
        // A finished table waits for the next character to show the block
        // ended; at the end of the stream there is none.
        out.push_str(&self.render_table_buffer());
        if self.state == State::Fence && !self.hold.is_empty() {
            if self.fence_info_pending {
                self.fence_info_pending = false;
            } else {
                out.push_str(&fence_line(&self.hold));
            }
            self.hold.clear();
        }
        out.push_str(&self.resolve_pending_stars());
        out.push_str(&self.close_styles());
        out
    }

    fn feed_char(&mut self, c: char) -> String {
        if c == '\r' {
            return String::new();
        }
        match self.state {
            State::Fence => self.feed_fence(c),
            State::Table => self.feed_table(c),
            State::LineStart => self.feed_line_start(c),
            State::Inline => self.feed_inline(c),
        }
    }

    fn feed_line_start(&mut self, c: char) -> String {
        let mut flushed = String::new();
        if !self.table_lines.is_empty() {
            if c == '|' {
                self.state = State::Table;
                self.table_current = "|".into();
                return String::new();
            }
            flushed = self.render_table_buffer();
        }
        if c == '\n' {
            let resolved = self.resolve_line_start_hold();
            self.state = State::LineStart;
            return format!(
                "{flushed}{resolved}{}{}\n",
                self.resolve_pending_stars(),
                self.close_styles()
            );
        }
        if c == ' ' && self.hold.is_empty() {
            self.indent.push(c);
            return flushed;
        }
        let candidate = format!("{}{c}", self.hold);
        if could_become_marker(&candidate) {
            self.hold = candidate;
            return flushed;
        }
        if let Some(marker) = self.resolve_marker(&candidate) {
            return flushed + &marker;
        }
        self.hold.clear();
        self.state = State::Inline;
        let prefix = std::mem::take(&mut self.indent);
        flushed + &prefix + &self.feed_inline_text(&candidate)
    }

    fn resolve_marker(&mut self, text: &str) -> Option<String> {
        let indent = self.indent.clone();
        let mut enter = |state: State| {
            self.state = state;
            self.hold.clear();
            self.indent.clear();
        };
        if text == "|" {
            enter(State::Table);
            self.table_current = "|".into();
            return Some(String::new());
        }
        let hashes = text.trim_end_matches(' ');
        if text.ends_with(' ')
            && text.len() == hashes.len() + 1
            && (1..=6).contains(&hashes.len())
            && hashes.chars().all(|c| c == '#')
        {
            enter(State::Inline);
            self.header = true;
            return Some(format!("{indent}{BOLD}"));
        }
        if text == "- " || text == "* " {
            enter(State::Inline);
            return Some(format!("{indent}• "));
        }
        if text == "> " {
            enter(State::Inline);
            return Some(format!("{indent}{DIM}┃{DIM_OFF} "));
        }
        if text == "```" {
            enter(State::Fence);
            self.fence_info_pending = true;
            return Some(String::new());
        }
        None
    }

    fn resolve_line_start_hold(&mut self) -> String {
        let text = std::mem::take(&mut self.hold);
        let indent = std::mem::take(&mut self.indent);
        if text.is_empty() {
            return indent;
        }
        let first = text.chars().next().unwrap_or(' ');
        if count(&text) >= 3 && text.chars().all(|c| c == first) && HR_CHARS.contains(&first) {
            let width = count(&text).min(40);
            return format!("{indent}{DIM}{}{DIM_OFF}", "─".repeat(width.max(8)));
        }
        self.state = State::Inline;
        let out = indent + &self.feed_inline_text(&text);
        self.state = State::LineStart;
        out
    }

    fn feed_inline_text(&mut self, text: &str) -> String {
        text.chars().map(|c| self.feed_inline(c)).collect()
    }

    fn feed_inline(&mut self, c: char) -> String {
        if c == '\n' {
            let out = format!("{}{}\n", self.resolve_pending_stars(), self.close_styles());
            self.state = State::LineStart;
            return out;
        }
        if self.code {
            if c == '`' {
                self.code = false;
                return CODE_OFF.into();
            }
            return c.into();
        }
        if c == '*' {
            self.pending_stars += 1;
            if self.pending_stars == 2 {
                self.pending_stars = 0;
                self.bold = !self.bold;
                return if self.bold { BOLD } else { BOLD_OFF }.into();
            }
            return String::new();
        }
        let pending = self.resolve_pending_stars();
        if c == '`' {
            self.code = true;
            return pending + CODE;
        }
        let mut out = pending;
        out.push(c);
        out
    }

    fn resolve_pending_stars(&mut self) -> String {
        let stars = "*".repeat(self.pending_stars);
        self.pending_stars = 0;
        stars
    }

    fn close_styles(&mut self) -> String {
        let mut out = String::new();
        if std::mem::take(&mut self.bold) {
            out.push_str(BOLD_OFF);
        }
        if std::mem::take(&mut self.code) {
            out.push_str(CODE_OFF);
        }
        if std::mem::take(&mut self.header) {
            out.push_str(BOLD_OFF);
        }
        out
    }

    fn feed_fence(&mut self, c: char) -> String {
        if c != '\n' {
            self.hold.push(c);
            return String::new();
        }
        let line = std::mem::take(&mut self.hold);
        if self.fence_info_pending {
            self.fence_info_pending = false;
            return String::new();
        }
        if kassiber_pytext::strip(&line) == "```" {
            self.state = State::LineStart;
            return String::new();
        }
        fence_line(&line) + "\n"
    }

    fn feed_table(&mut self, c: char) -> String {
        if c != '\n' {
            self.table_current.push(c);
            return String::new();
        }
        let current = std::mem::take(&mut self.table_current);
        self.table_lines.push(current);
        self.state = State::LineStart;
        String::new()
    }

    fn render_table_buffer(&mut self) -> String {
        let lines = std::mem::take(&mut self.table_lines);
        if lines.is_empty() {
            return String::new();
        }
        let rendered = render_markdown_table(&lines);
        if rendered.is_empty() {
            rendered
        } else {
            rendered + "\n"
        }
    }
}

/// `_could_become_marker(text)`.
fn could_become_marker(text: &str) -> bool {
    let Some(first) = text.chars().next() else {
        return true;
    };
    let length = count(text);
    if text.chars().all(|c| c == '#') {
        return length <= 6;
    }
    if text.chars().all(|c| c == '`') {
        return length <= 2;
    }
    if text.chars().all(|c| c == first) && HR_CHARS.contains(&first) {
        return length <= 1 || (['-', '_'].contains(&first) && length <= 80);
    }
    text == ">"
}

fn fence_line(line: &str) -> String {
    format!("  {CODE}{line}{CODE_OFF}")
}

const PRIORITY_COLUMNS: [&str; 23] = [
    "occurred_at",
    "date",
    "year",
    "period",
    "label",
    "name",
    "title",
    "kind",
    "direction",
    "asset",
    "wallet",
    "amount",
    "quantity",
    "fiat_value",
    "fiat_currency",
    "fee",
    "rate",
    "status",
    "confidence",
    "count",
    "total",
    "balance",
    "note",
];

/// `_primary_rows(data)`: the longest list made only of objects.
fn primary_rows(data: &Value) -> Vec<&Value> {
    let candidates: Vec<&Vec<Value>> = match data {
        Value::List(items) => vec![items],
        Value::Dict(pairs) => pairs
            .iter()
            .filter_map(|(_, value)| match value {
                Value::List(items) => Some(items),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    let mut best: Vec<&Value> = Vec::new();
    for candidate in candidates {
        let rows: Vec<&Value> = candidate
            .iter()
            .filter(|row| matches!(row, Value::Dict(_)))
            .collect();
        if rows.len() == candidate.len() && rows.len() > best.len() {
            best = rows;
        }
    }
    best
}

fn key_text(key: &Key) -> String {
    match key {
        Key::Str(text) => kassiber_envelope::py_str(&Value::Str(text.clone())),
        other => kassiber_envelope::py_str(&match other {
            Key::Int(n) => Value::Int(n.clone()),
            Key::Bool(flag) => Value::Bool(*flag),
            Key::Float(f) => Value::Float(*f),
            _ => Value::Null,
        }),
    }
}

fn cell(row: &Value, column: &str) -> String {
    crate::render::table_value(get(row, column).unwrap_or(&Value::Null))
}

/// `_select_columns(rows, width_budget)`.
fn select_columns(rows: &[&Value], budget: usize) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for row in rows {
        if let Value::Dict(pairs) = row {
            for (key, _) in pairs {
                let key = key_text(key);
                if !seen.contains(&key) {
                    seen.push(key);
                }
            }
        }
    }
    let usable: Vec<String> = seen
        .into_iter()
        .filter(|key| {
            !matches!(key.as_str(), "id" | "schema_version")
                && !["_msat", "_id", "_json"]
                    .iter()
                    .any(|suffix| key.ends_with(suffix))
        })
        .filter(|key| {
            !rows.iter().all(|row| {
                matches!(
                    get(row, key),
                    None | Some(Value::Null) | Some(Value::Dict(_)) | Some(Value::List(_))
                )
            })
        })
        .collect();
    let mut ordered: Vec<String> = PRIORITY_COLUMNS
        .iter()
        .filter(|key| usable.iter().any(|usable| usable == *key))
        .map(|key| (*key).to_owned())
        .collect();
    for key in usable {
        if !ordered.contains(&key) {
            ordered.push(key);
        }
    }
    let mut selected: Vec<String> = Vec::new();
    let mut used = 0;
    for key in ordered {
        let width = std::iter::once(count(&key))
            .chain(rows.iter().map(|row| count(&cell(row, &key))))
            .max()
            .unwrap_or(0)
            .min(MAX_CELL_CHARS);
        if !selected.is_empty() && used + width + 3 > budget {
            continue;
        }
        selected.push(key);
        used += width + 3;
    }
    selected
}

fn clip(text: &str) -> String {
    if count(text) <= MAX_CELL_CHARS {
        return text.to_owned();
    }
    let mut clipped: String = text.chars().take(MAX_CELL_CHARS - 1).collect();
    clipped.push('…');
    clipped
}

/// `shutil.get_terminal_size((100, 24)).columns` from `COLUMNS`.
pub fn terminal_width(columns: Option<&str>) -> usize {
    columns
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|width| *width > 0)
        .unwrap_or(DEFAULT_TERMINAL_WIDTH)
}

/// `render_envelope_table(envelope, max_rows=8, terminal_width=...)`:
/// `None` when the payload has no list of objects to show.
pub fn render_envelope_table(envelope: &Value, width: usize) -> Option<String> {
    let max_rows = 8;
    if !matches!(envelope, Value::Dict(_)) {
        return None;
    }
    let rows = primary_rows(get(envelope, "data").unwrap_or(&Value::Null));
    if rows.is_empty() {
        return None;
    }
    let shown: Vec<&Value> = rows.iter().take(max_rows).copied().collect();
    let columns = select_columns(&shown, width.saturating_sub(2).max(40));
    if columns.is_empty() {
        return None;
    }
    let header = columns
        .iter()
        .map(|column| {
            let text = clip(column);
            let width = count(&text);
            (text, width)
        })
        .collect();
    let body = shown
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|column| {
                    let text = clip(&cell(row, column));
                    let width = count(&text);
                    (text, width)
                })
                .collect()
        })
        .collect();
    let mut table = box_table(header, body);
    if rows.len() > max_rows {
        table.push_str(&format!(
            "\n{DIM}… {} more rows{DIM_OFF}",
            rows.len() - max_rows
        ));
    }
    Some(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(text: &str) -> String {
        let mut stream = MarkdownStream::default();
        let mut out = stream.feed(text);
        out.push_str(&stream.flush());
        out
    }

    #[test]
    fn inline_styles_match_python() {
        assert_eq!(
            render("**Ready** to `help`."),
            "\x1b[1mReady\x1b[22m to \x1b[36mhelp\x1b[39m."
        );
        assert_eq!(render("# Title\n"), "\x1b[1mTitle\x1b[22m\n");
        assert_eq!(render("- item\n"), "• item\n");
        assert_eq!(render("---\n"), "\x1b[2m────────\x1b[22m\n");
        assert_eq!(
            render("```py\ncode\n```\nafter"),
            "  \x1b[36mcode\x1b[39m\nafter"
        );
        assert_eq!(render("a * b"), "a * b");
    }

    #[test]
    fn chunking_never_changes_the_bytes() {
        let document =
            "# Head\n| a | b |\n|---|---|\n| **1** | `x` |\n\nText **bold** and `code`\n> quote\n";
        let whole = render(document);
        let mut stream = MarkdownStream::default();
        let mut split = String::new();
        for c in document.chars() {
            split.push_str(&stream.feed(&c.to_string()));
        }
        split.push_str(&stream.flush());
        assert_eq!(whole, split);
        assert!(whole.contains("│"));
    }

    #[test]
    fn envelope_tables_skip_ids_and_clip() {
        let row = |label: &str| {
            kassiber_envelope::dict([
                ("id", kassiber_envelope::s("x")),
                ("label", kassiber_envelope::s(label)),
                ("amount_msat", Value::Int(1.into())),
                ("amount", Value::Float(0.5)),
            ])
        };
        let envelope = kassiber_envelope::dict([(
            "data",
            kassiber_envelope::dict([(
                "rows",
                Value::List(
                    (0..10)
                        .map(|i| row(&format!("{i}{}", "y".repeat(40))))
                        .collect(),
                ),
            )]),
        )]);
        let table = render_envelope_table(&envelope, 100).unwrap();
        assert!(table.contains("label"));
        assert!(table.contains("0.5"));
        assert!(!table.contains("amount_msat"));
        assert!(table.contains("… 2 more rows"));
        assert!(table.contains("…"));
    }
}
