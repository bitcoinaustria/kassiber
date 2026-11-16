//! ChatMarkdown.tsx: assistant answers as Markdown (the GFM subset models write: headings,
//! paragraphs, emphasis, inline code, links and autolinks, lists, block quotes, fenced code,
//! tables, rules), styled like the web's component map: `text-sm leading-relaxed
//! text-foreground/85`, headings from `text-lg` down, `bg-foreground/10` inline code, bordered
//! tables with a `bg-muted/70` head, and fenced code under a header with its language and a Copy
//! button. rune-kit's Markdown is behind a feature this crate's Cargo.toml doesn't enable, so the
//! parser is a small one of its own.
//!
//! Model output is untrusted: links open only on a press (http(s) in the system browser, app
//! routes in place), and images are never fetched: `![alt](src)` becomes the web's "Open image"
//! link, as ChatImageReference does.

use rune::{Color, El, Link, Span, TextStyle, Tx, col, el, rich, row, text};

use crate::app::App;
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

/// How the text should read: the answer, or a reasoning pane (`text-xs leading-6 muted`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Answer,
    Reasoning,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading(u8, String),
    Para(String),
    List {
        ordered: bool,
        start: u64,
        items: Vec<Vec<Block>>,
    },
    Quote(Vec<Block>),
    Code {
        lang: Option<String>,
        text: String,
    },
    Table {
        head: Vec<String>,
        align: Vec<Cell>,
        rows: Vec<Vec<String>>,
    },
    Rule,
}

fn fence_open(line: &str) -> Option<(char, usize, Option<String>)> {
    let t = line.trim_start_matches(' ');
    if line.len() - t.len() > 3 {
        return None;
    }
    let c = t.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = t.chars().take_while(|x| *x == c).count();
    if n < 3 {
        return None;
    }
    let info = t[n..].trim();
    if c == '`' && info.contains('`') {
        return None;
    }
    Some((c, n, info.split_whitespace().next().map(str::to_owned)))
}

fn heading(line: &str) -> Option<(u8, String)> {
    let t = line.trim_start_matches(' ');
    let n = t.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&n) && (t.len() == n || t[n..].starts_with(' ')) {
        Some((
            n as u8,
            t[n..].trim().trim_end_matches('#').trim().to_owned(),
        ))
    } else {
        None
    }
}

fn is_rule(line: &str) -> bool {
    let t: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    t.len() >= 3
        && (t.chars().all(|c| c == '-')
            || t.chars().all(|c| c == '*')
            || t.chars().all(|c| c == '_'))
}

/// A list marker: (ordered, number, content column).
fn list_marker(line: &str) -> Option<(bool, u64, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let t = &line[indent..];
    if let Some(c) = t.chars().next()
        && matches!(c, '-' | '*' | '+')
        && (t.len() == 1 || t[1..].starts_with(' '))
        && !is_rule(line)
    {
        return Some((false, 0, indent + 2.min(t.len())));
    }
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if (1..=9).contains(&digits) {
        let rest = &t[digits..];
        if (rest.starts_with(". ") || rest.starts_with(") ")) || rest == "." || rest == ")" {
            return Some((
                true,
                t[..digits].parse().unwrap_or(1),
                indent + digits + 2.min(rest.len()),
            ));
        }
    }
    None
}

fn split_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    let mut cells = vec![];
    let mut cur = String::new();
    let mut esc = false;
    let mut code = false;
    for ch in t.chars() {
        if esc {
            if ch != '|' {
                cur.push('\\');
            }
            cur.push(ch);
            esc = false;
        } else if ch == '\\' {
            esc = true;
        } else if ch == '`' {
            code = !code;
            cur.push(ch);
        } else if ch == '|' && !code {
            cells.push(cur.trim().to_owned());
            cur.clear();
        } else {
            cur.push(ch);
        }
    }
    cells.push(cur.trim().to_owned());
    cells
}

