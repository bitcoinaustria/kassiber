use rusqlite::{Connection, backup::Backup};
use std::{env, error::Error, time::Duration};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn literal(s: &str) -> Result<String> {
    if s.is_empty() || s.contains('\0') {
        return Err("empty and NUL-containing keys are refused".into());
    }
    Ok(format!("'{}'", s.replace('\'', "''")))
}

fn open(path: &str, key: &str) -> Result<Connection> {
    let db = Connection::open(path)?;
    db.execute_batch(&format!(
        "PRAGMA key={}; PRAGMA cipher_compatibility=4; PRAGMA kdf_iter=256000; PRAGMA cipher_page_size=4096;",
        literal(key)?
    ))?;
    db.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))?;
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=30000;")?;
    Ok(db)
}

fn journal(version: &str) -> (&'static str, &'static str) {
    let parts: Vec<_> = version.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|c| c.is_ascii_digit()))
    {
        return ("delete", "FULL");
    }
    let Ok(v) = parts
        .iter()
        .map(|s| s.parse::<u64>())
        .collect::<std::result::Result<Vec<_>, _>>()
    else {
        return ("delete", "FULL");
    };
    let fixed = v.as_slice() >= [3, 51, 3].as_slice()
        || (v[..2] == [3, 50] && v[2] >= 7)
        || (v[..2] == [3, 44] && v[2] >= 6);
    if fixed {
        ("wal", "NORMAL")
    } else {
        ("delete", "FULL")
    }
}

fn probe(db: &Connection) -> Result<()> {
    let sqlite: String = db.query_row("SELECT sqlite_version()", [], |r| r.get(0))?;
    let cipher: String = db.query_row("PRAGMA cipher_version", [], |r| r.get(0))?;
    let (mode, sync) = journal(&sqlite);
    let actual: String = db.query_row(&format!("PRAGMA journal_mode={mode}"), [], |r| r.get(0))?;
    assert_eq!(actual, mode);
    db.execute_batch(&format!("PRAGMA synchronous={sync}"))?;
    let json: i64 = db.query_row("SELECT json_extract('{\"a\":42}', '$.a')", [], |r| r.get(0))?;
    assert_eq!(json, 42);
    let json_each: i64 = db.query_row("SELECT sum(value) FROM json_each('[1,2,3]')", [], |r| {
        r.get(0)
    })?;
    assert_eq!(json_each, 6);
    let json_more: String = db.query_row("SELECT json_group_array(json_extract(value,'$.k')) FROM json_each('[{\"k\":1},{\"k\":2}]') WHERE json_valid(value)", [], |r| r.get(0))?;
    assert_eq!(json_more, "[1,2]");
    db.execute_batch("CREATE TABLE IF NOT EXISTS features(k INTEGER PRIMARY KEY, v INTEGER);")?;
    let v: i64 = db.query_row(
        "INSERT INTO features VALUES(1, 2) ON CONFLICT(k) DO UPDATE SET v=v+1 RETURNING v",
        [],
        |r| r.get(0),
    )?;
    assert!(v >= 2);
    let rank: i64 = db.query_row(
        "SELECT row_number() OVER (ORDER BY k) FROM features",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(rank, 1);
    let integrity: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    assert_eq!(integrity, "ok");
    let errors = db
        .prepare("PRAGMA cipher_integrity_check")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert!(errors.is_empty(), "{errors:?}");
    println!(
        "{}",
        serde_json::json!({"sqlite":sqlite,"sqlcipher":cipher,"journal":mode,"synchronous":sync,"json1":true,"window":true,"upsert_returning":true,"integrity":"ok"})
    );
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = env::args().collect();
    let [_, action, path, key, rest @ ..] = args.as_slice() else {
        return Err("action path key [destination/new-key]".into());
    };
    match action.as_str() {
        "schema" => {
            let db = open(path, key)?;
            db.execute_batch(&std::fs::read_to_string(&rest[0])?)?;
            let tables: i64 = db.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )?;
            assert!(tables >= 91);
            println!("current product SCHEMA created from Rust: {tables} tables");
            probe(&db)?;
        }
        "create" => {
            let db = open(path, key)?;
            db.execute_batch("CREATE TABLE evidence(id INTEGER PRIMARY KEY, value TEXT NOT NULL); INSERT INTO evidence VALUES(1, 'rust'); PRAGMA user_version=21;")?;
            probe(&db)?;
        }
        "read-write" => {
            let db = open(path, key)?;
            let value: String =
                db.query_row("SELECT value FROM evidence WHERE id=1", [], |r| r.get(0))?;
            assert!(value.starts_with("python") || value.starts_with("rust"));
            db.execute("INSERT INTO evidence(value) VALUES('rust-write')", [])?;
            probe(&db)?;
        }
        "rekey" => {
            let db = open(path, key)?;
            db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")?;
            db.execute_batch(&format!("PRAGMA rekey={}", literal(&rest[0])?))?;
        }
        "backup" => {
            let db = open(path, key)?;
            let mut destination = open(&rest[0], key)?;
            // A second handle remains open and a committed write precedes the hot snapshot.
            let writer = open(path, key)?;
            writer.execute(
                "INSERT INTO evidence(value) VALUES('before-hot-backup')",
                [],
            )?;
            Backup::new(&db, &mut destination)?.run_to_completion(
                1,
                Duration::from_millis(1),
                None,
            )?;
            assert_eq!(
                destination.query_row(
                    "SELECT count(*) FROM evidence WHERE value='before-hot-backup'",
                    [],
                    |r| r.get::<_, i64>(0)
                )?,
                1
            );
            probe(&destination)?;
        }
        "export" => {
            let db = Connection::open(path)?;
            db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")?;
            let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            let vacuum: i64 = db.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))?;
            db.execute_batch(&format!("ATTACH DATABASE {} AS encrypted KEY {}; PRAGMA encrypted.cipher_compatibility=4; PRAGMA encrypted.kdf_iter=256000; PRAGMA encrypted.cipher_page_size=4096; PRAGMA encrypted.auto_vacuum={vacuum}; SELECT sqlcipher_export('encrypted'); PRAGMA encrypted.user_version={version}; DETACH DATABASE encrypted;", literal(&rest[0])?, literal(key)?))?;
            let out = open(&rest[0], key)?;
            assert_eq!(
                out.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
                version
            );
            assert_eq!(
                out.query_row("PRAGMA auto_vacuum", [], |r| r.get::<_, i64>(0))?,
                vacuum
            );
            probe(&out)?;
        }
        _ => return Err("unknown action".into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fail_closed_keys_and_wal_versions() {
        assert!(literal("").is_err());
        assert!(literal("x\0y").is_err());
        assert_eq!(literal("a'b\n🔑").unwrap(), "'a''b\n🔑'");
        for v in ["3.51.3", "3.50.7", "3.44.6", "3.53.2"] {
            assert_eq!(journal(v).0, "wal");
        }
        for v in [
            "3.51.2",
            "3.50.6",
            "3.44.5",
            "3.45.9",
            "3.51.3-vendor",
            "３.51.3",
            "garbage",
        ] {
            assert_eq!(journal(v).0, "delete");
        }
    }
}
