//! Setup writes wait for another connection's writer; setup reads never do.
mod support;
use kassiber_envelope::{dict, s};
use kassiber_pyjson::Value;
use kassiber_store::{Book, OpenOptions};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use support::*;

const HOLD: Duration = Duration::from_millis(700);

fn hold_writer(f: &Fixture) -> thread::JoinHandle<()> {
    let root = f._root.path().join("data");
    let (held, wait) = mpsc::channel();
    let holder = thread::spawn(move || {
        let other = Book::open(&root, OpenOptions::default()).unwrap();
        other.connection().execute_batch("BEGIN IMMEDIATE").unwrap();
        held.send(()).unwrap();
        thread::sleep(HOLD);
        other.connection().execute_batch("COMMIT").unwrap();
    });
    wait.recv().unwrap();
    holder
}

#[test]
fn setup_writes_wait_for_a_held_writer() {
    let f = Fixture::seed();
    let profile = || {
        dict([
            ("label", s("Held book")),
            ("workspace", s("Main")),
            ("fiat_currency", s("USD")),
            ("tax_country", s("generic")),
            ("tax_long_term_days", Value::Int(365.into())),
        ])
    };
    for (op, args) in [
        ("workspaces.create", dict([("label", s("Held"))])),
        ("ui.workspace.create", dict([("label", s("Held desktop"))])),
        // These read (resolve their scope) before they write.
        ("profiles.create", profile()),
        (
            "accounts.create",
            dict([
                ("code", s("held")),
                ("label", s("Held")),
                ("type", s("asset")),
            ]),
        ),
    ] {
        let holder = hold_writer(&f);
        let started = Instant::now();
        f.run(op, args);
        assert!(
            started.elapsed() >= HOLD / 2,
            "{op} did not wait for the writer"
        );
        holder.join().unwrap();
    }
}

#[test]
fn setup_reads_do_not_wait_behind_a_writer() {
    let f = Fixture::seed();
    let holder = hold_writer(&f);
    let started = Instant::now();
    for op in ["workspaces.list", "profiles.list", "context.show"] {
        f.run(op, dict::<&str>([]));
    }
    assert!(
        started.elapsed() < HOLD / 2,
        "a setup read waited for the writer"
    );
    holder.join().unwrap();
}