fn table_align(line: &str) -> Option<Vec<Cell>> {
    if !line.contains('-') {
        return None;
    }
    let cells = split_row(line);
    cells
        .iter()
        .map(|c| {
            let c = c.trim();
            if c.is_empty() || !c.trim_matches(':').chars().all(|x| x == '-') || !c.contains('-') {
                return None;
            }
            Some(match (c.starts_with(':'), c.ends_with(':')) {
                (true, true) => Cell::Center,
                (false, true) => Cell::Right,
                _ => Cell::Left,
            })
        })
        .collect()
}

fn starts_block(line: &str, next: Option<&str>) -> bool {
    let t = line.trim_start();
    fence_open(line).is_some()
        || heading(line).is_some()
        || t.starts_with('>')
        || is_rule(line)
        || list_marker(line).is_some()
        || (line.contains('|') && next.and_then(table_align).is_some())
}

/// Parses Markdown into blocks.
pub fn parse(src: &str) -> Vec<Block> {
    let lines: Vec<&str> = src.lines().collect();
    parse_lines(&lines)
}

fn parse_lines(lines: &[&str]) -> Vec<Block> {
    let mut out = vec![];
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            i += 1;
            continue;
        }
        if let Some((c, n, lang)) = fence_open(line) {
            let indent = line.len() - line.trim_start_matches(' ').len();
            let mut body = vec![];
            i += 1;
            while i < lines.len() {
                let l = lines[i];
                let t = l.trim_start_matches(' ');
                if t.chars().take_while(|x| *x == c).count() >= n
                    && t.trim_start_matches(c).trim().is_empty()
                {
                    i += 1;
                    break;
                }
                let strip = (l.len() - t.len()).min(indent);
                body.push(&l[strip..]);
                i += 1;
            }
            out.push(Block::Code {
                lang,
                text: body.join("\n"),
            });
            continue;
        }
        if let Some((level, text)) = heading(line) {
            out.push(Block::Heading(level, text));
            i += 1;
            continue;
        }
        if is_rule(line) {
            out.push(Block::Rule);
            i += 1;
            continue;
        }
        if line.trim_start().starts_with('>') {
            let mut inner = vec![];
            while i < lines.len() && !lines[i].trim().is_empty() {
                let t = lines[i].trim_start();
                let stripped = t
                    .strip_prefix('>')
                    .map(|r| r.strip_prefix(' ').unwrap_or(r))
                    .unwrap_or(t);
                inner.push(stripped);
                i += 1;
            }
            out.push(Block::Quote(parse_lines(&inner)));
            continue;
        }
        if line.contains('|')
            && let Some(align) = lines.get(i + 1).and_then(|l| table_align(l))
        {
            let head = split_row(line);
            if head.len() == align.len() {
                i += 2;
                let mut rows = vec![];
                while i < lines.len() && lines[i].contains('|') && !lines[i].trim().is_empty() {
                    let mut r = split_row(lines[i]);
                    r.resize(head.len(), String::new());
                    rows.push(r);
                    i += 1;
                }
                out.push(Block::Table { head, align, rows });
                continue;
            }
        }
        if let Some((ordered, start, _)) = list_marker(line) {
            let base = line.len() - line.trim_start_matches(' ').len();
            let mut items: Vec<Vec<String>> = vec![];
            while i < lines.len() {
                let l = lines[i];
                if l.trim().is_empty() {
                    // A blank line ends the list unless an indented continuation or another
                    // item of the same list follows.
                    let next = lines.get(i + 1).copied().unwrap_or("");
                    let next_indent = next.len() - next.trim_start_matches(' ').len();
                    if next.trim().is_empty()
                        || (next_indent <= base
                            && list_marker(next).is_none_or(|(o, _, _)| o != ordered))
                    {
                        break;
                    }
                    if let Some(it) = items.last_mut() {
                        it.push(String::new())
                    }
                    i += 1;
                    continue;
                }
                let indent = l.len() - l.trim_start_matches(' ').len();
                match list_marker(l) {
                    Some((o, _, col)) if indent == base && o == ordered => {
                        items.push(vec![l.get(col..).unwrap_or("").to_owned()]);
                    }
                    Some(_) if indent == base => break,
                    _ if indent > base => {
                        let strip = indent.min(base + 2).max(base + 1).min(indent);
                        let content = &l[strip..];
                        match items.last_mut() {
                            Some(it) => it.push(content.to_owned()),
                            None => break,
                        }
                    }
                    _ => {
                        // A lazy continuation of the item's paragraph.
                        if starts_block(l, lines.get(i + 1).copied()) {
                            break;
                        }
                        match items.last_mut() {
                            Some(it) => it.push(l.trim().to_owned()),
                            None => break,
                        }
                    }
                }
                i += 1;
            }
            let items = items
                .iter()
                .map(|it| parse_lines(&it.iter().map(String::as_str).collect::<Vec<_>>()))
                .collect();
            out.push(Block::List {
                ordered,
                start,
                items,
            });
            continue;
        }
        let mut para = vec![];
        while i < lines.len()
            && !lines[i].trim().is_empty()
            && (para.is_empty() || !starts_block(lines[i], lines.get(i + 1).copied()))
        {
            para.push(lines[i]);
            i += 1;
        }
        // Hard breaks: two trailing spaces or a trailing backslash.
        let mut text = String::new();
        for (k, l) in para.iter().enumerate() {
            let hard = l.ends_with("  ") || l.ends_with('\\');
            let l = l.trim_start();
            let l = if hard {
                l.trim_end().trim_end_matches('\\')
            } else {
                l.trim_end()
            };
            text.push_str(l);
            if k + 1 < para.len() {
                text.push(if hard { '\n' } else { ' ' });
            }
        }
        out.push(Block::Para(text));
    }
    out
}

