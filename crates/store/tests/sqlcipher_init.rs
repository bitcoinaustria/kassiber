//! SQLCipher registers `sqlcipher_export` at the end of SQLite's one-time
//! initialization, after marking SQLite initialized: a thread whose first
//! connection opened inside that window got none (seen in parallel CI tests,
//! #800). The store finishes initialization before any connection opens, so
//! books opened by racing threads in a fresh process all have it. One test in
//! its own binary: the process must not have touched SQLite before.
use kassiber_store::Book;
use std::sync::{Arc, Barrier};

#[test]
fn books_opened_on_racing_first_threads_all_have_sqlcipher_export() {
    let root = tempfile::tempdir().unwrap();
    let threads = 16;
    let barrier = Arc::new(Barrier::new(threads));
    let handles: Vec<_> = (0..threads)
        .map(|n| {
            let barrier = Arc::clone(&barrier);
            let data = root.path().join(format!("book-{n}"));
            std::thread::spawn(move || {
                barrier.wait();
                let book = Book::create(&data, None).unwrap();
                // A missing function is "no such function"; with it, the
                // unknown schema name is the error.
                let error = book
                    .connection()
                    .query_row("SELECT sqlcipher_export('kassiber_absent')", [], |_| Ok(()))
                    .unwrap_err()
                    .to_string();
                assert!(!error.contains("no such function"), "{error}");
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
}
