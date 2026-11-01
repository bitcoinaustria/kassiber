use super::*;
use std::{
    ffi::OsString,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_DIMENSION: u64 = 2400;
const MAX_PAGE_BYTES: u64 = 24 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 96 * 1024 * 1024;

pub(super) struct Prepared {
    pub(super) parts: Vec<Py>,
    pub(super) metadata: Py,
    _temp: files::Temp,
}
fn missing(tool: &str) -> AppError {
    AppError::new(
        "document_import_pdf_renderer_missing",
        format!("PDF OCR import requires Poppler's {tool}"),
    )
    .with_hint("Install poppler-utils, or export the statement pages as images and import those.")
    .with_details(dict([("tool", s(tool))]))
}
fn find(tool: &str) -> Result<PathBuf, AppError> {
    let paths = std::env::var_os("PATH").unwrap_or_default();
    for directory in std::env::split_paths(&paths) {
        let candidate = directory.join(tool);
        if candidate.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if fs::metadata(&candidate)
                    .map_err(files::io)?
                    .permissions()
                    .mode()
                    & 0o111
                    == 0
                {
                    continue;
                }
            }
            return Ok(candidate);
        }
        #[cfg(windows)]
        for suffix in std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
        {
            if [".bat", ".cmd"].contains(&suffix.to_ascii_lowercase().as_str()) {
                continue;
            }
            let candidate = directory.join(format!("{tool}{suffix}"));
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(missing(tool))
}
struct Output {
    success: bool,
    stdout: String,
    stderr: String,
}
fn capture(mut stream: impl Read) -> std::io::Result<Vec<u8>> {
    let mut result = Vec::new();
    stream.read_to_end(&mut result)?;
    Ok(result)
}
fn run(
    program: &Path,
    args: &[OsString],
    timeout: Duration,
    inspection: bool,
) -> Result<Output, AppError> {
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(files::io)?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let out = std::thread::spawn(move || capture(stdout));
    let err = std::thread::spawn(move || capture(stderr));
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(AppError::new(
                    "document_import_pdf_render_timeout",
                    if inspection {
                        "PDF inspection timed out"
                    } else {
                        "PDF rendering timed out"
                    },
                )
                .with_hint("Export only the transaction pages as PNG/JPEG and import those images.")
                .with_details(dict([("timeout_seconds", Py::from(30))])));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(files::io(e));
            }
        }
    };
    let stdout = out.join();
    let stderr = err.join();
    let stdout = stdout
        .map_err(|_| AppError::new("internal_error", "Could not capture PDF inspection output"))?
        .map_err(files::io)?;
    let stderr = stderr
        .map_err(|_| AppError::new("internal_error", "Could not capture PDF renderer output"))?
        .map_err(files::io)?;
    let status = status?;
    let decode = |bytes| {
        String::from_utf8(bytes)
            .map(|s| s.replace("\r\n", "\n").replace('\r', "\n"))
            .map_err(|e| AppError::new("internal_error", e.to_string()))
    };
    let stdout = decode(stdout)?;
    let stderr = decode(stderr)?;
    Ok(Output {
        success: status.success(),
        stdout,
        stderr,
    })
}
fn page_count(
    info: &Path,
    path: &Path,
    runner: &impl Fn(&Path, &[OsString], Duration, bool) -> Result<Output, AppError>,
) -> Result<usize, AppError> {
    let output = runner(info, &[path.as_os_str().to_owned()], TIMEOUT, true)?;
    if !output.success {
        return Err(AppError::new("document_import_pdf_render_failed","Could not inspect the PDF page count").with_hint("Check whether the PDF is password-protected, then export the transaction pages as images."));
    }
    let pattern = format!(
        r"(?m)^Pages:{w}*({d}+){w}*$",
        w = kassiber_pytext::REGEX_SPACE,
        d = kassiber_pytext::REGEX_DIGIT
    );
    let count = regex::Regex::new(&pattern)
        .unwrap()
        .captures(&output.stdout)
        .and_then(|v| kassiber_pytext::parse_int(v.get(1).unwrap().as_str()).ok())
        .and_then(|n| n.to_string().parse::<usize>().ok())
        .filter(|n| *n > 0);
    count.ok_or_else(|| {
        AppError::new(
            "document_import_pdf_render_failed",
            "Could not determine the PDF page count",
        )
        .with_hint("Export the transaction pages as PNG/JPEG and import those images.")
    })
}
pub(super) fn selected(value: &Py, total: usize, max: usize) -> Result<(Vec<i64>, bool), AppError> {
    let raw = if truthy(value) {
        kassiber_pytext::strip(&rendered(value)).to_owned()
    } else {
        String::new()
    };
    if raw.is_empty() {
        if total > max {
            return Err(AppError::new("document_import_pdf_page_selection_required","This PDF needs an explicit page range before OCR").with_hint(format!("Choose a contiguous range of at most {max} pages, for example 1-{max}, or split the statement into smaller PDFs.")).with_details(dict([("total_pages",Py::from(total as i64)),("max_pages",Py::from(max as i64))])));
        }
        return Ok(((1..=total as i64).collect(), false));
    }
    let pattern = format!(
        r"^({d}+)(?:{w}*-{w}*({d}+))?$",
        d = kassiber_pytext::REGEX_DIGIT,
        w = kassiber_pytext::REGEX_SPACE,
    );
    let expression = regex::Regex::new(&pattern).unwrap();
    let details = || dict([("pages", s(&raw)), ("total_pages", Py::from(total as i64))]);
    let Some(m) = expression.captures(&raw) else {
        return Err(AppError::new(
            "validation",
            "PDF pages must be one page or a contiguous range such as 2-6",
        )
        .with_details(details()));
    };
    let start = kassiber_pytext::parse_int(m.get(1).unwrap().as_str()).unwrap();
    let end = m.get(2).map_or_else(
        || start.clone(),
        |v| kassiber_pytext::parse_int(v.as_str()).unwrap(),
    );
    if start < 1.into() || end < start || end > (total as u64).into() {
        return Err(
            AppError::new("validation", "PDF page range is outside the document")
                .with_details(details()),
        );
    }
    let start: i64 = start.to_string().parse().unwrap();
    let end: i64 = end.to_string().parse().unwrap();
    let count = end - start + 1;
    if count > max as i64 {
        return Err(AppError::new(
            "validation",
            format!("PDF page range may contain at most {max} pages"),
        )
        .with_details(dict([
            ("pages", s(raw)),
            ("selected_pages", Py::from(count)),
            ("max_pages", Py::from(max as i64)),
        ])));
    }
    Ok(((start..=end).collect(), true))
}
fn validate(paths: &[PathBuf]) -> Result<(), AppError> {
    let mut total = 0;
    for path in paths {
        let size = fs::metadata(path).map_err(files::io)?.len();
        total += size;
        let mut header = [0; 24];
        let mut file = fs::File::open(path).map_err(files::io)?;
        let complete = match file.read_exact(&mut header) {
            Ok(()) => true,
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => false,
            Err(e) => return Err(files::io(e)),
        };
        if !complete || &header[..8] != b"\x89PNG\r\n\x1a\n" || &header[12..16] != b"IHDR" {
            return Err(AppError::new(
                "document_import_pdf_render_failed",
                "PDF renderer produced an invalid image",
            ));
        }
        let width = u64::from(u32::from_be_bytes(header[16..20].try_into().unwrap()));
        let height = u64::from(u32::from_be_bytes(header[20..24].try_into().unwrap()));
        if !(1..=MAX_PAGE_BYTES).contains(&size)
            || width < 1
            || height < 1
            || width > MAX_DIMENSION
            || height > MAX_DIMENSION
            || width * height > MAX_DIMENSION * MAX_DIMENSION
            || total > MAX_TOTAL_BYTES
        {
            return Err(AppError::new(
                "document_import_pdf_render_too_large",
                "Rendered PDF pages exceed the local OCR safety budget",
            )
            .with_hint("Export the transaction pages as smaller PNG/JPEG images and import those.")
            .with_details(dict([
                ("page_bytes", Py::from(size as i64)),
                ("width", Py::from(width as i64)),
                ("height", Py::from(height as i64)),
                ("total_bytes", Py::from(total as i64)),
                ("max_dimension", Py::from(MAX_DIMENSION as i64)),
                ("max_page_bytes", Py::from(MAX_PAGE_BYTES as i64)),
                ("max_total_bytes", Py::from(MAX_TOTAL_BYTES as i64)),
            ])));
        }
    }
    Ok(())
}
pub(super) fn prepare(path: &Path, max: usize, pages: &Py) -> Result<Prepared, AppError> {
    let ppm = find("pdftoppm")?;
    let info = find("pdfinfo")?;
    prepare_with(path, max, pages, &ppm, &info, &run)
}
fn prepare_with(
    path: &Path,
    max: usize,
    pages: &Py,
    ppm: &Path,
    info: &Path,
    runner: &impl Fn(&Path, &[OsString], Duration, bool) -> Result<Output, AppError>,
) -> Result<Prepared, AppError> {
    let total = page_count(info, path, runner)?;
    let (pages, explicit) = selected(pages, total, max)?;
    let temp = files::Temp::new("document-ocr")?;
    let prefix = temp.path().join("page");
    let args = [
        OsString::from("-png"),
        OsString::from("-r"),
        OsString::from("180"),
        OsString::from("-scale-to"),
        OsString::from("2400"),
        OsString::from("-f"),
        pages[0].to_string().into(),
        OsString::from("-l"),
        pages.last().unwrap().to_string().into(),
        path.as_os_str().to_owned(),
        prefix.as_os_str().to_owned(),
    ];
    let output = runner(ppm, &args, TIMEOUT, false)?;
    if !output.success {
        return Err(AppError::new("document_import_pdf_render_failed","Could not render PDF pages for OCR").with_hint("Open the PDF locally and export the transaction pages as PNG/JPEG, then import the images.").with_details(dict([("stderr",s(kassiber_pytext::strip(&output.stderr).chars().rev().take(2048).collect::<Vec<_>>().into_iter().rev().collect::<String>()))])));
    }
    let mut rendered = fs::read_dir(temp.path())
        .map_err(files::io)?
        .map(|p| p.map(|v| v.path()).map_err(files::io))
        .collect::<Result<Vec<_>, _>>()?;
    rendered.retain(|p| {
        p.file_name().is_some_and(|v| {
            let v = v.to_string_lossy();
            v.starts_with("page-") && v.ends_with(".png")
        })
    });
    let pattern = format!(r"-({}+)\.png$", kassiber_pytext::REGEX_DIGIT);
    let pattern = regex::Regex::new(&pattern).unwrap();
    rendered.sort_by_key(|p| {
        pattern
            .captures(&p.file_name().unwrap().to_string_lossy())
            .and_then(|m| kassiber_pytext::parse_int(m.get(1).unwrap().as_str()).ok())
            .unwrap_or_else(|| 0.into())
    });
    if rendered.len() != pages.len() {
        return Err(AppError::new("document_import_pdf_render_failed","PDF rendering did not produce the reviewed page range").with_hint("Check whether the PDF is password-protected, then export the transaction pages as images.").with_details(dict([("expected_pages",Py::from(pages.len() as i64)),("rendered_pages",Py::from(rendered.len() as i64))])));
    }
    validate(&rendered)?;
    let mut parts = vec![];
    for (page, path) in pages.iter().zip(rendered) {
        parts.push(dict([
            ("type", s("text")),
            ("text", s(format!("PDF page {page}:"))),
        ]));
        parts.push(files::image_part(&path)?);
    }
    let selection = if pages.len() == 1 {
        pages[0].to_string()
    } else {
        format!("{}-{}", pages[0], pages.last().unwrap())
    };
    let metadata = dict([
        ("total_pages", Py::from(total as i64)),
        (
            "rendered_pages",
            Py::List(pages.iter().copied().map(Py::from).collect()),
        ),
        ("complete", Py::Bool(pages.len() == total)),
        ("selection_explicit", Py::Bool(explicit)),
        ("selection", s(selection)),
    ]);
    Ok(Prepared {
        parts,
        metadata,
        _temp: temp,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn png(path: &Path, width: u32, height: u32, size: u64) {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        bytes.extend(width.to_be_bytes());
        bytes.extend(height.to_be_bytes());
        fs::write(path, bytes).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(size)
            .unwrap();
    }
    fn output(success: bool, stdout: &str, stderr: &str) -> Output {
        Output {
            success,
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }
    #[test]
    fn document_pdf_preparation_pins_arguments_order_page_labels_and_cleanup() {
        let observed = RefCell::new(None);
        let calls = RefCell::new(0);
        let prepared = prepare_with(
            Path::new("statement with spaces.pdf"),
            8,
            &s("9-12"),
            Path::new("pdftoppm"),
            Path::new("pdfinfo"),
            &|program, args, timeout, inspection| {
                assert_eq!(timeout, Duration::from_secs(30));
                *calls.borrow_mut() += 1;
                if inspection {
                    assert_eq!(program, Path::new("pdfinfo"));
                    assert_eq!(args, &[OsString::from("statement with spaces.pdf")]);
                    return Ok(output(true, "Pages: 12\n", ""));
                }
                assert_eq!(program, Path::new("pdftoppm"));
                let prefix = Path::new(args.last().unwrap());
                assert_eq!(
                    args[..10],
                    [
                        "-png",
                        "-r",
                        "180",
                        "-scale-to",
                        "2400",
                        "-f",
                        "9",
                        "-l",
                        "12",
                        "statement with spaces.pdf"
                    ]
                    .map(OsString::from)
                );
                assert_eq!(prefix.file_name().unwrap(), "page");
                assert!(prefix.starts_with(std::env::temp_dir()));
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    assert_eq!(
                        fs::metadata(prefix.parent().unwrap())
                            .unwrap()
                            .permissions()
                            .mode()
                            & 0o777,
                        0o700
                    );
                }
                *observed.borrow_mut() = Some(prefix.parent().unwrap().to_path_buf());
                for page in [12, 10, 9, 11] {
                    png(
                        &prefix.with_file_name(format!("page-{page}.png")),
                        page,
                        1,
                        24,
                    );
                }
                Ok(output(true, "", ""))
            },
        )
        .unwrap();
        assert_eq!(*calls.borrow(), 2);
        assert_eq!(field(&prepared.metadata, "selection"), &s("9-12"));
        assert_eq!(field(&prepared.metadata, "complete"), &Py::Bool(false));
        assert_eq!(
            field(&prepared.metadata, "selection_explicit"),
            &Py::Bool(true)
        );
        assert_eq!(
            field(&prepared.metadata, "rendered_pages"),
            &Py::List((9..=12).map(Py::from).collect())
        );
        assert_eq!(prepared.parts.len(), 8);
        for (index, page) in (9..=12).enumerate() {
            assert_eq!(
                field(&prepared.parts[index * 2], "text"),
                &s(format!("PDF page {page}:"))
            );
            let part = &prepared.parts[index * 2 + 1];
            let expected = files::image_part(
                &observed
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .join(format!("page-{page}.png")),
            )
            .unwrap();
            assert_eq!(*part, expected);
        }
        let directory = observed.into_inner().unwrap();
        assert!(directory.exists());
        drop(prepared);
        assert!(!directory.exists());
    }
    #[test]
    fn document_pdf_render_failures_remove_all_temporary_images() {
        for failure in [
            "exit",
            "timeout",
            "count",
            "invalid",
            "signature",
            "geometry",
        ] {
            let directory = RefCell::new(None);
            let result = prepare_with(
                Path::new("statement.pdf"),
                8,
                &Py::Null,
                Path::new("pdftoppm"),
                Path::new("pdfinfo"),
                &|_, args, _, inspection| {
                    if inspection {
                        return Ok(output(true, "Pages: 1\n", ""));
                    }
                    let prefix = Path::new(args.last().unwrap());
                    *directory.borrow_mut() = Some(prefix.parent().unwrap().to_path_buf());
                    let path = prefix.with_file_name("page-1.png");
                    if failure != "count" {
                        png(&path, if failure == "geometry" { 2401 } else { 1 }, 1, 24);
                    }
                    if failure == "invalid" {
                        fs::write(&path, b"bad png").unwrap();
                    }
                    if failure == "signature" {
                        let mut bytes = fs::read(&path).unwrap();
                        bytes[0] = 0;
                        fs::write(&path, bytes).unwrap();
                    }
                    match failure {
                        "exit" => Ok(output(false, "", " \nprivate local renderer diagnostic\n ")),
                        "timeout" => Err(AppError::new(
                            "document_import_pdf_render_timeout",
                            "PDF rendering timed out",
                        )),
                        _ => Ok(output(true, "", "")),
                    }
                },
            );
            let error = result.err().unwrap();
            assert_eq!(
                error.code,
                match failure {
                    "timeout" => "document_import_pdf_render_timeout",
                    "geometry" => "document_import_pdf_render_too_large",
                    _ => "document_import_pdf_render_failed",
                }
            );
            if failure == "exit" {
                assert_eq!(
                    field(error.details.as_ref().unwrap(), "stderr"),
                    &s("private local renderer diagnostic")
                );
            }
            assert!(!directory.into_inner().unwrap().exists());
        }
    }
    #[test]
    fn document_pdf_png_budgets_reject_geometry_page_and_total_size() {
        let root = files::Temp::new("pdf-budget").unwrap();
        let path = root.path().join("page.png");
        png(&path, 2400, 2400, MAX_PAGE_BYTES);
        validate(std::slice::from_ref(&path)).unwrap();
        for (width, height, bytes) in [
            (2401, 1, 24),
            (1, 2401, 24),
            (0, 1, 24),
            (1, 0, 24),
            (1, 1, MAX_PAGE_BYTES + 1),
        ] {
            png(&path, width, height, bytes);
            assert_eq!(
                validate(std::slice::from_ref(&path)).unwrap_err().code,
                "document_import_pdf_render_too_large"
            );
        }
        png(&path, 1, 1, MAX_PAGE_BYTES);
        validate(&vec![path.clone(); 4]).unwrap();
        assert_eq!(
            validate(&vec![path; 5]).unwrap_err().code,
            "document_import_pdf_render_too_large"
        );
    }
    #[test]
    fn document_pdf_page_count_fails_closed_before_rasterization() {
        for (success, stdout, expected) in [
            (false, "Pages: 1\n", "Could not inspect the PDF page count"),
            (true, "Pages: 0\n", "Could not determine the PDF page count"),
            (
                true,
                "not a page count",
                "Could not determine the PDF page count",
            ),
        ] {
            let error = prepare_with(
                Path::new("source.pdf"),
                8,
                &Py::Null,
                Path::new("pdftoppm"),
                Path::new("pdfinfo"),
                &|_, _, _, inspection| {
                    assert!(inspection);
                    Ok(output(success, stdout, ""))
                },
            )
            .err()
            .unwrap();
            assert_eq!(error.code, "document_import_pdf_render_failed");
            assert_eq!(error.message, expected);
        }
        let error = prepare_with(
            Path::new("source.pdf"),
            8,
            &Py::Null,
            Path::new("pdftoppm"),
            Path::new("pdfinfo"),
            &|_, _, _, inspection| {
                assert!(inspection);
                Ok(output(true, "Pages: 9\n", ""))
            },
        )
        .err()
        .unwrap();
        assert_eq!(error.code, "document_import_pdf_page_selection_required");
    }
    #[cfg(unix)]
    #[test]
    fn document_pdf_subprocess_timeout_kills_reaps_and_joins_capture_threads() {
        use std::os::unix::fs::PermissionsExt;
        let root = files::Temp::new("pdf-timeout").unwrap();
        let tool = root.path().join("fake-poppler");
        fs::write(&tool, "#!/usr/bin/python3\nimport os, sys, time\nprint(os.getpid(), flush=True)\nprint('diagnostic', file=sys.stderr, flush=True)\ntime.sleep(10)\n").unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o700)).unwrap();
        for inspection in [true, false] {
            let start = Instant::now();
            let error = run(&tool, &[], Duration::from_millis(100), inspection)
                .err()
                .unwrap();
            assert!(start.elapsed() < Duration::from_secs(5));
            assert_eq!(error.code, "document_import_pdf_render_timeout");
            assert_eq!(
                error.message,
                if inspection {
                    "PDF inspection timed out"
                } else {
                    "PDF rendering timed out"
                }
            );
            assert_eq!(
                field(error.details.as_ref().unwrap(), "timeout_seconds"),
                &Py::from(30)
            );
        }
        fs::write(&tool, "#!/usr/bin/python3\nimport sys\nsys.stdout.write('Pages: 1\\r\\n')\nsys.stderr.write('local\\rdiagnostic')\n").unwrap();
        let output = run(&tool, &[], TIMEOUT, true).unwrap();
        assert!(output.success);
        assert_eq!(output.stdout, "Pages: 1\n");
        assert_eq!(output.stderr, "local\ndiagnostic");
    }
}
