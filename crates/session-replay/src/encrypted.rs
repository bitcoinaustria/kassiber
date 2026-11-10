//! Encrypted books in recordings (plan 22, decision B).
//!
//! A SQLCipher book's pages are random bytes, so a recording keeps it
//! decrypted, with the synthetic test passphrase that opens it, and a
//! replay encrypts it again before the process starts. Both directions go
//! through the product's own store code: `kassiber_store::encryption`'s
//! `open_container` (the product's keying: SQLCipher 4 compatibility, its
//! KDF iterations and page size) and `encrypt_plaintext` (the product's
//! encryption of a plaintext book). Every re-encrypted book is checked: the
//! product opens it with the recorded passphrase, its page size and page
//! count match the recorded original's, and its decrypted contents equal the
//! stored plaintext. SQLCipher's only file header is a random salt, so the
//! salt itself is not reproduced.
//!
//! Only passphrases that occur literally in the checked-in test sources may
//! enter a recording; the recorder refuses any session that hands its process
//! another (see `Allowlist`).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use kassiber_ownership::OwnerNamespace;
use kassiber_store::encryption::{encrypt_plaintext, open_container};
use kassiber_store::rusqlite::{self, Connection};
use serde::{Deserialize, Serialize};

/// How a recorded encrypted book is stored.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Encryption {
    /// The synthetic test passphrase that opens it.
    pub passphrase: String,
    /// The original ciphertext, kept only for the few sessions that cover
    /// opening real recorded ciphertext (unlock, rekey, wrong passphrase,
    /// crash recovery); otherwise the entry's blob is the plaintext.
    #[serde(default)]
    pub ciphertext: Option<String>,
    pub page_size: u32,
    pub page_count: u64,
}

/// Synthetic passphrases: every quoted literal in the test sources.
#[derive(Default)]
pub struct Allowlist(HashSet<String>);

impl Allowlist {
    /// One literal per line (record-sessions.sh extracts them from tests/).
    pub fn load(path: &Path) -> Self {
        Allowlist(
            fs::read_to_string(path)
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect(),
        )
    }

    pub fn allows(&self, passphrase: &str) -> bool {
        self.0.contains(passphrase)
    }
}

/// Whether `bytes` may be a SQLCipher book: not plaintext SQLite, whole
/// 1 KiB pages, and not text.
pub fn maybe_encrypted(bytes: &[u8]) -> bool {
    bytes.len() >= 4096
        && bytes.len().is_multiple_of(1024)
        && !bytes.starts_with(b"SQLite format 3\0")
        && std::str::from_utf8(&bytes[..256]).is_err()
}

fn page_facts(conn: &Connection) -> rusqlite::Result<(u32, u64)> {
    // A keyed connection answers `page_size` as SQLCipher's text setting.
    let number = |sql: &str| -> rusqlite::Result<i64> {
        conn.query_row(sql, [], |r| {
            Ok(match r.get_ref(0)? {
                rusqlite::types::ValueRef::Integer(i) => i,
                rusqlite::types::ValueRef::Text(t) => {
                    String::from_utf8_lossy(t).trim().parse().unwrap_or(-1)
                }
                _ => -1,
            })
        })
    };
    Ok((
        number("PRAGMA page_size")? as u32,
        number("PRAGMA page_count")? as u64,
    ))
}

/// Decrypt `bytes` with the first candidate passphrase the product accepts:
/// the plaintext book and how it was stored, or `None`.
pub fn decrypt(
    bytes: &[u8],
    candidates: &[String],
    scratch: &Path,
) -> Option<(Vec<u8>, Encryption)> {
    if !maybe_encrypted(bytes) {
        return None;
    }
    fs::create_dir_all(scratch).ok()?;
    let source = scratch.join("encrypted.sqlite3");
    fs::write(&source, bytes).ok()?;
    let result = candidates.iter().find_map(|passphrase| {
        let conn = open_container(&source, Some(passphrase)).ok()?;
        let (page_size, page_count) = page_facts(&conn).ok()?;
        let plain = scratch.join("plain.sqlite3");
        let _ = fs::remove_file(&plain);
        // SQLCipher refuses the backup API across keys; export instead. The
        // product's connection does not create files, so ATTACH needs one.
        fs::write(&plain, b"").ok()?;
        let quoted = plain.to_string_lossy().replace('\'', "''");
        conn.execute_batch(&format!(
            "ATTACH DATABASE '{quoted}' AS plaintext KEY ''; \
             SELECT sqlcipher_export('plaintext'); DETACH DATABASE plaintext;"
        ))
        .ok()?;
        drop(conn);
        let plaintext = fs::read(&plain).ok()?;
        Some((
            plaintext,
            Encryption {
                passphrase: passphrase.clone(),
                ciphertext: None,
                page_size,
                page_count,
            },
        ))
    });
    let _ = fs::remove_dir_all(scratch);
    result
}

