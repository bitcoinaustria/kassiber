//! The Unicode tables are frozen CPython 3.11 data (Unicode 14.0.0): their
//! generator, and the test that compared them with the running interpreter,
//! went with the Python tree (plan 22). Stored bytes depend on them, so any
//! edit below the header fails here. Change them only through the reviewed
//! regeneration flow, with owner sign-off, and pin the new fingerprint.

#[test]
fn unicode_tables_are_unchanged() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/unicode_tables.rs");
    let text = std::fs::read(path).unwrap();
    let marker = b"pub(crate) const UNICODE_VERSION";
    let start = text
        .windows(marker.len())
        .position(|w| w == marker)
        .expect("UNICODE_VERSION");
    let body = &text[start..];
    // FNV-1a over the tables, from UNICODE_VERSION to the end of the file.
    let fingerprint = body.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    assert_eq!(
        (body.len(), fingerprint),
        (41_695, 0x9a3d_5ed7_66b1_8fc5),
        "{path} changed; its tables are frozen CPython 3.11 data"
    );
}