/// Inline content: plain text with styled ranges and links.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inline {
    pub text: String,
    /// (start, end, bold, italic, code, link).
    pub marks: Vec<(usize, usize, Mark)>,
    pub links: Vec<(usize, usize, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Bold,
    Italic,
    Code,
    Strike,
}

fn find_closing(s: &str, from: usize, delim: &str) -> Option<usize> {
    let mut i = from;
    while let Some(off) = s[i..].find(delim) {
        let at = i + off;
        if at > from && !s[..at].ends_with('\\') && !s[..at].ends_with(' ') {
            return Some(at);
        }
        i = at + delim.len();
        if i >= s.len() {
            return None;
        }
    }
    None
}

/// Parses emphasis, code, strikethrough, links, images (as "Open image" links) and autolinks.
pub fn inline(src: &str) -> Inline {
    let mut out = Inline::default();
    inline_into(src, &mut out);
    out
}

fn inline_into(src: &str, out: &mut Inline) {
    let b = src.as_bytes();
    let mut i = 0;
    while i < src.len() {
        let rest = &src[i..];
        let c = b[i];
        if c == b'\\' && i + 1 < src.len() && b[i + 1].is_ascii_punctuation() {
            out.text.push(b[i + 1] as char);
            i += 2;
            continue;
        }
        if c == b'`' {
            let n = rest.chars().take_while(|x| *x == '`').count();
            let delim = "`".repeat(n);
            if let Some(end) = rest[n..].find(&delim) {
                let code = rest[n..n + end].trim();
                let s = out.text.len();
                out.text.push_str(code);
                out.marks.push((s, out.text.len(), Mark::Code));
                i += n + end + n;
                continue;
            }
        }
        let image = c == b'!' && rest.as_bytes().get(1) == Some(&b'[');
        if c == b'[' || image {
            let open = if image { 2 } else { 1 };
            if let Some(close) = rest[open..].find("](") {
                let label = &rest[open..open + close];
                if let Some(paren) = rest[open + close + 2..].find(')') {
                    let url = rest[open + close + 2..open + close + 2 + paren]
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .to_owned();
                    let start = out.text.len();
                    if image {
                        let name = if label.trim().is_empty() {
                            host_of(&url).unwrap_or_else(|| t("assistant:message.imageReference"))
                        } else {
                            label.to_owned()
                        };
                        if url.starts_with("http://") || url.starts_with("https://") {
                            out.text
                                .push_str(&tr("assistant:message.openImage", &[("name", &name)]));
                            out.links.push((start, out.text.len(), url));
                        } else {
                            // An unsupported reference stays text; rendering never fetches it.
                            out.text.push_str(&name);
                        }
                    } else {
                        inline_into(label, out);
                        out.links.push((start, out.text.len(), url));
                    }
                    i += open + close + 2 + paren + 1;
                    continue;
                }
            }
        }
        if c == b'<'
            && let Some(end) = rest.find('>')
        {
            let inner = &rest[1..end];
            if (inner.starts_with("http://") || inner.starts_with("https://"))
                && !inner.contains(' ')
            {
                let s = out.text.len();
                out.text.push_str(inner);
                out.links.push((s, out.text.len(), inner.to_owned()));
                i += end + 1;
                continue;
            }
        }
        if (rest.starts_with("https://") || rest.starts_with("http://"))
            && (i == 0 || !b[i - 1].is_ascii_alphanumeric())
        {
            let mut end = rest
                .find(|ch: char| ch.is_whitespace() || ch == '<')
                .unwrap_or(rest.len());
            while end > 0 && rest[..end].ends_with(['.', ',', ';', ':', '!', '?', ')', '\'', '"']) {
                end -= 1;
            }
            let url = &rest[..end];
            let s = out.text.len();
            out.text.push_str(url);
            out.links.push((s, out.text.len(), url.to_owned()));
            i += end;
            continue;
        }
        for (delim, mark) in [
            ("**", Mark::Bold),
            ("__", Mark::Bold),
            ("~~", Mark::Strike),
            ("*", Mark::Italic),
            ("_", Mark::Italic),
        ] {
            if rest.starts_with(delim)
                && rest.len() > delim.len()
                && !rest[delim.len()..].starts_with(' ')
            {
                // `_` inside a word is not emphasis.
                if delim.starts_with('_') && i > 0 && b[i - 1].is_ascii_alphanumeric() {
                    continue;
                }
                if let Some(end) = find_closing(rest, delim.len(), delim) {
                    if delim.starts_with('_')
                        && rest
                            .as_bytes()
                            .get(end + delim.len())
                            .is_some_and(|x| x.is_ascii_alphanumeric())
                    {
                        continue;
                    }
                    let s = out.text.len();
                    inline_into(&rest[delim.len()..end], out);
                    out.marks.push((s, out.text.len(), mark));
                    i += end + delim.len();
                    break;
                }
            }
        }
        if i < src.len() && src[i..].as_ptr() == rest.as_ptr() {
            let ch = rest.chars().next().unwrap_or(' ');
            out.text.push(ch);
            i += ch.len_utf8();
        }
    }
}

fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty()).then(|| host.to_owned())
}

/// Styles for one look.
struct Styles {
    body: TextStyle,
    ink: Color,
    headings: [TextStyle; 4],
}

fn styles(tk: &Tk, look: Look) -> Styles {
    let heading = |rem: f32| TextStyle::new(tk.r(rem), 600.0).lh(tk.r(rem) * 1.375);
    match look {
        Look::Answer => Styles {
            body: tk.tsm(400.0).lh(tk.r(0.875) * 1.625),
            ink: tk.fg.alpha(0.85),
            headings: [heading(1.125), heading(1.0), heading(0.875), heading(0.875)],
        },
        Look::Reasoning => Styles {
            body: tk.txs(400.0).lh(tk.r(1.5)),
            ink: tk.muted_fg,
            headings: [heading(0.875), heading(0.875), heading(0.75), heading(0.75)],
        },
    }
}

/// A paragraph-like run of inline content as one rich text element.
fn inline_el(tk: &Tk, src: &str, style: TextStyle, ink: Color) -> El {
    let p = inline(src);
    let mut spans = vec![];
    for (s, e, m) in &p.marks {
        if s == e {
            continue;
        }
        spans.push(match m {
            Mark::Bold => Span::new(*s, *e).weight(600.0),
            Mark::Italic => Span::new(*s, *e).italic(),
            Mark::Strike => Span::new(*s, *e).color(ink.alpha(ink.a * 0.7)),
            Mark::Code => Span::new(*s, *e)
                .mono()
                .size(style.size * 0.85)
                .bg(tk.fg.alpha(0.10), style.size * 0.3),
        });
    }
    for (s, e, _) in &p.links {
        spans.push(
            Span::new(*s, *e)
                .weight(500.0)
                .color(tk.primary)
                .underline(tk.primary),
        );
    }
    let mut el = rich(p.text, spans).font(style).color(ink).selectable(true);
    for (s, e, url) in p.links {
        el = el.link(Link::new(s..e, url.clone()).help(url));
    }
    el.on_link(|a: &mut App, url: &str, _tx: &mut Tx| open_link(a, url))
}

