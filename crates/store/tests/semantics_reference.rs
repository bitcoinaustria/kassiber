//! The native journal calculation semantics follow the Python reference's.

use kassiber_store::semantics::JOURNAL_CALCULATION_SEMANTICS_VERSION;

#[test]
fn journal_semantics_version_matches_the_python_reference() {
    let reference = include_str!("../../../kassiber/db.py");
    let line = reference
        .lines()
        .find(|line| line.starts_with("JOURNAL_CALCULATION_SEMANTICS_VERSION = "))
        .expect("the reference names its calculation semantics");
    let python: i64 = line.rsplit(' ').next().unwrap().parse().unwrap();
    assert_eq!(JOURNAL_CALCULATION_SEMANTICS_VERSION, python);
}
