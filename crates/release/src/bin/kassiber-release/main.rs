//! `kassiber-release`: the release tooling the workflows run, ported from
//! `scripts/{release_manifest,render_aur,render_nix,render_homebrew,
//! release_notes,write_build_info,macos_release,prepare_macos_release}.py`.
//!
//!     kassiber-release manifest generate|sign|verify-artifacts|verify-release|policy ...
//!     kassiber-release render-aur desktop|cli --version V --sha256 H --output DIR
//!     kassiber-release render-nix --version V --sha256-desktop H --sha256-cli H --output DIR
//!     kassiber-release render-homebrew cask|cli-formula ...
//!     kassiber-release release-notes --version V [--changelog FILE] --output FILE
//!     kassiber-release build-info --output FILE --version V --commit C --ref R --channel CH ...
//!     kassiber-release macos sign|notarize|verify|prepare ...
//!
//! Outputs are byte-identical to the scripts' (crates/release/tests/tool.rs
//! pins them against frozen script output). Build it without the update
//! check's network and store crates:
//!     cargo build --locked --release -p kassiber-release --no-default-features --bin kassiber-release
//!
//! The repository root (signing policy, LICENSE, the AUR icon) is
//! `KASSIBER_REPO_ROOT` when set, else the nearest ancestor of the current
//! directory holding `packaging/release`.
mod args;
mod macos;
mod notes;
mod render;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use args::{append, flag, parse, required, value, Spec, Usage};
use kassiber_envelope::{s, AppError};
use kassiber_pyjson::{Options, Value};
use kassiber_release::publish;

pub fn usage(message: impl Into<String>) -> Usage {
    Usage(message.into())
}

fn repo_root() -> Result<PathBuf, String> {
    if let Some(root) = std::env::var_os("KASSIBER_REPO_ROOT").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(root));
    }
    let current = std::env::current_dir().map_err(|e| e.to_string())?;
    current
        .ancestors()
        .find(|dir| dir.join("packaging/release").is_dir() && dir.join("LICENSE").is_file())
        .map(Path::to_path_buf)
        .ok_or_else(|| "run inside a Kassiber checkout or set KASSIBER_REPO_ROOT".to_owned())
}

