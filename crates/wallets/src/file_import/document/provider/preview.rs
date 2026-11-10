use super::*;

pub(super) fn max_pages(value: &Py) -> Result<usize, AppError> {
    if missing(value) {
        return Ok(8);
    }
    let number = match value {
        Py::Int(_) | Py::Str(_) => kassiber_pytext::parse_int(&rendered(value)).ok(),
        Py::Bool(v) => Some(i64::from(*v).into()),
        Py::Float(v) if v.is_finite() => Some((*v as i64).into()),
        _ => None,
    };
    let Some(number) = number else {
        return Err(AppError::new("validation", "max_pages must be an integer"));
    };
    if number < 1.into() || number > 8.into() {
        return Err(
            AppError::new("validation", "max_pages must be between 1 and 8")
                .with_details(dict([("max_pages", Py::from(8))])),
        );
    }
    Ok(number.to_string().parse().unwrap())
}
/// The book a desktop preview reads under: its consent is re-checked before
/// every request, and a detected API format is written at once. A chat's
/// reader has none; its caller records the format on the request loop.
pub(super) struct Book<'a> {
    pub(super) conn: &'a Connection,
    pub(super) request: &'a DocumentPreview,
    pub(super) consent: &'a DocumentConsent,
}

/// What one read needs beyond the provider: the checked arguments.
struct Read<'a> {
    path: &'a std::path::Path,
    threshold: &'a Decimal,
    page_limit: usize,
    pages: &'a Py,
    model: Option<&'a str>,
    expected_fiat_currency: Option<&'a str>,
}

impl Providers {
    pub fn preview(
        &self,
        conn: &Connection,
        request: &DocumentPreview,
        consent: Option<DocumentConsent>,
    ) -> Result<Py, AppError> {
        let provider = resolve_provider(conn, request.provider_name.as_deref())?;
        kassiber_ai::providers::require_contactable(&provider)?;
        let path = files::source_path(&request.source_file)?;
        let threshold = threshold(&request.confidence_threshold)?;
        let page_limit = max_pages(&request.max_pages)?;
        let consent = consent.ok_or_else(consent_error)?;
        consent.verify(conn, request, &provider)?;
        validate_local(&provider)?;
        api_key(conn, &provider)?;
        let (draft, _) = self.read_with(
            Some(Book {
                conn,
                request,
                consent: &consent,
            }),
            &provider,
            Read {
                path: &path,
                threshold: &threshold,
                page_limit,
                pages: &request.pages,
                model: request.model.as_deref(),
                expected_fiat_currency: request.expected_fiat_currency.as_deref(),
            },
            &|| lifecycle::now(conn),
        )?;
        Ok(draft)
    }

    /// `read_document(provider, source_file=..., pages=...,
    /// expected_fiat_currency=...)` for a chat: no database. The caller
    /// resolved the provider on the request loop (`local_reader`) and records
    /// the API format this returns, if the server detected one; the model
    /// call runs on the caller's thread, off the request loop.
    pub fn read_document(
        &self,
        provider: &Py,
        source_file: &str,
        pages: &Py,
        expected_fiat_currency: Option<&str>,
    ) -> Result<(Py, Option<&'static str>), AppError> {
        let path = files::source_path(source_file)?;
        let threshold = threshold(&Py::Null)?;
        let page_limit = max_pages(&Py::Null)?;
        validate_local(provider)?;
        self.read_with(
            None,
            provider,
            Read {
                path: &path,
                threshold: &threshold,
                page_limit,
                pages,
                model: None,
                expected_fiat_currency,
            },
            &|| Ok(kassiber_ai::clock::now_iso()),
        )
    }

    fn read_with(
        &self,
        book: Option<Book<'_>>,
        provider: &Py,
        read: Read<'_>,
        now: &dyn Fn() -> Result<String, AppError>,
    ) -> Result<(Py, Option<&'static str>), AppError> {
        let Read {
            path,
            threshold,
            page_limit,
            pages,
            model,
            expected_fiat_currency,
        } = read;
        let invocation = http::Invocation::new(self, book, provider);
        let models = invocation.discover()?;
        let (model, installed) = choose_model(provider, &models, model)?;
        let snapshot = files::Snapshot::new(path)?;
        let hash = files::hash(&snapshot.path)?;
        let prepared = if files::suffix(path) == ".pdf" {
            Some(super::super::pdf::prepare(
                &snapshot.path,
                page_limit,
                pages,
            )?)
        } else {
            None
        };
        let image;
        let parts = if let Some(pdf) = &prepared {
            &pdf.parts
        } else {
            image = vec![files::image_part(&snapshot.path)?];
            &image
        };
        let text = invocation.ocr(&model, parts)?;
        let detected = invocation.detected();
        let pdf_metadata = prepared.as_ref().map(|pdf| pdf.metadata.clone());
        drop(prepared);
        drop(snapshot);
        let payload = extract(&text)?;
        if files::hash(path)? != hash {
            return Err(AppError::new(
                "document_import_source_changed",
                "The OCR source changed while preview was running",
            )
            .with_hint("Preview the document again before reviewing its rows.")
            .with_details(dict([(
                "filename",
                s(path.file_name().unwrap_or_default().to_string_lossy()),
            )])));
        }
        let allowed_pages = match pdf_metadata
            .as_ref()
            .map(|pdf| field(pdf, "rendered_pages"))
        {
            Some(Py::List(pages)) => pages
                .iter()
                .map(|v| rendered(v).parse::<i64>().unwrap())
                .collect(),
            _ => vec![1],
        };
        let rows = draft_rows(
            &payload,
            &hash,
            threshold,
            expected_fiat_currency.unwrap_or_default(),
            Some(&allowed_pages),
        )?;
        let ready = rows
            .iter()
            .filter(|r| field(r, "status") == &s("ready"))
            .count();
        let count = rows.len();
        let mut result = dict([
            (
                "source",
                dict([
                    ("path", s(path.to_string_lossy())),
                    (
                        "filename",
                        s(path.file_name().unwrap_or_default().to_string_lossy()),
                    ),
                    ("media_type", s(files::mime(path))),
                    (
                        "size_bytes",
                        Py::from(std::fs::metadata(path).map_err(files::io)?.len() as i64),
                    ),
                    ("sha256", s(hash)),
                    (
                        "kind",
                        s(if pdf_metadata.is_some() {
                            "pdf"
                        } else {
                            "image"
                        }),
                    ),
                ]),
            ),
            (
                "provider",
                dict([
                    ("name", field(provider, "name").clone()),
                    ("kind", field(provider, "kind").clone()),
                ]),
            ),
            ("model", s(model)),
            (
                "installed_models",
                Py::List(installed.iter().map(s).collect()),
            ),
            ("recommendations", recommendations()),
            ("confidence_threshold", Py::Float(float(threshold))),
            (
                "expected_fiat_currency",
                expected_fiat_currency
                    .filter(|v| !v.is_empty())
                    .map_or(Py::Null, |v| {
                        s(kassiber_pytext::upper(kassiber_pytext::strip(v)))
                    }),
            ),
            ("rows", Py::List(rows)),
            (
                "summary",
                dict([
                    ("rows", Py::from(count as i64)),
                    ("ready", Py::from(ready as i64)),
                    ("quarantined", Py::from((count - ready) as i64)),
                    ("has_importable_rows", Py::Bool(ready > 0)),
                ]),
            ),
            ("created_at", s(now()?)),
        ]);
        if let Some(metadata) = pdf_metadata {
            let mut source = field(&result, "source").clone();
            set(&mut source, "pdf", metadata);
            set(&mut result, "source", source);
        }
        Ok((result, detected))
    }
}
