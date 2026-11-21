//! Bank-statement evidence never reaches diagnostics
//! (docs/reference/privacy-and-security.md#bank-statements): a book holding
//! bank lines yields byte-identical state aggregates, with no name, IBAN,
//! remittance text or amount from them.
use kassiber_diagnostics::state;
use kassiber_pyjson::{dumps, Options};
use kassiber_store::Book;

const PLANTED: &[&str] = &[
    "Planted Counterparty GmbH",
    "AT** **** **** **** 9999",
    "planted remittance text",
    "PLANTED-BANK-REF",
    "-4321.09",
    "Planted Bank AG",
];

#[test]
fn bank_lines_stay_out_of_diagnostics_state() {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(&dir.path().join("data"), None).unwrap();
    let conn = book.connection();
    conn.execute_batch("INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025'); INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','w','Main','2025');").unwrap();
    let config = kassiber_pytext::loads("{}").unwrap();
    let render = || {
        dumps(
            &state::state(conn, state::backends(&config)).unwrap(),
            &Options::default(),
        )
        .unwrap()
    };
    let before = render();
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    kassiber_store::optional::ensure_bank_statement_schema(conn).unwrap();
    conn.execute_batch("COMMIT").unwrap();
    conn.execute_batch("INSERT INTO external_documents(id,workspace_id,profile_id,document_type,label,created_at,updated_at) VALUES('d','w','p','statement','Planted Bank AG statement','2025','2025'); INSERT INTO bank_statement_imports(id,workspace_id,profile_id,document_id,source_format,file_sha256,file_key,account_keys,bank_name,imported_at) VALUES('i','w','p','d','csv','00','00','00','Planted Bank AG','2025'); INSERT INTO bank_statement_lines(id,workspace_id,profile_id,import_id,document_id,line_index,booking_date,amount,currency,counterparty_name,counterparty_iban_masked,remittance,bank_reference,account_id,dedupe_group,dedupe_key,created_at) VALUES('l','w','p','i','d',1,'2025-03-03','-4321.09','EUR','Planted Counterparty GmbH','AT** **** **** **** 9999','planted remittance text','PLANTED-BANK-REF','AT000PLANTEDOWNACCOUNT9','g','k','2025');").unwrap();
    let after = render();
    assert_eq!(after, before);
    for planted in PLANTED {
        assert!(!after.contains(planted), "leaked {planted}");
    }
}
