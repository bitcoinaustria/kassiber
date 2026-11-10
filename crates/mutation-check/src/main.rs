//! `mutation-check`: validate the mutation catalogs, or run one suite.
//!
//!     cargo run -p kassiber-mutation-check -- check [--strict] [SUITE ...]
//!     cargo run -p kassiber-mutation-check -- run SUITE [--only ID ...] [--keep]
//!     cargo run -p kassiber-mutation-check -- list
//!
//! `check` validates every catalog against the worktree; `--strict` also
//! refuses entries that need Python. `run` copies the worktree under
//! `target/mutation-scratch/` and kills each entry there. Set
//! `CARGO_BUILD_JOBS`, `RUST_TEST_THREADS` and the profile debug level in the
//! environment as for any build.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use kassiber_mutation_check::runner::Runner;
use kassiber_mutation_check::{check, load_config, Config};

fn root() -> PathBuf {
    // crates/mutation-check -> the repository.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: mutation-check check [--strict] [SUITE ...]\n       \
         mutation-check run SUITE [--only ID ...] [--keep]\n       mutation-check list"
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = root();
    let config = match load_config(&root) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    match args.first().map(String::as_str) {
        Some("check") => run_check(&root, &config, &args[1..]),
        Some("run") => run_suite(&root, &config, &args[1..]),
        Some("list") => {
            for suite in &config.suites {
                let count = suite.mutations(&root).map(|m| m.len()).unwrap_or(0);
                println!("{}\t{count}\t{}", suite.name, suite.catalogs.join(" "));
            }
            ExitCode::SUCCESS
        }
        _ => usage(),
    }
}

fn run_check(root: &Path, config: &Config, args: &[String]) -> ExitCode {
    let strict = args.iter().any(|a| a == "--strict");
    let only: Vec<String> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .collect();
    let report = check(root, config, &only, strict);
    for problem in &report.problems {
        println!("{problem}");
    }
    println!(
        "{} entries in {} suites; {} still need Python",
        report.entries,
        if only.is_empty() {
            config.suites.len()
        } else {
            only.len()
        },
        report.legacy
    );
    if report.problems.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn run_suite(root: &Path, config: &Config, args: &[String]) -> ExitCode {
    let Some(name) = args.first() else {
        return usage();
    };
    let Some(suite) = config.suites.iter().find(|s| &s.name == name) else {
        eprintln!("no suite {name}");
        return ExitCode::from(2);
    };
    let mut only = Vec::new();
    let mut keep = false;
    let mut rest = args[1..].iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--only" => match rest.next() {
                Some(id) => only.push(id.clone()),
                None => return usage(),
            },
            "--keep" => keep = true,
            _ => return usage(),
        }
    }
    let mut mutations = match suite.mutations(root) {
        Ok(mutations) => mutations,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    if !only.is_empty() {
        let unknown: Vec<&String> = only
            .iter()
            .filter(|id| !mutations.iter().any(|m| &m.id == *id))
            .collect();
        if !unknown.is_empty() {
            eprintln!("--only names unknown entries: {unknown:?}");
            return ExitCode::from(2);
        }
        mutations.retain(|m| only.contains(&m.id));
    }
    let legacy: Vec<&str> = mutations
        .iter()
        .filter(|m| m.is_legacy())
        .map(|m| m.id.as_str())
        .collect();
    if !legacy.is_empty() {
        eprintln!("these entries still need Python; give them Rust killers first: {legacy:?}");
        return ExitCode::from(2);
    }
    let target = match root.join("target").canonicalize() {
        Ok(target) => target,
        Err(error) => {
            eprintln!("target/: {error}");
            return ExitCode::from(2);
        }
    };
    let runner = Runner::new(suite, root, &target);
    if let Err(error) = runner.prepare() {
        eprintln!("{error}");
        return ExitCode::from(2);
    }
    let mut stdout = std::io::stdout();
    let result = runner.run(&mutations, &mut stdout);
    if keep {
        println!("scratch tree kept at {}", runner.scratch().display());
    } else {
        runner.cleanup();
    }
    match result {
        Ok(failures) if failures.is_empty() => {
            println!("all {} mutations killed", mutations.len());
            ExitCode::SUCCESS
        }
        Ok(failures) => {
            let _ = writeln!(stdout);
            for failure in failures {
                eprintln!("{failure}");
            }
            ExitCode::from(1)
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}
