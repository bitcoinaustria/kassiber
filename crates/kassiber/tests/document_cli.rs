//! `wallets preview-document` / `import-document` on the CLI, interactive
//! review included, against frozen goldens (see `support`). These replaced
//! the plaintext halves of `tests/test_rust_document_cli.py`, which compared
//! the same runs with Python's CLI. The review prompt needs a terminal on
//! stdin: `script(1)` provides one, while stdout and stderr go to files.

#![cfg(feature = "test-harness")]

mod support;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use support::Desk;

const PROMPT: &str = "Enter comma-separated row numbers to import, or press Enter to cancel: ";
const TIME: &str = "2026-01-02T03:04:05Z";
const SOURCE: &[u8] = b"Kassiber synthetic local OCR evidence\n";

/// `generate_document_vectors.base()`, with `changes` applied (raw JSON
/// members replacing the defaults of the same key).
fn row(changes: &[(&str, &str)]) -> String {
    let mut fields: Vec<(String, String)> = [
        ("occurred_at", r#""2026-01-02""#),
        ("direction", r#""inbound""#),
        ("asset", r#""BTC""#),
        ("amount_btc", r#""0.01000000""#),
        ("fee_btc", r#""0""#),
        ("fiat_currency", r#""EUR""#),
        ("fiat_value", r#""500.00""#),
        ("counterparty", r#""OTC Desk""#),
        ("description", r#""Receipt row""#),
        ("confidence", "0.94"),
        (
            "cell_confidences",
            r#"{"amount_btc": 0.99, "asset": 0.99, "counterparty": 0.99, "description": 0.99, "direction": 0.99, "fee_btc": 0.99, "fiat_currency": 0.99, "fiat_rate": 0.99, "fiat_value": 0.99, "occurred_at": 0.99}"#,
        ),
        (
            "source_region",
            r#"{"page": 1, "x": 0.1, "y": 0.2, "width": 0.7, "height": 0.1, "unit": "relative"}"#,
        ),
        (
            "evidence_text",
            r#""02.01.2026 BTC 0.01000000 EUR 500 OTC Desk""#,
        ),
    ]
    .iter()
    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
    .collect();
    for (key, value) in changes {
        match fields.iter_mut().find(|(k, _)| k == key) {
            Some(field) => field.1 = (*value).to_owned(),
            None => fields.push(((*key).to_owned(), (*value).to_owned())),
        }
    }
    let members: Vec<String> = fields
        .iter()
        .map(|(k, v)| format!("\"{k}\": {v}"))
        .collect();
    format!("{{{}}}", members.join(", "))
}

/// The local OCR provider (tests/test_rust_document_cli.py::Handler).
struct Ocr {
    port: u16,
    rows: Arc<Mutex<Vec<String>>>,
    hits: Arc<Mutex<Vec<String>>>,
}

impl Ocr {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let rows = Arc::new(Mutex::new(vec![row(&[])]));
        let hits = Arc::new(Mutex::new(Vec::new()));
        let (served, recorded) = (rows.clone(), hits.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let (rows, hits) = (served.clone(), recorded.clone());
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let mut parts = line.split_whitespace();
                    let method = parts.next().unwrap_or_default().to_owned();
                    let target = parts.next().unwrap_or_default().to_owned();
                    let mut length = 0;
                    loop {
                        let mut header = String::new();
                        reader.read_line(&mut header).unwrap();
                        if header.trim().is_empty() {
                            break;
                        }
                        if let Some((name, value)) = header.split_once(':') {
                            if name.eq_ignore_ascii_case("content-length") {
                                length = value.trim().parse().unwrap_or(0);
                            }
                        }
                    }
                    let mut body = vec![0; length];
                    reader.read_exact(&mut body).unwrap();
                    hits.lock().unwrap().push(format!(
                        "{method} {target} {}",
                        String::from_utf8_lossy(&body)
                    ));
                    let reply = if method == "GET" {
                        r#"{"data": [{"id": "glm-ocr"}]}"#.to_owned()
                    } else {
                        let text = format!(r#"{{"rows": [{}]}}"#, rows.lock().unwrap().join(", "));
                        serde_json::json!({"output": [{"type": "message", "content": [{"type": "output_text", "text": text}]}]})
                            .to_string()
                    };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                        reply.len()
                    );
                });
            }
        });
        Self { port, rows, hits }
    }
}

struct Cli {
    desk: Desk,
    ocr: Ocr,
    source: PathBuf,
    runs: usize,
    /// The run's place in its batch: its generated ids start 50 further on.
    index: usize,
}

/// `00000000-0000-4000-8000-{i:012}` for `i` in `from..end`, comma-joined.
fn id_range(from: usize, end: usize) -> String {
    (from..end)
        .map(|i| format!("00000000-0000-4000-8000-{i:012}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// What one run of the CLI left.
struct Outcome {
    code: Option<i32>,
    stdout: String,
}

impl Cli {
    /// `RustDocumentCliTest.setUp` and `.baseline(False)`.
    fn new(golden: &str) -> Self {
        let mut desk = Desk::with_seed(golden, None);
        let base = desk.base_path();
        let offline = base.join("preferences/offline.json");
        std::fs::write(&offline, r#"{"schema_version":1,"enabled":true}"#).unwrap();
        for (key, value) in [
            ("KASSIBER_RUST_TEST_DOCUMENT_TIME", TIME.to_owned()),
            ("KASSIBER_RUST_TEST_IMPORT_TIME", TIME.to_owned()),
            ("KASSIBER_RUST_TEST_AI_TIME", TIME.to_owned()),
            ("HTTP_PROXY", "http://192.0.2.2:9".to_owned()),
            ("HTTPS_PROXY", "http://192.0.2.2:9".to_owned()),
        ] {
            desk.env.insert(key.to_owned(), value);
        }
        let ocr = Ocr::new();
        desk.replacements
            .push((format!("127.0.0.1:{}", ocr.port), "$OCR".to_owned()));
        desk.change(
            "INSERT INTO workspaces(id,label,created_at) VALUES('ws','Workspace','2025-01-01');\
             INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','ws','Book','2025-01-01');\
             INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) \
             VALUES('wallet','ws','p','Source','custom','{}','2025-01-01');\
             INSERT INTO settings(key,value) VALUES('context_workspace','ws'),('context_profile','p');\
             UPDATE profiles SET last_processed_at='2025-01-01',last_processed_tx_count=3,\
             journal_input_version=7,fiat_currency='EUR';",
        );
        desk.setup_cli(&["init"]);
        desk.change(&format!(
            "INSERT INTO settings(key,value) VALUES('ai_providers_seeded','1') \
             ON CONFLICT(key) DO UPDATE SET value='1';\
             INSERT INTO ai_providers(name,base_url,kind,created_at,updated_at) \
             VALUES('local','http://127.0.0.1:{}/v1','local','2026-01-01','2026-01-01');",
            ocr.port
        ));
        let source = base.join("receipt.png");
        std::fs::write(&source, SOURCE).unwrap();
        Self {
            desk,
            ocr,
            source,
            runs: 0,
            index: 0,
        }
    }

    fn args(&self, action: &str, extra: &[&str]) -> Vec<String> {
        let mut args = vec!["wallets".to_owned(), action.to_owned()];
        if action != "preview-document" {
            args.extend(["--wallet".to_owned(), "Source".to_owned()]);
        }
        args.extend([
            "--file".to_owned(),
            self.source.to_string_lossy().into_owned(),
            "--provider".to_owned(),
            "local".to_owned(),
        ]);
        args.extend(extra.iter().map(|a| (*a).to_owned()));
        args
    }

    fn set_rows(&self, rows: Vec<String>) {
        *self.ocr.rows.lock().unwrap() = rows;
    }

    /// Every file the runs stored as attachments.
    fn attachments(&self) -> Vec<String> {
        fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, base, out);
                } else if path.to_string_lossy().contains("/attachments/") {
                    let bytes = std::fs::read(&path).unwrap();
                    out.push(format!(
                        "{} {} fnv1a128:{:032x}",
                        path.strip_prefix(base).unwrap().display(),
                        bytes.len(),
                        support::fnv1a128(&bytes)
                    ));
                }
            }
        }
        let base = self.desk.base_path();
        let mut out = Vec::new();
        walk(&base, &base, &mut out);
        out.sort();
        out
    }

    /// One run; `globals` precede `--data-root`. With `interactive`, stdin
    /// is a terminal: at the review prompt `at_prompt` runs, then `response`
    /// is typed (or the process gets SIGINT when it is `None`).
    fn run(
        &mut self,
        globals: &[&str],
        args: &[String],
        interactive: bool,
        at_prompt: Option<&dyn Fn()>,
        response: Option<&str>,
    ) -> Outcome {
        let before = self.desk.tables();
        let attachments = self.attachments();
        // Python's IDS[offset:] and ATTACHMENT_IDS[offset:], offset = index * 50.
        let offset = self.index * 50;
        self.index += 1;
        self.desk.env.insert(
            "KASSIBER_RUST_TEST_IMPORT_IDS".to_owned(),
            id_range(1 + offset, 1001),
        );
        self.desk.env.insert(
            "KASSIBER_RUST_TEST_DOCUMENT_ATTACHMENT_IDS".to_owned(),
            id_range(2001 + offset, 3001),
        );
        self.runs += 1;
        let run = self.desk.base_path().join(format!("run-{}", self.runs));
        std::fs::create_dir_all(&run).unwrap();
        let mut argv: Vec<String> = globals.iter().map(|g| (*g).to_owned()).collect();
        argv.extend([
            "--data-root".to_owned(),
            self.desk.root.to_string_lossy().into_owned(),
        ]);
        argv.extend(args.iter().cloned());
        let (code, stdout, stderr) = if interactive {
            self.interactive(&run, &argv, at_prompt, response)
        } else {
            let mut command = self.desk.raw_command(&argv);
            command.stdin(Stdio::null());
            let output = command.output().unwrap();
            (
                output.status.code(),
                String::from_utf8_lossy(&output.stdout).into_owned(),
                String::from_utf8_lossy(&output.stderr).into_owned(),
            )
        };
        let shown: Vec<String> = argv.iter().map(|a| self.desk.replace_run_text(a)).collect();
        self.desk.golden.line(&format!("$ {}", shown.join(" ")));
        self.desk.golden.line(&format!("exit {code:?}"));
        for line in stdout.lines() {
            let line = self.desk.replace_run_text(line);
            self.desk.golden.line(&format!("stdout {line}"));
        }
        for line in stderr.lines() {
            let line = self.desk.replace_run_text(line);
            self.desk.golden.line(&format!("stderr {line}"));
        }
        for line in support::table_changes(&before, &self.desk.tables()) {
            let line = self.desk.replace_run_text(&line);
            self.desk.golden.line(&line);
        }
        for file in self.attachments() {
            if !attachments.contains(&file) {
                self.desk.golden.line(&format!("+ file {file}"));
            }
        }
        Outcome { code, stdout }
    }

    fn interactive(
        &self,
        run: &Path,
        argv: &[String],
        at_prompt: Option<&dyn Fn()>,
        response: Option<&str>,
    ) -> (Option<i32>, String, String) {
        let (out, err, pid) = (run.join("stdout"), run.join("stderr"), run.join("pid"));
        let wrapper = run.join("run.sh");
        std::fs::write(
            &wrapper,
            "echo $$ > \"$KASSIBER_DOC_PID\"\nexec \"$KASSIBER_DOC_BIN\" \"$@\" > \"$KASSIBER_DOC_OUT\" 2> \"$KASSIBER_DOC_ERR\"\n",
        )
        .unwrap();
        let quote = |a: &str| format!("'{}'", a.replace('\'', "'\\''"));
        let line = std::iter::once(format!("sh {}", quote(&wrapper.to_string_lossy())))
            .chain(argv.iter().map(|a| quote(a)))
            .collect::<Vec<_>>()
            .join(" ");
        let template = self.desk.raw_command::<&str>(&[]);
        let mut command = {
            let mut script = Command::new("script");
            script.args(["-q", "-e", "-c", &line, "/dev/null"]);
            for (key, value) in template.get_envs() {
                match value {
                    Some(value) => script.env(key, value),
                    None => script.env_remove(key),
                };
            }
            if let Some(dir) = template.get_current_dir() {
                script.current_dir(dir);
            }
            script
        };
        command
            .env("SHELL", "/bin/sh")
            .env("KASSIBER_DOC_BIN", env!("CARGO_BIN_EXE_kassiber"))
            .env("KASSIBER_DOC_OUT", &out)
            .env("KASSIBER_DOC_ERR", &err)
            .env("KASSIBER_DOC_PID", &pid)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = command.spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let deadline = Instant::now() + Duration::from_secs(90);
        let mut prompted = false;
        while Instant::now() < deadline {
            if std::fs::read(&out).is_ok_and(|b| String::from_utf8_lossy(&b).contains(PROMPT)) {
                prompted = true;
                break;
            }
            if child.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if prompted {
            if let Some(at_prompt) = at_prompt {
                at_prompt();
            }
            match response {
                Some(response) => {
                    stdin.write_all(response.as_bytes()).unwrap();
                    stdin.flush().unwrap();
                }
                None => {
                    let pid = std::fs::read_to_string(&pid).unwrap();
                    let status = Command::new("kill")
                        .args(["-INT", pid.trim()])
                        .status()
                        .unwrap();
                    assert!(status.success());
                }
            }
        }
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                panic!("the CLI did not finish: {argv:?}");
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        drop(stdin);
        let read = |path: &Path| {
            let mut text = String::new();
            if let Ok(mut file) = std::fs::File::open(path) {
                file.read_to_string(&mut text).unwrap();
            }
            text
        };
        (status.code(), read(&out), read(&err))
    }

    fn record_hits(&mut self) {
        let hits: Vec<String> = std::mem::take(&mut *self.ocr.hits.lock().unwrap());
        for hit in hits {
            let hit = self.desk.replace_run_text(&hit);
            if hit.len() > 4096 {
                let head: String = hit.chars().take(160).collect();
                self.desk.golden.line(&format!(
                    "ocr {head}... bytes={} fnv1a128:{:032x}",
                    hit.len(),
                    support::fnv1a128(hit.as_bytes())
                ));
            } else {
                self.desk.golden.line(&format!("ocr {hit}"));
            }
        }
    }

    fn hits(&self) -> usize {
        self.ocr.hits.lock().unwrap().len()
    }
}

#[test]
fn document_cli_refuses_before_provider_contact_and_resolves_scope_first() {
    let mut cli = Cli::new("document_cli_refuses_before_provider_contact_and_resolves_scope_first");
    let absent = cli.desk.base_path().join("absent.png");
    let absent = absent.to_string_lossy().into_owned();
    let runs: Vec<(Vec<&str>, Vec<String>, bool)> = vec![
        (vec!["--machine"], cli.args("import-document", &[]), false),
        (
            vec!["--format", "json"],
            cli.args("import-document", &[]),
            false,
        ),
        (
            vec!["--format", "json", "--non-interactive"],
            cli.args("import-document", &[]),
            true,
        ),
        (
            vec!["--machine"],
            cli.args("import-document", &["--wallet", "missing"]),
            false,
        ),
        (
            vec!["--machine"],
            cli.args("preview-document", &["--profile", "missing"]),
            false,
        ),
        (
            vec!["--machine"],
            cli.args("preview-document", &["--file", &absent]),
            false,
        ),
        (
            vec!["--machine"],
            cli.args("preview-document", &["--provider", "missing"]),
            false,
        ),
    ];
    for (globals, args, interactive) in runs {
        let outcome = cli.run(&globals, &args, interactive, None, Some("1\n"));
        assert_eq!(outcome.code, Some(1), "{args:?}: {}", outcome.stdout);
    }
    assert_eq!(cli.hits(), 0);
    cli.desk.check();
}

#[test]
fn document_cli_preview_review_reimport_and_zero_fallback() {
    let mut cli = Cli::new("document_cli_preview_review_reimport_and_zero_fallback");
    let preview = cli.args("preview-document", &[]);
    let import = cli.args("import-document", &[]);
    let codes = [
        cli.run(&["--machine"], &preview, false, None, None).code,
        cli.run(&["--format", "json"], &import, true, None, Some("1\n"))
            .code,
        cli.run(&["--format", "json"], &import, true, None, Some("1\n"))
            .code,
    ];
    assert_eq!(codes, [Some(0); 3]);
    assert_eq!(cli.attachments().len(), 2);
    assert_eq!(cli.hits(), 6);
    cli.record_hits();
    cli.desk.check();
}

#[test]
fn document_cli_imports_only_selected_rows() {
    let mut cli = Cli::new("document_cli_imports_only_selected_rows");
    cli.set_rows(vec![row(&[]), row(&[("amount_btc", r#""0.02""#)])]);
    let import = cli.args("import-document", &[]);
    let outcome = cli.run(&["--format", "json"], &import, true, None, Some("2\n"));
    assert_eq!(outcome.code, Some(0));
    assert_eq!(cli.attachments().len(), 1);
    let imported: i64 = cli
        .desk
        .connection()
        .query_row("SELECT COUNT(*) FROM transactions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(imported, 1);
    cli.record_hits();
    cli.desk.check();
}

#[test]
fn document_cli_changed_source_and_atomic_database_failure() {
    let mut cli = Cli::new("document_cli_changed_source_and_atomic_database_failure");
    let import = cli.args("import-document", &[]);
    let source = cli.source.clone();
    let change = move || std::fs::write(&source, b"changed").unwrap();
    let outcome = cli.run(
        &["--format", "json"],
        &import,
        true,
        Some(&change),
        Some("1\n"),
    );
    assert_eq!(outcome.code, Some(1));
    assert!(
        outcome.stdout.contains("document_import_source_changed"),
        "{}",
        outcome.stdout
    );
    assert!(cli.attachments().is_empty());
    std::fs::write(&cli.source, SOURCE).unwrap();
    // A second batch, as the Python test's second comparison.
    cli.index = 0;
    let database = kassiber_paths::resolve_database_path(&cli.desk.root);
    let fail = move || {
        let conn = kassiber_store::rusqlite::Connection::open(&database).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER deny_evidence BEFORE INSERT ON attachments BEGIN SELECT RAISE(ABORT,'private-source-value'); END;",
        )
        .unwrap();
    };
    let outcome = cli.run(
        &["--format", "json"],
        &import,
        true,
        Some(&fail),
        Some("1\n"),
    );
    assert_eq!(outcome.code, Some(1));
    assert!(
        outcome.stdout.contains("database operation failed"),
        "{}",
        outcome.stdout
    );
    assert!(!outcome.stdout.contains("private-source-value"));
    assert!(cli.attachments().is_empty());
    cli.record_hits();
    cli.desk.check();
}

#[test]
fn document_cli_sigint_cancels_without_writes() {
    let mut cli = Cli::new("document_cli_sigint_cancels_without_writes");
    let import = cli.args("import-document", &[]);
    let outcome = cli.run(&["--format", "json"], &import, true, None, None);
    assert_eq!(outcome.code, Some(1));
    assert!(
        outcome.stdout.contains("document_import_review_cancelled"),
        "{}",
        outcome.stdout
    );
    assert!(cli.attachments().is_empty());
    cli.record_hits();
    cli.desk.check();
}

#[test]
fn document_cli_pdf_options_and_full_book_parity() {
    let poppler = ["pdfinfo", "pdftoppm"].iter().all(|tool| {
        Command::new(tool)
            .arg("-v")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
    });
    if !poppler {
        eprintln!("skipped: Poppler (pdfinfo, pdftoppm) is required");
        return;
    }
    let mut cli = Cli::new("document_cli_pdf_options_and_full_book_parity");
    cli.source = cli.desk.base_path().join("statement.pdf");
    std::fs::write(
        &cli.source,
        include_bytes!("data/statement-10-pages.pdf").as_slice(),
    )
    .unwrap();
    cli.set_rows(vec![row(&[("source_region", r#"{"page": 9}"#)])]);
    let preview = cli.args("preview-document", &["--pages", "9-10", "--max-pages", "2"]);
    let import = cli.args("import-document", &["--pages", "9-10", "--max-pages", "2"]);
    let codes = [
        cli.run(&["--machine"], &preview, false, None, None).code,
        cli.run(&["--format", "json"], &import, true, None, Some("1\n"))
            .code,
    ];
    assert_eq!(codes, [Some(0); 2]);
    assert_eq!(cli.attachments().len(), 1);
    cli.record_hits();
    cli.desk.check();
}