/// Renders `content`. `key` scopes the copy feedback of its code blocks.
pub fn view(app: &App, tk: &Tk, key: &str, content: &str, look: Look) -> El {
    let st = styles(tk, look);
    blocks(app, tk, key, &parse(content), &st, &mut 0)
}

fn blocks(
    app: &App,
    tk: &Tk,
    key: &str,
    list: &[Block],
    st: &Styles,
    code_index: &mut usize,
) -> El {
    let mut c = col().w_full().min_w(0.0);
    for (i, b) in list.iter().enumerate() {
        let gap = if i == 0 {
            0.0
        } else {
            match b {
                Block::Heading(1, _) => tk.r(1.5),
                Block::Heading(2, _) => tk.r(1.25),
                Block::Heading(..) => tk.r(1.0),
                Block::Code { .. } | Block::Table { .. } | Block::Quote(_) => tk.r(1.0),
                Block::Rule => tk.r(1.5),
                _ => match list.get(i - 1) {
                    Some(Block::Heading(..)) => tk.r(0.5),
                    Some(Block::Code { .. } | Block::Table { .. } | Block::Quote(_)) => tk.r(1.0),
                    Some(Block::Rule) => tk.r(1.5),
                    _ => tk.r(0.75),
                },
            }
        };
        c = c.child(block(app, tk, key, b, st, code_index).mt(gap));
    }
    c
}