/// Encrypt a recorded plaintext book again with the product's own
/// `encrypt_plaintext`, and verify it as the module documentation says.
pub fn encrypt(
    plaintext: &[u8],
    encryption: &Encryption,
    scratch: &Path,
) -> Result<Vec<u8>, String> {
    let fail = |what: &str| format!("re-encrypting a recorded book: {what}");
    let root = scratch.join("encrypt-root");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).map_err(|e| fail(&e.to_string()))?;
    let database: PathBuf = kassiber_paths::resolve_database_path(&root);
    fs::write(&database, plaintext).map_err(|e| fail(&e.to_string()))?;
    let namespace = OwnerNamespace::isolated(scratch.join("encrypt-owners"));
    let result = encrypt_plaintext(&namespace, &root, &encryption.passphrase, None)
        .map_err(|e| fail(&format!("{e:?}")))?;
    let encrypted = fs::read(&result.encrypted_path).map_err(|e| fail(&e.to_string()))?;
    // The product must open it with the recorded passphrase, and it must
    // hold exactly the recorded pages and contents.
    let conn = open_container(&result.encrypted_path, Some(&encryption.passphrase))
        .map_err(|e| fail(&format!("the product does not open it: {e:?}")))?;
    let (page_size, page_count) = page_facts(&conn).map_err(|e| fail(&e.to_string()))?;
    if (page_size, page_count) != (encryption.page_size, encryption.page_count) {
        return Err(fail(&format!(
            "page size/count {page_size}/{page_count}, recorded {}/{}",
            encryption.page_size, encryption.page_count
        )));
    }
    drop(conn);
    let check = scratch.join("encrypt-check");
    let (again, _) = decrypt(
        &encrypted,
        std::slice::from_ref(&encryption.passphrase),
        &check,
    )
    .ok_or_else(|| fail("it does not decrypt again"))?;
    let original = scratch.join("encrypt-original.sqlite3");
    let roundtrip = scratch.join("encrypt-roundtrip.sqlite3");
    fs::write(&original, plaintext).map_err(|e| fail(&e.to_string()))?;
    fs::write(&roundtrip, &again).map_err(|e| fail(&e.to_string()))?;
    let same = crate::dbdump::dump(&original).map_err(|e| fail(&e))?
        == crate::dbdump::dump(&roundtrip).map_err(|e| fail(&e))?;
    for path in [&original, &roundtrip] {
        let _ = fs::remove_file(path);
    }
    let _ = fs::remove_dir_all(&root);
    if !same {
        return Err(fail("its decrypted contents differ from the recording"));
    }
    Ok(encrypted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_book_round_trips_through_the_product_encryption() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("book.sqlite3");
        let conn = Connection::open(&plain).unwrap();
        conn.execute_batch("CREATE TABLE t(a TEXT); INSERT INTO t VALUES ('x'), ('y');")
            .unwrap();
        drop(conn);
        let plaintext = fs::read(&plain).unwrap();
        let probe = Encryption {
            passphrase: "synthetic-test-only".into(),
            ciphertext: None,
            page_size: 4096,
            page_count: 0,
        };
        // Learn the page count the product's encryption produces.
        let error = encrypt(&plaintext, &probe, &dir.path().join("a")).unwrap_err();
        assert!(error.contains("page size/count"), "{error}");
        let count: u64 = error
            .split("page size/count 4096/")
            .nth(1)
            .unwrap()
            .split(',')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let recorded = Encryption {
            page_count: count,
            ..probe
        };
        let encrypted = encrypt(&plaintext, &recorded, &dir.path().join("b")).unwrap();
        assert!(maybe_encrypted(&encrypted));
        let (back, found) = decrypt(
            &encrypted,
            &["wrong".into(), "synthetic-test-only".into()],
            &dir.path().join("c"),
        )
        .unwrap();
        assert_eq!(found.passphrase, "synthetic-test-only");
        assert_eq!(found.page_count, count);
        let back_path = dir.path().join("back.sqlite3");
        fs::write(&back_path, back).unwrap();
        assert_eq!(
            crate::dbdump::dump(&back_path).unwrap(),
            crate::dbdump::dump(&plain).unwrap()
        );
        assert!(decrypt(&encrypted, &["wrong".into()], &dir.path().join("d")).is_none());
    }
}
