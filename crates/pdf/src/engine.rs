//! A sealed Typst world: embedded templates, bundled fonts, in-memory images.
//!
//! The world answers only for the files the caller hands it. It never scans
//! system fonts, reads the file system, resolves packages or touches the
//! network, and its clock is the report's own timestamp, so the same input
//! renders the same bytes in every process.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::LazyLock;
use typst::diag::{FileError, FileResult, SourceDiagnostic};
use typst::foundations::{Bytes, Datetime, Dict, Duration, Smart, Value};
use typst::layout::{Abs, Frame, FrameItem, Point, Size, Transform};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World, WorldExt};
use typst_layout::PagedDocument;

/// DM Sans and JetBrains Mono, with Liberation Sans and, for Japanese
/// books' kana and kanji, Noto Sans JP only as glyph fallbacks.
const FONTS: [&[u8]; 5] = [
    include_bytes!("../fonts/DMSans-Regular.ttf"),
    include_bytes!("../fonts/DMSans-Medium.ttf"),
    include_bytes!("../fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../fonts/LiberationSans-Regular.ttf"),
    include_bytes!("../fonts/NotoSansJP-Regular.otf"),
];
/// The shared report design every report template imports.
const DESIGN: &str = include_str!("../templates/report.typ");
/// The main file of a report: the design applied to the input document.
pub const REPORT: &str = "#import \"report.typ\": report\n#report(sys.inputs.data)\n";

static BOOK: LazyLock<(Vec<Font>, LazyHash<FontBook>)> = LazyLock::new(|| {
    let fonts: Vec<Font> = FONTS
        .iter()
        .map(|bytes| Font::new(Bytes::new(*bytes), 0).expect("bundled font"))
        .collect();
    let book = LazyHash::new(FontBook::from_fonts(&fonts));
    (fonts, book)
});

/// A UTC timestamp for the document metadata and the world's clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl Stamp {
    /// The leading `YYYY-MM-DDTHH:MM:SS` of an ISO timestamp.
    pub fn parse(raw: &str) -> Option<Self> {
        let digits = |range: std::ops::Range<usize>| -> Option<u32> {
            let text = raw.get(range)?;
            text.bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| text.parse().ok())?
        };
        if raw.len() < 19 || &raw[4..5] != "-" || &raw[7..8] != "-" || &raw[13..14] != ":" {
            return None;
        }
        Some(Self {
            year: digits(0..4)? as i32,
            month: digits(5..7)? as u8,
            day: digits(8..10)? as u8,
            hour: digits(11..13)? as u8,
            minute: digits(14..16)? as u8,
            second: digits(17..19)? as u8,
        })
    }
    fn datetime(self) -> Option<Datetime> {
        Datetime::from_ymd_hms(
            self.year,
            self.month,
            self.day,
            self.hour,
            self.minute,
            self.second,
        )
    }
}

/// One report to render.
pub struct Document<'a> {
    /// The main template source.
    pub template: &'a str,
    /// The presentation model, exposed to the template as `sys.inputs.data`.
    /// It is handed over as Typst values, never as JSON: the product links
    /// serde_json with `arbitrary_precision`, under which Typst's own JSON
    /// decoder turns every number into a dictionary.
    pub data: kassiber_pyjson::Value,
    /// In-memory files the template may load by name, such as SVG images.
    pub files: Vec<(String, Vec<u8>)>,
    /// The document date and the world's clock.
    pub created: Stamp,
}

/// A rendered PDF with its page geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    pub bytes: Vec<u8>,
    /// Page sizes in points, in order.
    pub pages: Vec<(f64, f64)>,
    /// Text the layout placed past a page edge outside any clipping box,
    /// which no reader sees (a table wider than its page), as
    /// `page N: text`. Every report keeps this empty.
    pub off_page: Vec<String>,
}

/// Collect the text runs of `frame` that start or end past `page`'s edges.
/// A group that clips its content (a fixed-size box) is the template's own
/// choice and is not followed.
fn off_page(frame: &Frame, ts: Transform, page: Size, number: usize, out: &mut Vec<String>) {
    let slack = Abs::pt(0.5);
    let outside =
        |p: Point| p.x < -slack || p.y < -slack || p.x > page.x + slack || p.y > page.y + slack;
    for (pos, item) in frame.items() {
        match item {
            FrameItem::Group(group) if group.clip.is_none() => {
                let inner =
                    ts.pre_concat(Transform::translate(pos.x, pos.y).pre_concat(group.transform));
                off_page(&group.frame, inner, page, number, out);
            }
            FrameItem::Text(text) => {
                let end = Point::new(pos.x + text.width(), pos.y);
                if outside(pos.transform(ts)) || outside(end.transform(ts)) {
                    out.push(format!("page {number}: {}", text.text));
                }
            }
            _ => {}
        }
    }
}

struct SealedWorld {
    library: LazyHash<Library>,
    main: Source,
    design: Source,
    files: HashMap<FileId, Bytes>,
    today: Option<Datetime>,
}