fn block(app: &App, tk: &Tk, key: &str, b: &Block, st: &Styles, code_index: &mut usize) -> El {
    match b {
        Block::Heading(level, s) => {
            let style = st.headings[(*level as usize).clamp(1, 4) - 1];
            let ink = if *level >= 4 { tk.muted_fg } else { st.ink };
            inline_el(tk, s, style, ink).role("heading").w_full()
        }
        Block::Para(s) => inline_el(tk, s, st.body, st.ink).w_full(),
        Block::Rule => el().w_full().h(1.0).fill(tk.border).a11y_hidden(),
        Block::Quote(inner) => col()
            .w_full()
            .pl(tk.r(1.0))
            .border_sides([0.0, 0.0, 0.0, 2.0], tk.border)
            .child(blocks(
                app,
                tk,
                key,
                inner,
                &Styles {
                    body: st.body,
                    ink: tk.muted_fg,
                    headings: st.headings,
                },
                code_index,
            )),
        Block::List {
            ordered,
            start,
            items,
        } => {
            let mut l = col().w_full().gap(tk.r(0.375)).role("list");
            for (n, item) in items.iter().enumerate() {
                let marker = if *ordered {
                    format!("{}.", start + n as u64)
                } else {
                    "•".to_owned()
                };
                l = l.child(
                    row()
                        .w_full()
                        .role("listitem")
                        .child(
                            row().w(tk.r(1.25)).justify_end().pr(tk.r(0.375)).child(
                                text(marker)
                                    .font(st.body)
                                    .color(if *ordered { st.ink } else { tk.muted_fg })
                                    .a11y_hidden(),
                            ),
                        )
                        .child(
                            col()
                                .grow(1.0)
                                .min_w(0.0)
                                .pl(tk.r(0.25))
                                .child(blocks(app, tk, key, item, st, code_index)),
                        ),
                );
            }
            l
        }
        Block::Code { lang, text: code } => {
            let index = *code_index;
            *code_index += 1;
            code_block(
                app,
                tk,
                &format!("{key}-code-{index}"),
                lang.as_deref(),
                code,
            )
        }
        Block::Table { head, align, rows } => {
            // `overflow-x-auto rounded-lg border` around a `min-w-full` auto table: headers don't
            // wrap (`whitespace-nowrap`) and body cells wrap at words, so a column is at least its
            // header or its longest word wide; past the bubble's width the table scrolls.
            let head_style = TextStyle {
                weight: 500.0,
                ..st.body
            };
            let body_style = st.body.features("'tnum' 1");
            let measure = |s: &str, style: TextStyle| rune::text::measure(s, &[], style, None).0;
            let least: Vec<f32> = (0..head.len())
                .map(|k| {
                    let words = rows
                        .iter()
                        .filter_map(|r| r.get(k))
                        .flat_map(|v| v.split_whitespace())
                        .map(|w| measure(w.trim_matches(['*', '`', '_']), body_style))
                        .fold(0.0, f32::max);
                    head.get(k)
                        .map(|h| measure(h, head_style))
                        .unwrap_or(0.0)
                        .max(words)
                        + tk.r(1.5)
                })
                .collect();
            let table_key = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                (head, rows).hash(&mut h);
                format!("{key}-table-{:x}", h.finish())
            };
            let total: f32 = least.iter().sum();
            let table_w = crate::ui::table::width(&table_key, total, total);
            let cell = |s: &str, k: usize, header: bool| {
                let style = if header { head_style } else { body_style };
                let mut c = row()
                    .w(0.0)
                    .grow(1.0)
                    .min_w(least.get(k).copied().unwrap_or(0.0))
                    .px(tk.r(0.75))
                    .py(tk.r(0.5));
                c = match align.get(k).copied().unwrap_or(Cell::Left) {
                    Cell::Right => c.justify_end(),
                    Cell::Center => c.justify_center(),
                    Cell::Left => c,
                };
                c.child(inline_el(tk, s, style, tk.fg))
            };
            let mut hr = row().w_full().fill(tk.muted.alpha(0.7));
            for (k, h) in head.iter().enumerate() {
                hr = hr.child(cell(h, k, true));
            }
            let mut t = col().w(table_w).role("table").child(hr);
            for r in rows {
                let mut line = row().w_full().border_sides([1.0, 0.0, 0.0, 0.0], tk.border);
                for (k, v) in r.iter().enumerate() {
                    line = line.child(cell(v, k, false));
                }
                t = t.child(line);
            }
            // The scroll region is named by its columns (the web's box has no name of its own).
            let scroller = crate::ui::table::hscroll(&table_key, head.join(", "), total, t);
            col()
                .w_full()
                .min_w(0.0)
                .radius(tk.r(0.5))
                .border(1.0, tk.border)
                .border_layout()
                .clip_rounded(tk.r(0.5))
                .child(scroller)
        }
    }
}

/// `ChatCodeBlock`: a header with the language (or "Code") and Copy, over the code.
fn code_block(app: &App, tk: &Tk, copy_key: &str, lang: Option<&str>, code: &str) -> El {
    let is_copied = app
        .assistant
        .thread
        .copied
        .as_ref()
        .is_some_and(|(k, until)| k == copy_key && *until > app.clock);
    let label = if is_copied {
        t("assistant:message.copied")
    } else {
        t("assistant:message.copy")
    };
    let source = code.to_owned();
    let ck = copy_key.to_owned();
    let header = row()
        .w_full()
        .gap(tk.r(0.5))
        .center_y()
        .pl(tk.r(0.75))
        .pr(tk.r(0.375))
        .py(tk.r(0.25))
        .fill(tk.muted.alpha(0.5))
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border.alpha(tk.border.a * 0.6))
        .child(
            text(
                lang.map(str::to_owned)
                    .unwrap_or_else(|| t("assistant:message.code"))
                    .to_uppercase(),
            )
            .font(tk.t2xs(400.0).mono().tracking(0.025 * tk.r(0.694)))
            .color(tk.muted_fg)
            .grow(1.0),
        )
        .child(
            row()
                .key(format!("{copy_key}-btn"))
                .gap(tk.r(0.25))
                .center_y()
                .radius(tk.r(0.375))
                .px(tk.r(0.375))
                .py(tk.r(0.25))
                .hover(tk.muted)
                .role("button")
                .label(label.clone())
                .help(label.clone())
                .focusable(true)
                .on_press_tx(move |a: &mut App, tx: &mut Tx| {
                    crate::settings::effects::copy(a, tx, source.clone());
                    super::mark_copied(a, tx, &ck);
                })
                .child(
                    ic(
                        if is_copied { &I::CHECK } else { &I::COPY },
                        tk.r(0.75),
                        tk.muted_fg,
                    )
                    .a11y_hidden(),
                )
                .child(text(label).font(tk.t2xs(500.0)).color(tk.muted_fg)),
        );
    col()
        .key(copy_key.to_owned())
        .w_full()
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.muted.alpha(0.7))
        .clip_rounded(tk.r(0.5))
        .child(header)
        .child(
            row().w_full().scroll_x().padding(tk.r(0.75)).child(
                text(code.to_owned())
                    .font(tk.txs(400.0).mono().lh(tk.r(0.75) * 1.625))
                    .color(tk.fg)
                    .selectable(true),
            ),
        )
}

