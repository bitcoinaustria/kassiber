//! The rate, threshold and allowance tables of the countries to come, from
//! their specifications (`docs/reference/tax-country-<code>.md`, sources
//! retrieved 2026-11-07). A country's registry entry lists the ones it
//! applies ([`crate::Country::tax_tables`]); the country vectors check
//! them (`crates/reports/tests/tax_country_vectors.rs`).

use crate::years::{Schedule, Step};

/// Germany, § 23 Abs. 3 Satz 5 EStG: private disposal gains are tax-free
/// when the year's total is below the Freigrenze (DE-FG-23). The
/// specification gives 600 € for every year through 2023; Bitcoin's first
/// year bounds the table.
pub const DE_P23_FREIGRENZE: Schedule = Schedule {
    id: "de.p23_freigrenze",
    steps: &[
        ("2009-01-01", Step::Value("600.00")),
        ("2024-01-01", Step::Value("1000.00")),
    ],
    known_until: Some("2026-12-31"),
    source:
        "§ 23 Abs. 3 Satz 5 EStG; Wachstumschancengesetz from 2024 (ELSTER Anlage SO 2024, 2025)",
};

/// Germany, § 22 Nr. 3 Satz 2 EStG: other income from services below 256 €
/// a year is not taxable (DE-FG-22-3).
pub const DE_P22_3_FREIGRENZE: Schedule = Schedule {
    id: "de.p22_3_freigrenze",
    steps: &[("2009-01-01", Step::Value("256.00"))],
    known_until: Some("2026-12-31"),
    source: "§ 22 Nr. 3 Satz 2 EStG; BMF 6 March 2025 Rn. 45",
};

/// The United Kingdom's annual exempt amount for individuals, by tax year
/// (6 April); 2026-27 is the last published (GB-AEA).
pub const GB_ANNUAL_EXEMPT_AMOUNT: Schedule = Schedule {
    id: "gb.annual_exempt_amount",
    steps: &[
        ("2018-04-06", Step::Value("11700.00")),
        ("2019-04-06", Step::Value("12000.00")),
        ("2020-04-06", Step::Value("12300.00")),
        ("2023-04-06", Step::Value("6000.00")),
        ("2024-04-06", Step::Value("3000.00")),
    ],
    known_until: Some("2027-04-05"),
    source: "GOV.UK Capital Gains Tax rates and annual tax-free allowances (updated 13 April 2026)",
};

/// The proceeds above which the SA108 pages are needed: four times the
/// annual exempt amount until 2022-23, £50,000 from 2023-24
/// (GB-REPORT-THRESHOLD).
pub const GB_PROCEEDS_THRESHOLD: Schedule = Schedule {
    id: "gb.proceeds_threshold",
    steps: &[
        ("2018-04-06", Step::Value("46800.00")),
        ("2019-04-06", Step::Value("48000.00")),
        ("2020-04-06", Step::Value("49200.00")),
        ("2023-04-06", Step::Value("50000.00")),
    ],
    known_until: Some("2027-04-05"),
    source: "SA108 notes 2023-24 to 2025-26",
};

/// The main CGT rates (basic/higher), whose change on 30 October 2024 splits
/// 2024-25 (GB-RATE-SPLIT). Kassiber computes no tax; the split is the
/// report's.
pub const GB_CGT_MAIN_RATES: Schedule = Schedule {
    id: "gb.cgt_main_rates",
    steps: &[
        ("2024-04-06", Step::Value("0.10/0.20")),
        ("2024-10-30", Step::Value("0.18/0.24")),
    ],
    known_until: Some("2027-04-05"),
    source: "SA108 notes 2024-25; GOV.UK Capital Gains Tax rates",
};

/// Ireland's CGT rate by the disposal's Dublin date (IE-R5); earlier
/// disposals are not modelled.
pub const IE_CGT_RATE: Schedule = Schedule {
    id: "ie.cgt_rate",
    steps: &[
        (
            "1970-01-01",
            Step::Refused("ie_rate_before_2012_12_06_not_modelled"),
        ),
        ("2012-12-06", Step::Value("0.33")),
        ("2026-10-07", Step::Value("0.31")),
    ],
    known_until: Some("2026-12-31"),
    source: "Revenue TDM 02-03-01A (October 2026); eBrief No. 142/26",
};

/// Ireland's annual exemption per individual, s.601 TCA (IE-R6); the years
/// before the rate table are refused there.
pub const IE_ANNUAL_EXEMPTION: Schedule = Schedule {
    id: "ie.annual_exemption",
    steps: &[("2012-01-01", Step::Value("1270.00"))],
    known_until: Some("2026-12-31"),
    source: "Revenue TDM 02-01-03; Notes for Guidance s.601",
};

/// Japan's regime for crypto gains: miscellaneous income (雑所得) through
/// 2026; from 2027 the separate taxation's start is not confirmed, so those
/// years are refused (JP-R16).
pub const JP_TAX_REGIME: Schedule = Schedule {
    id: "jp.tax_regime",
    steps: &[
        ("2009-01-01", Step::Value("zatsu_shotoku")),
        (
            "2027-01-01",
            Step::Refused("jp_separate_taxation_regime_unconfirmed"),
        ),
    ],
    known_until: None,
    source: "NTA crypto FAQ (December 2025); MOF 令和8年度税制改正の大綱",
};

/// Every table, for the checks every table must pass.
pub const TABLES: &[&Schedule] = &[
    &DE_P23_FREIGRENZE,
    &DE_P22_3_FREIGRENZE,
    &GB_ANNUAL_EXEMPT_AMOUNT,
    &GB_PROCEEDS_THRESHOLD,
    &GB_CGT_MAIN_RATES,
    &IE_CGT_RATE,
    &IE_ANNUAL_EXEMPTION,
    &JP_TAX_REGIME,
];
