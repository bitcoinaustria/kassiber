#[path = "../tests/support/source_funds_mutations.rs"]
mod support;
use kassiber_store::rusqlite::Connection;
use std::io::{self, BufRead, Write};
fn main() {
    let mut stdout = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let request = kassiber_pytext::loads(&line.unwrap()).unwrap();
        let conn = Connection::open(support::text(&request, "database")).unwrap();
        if let kassiber_pyjson::Value::Str(passphrase) = support::value(&request, "passphrase") {
            conn.pragma_update(None, "key", passphrase.as_str().unwrap())
                .unwrap();
            conn.execute_batch("PRAGMA cipher_log_level=NONE;PRAGMA cipher_compatibility=4;PRAGMA kdf_iter=256000;PRAGMA cipher_page_size=4096;").unwrap();
        }
        conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        let case = support::value(&request, "case");
        conn.execute_batch(support::text(case, "sql")).unwrap();
        writeln!(
            stdout,
            "{}",
            support::encode(&support::outcome(&conn, case))
        )
        .unwrap();
        stdout.flush().unwrap();
    }
}
