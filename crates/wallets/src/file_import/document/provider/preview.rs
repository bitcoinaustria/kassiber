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
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);
        out.push(ABC[(a >> 2) as usize] as char);
        out.push(ABC[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 {
            ABC[(((b & 15) << 2) | (c >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ABC[(c & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
pub(super) fn image_part(path: &Path) -> Result<Py, AppError> {
    let bytes = std::fs::read(path).map_err(files::io)?;
    Ok(dict([
        ("type", s("image_url")),
        (
            "image_url",
            dict([(
                "url",
                s(format!(
                    "data:{};base64,{}",
                    files::mime(path),
                    base64(&bytes)
                )),
            )]),
        ),
    ]))
}
impl Providers {
    pub fn preview(
        &self,
        conn: &Connection,
        request: &DocumentPreview,
        consent: Option<DocumentConsent>,
    ) -> Result<Py, AppError> {
        let path = files::source_path(&request.source_file)?;
        let threshold = threshold(&request.confidence_threshold)?;
        let _pages = max_pages(&request.max_pages)?;
        let provider = resolve_provider(conn, request.provider_name.as_deref())?;
        let consent = consent.ok_or_else(consent_error)?;
        consent.verify(conn, request, &provider)?;
        validate_local(&provider)?;
        api_key(conn, &provider)?;
        let invocation = http::Invocation::new(self, conn, request, &consent, &provider);
        let models = invocation.discover()?;
        let (model, installed) = choose_model(&provider, &models, request.model.as_deref())?;
        let snapshot = files::Snapshot::new(&path)?;
        let hash = files::hash(&snapshot.path)?;
        if files::suffix(&path) == ".pdf" {
            return Err(AppError::new(
                "not_implemented",
                "PDF OCR preparation is not registered",
            ));
        }
        let parts = vec![image_part(&snapshot.path)?];
        let text = invocation.ocr(&model, &parts)?;
        drop(snapshot);
        let payload = extract(&text)?;
        if files::hash(&path)? != hash {
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
        let rows = draft_rows(
            &payload,
            &hash,
            &threshold,
            request
                .expected_fiat_currency
                .as_deref()
                .unwrap_or_default(),
            Some(&[1]),
        )?;
        let ready = rows
            .iter()
            .filter(|r| field(r, "status") == &s("ready"))
            .count();
        let count = rows.len();
        Ok(dict([
            (
                "source",
                dict([
                    ("path", s(path.to_string_lossy())),
                    (
                        "filename",
                        s(path.file_name().unwrap_or_default().to_string_lossy()),
                    ),
                    ("media_type", s(files::mime(&path))),
                    (
                        "size_bytes",
                        Py::from(std::fs::metadata(&path).map_err(files::io)?.len() as i64),
                    ),
                    ("sha256", s(hash)),
                    ("kind", s("image")),
                ]),
            ),
            (
                "provider",
                dict([
                    ("name", field(&provider, "name").clone()),
                    ("kind", field(&provider, "kind").clone()),
                ]),
            ),
            ("model", s(model)),
            (
                "installed_models",
                Py::List(installed.iter().map(s).collect()),
            ),
            ("recommendations", recommendations()),
            ("confidence_threshold", Py::Float(float(&threshold))),
            (
                "expected_fiat_currency",
                request
                    .expected_fiat_currency
                    .as_ref()
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
            ("created_at", s(lifecycle::now(conn)?)),
        ]))
    }
}
