//! The fixed-pitch text report (`kassiber/pdf_report.py`), byte for byte.
//!
//! Lines are transliterated to Latin-1 for the base-14 Courier font, wrapped
//! like Python's `textwrap.wrap` with the writer's options, paginated and
//! written as an uncompressed PDF 1.4 file without dates or identifiers.

const PAGE_WIDTH: u32 = 842;
const PAGE_HEIGHT: u32 = 595;
const LEFT_MARGIN: u32 = 40;
const TOP_MARGIN: u32 = 42;
const BOTTOM_MARGIN: u32 = 42;
const FONT_SIZE: u32 = 9;
const LINE_HEIGHT: u32 = 11;
/// `textwrap` width of one rendered line.
pub const MAX_LINE_CHARS: usize = 138;
/// `int((PAGE_HEIGHT - TOP_MARGIN - BOTTOM_MARGIN) / LINE_HEIGHT)`.
pub const LINES_PER_PAGE: usize =
    ((PAGE_HEIGHT - TOP_MARGIN - BOTTOM_MARGIN) / LINE_HEIGHT) as usize;

/// `_TRANSLITERATIONS` for characters outside Latin-1.
fn transliteration(c: char) -> Option<&'static str> {
    Some(match c {
        '€' => "EUR",
        '₿' => "BTC",
        '↔' => "<->",
        '→' => "->",
        '←' => "<-",
        '—' | '–' | '\u{2011}' => "-",
        '⚠' => "(!)",
        '…' => "...",
        '’' | '‘' => "'",
        '“' | '”' | '„' => "\"",
        '‚' => ",",
        '≈' => "~",
        '•' => "-",
        _ => return None,
    })
}

/// `_ascii_text`: transliterate, then replace anything outside Latin-1 by `?`.
pub fn ascii_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match transliteration(c) {
            Some(text) => out.push_str(text),
            None if (c as u32) <= 0xff => out.push(c),
            None => out.push('?'),
        }
    }
    out
}

fn escape(raw: &str) -> String {
    ascii_text(raw)
        .replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

fn ellipsize(raw: &str, width: usize) -> String {
    let text = ascii_text(raw);
    let count = text.chars().count();
    if count <= width {
        return text;
    }
    if width <= 3 {
        return text.chars().take(width).collect();
    }
    text.chars().take(width - 3).collect::<String>() + "..."
}

/// `str.rstrip()` over the Latin-1 text the table cells are reduced to.
fn rstrip(text: &str) -> &str {
    text.trim_end_matches(|c: char| kassiber_pytext::is_space(c as u32))
}

/// `format_table`: a header, a dashed rule and one line per row. Cells are
/// paired with widths like Python's `zip`, so extra cells are dropped.
pub fn format_table<S: AsRef<str>>(
    headers: &[S],
    rows: &[Vec<String>],
    widths: &[usize],
    right: &[usize],
) -> Vec<String> {
    let render = |values: &mut dyn Iterator<Item = &str>| {
        let cells: Vec<String> = values
            .zip(widths)
            .enumerate()
            .map(|(index, (value, &width))| {
                let text = ellipsize(value, width);
                let padding = " ".repeat(width.saturating_sub(text.chars().count()));
                if right.contains(&index) {
                    padding + &text
                } else {
                    text + &padding
                }
            })
            .collect();
        rstrip(&cells.join("  ")).to_owned()
    };
    let mut lines = vec![render(&mut headers.iter().map(AsRef::as_ref))];
    let rule: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
    lines.push(rstrip(&rule.join("  ")).to_owned());
    for row in rows {
        lines.push(render(&mut row.iter().map(String::as_str)));
    }
    lines
}

/// `str.expandtabs(8)`: columns restart after `\n` and `\r`.
fn expand_tabs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut column = 0;
    for c in text.chars() {
        match c {
            '\t' => {
                let spaces = 8 - column % 8;
                out.extend(std::iter::repeat_n(' ', spaces));
                column += spaces;
            }
            '\n' | '\r' => {
                out.push(c);
                column = 0;
            }
            _ => {
                out.push(c);
                column += 1;
            }
        }
    }
    out
}

/// `textwrap`'s whitespace: `\t\n\x0b\x0c\r` and space.
fn wrap_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\x0b' | '\x0c' | '\r' | ' ')
}