/// A link in model output, pressed: app routes navigate in place; web pages open in the
/// system browser (the user's explicit action, as the web's `target="_blank"`).
pub fn open_link(app: &mut App, url: &str) {
    let url = url.trim();
    if url.starts_with('/') && !url.starts_with("//") {
        let path = url.split(['?', '#']).next().unwrap_or("");
        if ["/overview", "/reports", "/transactions", "/review"].contains(&path) {
            app.navigate(url);
        }
    } else if url.starts_with("https://") || url.starts_with("http://") {
        crate::txdetail::open_external(url);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_parse_like_gfm() {
        let b = parse(
            "# Title\n\nA *b* line\nwraps.\n\n- one\n- two\n  - nested\n\n1. first\n2. second\n\n> quote\n\n```sh\nkassiber journals process\n```\n\n| a | b |\n| --- | ---: |\n| 1 | 2 |\n\n---\n",
        );
        assert_eq!(b[0], Block::Heading(1, "Title".into()));
        assert_eq!(b[1], Block::Para("A *b* line wraps.".into()));
        match &b[2] {
            Block::List {
                ordered: false,
                items,
                ..
            } => {
                assert_eq!(items.len(), 2);
                assert!(
                    matches!(items[1].get(1), Some(Block::List { .. })),
                    "{:?}",
                    items[1]
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(
            matches!(&b[3], Block::List { ordered: true, start: 1, items } if items.len() == 2)
        );
        assert!(matches!(&b[4], Block::Quote(_)));
        assert_eq!(
            b[5],
            Block::Code {
                lang: Some("sh".into()),
                text: "kassiber journals process".into()
            }
        );
        assert!(
            matches!(&b[6], Block::Table { head, align, rows } if head.len() == 2 && align[1] == Cell::Right && rows.len() == 1)
        );
        assert_eq!(b[7], Block::Rule);
    }

    #[test]
    fn inline_marks_links_and_images() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        let p = inline("a **bold** `code` [Reports](/reports) and https://example.org.");
        assert_eq!(p.text, "a bold code Reports and https://example.org.");
        assert!(p.marks.contains(&(2, 6, Mark::Bold)));
        assert!(p.marks.contains(&(7, 11, Mark::Code)));
        assert_eq!(
            p.links,
            vec![
                (12, 19, "/reports".into()),
                (24, 43, "https://example.org".into())
            ]
        );
        let img = inline("![chart](https://example.org/a.png)");
        assert_eq!(img.text, "Open image: chart");
        assert_eq!(img.links[0].2, "https://example.org/a.png");
        assert_eq!(inline("![x](file:///etc/passwd)").links, vec![]);
        assert_eq!(inline("snake_case_name").text, "snake_case_name");
        assert_eq!(inline("\\*not\\*").text, "*not*");
    }

    #[test]
    fn unfinished_markdown_stays_text() {
        assert_eq!(inline("an **open").text, "an **open");
        assert_eq!(
            parse("```\nstill streaming"),
            vec![Block::Code {
                lang: None,
                text: "still streaming".into()
            }]
        );
    }
}