fn dumps(pairs: Vec<(&str, Value)>) -> String {
    kassiber_pyjson::dumps(&kassiber_envelope::dict(pairs), &Options::new()).unwrap()
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// `release_manifest.py`'s output: `{"ok": true, ...}` on stdout, or the
/// error object on stderr and exit 1.
fn manifest(argv: &[String]) -> Result<ExitCode, Usage> {
    let Some((command, rest)) = argv.split_first() else {
        return Err(usage("the following arguments are required: command"));
    };
    let specs: Vec<Spec> = match command.as_str() {
        "generate" => vec![required("release-dir"), required("version"), append("exclude")],
        "sign" => vec![
            required("manifest"),
            required("public-key"),
            value("policy"),
            value("output"),
            value("ssh-keygen"),
            flag("overwrite"),
        ],
        "verify-artifacts" => vec![
            required("release-dir"),
            required("manifest"),
            flag("allow-subset"),
        ],
        "verify-release" => vec![
            required("release-dir"),
            required("manifest"),
            required("signature"),
            required("public-key"),
            required("fingerprint"),
            value("ssh-keygen"),
            flag("allow-subset"),
        ],
        "policy" => vec![required("policy"), flag("require-enabled")],
        other => {
            return Err(usage(format!(
                "argument command: invalid choice: '{other}' (choose from 'generate', 'sign', 'verify-artifacts', 'verify-release', 'policy')"
            )))
        }
    };
    let args = parse(rest, &specs)?;
    let run = || -> Result<Vec<(&'static str, Value)>, AppError> {
        let root =
            || repo_root().map_err(|message| AppError::new("release_repository_missing", message));
        match command.as_str() {
            "generate" => {
                let directory = publish::user_path(args.req("release-dir"));
                let path =
                    publish::generate(&directory, args.req("version"), &args.all("exclude"))?;
                let entries = publish::parse_manifest(&path)?.len();
                Ok(vec![
                    ("manifest", s(path_text(&path))),
                    ("entries", Value::Int(entries.into())),
                ])
            }
            "sign" => {
                let root = root()?;
                let policy = match args.get("policy") {
                    Some(policy) => publish::py_path(Path::new(policy)),
                    None => root.join("packaging/release/signing-policy.json"),
                };
                let policy =
                    publish::load_policy(&publish::user_path(&path_text(&policy)), &root, true)?;
                let fingerprint = match &policy[1].1 {
                    Value::Str(text) => text.as_str().unwrap_or_default().to_owned(),
                    _ => String::new(),
                };
                let manifest = publish::py_path(Path::new(args.req("manifest")));
                let signature = publish::sign(
                    &manifest,
                    &publish::user_path(args.req("public-key")),
                    &fingerprint,
                    args.get("output")
                        .map(|o| publish::py_path(Path::new(o)))
                        .as_deref(),
                    args.get("ssh-keygen"),
                    args.flag("overwrite"),
                )?;
                Ok(vec![
                    ("manifest", s(path_text(&manifest))),
                    ("signature", s(path_text(&signature))),
                    ("signer_fingerprint", s(fingerprint)),
                ])
            }
            "verify-artifacts" => publish::verify_artifacts(
                &publish::user_path(args.req("release-dir")),
                &publish::user_path(args.req("manifest")),
                !args.flag("allow-subset"),
            ),
            "verify-release" => publish::verify_release(
                &publish::user_path(args.req("release-dir")),
                &publish::user_path(args.req("manifest")),
                &publish::user_path(args.req("signature")),
                &publish::user_path(args.req("public-key")),
                args.req("fingerprint"),
                !args.flag("allow-subset"),
            ),
            _ => publish::load_policy(
                &publish::user_path(args.req("policy")),
                &root()?,
                args.flag("require-enabled"),
            ),
        }
    };
    Ok(match run() {
        Ok(payload) => {
            let mut pairs = vec![("ok", Value::Bool(true))];
            pairs.extend(payload);
            println!("{}", dumps(pairs));
            ExitCode::SUCCESS
        }
        Err(error) => {
            let error_pairs = vec![
                ("code", s(error.code.as_str())),
                ("message", s(error.message.as_str())),
                ("hint", error.hint.as_deref().map(s).unwrap_or(Value::Null)),
                ("details", error.details.clone().unwrap_or(Value::Null)),
            ];
            eprintln!(
                "{}",
                dumps(vec![
                    ("ok", Value::Bool(false)),
                    ("error", kassiber_envelope::dict(error_pairs)),
                ])
            );
            ExitCode::from(1)
        }
    })
}

fn failed(message: String) -> ExitCode {
    eprintln!("kassiber-release: error: {message}");
    ExitCode::from(1)
}

fn render_aur(argv: &[String]) -> Result<ExitCode, Usage> {
    let Some((surface, rest)) = argv.split_first() else {
        return Err(usage("the following arguments are required: surface"));
    };
    if surface != "desktop" && surface != "cli" {
        return Err(usage(format!(
            "argument surface: invalid choice: '{surface}' (choose from 'desktop', 'cli')"
        )));
    }
    let args = parse(
        rest,
        &[required("version"), required("sha256"), required("output")],
    )?;
    let root = match repo_root() {
        Ok(root) => root,
        Err(message) => return Ok(failed(message)),
    };
    let output = publish::py_path(Path::new(args.req("output")));
    let result = if surface == "desktop" {
        render::aur_desktop(&root, args.req("version"), args.req("sha256"), &output)
    } else {
        render::aur_cli(&root, args.req("version"), args.req("sha256"), &output)
    };
    Ok(result.map_or_else(failed, |()| ExitCode::SUCCESS))
}

fn render_nix(argv: &[String]) -> Result<ExitCode, Usage> {
    let args = parse(
        argv,
        &[
            required("version"),
            required("sha256-desktop"),
            required("sha256-cli"),
            required("output"),
        ],
    )?;
    let root = match repo_root() {
        Ok(root) => root,
        Err(message) => return Ok(failed(message)),
    };
    let result = render::nix(
        &root,
        args.req("version"),
        args.req("sha256-desktop"),
        args.req("sha256-cli"),
        &publish::py_path(Path::new(args.req("output"))),
    );
    Ok(result.map_or_else(failed, |()| ExitCode::SUCCESS))
}

fn render_homebrew(argv: &[String]) -> Result<ExitCode, Usage> {
    let Some((kind, rest)) = argv.split_first() else {
        return Err(usage("the following arguments are required: kind"));
    };
    let rendered = match kind.as_str() {
        "cask" => {
            let args = parse(
                rest,
                &[required("version"), required("sha256"), required("output")],
            )?;
            render::cask(args.req("version"), args.req("sha256"))
                .map(|text| (text, args.req("output").to_owned()))
        }
        "cli-formula" => {
            let args = parse(
                rest,
                &[
                    required("version"),
                    required("sha256-macos-arm64"),
                    required("sha256-linux-x64"),
                    required("output"),
                ],
            )?;
            render::cli_formula(
                args.req("version"),
                args.req("sha256-macos-arm64"),
                args.req("sha256-linux-x64"),
            )
            .map(|text| (text, args.req("output").to_owned()))
        }
        other => {
            return Err(usage(format!(
                "argument kind: invalid choice: '{other}' (choose from 'cask', 'cli-formula')"
            )))
        }
    };
    Ok(rendered
        .and_then(|(text, output)| {
            render::write_rendered(&publish::py_path(Path::new(&output)), &text)
        })
        .map_or_else(failed, |()| ExitCode::SUCCESS))
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some((command, rest)) = argv.split_first() else {
        eprintln!("kassiber-release: error: the following arguments are required: command");
        return ExitCode::from(2);
    };
    let result = match command.as_str() {
        "manifest" => manifest(rest),
        "render-aur" => render_aur(rest),
        "render-nix" => render_nix(rest),
        "render-homebrew" => render_homebrew(rest),
        "release-notes" => notes::release_notes(rest),
        "build-info" => notes::build_info(rest),
        "macos" => macos::main(rest),
        "-h" | "--help" => {
            println!(
                "usage: kassiber-release {{manifest,render-aur,render-nix,render-homebrew,release-notes,build-info,macos}} ..."
            );
            Ok(ExitCode::SUCCESS)
        }
        other => Err(usage(format!(
            "argument command: invalid choice: '{other}' (choose from 'manifest', 'render-aur', 'render-nix', 'render-homebrew', 'release-notes', 'build-info', 'macos')"
        ))),
    };
    match result {
        Ok(code) => code,
        Err(Usage(message)) => {
            eprintln!("kassiber-release: error: {message}");
            ExitCode::from(2)
        }
    }
}
