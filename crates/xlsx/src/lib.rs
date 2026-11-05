//! Render a workbook entirely in RAM. This crate has no database, clock,
//! publication or report-admission authority; callers supply reviewed content.
use rust_xlsxwriter::{
    ConditionalFormatText, ConditionalFormatTextRule, DocProperties, ExcelDateTime, Format,
    FormatAlign, FormatBorder, FormatUnderline, Formula, Url, Workbook, XlsxError,
};
use serde::{Deserialize, Serialize};
mod package;
pub use package::archive;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub title: String,
    pub subject: String,
    pub author: String,
    pub comments: String,
    /// ISO 8601, supplied by the export's clock. Both core timestamps use it.
    /// Never read the rendering machine's wall clock.
    pub created: String,
    pub formats: Vec<Style>,
    pub sheets: Vec<Sheet>,
    #[serde(default)]
    pub names: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub font_size: Option<f64>,
    pub font_color: Option<String>,
    pub bg_color: Option<String>,
    pub num_format: Option<String>,
    pub valign: Option<String>,
    pub align: Option<String>,
    pub text_wrap: bool,
    pub underline: u8,
    pub border: u8,
    pub bottom: u8,
}

impl Style {
    fn format(&self) -> Result<Format, XlsxError> {
        let mut out = Format::new();
        if self.bold {
            out = out.set_bold();
        }
        if self.italic {
            out = out.set_italic();
        }
        if let Some(n) = self.font_size {
            out = out.set_font_size(n);
        }
        if let Some(s) = &self.font_color {
            out = out.set_font_color(s.as_str());
        }
        if let Some(s) = &self.bg_color {
            out = out.set_background_color(s.as_str());
        }
        if let Some(s) = &self.num_format {
            out = out.set_num_format(s);
        }
        for align in [&self.align, &self.valign].into_iter().flatten() {
            out = out.set_align(match align.as_str() {
                "left" => FormatAlign::Left,
                "center" => FormatAlign::Center,
                "right" => FormatAlign::Right,
                "top" => FormatAlign::Top,
                "vcenter" => FormatAlign::VerticalCenter,
                "bottom" => FormatAlign::Bottom,
                _ => return Err(invalid("unsupported alignment")),
            });
        }
        if self.text_wrap {
            out = out.set_text_wrap();
        }
        match self.underline {
            0 => (),
            1 => out = out.set_underline(FormatUnderline::Single),
            _ => return Err(invalid("unsupported underline")),
        }
        match self.border {
            0 => (),
            1 => out = out.set_border(FormatBorder::Thin),
            _ => return Err(invalid("unsupported border")),
        }
        match self.bottom {
            0 => (),
            1 => out = out.set_border_bottom(FormatBorder::Thin),
            _ => return Err(invalid("unsupported bottom border")),
        }
        Ok(out)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Sheet {
    pub name: String,
    pub operations: Vec<Operation>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Cell {
    Blank,
    String(String),
    Number(f64),
    Boolean(bool),
    Formula { expression: String, result: String },
    Link { url: String, text: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Cell {
        row: u32,
        col: u16,
        value: Cell,
        style: usize,
    },
    Merge {
        first_row: u32,
        first_col: u16,
        last_row: u32,
        last_col: u16,
        text: String,
        style: usize,
    },
    Column {
        first: u16,
        last: u16,
        width: f64,
    },
    Row {
        row: u32,
        height: f64,
    },
    Freeze {
        row: u32,
        col: u16,
    },
    Autofilter {
        first_row: u32,
        first_col: u16,
        last_row: u32,
        last_col: u16,
    },
    Landscape,
    Fit {
        width: u16,
        height: u16,
    },
    Margins {
        left: f64,
        right: f64,
        top: f64,
        bottom: f64,
    },
    Contains {
        first_row: u32,
        first_col: u16,
        last_row: u32,
        last_col: u16,
        text: String,
        style: usize,
    },
}

fn invalid(message: &str) -> XlsxError {
    XlsxError::ParameterError(message.into())
}

/// Produce complete XLSX bytes, or an error without publishing anything.
pub fn render(document: &Document) -> Result<Vec<u8>, XlsxError> {
    if document.sheets.is_empty() {
        return Err(invalid("workbook must have a sheet"));
    }
    let formats = document
        .formats
        .iter()
        .map(Style::format)
        .collect::<Result<Vec<_>, _>>()?;
    let format = |index: usize| {
        formats
            .get(index)
            .ok_or_else(|| invalid("unknown cell style"))
    };
    let created = ExcelDateTime::parse_from_str(&document.created)?;
    let properties = DocProperties::new()
        .set_title(&document.title)
        .set_subject(&document.subject)
        .set_author(&document.author)
        .set_comment(&document.comments)
        .set_creation_datetime(&created);
    let mut workbook = Workbook::new();
    workbook.set_properties(&properties);
    for (name, formula) in &document.names {
        workbook.define_name(name, formula)?;
    }
    for sheet in &document.sheets {
        let ws = workbook.add_worksheet();
        ws.set_name(&sheet.name)?;
        for operation in &sheet.operations {
            match operation {
                Operation::Cell {
                    row,
                    col,
                    value,
                    style,
                } => {
                    let fmt = format(*style)?;
                    match value {
                        Cell::Blank => {
                            ws.write_blank(*row, *col, fmt)?;
                        }
                        Cell::String(v) => {
                            // Python XlsxWriter truncates write_string at 32,767
                            // Unicode characters. No formula/URL inference here.
                            let v: String = v.chars().take(32767).collect();
                            ws.write_string_with_format(*row, *col, &v, fmt)?;
                        }
                        Cell::Number(v) => {
                            if !v.is_finite() {
                                return Err(invalid("non-finite workbook number"));
                            }
                            // XlsxWriter serializes numbers with %.16G. Match
                            // that rounding before rust_xlsxwriter's dtoa.
                            let rounded = format!("{v:.15e}")
                                .parse::<f64>()
                                .map_err(|_| invalid("invalid workbook number"))?;
                            ws.write_number_with_format(*row, *col, rounded, fmt)?;
                        }
                        Cell::Boolean(v) => {
                            ws.write_boolean_with_format(*row, *col, *v, fmt)?;
                        }
                        Cell::Formula { expression, result } => {
                            ws.write_formula_with_format(
                                *row,
                                *col,
                                Formula::new(expression).set_result(result),
                                fmt,
                            )?;
                        }
                        Cell::Link { url, text } => {
                            ws.write_url_with_format(
                                *row,
                                *col,
                                Url::new(url).set_text(text),
                                fmt,
                            )?;
                        }
                    }
                }
                Operation::Merge {
                    first_row,
                    first_col,
                    last_row,
                    last_col,
                    text,
                    style,
                } => {
                    // Python ignores a one-cell merge (including its value).
                    if first_row != last_row || first_col != last_col {
                        ws.merge_range(
                            *first_row,
                            *first_col,
                            *last_row,
                            *last_col,
                            text,
                            format(*style)?,
                        )?;
                    }
                }
                Operation::Column { first, last, width } => {
                    if first > last {
                        return Err(invalid("reversed column range"));
                    }
                    for col in *first..=*last {
                        ws.set_column_width(col, *width)?;
                    }
                }
                Operation::Row { row, height } => {
                    ws.set_row_height(*row, *height)?;
                }
                Operation::Freeze { row, col } => {
                    ws.set_freeze_panes(*row, *col)?;
                }
                Operation::Autofilter {
                    first_row,
                    first_col,
                    last_row,
                    last_col,
                } => {
                    ws.autofilter(*first_row, *first_col, *last_row, *last_col)?;
                }
                Operation::Landscape => {
                    ws.set_landscape();
                }
                Operation::Fit { width, height } => {
                    ws.set_print_fit_to_pages(*width, *height);
                }
                Operation::Margins {
                    left,
                    right,
                    top,
                    bottom,
                } => {
                    ws.set_margins(*left, *right, *top, *bottom, 0.3, 0.3);
                }
                Operation::Contains {
                    first_row,
                    first_col,
                    last_row,
                    last_col,
                    text,
                    style,
                } => {
                    ws.add_conditional_format(
                        *first_row,
                        *first_col,
                        *last_row,
                        *last_col,
                        &ConditionalFormatText::new()
                            .set_rule(ConditionalFormatTextRule::Contains(text.clone()))
                            .set_format(format(*style)?),
                    )?;
                }
            }
        }
    }
    package::restore_heights(workbook.save_to_buffer()?, &document.sheets)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document() -> Document {
        Document {
            created: "2026-01-02T03:04:05Z".into(),
            formats: vec![Style::default()],
            sheets: vec![Sheet {
                name: "Data".into(),
                operations: vec![Operation::Cell {
                    row: 0,
                    col: 0,
                    value: Cell::String("€ 😀".into()),
                    style: 0,
                }],
            }],
            ..Default::default()
        }
    }
    #[test]
    fn rendering_is_byte_deterministic() {
        let spec = document();
        let a = render(&spec).unwrap();
        assert_eq!(a, render(&spec).unwrap());
        assert!(a.starts_with(b"PK"));
    }
    #[test]
    fn malformed_workbooks_fail_without_bytes() {
        let mut spec = document();
        spec.created = "invalid".into();
        assert!(render(&spec).is_err());
        spec = document();
        spec.sheets[0].name = "bad/name".into();
        assert!(render(&spec).is_err());
        spec = document();
        spec.formats.clear();
        assert!(render(&spec).is_err());
        spec = document();
        spec.sheets[0].operations.push(Operation::Cell {
            row: 1,
            col: 0,
            value: Cell::Number(f64::INFINITY),
            style: 0,
        });
        assert!(render(&spec).is_err());
    }
}
