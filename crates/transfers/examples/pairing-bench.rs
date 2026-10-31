//! Local test/bench adapter, never a product route. Use a disposable DB copy.
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::rusqlite::Connection;
use kassiber_transfers::{pairing, Scope, Sources};
use std::{cell::Cell, time::Instant};
struct Fixed {
    next: Cell<u128>,
}
impl Sources for Fixed {
    fn uuid(&self, _: &Connection) -> Result<String, AppError> {
        let n = self.next.get();
        self.next.set(n + 1);
        let h = format!("{n:032x}");
        Ok(format!(
            "{}-{}-{}-{}-{}",
            &h[..8],
            &h[8..12],
            &h[12..16],
            &h[16..20],
            &h[20..]
        ))
    }
    fn now(&self, _: &Connection) -> Result<String, AppError> {
        Ok("2026-10-30T12:00:00Z".into())
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if !(5..=6).contains(&args.len()) || (args.len() == 6 && args[5] != "--passphrase-stdin") {
        return Err("usage: pairing-bench DB WORKSPACE PROFILE auto|suggest|rules|bulk [--passphrase-stdin]".into());
    }
    let c = Connection::open(&args[1])?;
    if args.len() == 6 {
        let mut passphrase = String::new();
        std::io::stdin().read_line(&mut passphrase)?;
        c.pragma_update(None, "key", passphrase.trim_end_matches(['\r', '\n']))?;
        c.execute_batch(
            "PRAGMA cipher_compatibility=4; PRAGMA kdf_iter=256000; PRAGMA cipher_page_size=4096",
        )?;
    }
    c.execute_batch("PRAGMA foreign_keys=ON; BEGIN IMMEDIATE")?;
    let scope = Scope {
        workspace_id: &args[2],
        profile_id: &args[3],
    };
    let source = Fixed { next: Cell::new(1) };
    let mut opts = pairing::PairingOptions::default();
    opts.matching.now = source.now(&c).unwrap();
    let load = std::fs::read_to_string("/proc/loadavg")
        .unwrap_or_default()
        .split_whitespace()
        .next()
        .unwrap_or("unknown")
        .to_owned();
    let start = Instant::now();
    let result = match args[4].as_str() {
        "auto" => pairing::auto_pair_before_journals(&c, &scope, &source),
        "suggest" => pairing::suggestions(&c, &scope, &opts),
        "rules" => pairing::apply_rules(&c, &scope, &opts, None, &source),
        "bulk" => pairing::bulk_pair(&c, &scope, &opts, None, &source),
        _ => return Err("unknown test operation".into()),
    };
    let seconds = start.elapsed().as_secs_f64();
    let output = match result {
        Ok(v) => {
            c.execute_batch("COMMIT")?;
            v
        }
        Err(e) => {
            c.execute_batch("ROLLBACK")?;
            dict([(
                "error",
                dict([
                    ("code", s(e.code)),
                    ("message", s(e.message)),
                    ("hint", e.hint.map(s).unwrap_or(Value::Null)),
                    ("details", e.details.unwrap_or(Value::Null)),
                ]),
            )])
        }
    };
    println!("{}", dumps(&output, &Options::default())?);
    eprintln!(
        "{}",
        dumps(
            &dict([("seconds", Value::Float(seconds)), ("load_1m", s(load))]),
            &Options::default()
        )?
    );
    Ok(())
}