/// `textwrap.wrap(text, width, replace_whitespace=False,
/// drop_whitespace=False, break_long_words=True, break_on_hyphens=False)`.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let text = expand_tabs(text);
    // `wordsep_simple_re.split`: alternating runs of whitespace and words.
    let mut chunks: Vec<Vec<char>> = Vec::new();
    for c in text.chars() {
        match chunks.last_mut() {
            Some(chunk) if wrap_space(chunk[0]) == wrap_space(c) => chunk.push(c),
            _ => chunks.push(vec![c]),
        }
    }
    chunks.reverse();
    let mut lines = Vec::new();
    while !chunks.is_empty() {
        let mut line: Vec<char> = Vec::new();
        while let Some(chunk) = chunks.last() {
            if line.len() + chunk.len() > width {
                break;
            }
            line.extend(chunks.pop().unwrap());
        }
        if let Some(chunk) = chunks.last_mut() {
            if chunk.len() > width {
                let end = width - line.len();
                line.extend(chunk.drain(..end));
            }
        }
        if !line.is_empty() {
            lines.push(line.into_iter().collect());
        }
    }
    lines
}

/// `wrap_report_lines`.
pub fn wrap_report_lines<S: AsRef<str>>(lines: &[S]) -> Vec<String> {
    let mut wrapped = Vec::new();
    for line in lines {
        let text = ascii_text(line.as_ref());
        if text.is_empty() {
            wrapped.push(String::new());
            continue;
        }
        let pieces = wrap(&text, MAX_LINE_CHARS);
        if pieces.is_empty() {
            wrapped.push(String::new());
        } else {
            wrapped.extend(pieces);
        }
    }
    wrapped
}

/// A rendered text report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPdf {
    pub bytes: Vec<u8>,
    pub pages: usize,
}

fn latin1(text: &str) -> Vec<u8> {
    // Every character has passed through `ascii_text`.
    text.chars().map(|c| c as u32 as u8).collect()
}

/// `write_text_pdf`'s document bytes for `title` and report `lines`.
pub fn render<S: AsRef<str>>(title: &str, lines: &[S]) -> TextPdf {
    let wrapped = wrap_report_lines(lines);
    let mut pages: Vec<&[String]> = wrapped.chunks(LINES_PER_PAGE).collect();
    if pages.is_empty() {
        pages.push(&[]);
    }
    let mut objects: Vec<Vec<u8>> = vec![Vec::new(), Vec::new()];
    let font = 3;
    objects.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>".to_vec());
    let info = 4;
    objects.push(latin1(&format!(
        "<< /Title ({}) /Producer ({}) >>",
        escape(title),
        escape("kassiber pdf report")
    )));
    let start_y = PAGE_HEIGHT - TOP_MARGIN - FONT_SIZE;
    let mut ids = Vec::new();
    for page in &pages {
        let mut commands = vec![
            "BT".to_owned(),
            format!("/F1 {FONT_SIZE} Tf"),
            format!("{LINE_HEIGHT} TL"),
            format!("{LEFT_MARGIN} {start_y} Td"),
        ];
        for (index, line) in page.iter().enumerate() {
            if index > 0 {
                commands.push("T*".into());
            }
            commands.push(format!("({}) Tj", escape(line)));
        }
        commands.push("ET".into());
        let stream = latin1(&commands.join("\n"));
        let mut content = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
        content.extend(stream);
        content.extend(b"\nendstream");
        objects.push(content);
        let content = objects.len();
        objects.push(Vec::new());
        ids.push((objects.len(), content));
    }
    let kids: Vec<String> = ids.iter().map(|(page, _)| format!("{page} 0 R")).collect();
    objects[1] = format!(
        "<< /Type /Pages /Count {} /Kids [{}] >>",
        ids.len(),
        kids.join(" ")
    )
    .into_bytes();
    objects[0] = b"<< /Type /Catalog /Pages 2 0 R >>".to_vec();
    for (page, content) in &ids {
        objects[page - 1] = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_WIDTH} {PAGE_HEIGHT}] \
             /Resources << /Font << /F1 {font} 0 R >> >> /Contents {content} 0 R >>"
        )
        .into_bytes();
    }
    let mut output = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(output.len());
        output.extend(format!("{} 0 obj\n", index + 1).into_bytes());
        output.extend(object);
        output.extend(b"\nendobj\n");
    }
    let xref = output.len();
    let size = objects.len() + 1;
    output.extend(format!("xref\n0 {size}\n").into_bytes());
    output.extend(b"0000000000 65535 f \n");
    for offset in offsets {
        output.extend(format!("{offset:010} 00000 n \n").into_bytes());
    }
    output.extend(
        format!(
            "trailer\n<< /Size {size} /Root 1 0 R /Info {info} 0 R >>\nstartxref\n{xref}\n%%EOF\n"
        )
        .into_bytes(),
    );
    TextPdf {
        bytes: output,
        pages: ids.len(),
    }
}