fn typst_value(v: &kassiber_pyjson::Value) -> Value {
    use kassiber_pyjson::{Key, Value as Py};
    match v {
        Py::Null => Value::None,
        Py::Bool(b) => Value::Bool(*b),
        Py::Int(n) => i64::try_from(n.clone())
            .map(Value::Int)
            .unwrap_or_else(|_| Value::Float(n.to_string().parse().unwrap_or(f64::NAN))),
        Py::Float(f) => Value::Float(*f),
        Py::Decimal(d) => Value::Str(d.to_canonical().into()),
        Py::Str(text) => Value::Str(
            text.code_points()
                .map(|c| {
                    char::from_u32(c)
                        .filter(|c| covered(*c))
                        .unwrap_or('\u{fffd}')
                })
                .collect::<String>()
                .into(),
        ),
        Py::Bytes(_) => Value::None,
        Py::List(items) => Value::Array(items.iter().map(typst_value).collect()),
        Py::Dict(pairs) => {
            let mut dict = Dict::new();
            for (key, value) in pairs {
                let key = match key {
                    Key::Str(k) => k
                        .code_points()
                        .filter_map(char::from_u32)
                        .collect::<String>(),
                    other => format!("{other:?}"),
                };
                dict.insert(key.into(), typst_value(value));
            }
            Value::Dict(dict)
        }
    }
}

/// Whether a bundled font draws `c`. Anything else becomes U+FFFD (which
/// JetBrains Mono draws), so a label in an unbundled script never renders as
/// a missing glyph. Line breaks and tabs are layout, not glyphs.
pub fn covered(c: char) -> bool {
    matches!(c, '\n' | '\t' | '\r')
        || BOOK
            .0
            .iter()
            .any(|font| font.info().coverage.contains(c as u32))
}

fn file_id(name: &str) -> FileId {
    FileId::new(RootedPath::new(
        VirtualRoot::Project,
        VirtualPath::new(name).expect("embedded file name"),
    ))
}

fn missing(id: FileId) -> FileError {
    FileError::NotFound(PathBuf::from(id.vpath().get_with_slash()))
}

impl World for SealedWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        &BOOK.1
    }
    fn main(&self) -> FileId {
        self.main.id()
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() {
            Ok(self.main.clone())
        } else if id == self.design.id() {
            Ok(self.design.clone())
        } else {
            Err(missing(id))
        }
    }
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        self.files.get(&id).cloned().ok_or_else(|| missing(id))
    }
    fn font(&self, index: usize) -> Option<Font> {
        BOOK.0.get(index).cloned()
    }
    fn today(&self, _: Option<Duration>) -> Option<Datetime> {
        self.today
    }
}

fn describe(world: &SealedWorld, errors: &[SourceDiagnostic]) -> String {
    errors
        .iter()
        .map(|e| {
            let at = e
                .span
                .id()
                .and_then(|id| world.source(id).ok())
                .and_then(|source| {
                    let range = world.range(e.span)?;
                    let line = source.text().get(..range.start)?.matches('\n').count() + 1;
                    Some(format!(
                        " ({}:{line})",
                        source.id().vpath().get_with_slash()
                    ))
                })
                .unwrap_or_default();
            format!("{}{at}", e.message)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Compile `document` and write its PDF. Any Typst error or warning is a
/// renderer failure: a template must never silently drop content or a glyph.
pub fn render(document: Document<'_>) -> Result<Rendered, String> {
    let created = document
        .created
        .datetime()
        .ok_or_else(|| "invalid report timestamp".to_owned())?;
    let mut inputs = Dict::new();
    inputs.insert("data".into(), typst_value(&document.data));
    let world = SealedWorld {
        library: LazyHash::new(Library::builder().with_inputs(inputs).build()),
        main: Source::new(file_id("main.typ"), document.template.to_owned()),
        design: Source::new(file_id("report.typ"), DESIGN.to_owned()),
        files: document
            .files
            .into_iter()
            .map(|(name, bytes)| (file_id(&name), Bytes::new(bytes)))
            .collect(),
        today: Some(created),
    };
    let compiled = typst::compile::<PagedDocument>(&world);
    let paged = compiled
        .output
        .map_err(|errors| describe(&world, &errors))?;
    if !compiled.warnings.is_empty() {
        return Err(describe(&world, &compiled.warnings));
    }
    let options = typst_pdf::PdfOptions {
        ident: Smart::Auto,
        creator: Smart::Custom(Some("Kassiber".into())),
        timestamp: Some(typst_pdf::Timestamp::new_utc(created)),
        ..Default::default()
    };
    let bytes = typst_pdf::pdf(&paged, &options).map_err(|errors| describe(&world, &errors))?;
    let pages = paged
        .pages()
        .iter()
        .map(|page| {
            let size = page.frame.size();
            (size.x.to_pt(), size.y.to_pt())
        })
        .collect();
    let mut outside = Vec::new();
    for (index, page) in paged.pages().iter().enumerate() {
        let size = page.frame.size();
        off_page(
            &page.frame,
            Transform::identity(),
            size,
            index + 1,
            &mut outside,
        );
    }
    Ok(Rendered {
        bytes,
        pages,
        off_page: outside,
    })
}
