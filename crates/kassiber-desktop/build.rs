//! Stamps the build with its source commit (`KASSIBER_COMMIT`), shown in the sidebar's version
//! line. A release pipeline may set `KASSIBER_COMMIT` itself; otherwise git is asked, and a
//! build outside a checkout says "unknown" rather than a stale hash.
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn main() {
    println!("cargo:rerun-if-env-changed=KASSIBER_COMMIT");
    let commit = std::env::var("KASSIBER_COMMIT")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            // Rebuild when HEAD moves (a worktree's HEAD lives outside `.git/`, so ask git where).
            for path in ["HEAD", "packed-refs"] {
                if let Some(p) = git(&["rev-parse", "--git-path", path]) {
                    println!("cargo:rerun-if-changed={p}");
                }
            }
            if let Some(r) = git(&["symbolic-ref", "-q", "HEAD"])
                .and_then(|r| git(&["rev-parse", "--git-path", &r]))
            {
                println!("cargo:rerun-if-changed={r}");
            }
            git(&["rev-parse", "--short=7", "HEAD"])
        });
    println!(
        "cargo:rustc-env=KASSIBER_COMMIT={}",
        commit.as_deref().unwrap_or("unknown")
    );
}
