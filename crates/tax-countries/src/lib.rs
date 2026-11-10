//! The tax countries Kassiber supports.
//!
//! One registry answers every question a crate asks about a book's
//! `tax_country`: which methods, pool scopes and currency the profile may
//! use, how the holding period is measured, which calendar and language its
//! reports use, which rules the tax engine applies, and whether a tax
//! professional for the country has reviewed it (`preview` until then).
//!
//! Callers keep their own reading of the stored value (Python's stripping and
//! casing differ between call sites, and parity keeps them) and look the
//! normalized code up here. An unknown code has no entry, so every caller
//! refuses it. Adding a country is adding an entry: nothing else may branch
//! on a country code. See `docs/reference/tax-countries.md`.

use serde_json::{json, Value};

pub mod calendar;
pub mod tables;
pub mod years;

/// The country a profile without one uses.
pub const DEFAULT_CODE: &str = "generic";

/// Text every report and screen of a preview country carries.
pub const PREVIEW_NOTICE_EN: &str =
    "Not yet reviewed by a tax professional for this country — check results before filing.";
/// The German text of [`PREVIEW_NOTICE_EN`].
pub const PREVIEW_NOTICE_DE: &str = "Noch nicht von einer Steuerberaterin oder einem Steuerberater für dieses Land geprüft — Ergebnisse vor der Einreichung kontrollieren.";
/// The Spanish text of [`PREVIEW_NOTICE_EN`], for Spanish reports.
pub const PREVIEW_NOTICE_ES: &str = "Aún no revisado por un asesor fiscal de este país: compruebe los resultados antes de presentar la declaración.";
/// The Japanese text of [`PREVIEW_NOTICE_EN`], for Japanese reports.
pub const PREVIEW_NOTICE_JA: &str =
    "この国の税理士による確認はまだ行われていません。申告前に結果を確認してください。";

/// One tax country.
#[derive(Debug, PartialEq, Eq)]
pub struct Country {
    /// The stored `tax_country` value.
    pub code: &'static str,
    /// The ISO 3166-1 code, or `None` for the generic country.
    pub iso: Option<&'static str>,
    pub name_en: &'static str,
    pub name_de: &'static str,
    /// The name in the country's own report language when that is neither
    /// English nor German (Spain's `España`); the desktop shows English or
    /// German, so only reports read it.
    pub name_local: Option<&'static str>,
    /// Not yet reviewed by a tax professional for the country.
    pub preview: bool,
    /// The legal rules the tax engine and the country's reports apply.
    pub rules: Rules,
    /// The book's fiat currency.
    pub fiat: Fiat,
    /// How the holding period decides long-term gains.
    pub long_term: LongTerm,
    /// The accounting methods a profile may choose, as stored (upper case),
    /// sorted.
    pub methods: &'static [&'static str],
    /// The method a new profile gets.
    pub default_method: &'static str,
    /// The cost-basis pool scopes a profile may choose, sorted.
    pub pool_scopes: &'static [&'static str],
    /// The scope a new profile gets.
    pub default_pool_scope: &'static str,
    /// The locale of the country's reports: their string table and number
    /// and date formats (CSV stays machine format).
    pub locale: Locale,
    /// The IANA time zone whose local dates cut the book's tax years and
    /// report periods and count its holding periods. Only the zones in
    /// [`SUPPORTED_ZONES`] have a calendar in the report readers yet.
    pub timezone: &'static str,
    /// The first day (month, day) of the tax year in [`Country::timezone`];
    /// `(1, 1)` is the calendar year.
    pub tax_year_start: (u8, u8),
    /// Reviewed carrying-value pairs between unlike assets carry basis.
    /// Otherwise only the Bitcoin rail pair (BTC and LBTC) may.
    pub cross_asset_carry: bool,
    /// A BTC/LBTC rail pair needs the book's explicit policy for that pair:
    /// the profile's `bitcoin_rail_carrying_value` default does not decide
    /// it, and an undecided pair is held (`rail_policy_required`).
    pub rail_decision_required: bool,
    /// The report generators the country offers (RP2's names for the shared
    /// ones, `<code>.<report>` for a country report).
    pub report_generators: &'static [&'static str],
    /// The inbound kinds the country books as ordinary income at fair
    /// market value when received.
    pub income_categories: &'static [&'static str],
    /// The legal sources the rules follow.
    pub sources: &'static [Source],
    /// The rate, threshold and allowance tables the country's layer
    /// applies, by local date ([`tables`]).
    pub tax_tables: &'static [&'static years::Schedule],
    /// The shared engine rules the country is computed with, instead of
    /// its family's engine country (generic, Austria, the United States):
    /// its holding period ([`Country::long_term`]) counted in
    /// [`Country::timezone`]'s local dates, and the features
    /// [`EngineRules`] lists. `None` keeps the family's.
    pub engine: Option<EngineRules>,
}

/// The shared tax-engine features a country switches on. Each field's
/// default is the generic rule.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EngineRules {
    /// What fee units paid in the asset are.
    pub fee_units: FeeUnits,
    /// The values of a taxable exchange of one asset for another.
    pub swap_values: SwapValues,
    /// What gifts and donations are.
    pub gifts: Gifts,
    /// Kinds the country refuses to book until a reviewer settles them,
    /// each with its fail-closed code.
    pub refused_receipts: &'static [RefusedReceipt],
    /// Inbound kinds booked as acquisitions at zero cost, with no income.
    pub zero_cost_kinds: &'static [&'static str],
    /// What a sale or payment below the market value of the units given is
    /// worth, when the row records that market value.
    pub below_market: BelowMarket,
}

/// Kinds a country refuses (the engine's `refused_receipts`): an inbound
/// receipt is held as `acquisition_valuation_unsupported` with `code`, an
/// outbound row as `non_sale_disposal_kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RefusedReceipt {
    /// Whether the refusal reads inbound or outbound rows.
    pub direction: Direction,
    /// Normalized transaction kinds of an inbound row, or tokens of an
    /// outbound row's kind (as `lost` matches `lost_keys`), each a non-sale
    /// disposal token.
    pub kinds: &'static [&'static str],
    /// The country's fail-closed code (its specification's).
    pub code: &'static str,
    /// Why, in English.
    pub detail: &'static str,
}

/// A row's direction, as refusals read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Inbound,
    Outbound,
}

/// What a sale or payment below market value is worth (the engine's
/// `below_market`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BelowMarket {
    /// The agreed price (the generic rule).
    #[default]
    Price,
    /// The market value when it is higher (Spain, art. 35.2 LIRPF).
    MarketValue,
    /// 70 % of the market value when the price is below that (Japan, NTA
    /// FAQ 2-10).
    SeventyPercent,
}

impl BelowMarket {
    /// The engine's name.
    pub fn code(self) -> &'static str {
        match self {
            BelowMarket::Price => "price",
            BelowMarket::MarketValue => "market_value",
            BelowMarket::SeventyPercent => "seventy_percent",
        }
    }
}

/// What gifts and donations are (the engine's `gifts_given`,
/// `donations_given`, `gifts_received` and `gift_losses`). The default is
/// the generic rule: outbound gifts and donations are quarantined, inbound
/// gifts are acquisitions at their value, and losses count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gifts {
    /// Outbound gifts are disposals at their value.
    pub given_at_value: bool,
    /// Outbound donations are disposals at their value.
    pub donated_at_value: bool,
    /// Inbound gifts are refused: the donor's date and cost carry over and
    /// the book has neither (Germany).
    pub received_refused: bool,
    /// A gift's or donation's loss is reported but does not count (Spain).
    pub losses_not_deductible: bool,
}

/// The values of a taxable cross-asset exchange (the engine's
/// `swap_values`): the given asset's proceeds and the received asset's
/// cost. With one leg unpriced, every mode but `Own` uses the other's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SwapValues {
    /// Each leg at its own value.
    #[default]
    Own,
    /// Proceeds the received value, cost the given value (Germany, the
    /// United Kingdom).
    ReceivedGiven,
    /// Both the received value (Ireland, Japan).
    Received,
    /// Both the higher value (Spain).
    Higher,
}

impl SwapValues {
    /// The engine's name.
    pub fn code(self) -> &'static str {
        match self {
            SwapValues::Own => "own",
            SwapValues::ReceivedGiven => "received_given",
            SwapValues::Received => "received",
            SwapValues::Higher => "higher",
        }
    }
}

/// What fee units paid in the asset are (the engine's `fee_units`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FeeUnits {
    /// Their basis leaves the book without a gain; a sale's fee units are
    /// part of the sale.
    #[default]
    Basis,
    /// Disposals at their value; a sale's fee units stay part of the sale
    /// (Spain, Ireland).
    Disposal,
    /// Disposals at their value, a sale's taken before its other units, whose
    /// gain the fee's value reduces (Germany, the United Kingdom).
    DisposalBeforeSale,
}

impl FeeUnits {
    /// The engine's name.
    pub fn code(self) -> &'static str {
        match self {
            FeeUnits::Basis => "basis",
            FeeUnits::Disposal => "disposal",
            FeeUnits::DisposalBeforeSale => "disposal_before_sale",
        }
    }
}

impl EngineRules {
    /// The rules as the engine's `rules` country fields, leaving out each
    /// generic one.
    pub fn engine_fields(&self) -> serde_json::Map<String, Value> {
        let mut fields = serde_json::Map::new();
        if self.fee_units != FeeUnits::Basis {
            fields.insert("fee_units".into(), json!(self.fee_units.code()));
        }
        if self.swap_values != SwapValues::Own {
            fields.insert("swap_values".into(), json!(self.swap_values.code()));
        }
        let gifts = self.gifts;
        for (on, key, value) in [
            (gifts.given_at_value, "gifts_given", "market_value"),
            (gifts.donated_at_value, "donations_given", "market_value"),
            (gifts.received_refused, "gifts_received", "quarantine"),
            (gifts.losses_not_deductible, "gift_losses", "not_deductible"),
        ] {
            if on {
                fields.insert(key.into(), json!(value));
            }
        }
        if !self.refused_receipts.is_empty() {
            fields.insert(
                "refused_receipts".into(),
                Value::Array(
                    self.refused_receipts
                        .iter()
                        .map(|refused| {
                            let mut rule = json!({"kinds": refused.kinds, "code": refused.code, "detail": refused.detail});
                            // Inbound is the engine's default, so the
                            // receipts' rules keep their bytes.
                            if refused.direction == Direction::Outbound {
                                rule["direction"] = json!("outbound");
                            }
                            rule
                        })
                        .collect(),
                ),
            );
        }
        if !self.zero_cost_kinds.is_empty() {
            fields.insert("zero_cost_kinds".into(), json!(self.zero_cost_kinds));
        }
        if self.below_market != BelowMarket::Price {
            fields.insert("below_market".into(), json!(self.below_market.code()));
        }
        fields
    }
}

/// The legal rules a country's books follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rules {
    /// Kassiber's generic rules: a holding period in days and RP2's methods.
    Generic,
    /// Austria: Alt/Neu regimes, the moving average, E 1kv Kennzahlen and
    /// exit tax (`docs/austrian-handoff.md`).
    Austria,
    /// The United States: Form 8949 and Schedule D, wallet-by-wallet basis
    /// from 2025 (`docs/reference/tax-country-us.md`).
    UnitedStates,
    /// Germany: § 23 EStG one-year period by Berlin date, wallet-based
    /// FIFO, Freigrenzen, Anlage SO (`docs/reference/tax-country-de.md`).
    Germany,
    /// Switzerland: private gains tax-free; income, year-end wealth and the
    /// KS 36 indicators (`docs/reference/tax-country-ch.md`).
    Switzerland,
    /// Ireland: CGT by Dublin date, the annual exemption, payment periods
    /// and Form CG1 lines (`docs/reference/tax-country-ie.md`).
    Ireland,
    /// The United Kingdom: share matching, tax years from 6 April and the
    /// SA108 boxes (`docs/reference/tax-country-gb.md`).
    UnitedKingdom,
    /// Spain: IRPF ganancias patrimoniales, global FIFO, casilla 1804
    /// (`docs/reference/tax-country-es.md`).
    Spain,
    /// Japan: 雑所得, the total or moving average, the NTA calculation sheet
    /// (`docs/reference/tax-country-jp.md`).
    Japan,
}

/// The book's fiat currency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fiat {
    /// The profile chooses it.
    Profile,
    /// The country requires this ISO 4217 code; a profile's own is replaced.
    Fixed(&'static str),
}

/// How the holding period decides a long-term gain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LongTerm {
    /// The profile's `tax_long_term_days` (this default when unset): long
    /// term once the whole days held reach it.
    ProfileDays { default: i64 },
    /// Never long term (stored as `i64::MAX` days, RP2's `sys.maxsize`).
    Never,
    /// Held more than one year: from the day after acquisition through the
    /// day of disposal, by local date in the country's calendar, so a
    /// disposal on the anniversary is still short-term and 29 February's
    /// anniversary is 28 February (stored as 365 days, which nothing reads
    /// for this rule).
    MoreThanOneYear,
}

impl LongTerm {
    /// The `tax_long_term_days` a profile stores for this rule, given the
    /// days it asked for.
    pub fn stored_days(self, requested: Option<i64>) -> i64 {
        match self {
            LongTerm::ProfileDays { default } => requested.unwrap_or(default),
            LongTerm::Never => i64::MAX,
            LongTerm::MoreThanOneYear => 365,
        }
    }
}

/// A language for names and notices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    En,
    De,
    Es,
    Ja,
}

/// A report locale. Each has its own string table and formats: Austrian
/// German (`Jänner`, `1.234,56`) is not Germany's or Switzerland's German.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locale {
    /// English, `1,234.56`, ISO dates.
    En,
    /// Austrian German, `1.234,56`, `15.03.2025`.
    DeAt,
    /// Germany's German (`Januar`), `1.234,56`, `15.03.2025`.
    DeDe,
    /// Swiss German (no ß), `1'234.56`, `31.12.2024`.
    DeCh,
    /// Spanish (Spain), `1.234,56`, `15/03/2025`.
    Es,
    /// Japanese, `1,234.56`, `2025/03/15`.
    Ja,
}

impl Locale {
    /// The BCP 47 tag.
    pub fn code(self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::DeAt => "de-AT",
            Locale::DeDe => "de-DE",
            Locale::DeCh => "de-CH",
            Locale::Es => "es-ES",
            Locale::Ja => "ja-JP",
        }
    }
    /// The locale's language.
    pub fn language(self) -> Language {
        match self {
            Locale::En => Language::En,
            Locale::DeAt | Locale::DeDe | Locale::DeCh => Language::De,
            Locale::Es => Language::Es,
            Locale::Ja => Language::Ja,
        }
    }
}

impl Language {
    /// The ISO 639-1 code.
    pub fn code(self) -> &'static str {
        match self {
            Language::En => "en",
            Language::De => "de",
            Language::Es => "es",
            Language::Ja => "ja",
        }
    }
}

/// UTC, the generic and the US calendar.
pub const UTC: &str = "UTC";
/// Austria's calendar.
pub const VIENNA: &str = "Europe/Vienna";
/// Germany's calendar.
pub const BERLIN: &str = "Europe/Berlin";
/// Switzerland's calendar.
pub const ZURICH: &str = "Europe/Zurich";
/// Ireland's calendar.
pub const DUBLIN: &str = "Europe/Dublin";
/// The United Kingdom's calendar.
pub const LONDON: &str = "Europe/London";
/// Spain's calendar (peninsular time; the Canary Islands are open question
/// ES-Q7).
pub const MADRID: &str = "Europe/Madrid";
/// Japan's calendar.
pub const TOKYO: &str = "Asia/Tokyo";
/// The zones whose calendars the report readers implement (Vienna its
/// pinned table, the others `calendar`'s rule, checked against tzdata in
/// `kassiber-reports`). A country in another zone needs that check first (a
/// test keeps the registry to these).
pub const SUPPORTED_ZONES: &[&str] = &[UTC, VIENNA, BERLIN, ZURICH, DUBLIN, LONDON, MADRID, TOKYO];

/// A legal source a country's rules follow.
#[derive(Debug, PartialEq, Eq)]
pub struct Source {
    pub title: &'static str,
    pub url: &'static str,
    /// The day the source was last read (YYYY-MM-DD).
    pub retrieved: &'static str,
}

/// RP2's earn types: received units that are income at their value.
const EARN_KINDS: &[&str] = &[
    "airdrop", "hardfork", "income", "interest", "mining", "staking",
];

const GENERIC: Country = Country {
    code: "generic",
    iso: None,
    name_en: "Generic",
    name_de: "Allgemein",
    name_local: None,
    preview: false,
    rules: Rules::Generic,
    fiat: Fiat::Profile,
    long_term: LongTerm::ProfileDays { default: 365 },
    methods: &["FIFO", "HIFO", "LIFO", "LOFO", "MOVING_AVERAGE"],
    default_method: "FIFO",
    pool_scopes: &["global"],
    default_pool_scope: "global",
    locale: Locale::En,
    timezone: UTC,
    tax_year_start: (1, 1),
    cross_asset_carry: false,
    rail_decision_required: false,
    report_generators: &["open_positions", "rp2_full_report"],
    income_categories: EARN_KINDS,
    sources: &[],
    tax_tables: &[],
    engine: None,
};

const AUSTRIA: Country = Country {
    code: "at",
    iso: Some("AT"),
    name_en: "Austria",
    name_de: "Österreich",
    name_local: None,
    preview: false,
    rules: Rules::Austria,
    fiat: Fiat::Fixed("EUR"),
    long_term: LongTerm::Never,
    // Austria's moving average (§ 2 KryptowährungsVO) stays the default; the
    // generic lot methods are a deliberate choice.
    methods: &[
        "FIFO",
        "HIFO",
        "LIFO",
        "LOFO",
        "MOVING_AVERAGE",
        "MOVING_AVERAGE_AT",
    ],
    default_method: "MOVING_AVERAGE_AT",
    pool_scopes: &["global"],
    default_pool_scope: "global",
    locale: Locale::DeAt,
    timezone: VIENNA,
    tax_year_start: (1, 1),
    cross_asset_carry: true,
    rail_decision_required: false,
    report_generators: &["open_positions", "at.e1kv", "at.exit_tax"],
    income_categories: EARN_KINDS,
    sources: &[Source {
        title: "BMF: Steuerliche Behandlung von Kryptowährungen (§ 27b EStG, KryptowährungsVO)",
        url: "https://www.bmf.gv.at/themen/steuern/sparen-veranlagen/steuerliche-behandlung-von-kryptowaehrungen.html",
        retrieved: "2026-11-07",
    }],
    tax_tables: &[],
    engine: None,
};

const UNITED_STATES: Country = Country {
    code: "us",
    iso: Some("US"),
    name_en: "United States",
    name_de: "Vereinigte Staaten",
    name_local: None,
    preview: true,
    rules: Rules::UnitedStates,
    fiat: Fiat::Fixed("USD"),
    long_term: LongTerm::MoreThanOneYear,
    // FIFO is the default identification (IRS virtual currency FAQ 41); the
    // others are standing-order specific identification.
    methods: &["FIFO", "HIFO", "LIFO", "LOFO"],
    default_method: "FIFO",
    // Per wallet from 2025 (Treas. Reg. § 1.1012-1(j)); global (universal)
    // for books that end before 2025.
    pool_scopes: &["global", "wallet"],
    default_pool_scope: "wallet",
    locale: Locale::En,
    // A documented choice: Kassiber has no per-book zone yet.
    timezone: UTC,
    tax_year_start: (1, 1),
    cross_asset_carry: false,
    rail_decision_required: false,
    report_generators: &["open_positions", "us.form_8949"],
    income_categories: EARN_KINDS,
    sources: &[
        Source {
            title: "IRS Publication 544, Sales and Other Dispositions of Assets (holding period)",
            url: "https://www.irs.gov/publications/p544",
            retrieved: "2026-11-07",
        },
        Source {
            title: "IRS Publication 550, Investment Income and Expenses (holding period, digital assets)",
            url: "https://www.irs.gov/publications/p550",
            retrieved: "2026-11-07",
        },
        Source {
            title: "IRS Frequently Asked Questions on Virtual Currency Transactions",
            url: "https://www.irs.gov/individuals/international-taxpayers/frequently-asked-questions-on-virtual-currency-transactions",
            retrieved: "2026-11-07",
        },
        Source {
            title: "Rev. Proc. 2024-28 (basis per wallet or account from 2025)",
            url: "https://www.irs.gov/pub/irs-drop/rp-24-28.pdf",
            retrieved: "2026-11-07",
        },
        Source {
            title: "Rev. Rul. 2023-14 (staking rewards)",
            url: "https://www.irs.gov/pub/irs-drop/rr-23-14.pdf",
            retrieved: "2026-11-07",
        },
        Source {
            title: "Instructions for Form 8949 (2025)",
            url: "https://www.irs.gov/instructions/i8949",
            retrieved: "2026-11-07",
        },
        Source {
            title: "Instructions for Schedule D (Form 1040) (2025)",
            url: "https://www.irs.gov/instructions/i1040sd",
            retrieved: "2026-11-07",
        },
    ],
    tax_tables: &[],
    engine: None,
};

/// Hard forks: what the new coins cost is not modelled (Germany's split by
/// market values, Switzerland's unsettled treatment).
const HARD_FORK_KINDS: &[&str] = &["hard_fork", "hardfork"];

const GERMANY: Country = Country {
    code: "de",
    iso: Some("DE"),
    name_en: "Germany",
    name_de: "Deutschland",
    name_local: None,
    preview: true,
    rules: Rules::Germany,
    fiat: Fiat::Fixed("EUR"),
    // § 23 Abs. 1 Satz 1 Nr. 2 EStG: taxable while disposed of no more than
    // one year after acquisition, by Berlin calendar date (§§ 187, 188 BGB).
    long_term: LongTerm::MoreThanOneYear,
    // BMF 6 March 2025 Rn. 61: FIFO per wallet, or the average value with
    // FIFO deciding the holding period.
    methods: &["FIFO", "MOVING_AVERAGE_FIFO_PERIOD"],
    default_method: "FIFO",
    // Rn. 62: the order of use is per wallet.
    pool_scopes: &["wallet"],
    default_pool_scope: "wallet",
    locale: Locale::DeDe,
    timezone: BERLIN,
    tax_year_start: (1, 1),
    cross_asset_carry: false,
    // Whether L-BTC is the same Wirtschaftsgut as BTC is not settled.
    rail_decision_required: true,
    report_generators: &["open_positions", "de.anlage_so"],
    // § 22 Nr. 3 (staking, lending, routing) and block rewards, which the
    // report keeps apart until classified; airdrops and forks are refused.
    income_categories: &["income", "interest", "mining", "staking"],
    sources: &[
        Source {
            title: "§ 23 EStG, private Veräußerungsgeschäfte",
            url: "https://www.gesetze-im-internet.de/estg/__23.html",
            retrieved: "2026-11-07",
        },
        Source {
            title: "§ 22 EStG, Nr. 2 and Nr. 3",
            url: "https://www.gesetze-im-internet.de/estg/__22.html",
            retrieved: "2026-11-07",
        },
        Source {
            title: "§ 108 AO (periods: §§ 187, 188 BGB)",
            url: "https://www.gesetze-im-internet.de/ao_1977/__108.html",
            retrieved: "2026-11-07",
        },
        Source {
            title: "BMF-Schreiben vom 6. März 2025, Einzelfragen zur ertragsteuerrechtlichen Behandlung bestimmter Kryptowerte (BStBl I S. 658)",
            url: "https://www.bundesfinanzministerium.de/Content/DE/Downloads/BMF_Schreiben/Steuerarten/Einkommensteuer/2025-03-06-einzelfragen-kryptowerte.html",
            retrieved: "2026-11-07",
        },
        Source {
            title: "BFH, Urteil vom 11.01.2005, IX R 27/04 (Freigrenze before loss carry-back)",
            url: "https://dejure.org/dienste/vernetzung/rechtsprechung?Text=IX+R+27%2F04",
            retrieved: "2026-11-07",
        },
        Source {
            title: "ELSTER help, Einkommensteuer Anlage SO 2025",
            url: "https://www.elster.de/eportal/helpGlobal?themaGlobal=help_est_ufa_10_2025",
            retrieved: "2026-11-07",
        },
        Source {
            title: "ELSTER help, Einkommensteuer Anlage SO 2024",
            url: "https://www.elster.de/eportal/helpGlobal?themaGlobal=help_est_ufa_10_2024",
            retrieved: "2026-11-07",
        },
    ],
    tax_tables: &[&tables::DE_P23_FREIGRENZE, &tables::DE_P22_3_FREIGRENZE],
    engine: Some(EngineRules {
        // Rn. 54, 60: fee units are given for a service; their value is
        // Werbungskosten of the sale (Rn. 59).
        fee_units: FeeUnits::DisposalBeforeSale,
        // Rn. 58 and 59.
        swap_values: SwapValues::ReceivedGiven,
        // § 23 Abs. 1 Satz 3: the donor's date and cost carry over.
        gifts: Gifts {
            given_at_value: false,
            donated_at_value: false,
            received_refused: true,
            losses_not_deductible: false,
        },
        refused_receipts: &[
            RefusedReceipt {
                direction: Direction::Inbound,
                kinds: &["airdrop"],
                code: "airdrop_classification_required",
                detail: "An airdrop is § 22 Nr. 3 income only for a service (BMF 2025 Rn. 70-74); it needs a reviewed classification.",
            },
            RefusedReceipt {
                direction: Direction::Inbound,
                kinds: HARD_FORK_KINDS,
                code: "hardfork_cost_split_unsupported",
                detail: "Fork coins share the original acquisition date and split its cost by market values (BMF 2025 Rn. 67), which Kassiber does not model.",
            },
        ],
        zero_cost_kinds: &[],
        below_market: BelowMarket::Price,
    }),
};

const SWITZERLAND: Country = Country {
    code: "ch",
    iso: Some("CH"),
    name_en: "Switzerland",
    name_de: "Schweiz",
    name_local: None,
    preview: true,
    rules: Rules::Switzerland,
    fiat: Fiat::Fixed("CHF"),
    // A private investor's capital gains are tax-free (Art. 16 Abs. 3 DBG):
    // no holding period decides a gain. KS 36's six months is the layer's
    // indicator, not an exemption.
    long_term: LongTerm::Never,
    // Gains are informational; the method has no tax effect.
    methods: &["FIFO", "MOVING_AVERAGE"],
    default_method: "FIFO",
    pool_scopes: &["global"],
    default_pool_scope: "global",
    locale: Locale::DeCh,
    timezone: ZURICH,
    tax_year_start: (1, 1),
    cross_asset_carry: false,
    // A peg has no income-tax effect for a private investor (CH-RAIL).
    rail_decision_required: false,
    report_generators: &["open_positions", "ch.wealth_income"],
    income_categories: &["airdrop", "income", "interest", "mining", "staking"],
    sources: &[
        Source {
            title: "Bundesgesetz über die direkte Bundessteuer (DBG), SR 642.11",
            url: "https://www.fedlex.admin.ch/eli/cc/1991/1184_1184_1184/de",
            retrieved: "2026-11-07",
        },
        Source {
            title: "Steuerharmonisierungsgesetz (StHG), SR 642.14",
            url: "https://www.fedlex.admin.ch/eli/cc/1991/1256_1256_1256/de",
            retrieved: "2026-11-07",
        },
        Source {
            title: "ESTV Arbeitspapier Kryptowährungen und Initial Coin/Token Offerings (14. Dezember 2021)",
            url: "https://www.estv.admin.ch/dam/de/sd-web/HgzNhzz7q7YD/dbst-arbeitspapier-kryptowaehrungen-de.pdf",
            retrieved: "2026-11-07",
        },
        Source {
            title: "ESTV Dossier Steuerinformationen B: Kryptowährung (Oktober 2023)",
            url: "https://www.estv2.admin.ch/stp/ds/b-kryptowaehrung-de.pdf",
            retrieved: "2026-11-07",
        },
        Source {
            title: "ESTV Kreisschreiben Nr. 36: Gewerbsmässiger Wertschriftenhandel (27. Juli 2012)",
            url: "https://www.estv2.admin.ch/dvs/kreisschreiben/dbst-ks-2012-1-036-d-de.pdf",
            retrieved: "2026-11-07",
        },
        Source {
            title: "ESTV Kursliste / ICTax (year-end tax values)",
            url: "https://www.ictax.admin.ch/extern/de.html",
            retrieved: "2026-11-07",
        },
    ],
    tax_tables: &[],
    engine: Some(EngineRules {
        fee_units: FeeUnits::Basis,
        swap_values: SwapValues::Own,
        gifts: Gifts {
            given_at_value: false,
            donated_at_value: false,
            received_refused: false,
            losses_not_deductible: false,
        },
        refused_receipts: &[RefusedReceipt {
            direction: Direction::Inbound,
            kinds: HARD_FORK_KINDS,
            code: "hardfork_treatment_unsupported",
            detail: "No ESTV text settles whether fork coins are income or a tax-free split of an existing asset.",
        }],
        zero_cost_kinds: &[],
        below_market: BelowMarket::Price,
    }),
};

/// Ireland's Case IV income: staking, lending interest and non-trade mining
/// (IE-R9). Airdrops, hard forks, routing income and unlabelled income are
/// refused until reviewed.
const IE_INCOME: &[&str] = &[
    "interest",
    "lending_interest",
    "mining",
    "mining_reward",
    "staking",
];

const IRELAND: Country = Country {
    code: "ie",
    iso: Some("IE"),
    name_en: "Ireland",
    name_de: "Irland",
    name_local: None,
    preview: true,
    rules: Rules::Ireland,
    fiat: Fiat::Fixed("EUR"),
    // CGT has no holding period (RP2's IE plugin: sys.maxsize).
    long_term: LongTerm::Never,
    // s.580(1), applied to fungible assets by s.580(4).
    methods: &["FIFO"],
    default_method: "FIFO",
    // One pool per person across all wallets and exchanges.
    pool_scopes: &["global"],
    default_pool_scope: "global",
    locale: Locale::En,
    timezone: DUBLIN,
    tax_year_start: (1, 1),
    cross_asset_carry: false,
    // Whether a peg is one asset under s.580 is open (IE-Q9).
    rail_decision_required: false,
    report_generators: &["open_positions", "ie.cgt"],
    income_categories: IE_INCOME,
    sources: &[
        Source {
            title: "Revenue TDM 02-01-03, Taxation of Crypto-Asset Transactions (January 2026)",
            url: "https://www.revenue.ie/en/tax-professionals/tdm/income-tax-capital-gains-tax-corporation-tax/part-02/02-01-03.pdf",
            retrieved: "2026-11-07",
        },
        Source {
            title: "Revenue Notes for Guidance, Taxes Consolidation Act 1997, Part 19 (Finance Act 2025)",
            url: "https://www.revenue.ie/en/tax-professionals/documents/notes-for-guidance/tca/part19.pdf",
            retrieved: "2026-11-07",
        },
        Source {
            title: "Revenue TDM 02-03-01A, Capital Gains Tax: rate of charge (S.28) (October 2026)",
            url: "https://www.revenue.ie/en/tax-professionals/tdm/income-tax-capital-gains-tax-corporation-tax/part-02/02-03-01a.pdf",
            retrieved: "2026-11-07",
        },
        Source {
            title: "Revenue eBrief No. 142/26 (31 % from 7 October 2026)",
            url: "https://www.revenue.ie/en/tax-professionals/ebrief/2026/no-1422026.aspx",
            retrieved: "2026-11-07",
        },
        Source {
            title: "Revenue: How to calculate CGT",
            url: "https://www.revenue.ie/en/gains-gifts-and-inheritance/transfering-an-asset/how-to-calculate-cgt.aspx",
            retrieved: "2026-11-07",
        },
        Source {
            title: "Revenue: When and how do you pay and file CGT?",
            url: "https://www.revenue.ie/en/gains-gifts-and-inheritance/transfering-an-asset/when-and-how-do-you-pay-and-file-cgt.aspx",
            retrieved: "2026-11-07",
        },
        Source {
            title: "Revenue Form CG1 2025 and its helpsheet",
            url: "https://www.revenue.ie/en/gains-gifts-and-inheritance/documents/formcg1.pdf",
            retrieved: "2026-11-07",
        },
        Source {
            title: "Revenue: Cryptocurrencies and crypto-assets",
            url: "https://www.revenue.ie/en/companies-and-charities/financial-services/cryptocurrencies/index.aspx",
            retrieved: "2026-11-07",
        },
    ],
    tax_tables: &[&tables::IE_CGT_RATE, &tables::IE_ANNUAL_EXEMPTION],
    engine: Some(EngineRules {
        // IE-R10: fee units are disposals at their value; a sale's stay in it.
        fee_units: FeeUnits::Disposal,
        // IE-R1, IE-Q2: the received coins' value is the consideration and
        // their cost.
        swap_values: SwapValues::Received,
        // s.547: a gift is a disposal at market value; donations and gifts
        // to a spouse need a reviewed marker first (IE-Q6).
        gifts: Gifts {
            given_at_value: true,
            donated_at_value: false,
            received_refused: false,
            losses_not_deductible: false,
        },
        refused_receipts: &[
            RefusedReceipt {
                direction: Direction::Inbound,
                kinds: &["airdrop"],
                code: "ie_airdrop_requires_review",
                detail: "Revenue publishes no treatment of airdrops (IE-R9); a reviewer must classify the receipt.",
            },
            RefusedReceipt {
                direction: Direction::Inbound,
                kinds: HARD_FORK_KINDS,
                code: "ie_hardfork_requires_review",
                detail: "The cost of coins from a hard fork is not modelled (IE-R9).",
            },
            RefusedReceipt {
                direction: Direction::Inbound,
                kinds: &["routing_income"],
                code: "ie_routing_income_requires_review",
                detail: "Revenue publishes no treatment of Lightning routing income (IE-R9); a reviewer must classify it.",
            },
            RefusedReceipt {
                direction: Direction::Inbound,
                kinds: &["income"],
                code: "ie_income_requires_review",
                detail: "Unlabelled income has no Irish category; classify it before it is booked (IE-R9).",
            },
        ],
        zero_cost_kinds: &[],
        below_market: BelowMarket::Price,
    }),
};

/// The United Kingdom's miscellaneous income: non-trade mining and staking,
/// routing income and lending interest (GB-INC-MISC). Airdrops (no reviewed
/// service classification), hard forks and unlabelled income are refused.
const GB_INCOME: &[&str] = &[
    "interest",
    "lending_interest",
    "mining",
    "mining_reward",
    "routing_income",
    "staking",
];

const UNITED_KINGDOM: Country = Country {
    code: "gb",
    iso: Some("GB"),
    name_en: "United Kingdom",
    name_de: "Vereinigtes Königreich",
    name_local: None,
    preview: true,
    rules: Rules::UnitedKingdom,
    fiat: Fiat::Fixed("GBP"),
    // Capital Gains Tax has no short/long distinction.
    long_term: LongTerm::Never,
    // Fixed by statute (TCGA92 s104, s105, s106A): no choice of method.
    methods: &["UK_SHARE_MATCHING"],
    default_method: "UK_SHARE_MATCHING",
    // CRYPTO22200: "a single pooled asset" per token for the person.
    pool_scopes: &["global"],
    default_pool_scope: "global",
    locale: Locale::En,
    timezone: LONDON,
    tax_year_start: (4, 6),
    cross_asset_carry: false,
    // A peg returns another token: the book decides each rail pair
    // (tax-country-gb.md, Bitcoin rails), or it is held.
    rail_decision_required: true,
    report_generators: &["open_positions", "gb.sa108"],
    income_categories: GB_INCOME,
    sources: &[
        Source {
            title: "Taxation of Chargeable Gains Act 1992, section 104 (share pooling)",
            url: "https://www.legislation.gov.uk/ukpga/1992/12/section/104",
            retrieved: "2026-11-07",
        },
        Source {
            title: "Taxation of Chargeable Gains Act 1992, section 105 (same-day rule)",
            url: "https://www.legislation.gov.uk/ukpga/1992/12/section/105",
            retrieved: "2026-11-07",
        },
        Source {
            title: "Taxation of Chargeable Gains Act 1992, section 106A (30-day rule)",
            url: "https://www.legislation.gov.uk/ukpga/1992/12/section/106A",
            retrieved: "2026-11-07",
        },
        Source {
            title: "HMRC Cryptoassets Manual CRYPTO22100: what is a disposal",
            url: "https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22100",
            retrieved: "2026-11-07",
        },
        Source {
            title: "HMRC Cryptoassets Manual CRYPTO22150: allowable expenses",
            url: "https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22150",
            retrieved: "2026-11-07",
        },
        Source {
            title: "HMRC Cryptoassets Manual CRYPTO22200: pooling (updated 17 November 2025)",
            url: "https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22200",
            retrieved: "2026-11-07",
        },
        Source {
            title: "HMRC Cryptoassets Manual CRYPTO22250: pooling examples",
            url: "https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22250",
            retrieved: "2026-11-07",
        },
        Source {
            title: "HMRC Cryptoassets Manual CRYPTO22280: fees satisfied in tokens",
            url: "https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22280",
            retrieved: "2026-11-07",
        },
        Source {
            title: "HMRC Cryptoassets Manual CRYPTO21150, 21200, 21250: mining, staking and airdrops (Income Tax)",
            url: "https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto21250",
            retrieved: "2026-11-07",
        },
        Source {
            title: "GOV.UK Capital Gains Tax rates and annual tax-free allowances (updated 13 April 2026)",
            url: "https://www.gov.uk/government/publications/rates-and-allowances-capital-gains-tax/capital-gains-tax-rates-and-annual-tax-free-allowances",
            retrieved: "2026-11-07",
        },
        Source {
            title: "HMRC SA108 Capital Gains Tax summary notes 2025-26",
            url: "https://assets.publishing.service.gov.uk/media/6a02e5df4fb0713aa63ea77c/SA108-Notes-2026.pdf",
            retrieved: "2026-11-07",
        },
        Source {
            title: "HMRC HS325 Other taxable income 2026 (SA100 boxes 17 and 18)",
            url: "https://www.gov.uk/government/publications/other-taxable-income-hs325-self-assessment-helpsheet/other-taxable-income-2026-hs325",
            retrieved: "2026-11-07",
        },
    ],
    tax_tables: &[
        &tables::GB_ANNUAL_EXEMPT_AMOUNT,
        &tables::GB_PROCEEDS_THRESHOLD,
        &tables::GB_CGT_MAIN_RATES,
    ],
    engine: Some(EngineRules {
        // GB-FEE-TOKENS: fee tokens are a disposal at market value, which is
        // also an allowable cost of the sale they serve.
        fee_units: FeeUnits::DisposalBeforeSale,
        // GB-SWAP-VALUES: proceeds the received token's value, cost the
        // given token's.
        swap_values: SwapValues::ReceivedGiven,
        // GB-GIFT: an outgoing gift needs its recipient class (spouse,
        // charity, other), which the book does not record, so gifts and
        // donations stay quarantined.
        gifts: Gifts {
            given_at_value: false,
            donated_at_value: false,
            received_refused: false,
            losses_not_deductible: false,
        },
        refused_receipts: &[
            RefusedReceipt {
                direction: Direction::Inbound,
                kinds: &["airdrop"],
                code: "airdrop_classification_required",
                detail: "An airdrop is income only if received for a service (CRYPTO21250); it needs a reviewed classification.",
            },
            RefusedReceipt {
                direction: Direction::Inbound,
                kinds: HARD_FORK_KINDS,
                code: "hardfork_cost_split_unsupported",
                detail: "Splitting the pooled cost between the old and new tokens on a just and reasonable basis (CRYPTO22300) is not modelled.",
            },
            RefusedReceipt {
                direction: Direction::Inbound,
                kinds: &["income"],
                code: "income_kind_unsupported",
                detail: "Unlabelled income has no United Kingdom category; classify it before it is booked.",
            },
        ],
        zero_cost_kinds: &[],
        below_market: BelowMarket::Price,
    }),
};

const SPAIN: Country = Country {
    code: "es",
    iso: Some("ES"),
    name_en: "Spain",
    name_de: "Spanien",
    name_local: Some("España"),
    preview: true,
    rules: Rules::Spain,
    fiat: Fiat::Fixed("EUR"),
    // No holding period: every gain is a ganancia patrimonial of the
    // savings base (ES-R5).
    long_term: LongTerm::Never,
    // First in, first out across every wallet and exchange (art. 37.2
    // LIRPF by analogy, DGT V1149-18; ES-R3).
    methods: &["FIFO"],
    default_method: "FIFO",
    pool_scopes: &["global"],
    default_pool_scope: "global",
    locale: Locale::Es,
    timezone: MADRID,
    tax_year_start: (1, 1),
    cross_asset_carry: false,
    // A peg carries basis as in generic books until reviewed (ES-Q6).
    rail_decision_required: false,
    report_generators: &["open_positions", "es.irpf_monedas_virtuales"],
    income_categories: &["airdrop", "interest", "lending_interest", "mining", "staking"],
    sources: &[
        Source {
            title: "Ley 35/2006 del IRPF, texto consolidado (BOE-A-2006-20764)",
            url: "https://www.boe.es/buscar/act.php?id=BOE-A-2006-20764",
            retrieved: "2026-11-07",
        },
        Source {
            title: "AEAT, Manual práctico de Renta 2025: monedas virtuales",
            url: "https://sede.agenciatributaria.gob.es/Sede/ayuda/manuales-videos-folletos/manuales-practicos/irpf-2025/c11-ganancias-perdidas-patrimoniales/monedas-virtuales/compra-venta-monedas-virtuales-tributacion-inversor.html",
            retrieved: "2026-11-07",
        },
        Source {
            title: "AEAT, Informador de Renta 2025, 9.2.2 Venta y otras operaciones con monedas virtuales",
            url: "https://www2.agenciatributaria.gob.es/wlpl/AVAC-CALC/InformadorRenta2025?id=9.2.2",
            retrieved: "2026-11-07",
        },
        Source {
            title: "AEAT, Renta WEB: ganancias y pérdidas por transmisión o permuta de monedas virtuales",
            url: "https://sede.agenciatributaria.gob.es/Sede/Ayuda/25Presentacion/100/7_6_6_2/ganancias_perdidas_monedas_virtuales.html",
            retrieved: "2026-11-07",
        },
    ],
    tax_tables: &[],
    engine: Some(EngineRules {
        // A move's fee units are a permuta of their own; a sale's stay in
        // its one row (ES-R12).
        fee_units: FeeUnits::Disposal,
        // A permuta is worth the higher of the two values (art. 37.1.h).
        swap_values: SwapValues::Higher,
        gifts: Gifts {
            // Gifts are gratuitous transmissions at market value (art. 36);
            // their losses are not computable (art. 33.5.c). Donations may be
            // exempt (art. 33.4.a) and wait for review.
            given_at_value: true,
            donated_at_value: false,
            received_refused: false,
            losses_not_deductible: true,
        },
        refused_receipts: &[
            RefusedReceipt {
                direction: Direction::Inbound,
                kinds: HARD_FORK_KINDS,
                code: "es_hardfork_requires_review",
                detail: "The AEAT publishes no treatment of coins from a hard fork (ES-R7); a reviewer must settle their value and base.",
            },
            RefusedReceipt {
                direction: Direction::Inbound,
                kinds: &["routing_income"],
                code: "es_routing_income_requires_review",
                detail: "Lightning routing income has no settled Spanish treatment (ES-R7); a reviewer must classify it.",
            },
            RefusedReceipt {
                direction: Direction::Inbound,
                kinds: &["income"],
                code: "es_income_requires_review",
                detail: "Unlabelled income has no Spanish casilla; classify it before it is booked (ES-R7).",
            },
            RefusedReceipt {
                direction: Direction::Outbound,
                kinds: LOST_KINDS,
                code: "es_lost_requires_review",
                detail: "Lost or stolen coins are not a computable loss without a reviewed claim (arts. 33.5.a, 14.2.k; ES-R10).",
            },
            RefusedReceipt {
                direction: Direction::Outbound,
                kinds: &["donat"],
                code: "es_donation_requires_review",
                detail: "A donation to a Ley 49/2002 entity may be exempt (art. 33.4.a); it needs reviewed metadata naming the entity (ES-R8).",
            },
        ],
        zero_cost_kinds: &[],
        // A price below normal market value is replaced by it (art. 35.2).
        below_market: BelowMarket::MarketValue,
    }),
};

/// Lost, stolen and stolen-by-theft outflows: neither a loss nor a
/// disposal until reviewed (Spain's ES-R10, Japan's JP-Q8).
const LOST_KINDS: &[&str] = &["lost", "stolen", "theft"];

const JAPAN: Country = Country {
    code: "jp",
    iso: Some("JP"),
    name_en: "Japan",
    name_de: "Japan",
    name_local: Some("日本"),
    preview: true,
    rules: Rules::Japan,
    fiat: Fiat::Fixed("JPY"),
    // No holding period: crypto gains are miscellaneous income (雑所得).
    long_term: LongTerm::Never,
    // 総平均法 is the statutory method without a notification (所令119の5);
    // 移動平均法 needs one (所令119の3).
    methods: &["JP_MOVING_AVERAGE", "JP_TOTAL_AVERAGE"],
    default_method: "JP_TOTAL_AVERAGE",
    pool_scopes: &["global"],
    default_pool_scope: "global",
    locale: Locale::Ja,
    timezone: TOKYO,
    tax_year_start: (1, 1),
    cross_asset_carry: false,
    // A peg is no disposal, as in generic books, until reviewed (JP-Q7).
    rail_decision_required: false,
    report_generators: &["open_positions", "jp.crypto_keisansho"],
    income_categories: &["interest", "lending_interest", "mining", "staking"],
    sources: &[
        Source {
            title: "国税庁「暗号資産等に関する税務上の取扱いについて（FAQ）」令和7年12月",
            url: "https://www.nta.go.jp/publication/pamph/pdf/virtual_currency_faq_03.pdf",
            retrieved: "2026-11-07",
        },
        Source {
            title: "国税庁「暗号資産等に関する税務上の取扱い及び計算書について」",
            url: "https://www.nta.go.jp/publication/pamph/shotoku/kakuteishinkokukankei/kasoutuka/",
            retrieved: "2026-11-07",
        },
        Source {
            title: "国税庁 暗号資産の計算書（総平均法用）",
            url: "https://www.nta.go.jp/taxes/shiraberu/shinkoku/yoshiki/02/xlsx/002.xlsx",
            retrieved: "2026-11-07",
        },
        Source {
            title: "国税庁 暗号資産の計算書（移動平均法用）",
            url: "https://www.nta.go.jp/taxes/shiraberu/shinkoku/yoshiki/02/xlsx/001.xlsx",
            retrieved: "2026-11-07",
        },
        Source {
            title: "所得税法施行令 第119条の2〜第119条の6",
            url: "https://laws.e-gov.go.jp/law/340CO0000000096",
            retrieved: "2026-11-07",
        },
        Source {
            title: "国税庁 確定申告の手引き 令和7年分 雑所得",
            url: "https://www.nta.go.jp/taxes/shiraberu/shinkoku/tebiki/2025/03/order2/3-2_07.htm",
            retrieved: "2026-11-07",
        },
        Source {
            title: "国税庁 タックスアンサー No.1900",
            url: "https://www.nta.go.jp/taxes/shiraberu/taxanswer/shotoku/1900.htm",
            retrieved: "2026-11-07",
        },
        Source {
            title: "財務省「令和８年度税制改正の大綱」",
            url: "https://www.mof.go.jp/tax_policy/tax_reform/outline/fy2026/08taikou_01.htm",
            retrieved: "2026-11-07",
        },
    ],
    // From 2027 the separate taxation's start is unconfirmed (JP-R16).
    tax_tables: &[&tables::JP_TAX_REGIME],
    engine: Some(EngineRules {
        // A move's fee units are a disposal at their value, revenue and an
        // equal expense (JP-R8); a sale's fee units stay part of the sale.
        fee_units: FeeUnits::Disposal,
        // Revenue and the new coin's cost are the received coin's value
        // (FAQ 1-3, 所令119の6).
        swap_values: SwapValues::Received,
        gifts: Gifts {
            // A gift or donation puts the market value into revenue
            // (所得税法40条; FAQ 1-4, 2-10).
            given_at_value: true,
            donated_at_value: true,
            received_refused: false,
            losses_not_deductible: false,
        },
        refused_receipts: &[
            RefusedReceipt {
                direction: Direction::Inbound,
                kinds: &["airdrop"],
                code: "jp_airdrop_requires_review",
                detail: "The NTA FAQ gives no answer for airdrops (JP-R9, JP-Q2); a reviewer must classify the receipt.",
            },
            RefusedReceipt {
                direction: Direction::Inbound,
                kinds: &["routing_income"],
                code: "jp_routing_income_requires_review",
                detail: "Lightning routing income has no settled Japanese treatment (JP-R9); a reviewer must classify it.",
            },
            RefusedReceipt {
                direction: Direction::Inbound,
                kinds: &["income"],
                code: "jp_income_requires_review",
                detail: "Unlabelled income has no 雑所得 category; classify it before it is booked (JP-R9).",
            },
            RefusedReceipt {
                direction: Direction::Outbound,
                kinds: LOST_KINDS,
                code: "jp_lost_requires_review",
                detail: "Lost or stolen coins have no published FAQ answer; a casualty loss (雑損控除) may apply only to some cases (JP-Q8).",
            },
        ],
        // A fork's coins cost nothing and are no income (FAQ 1-6).
        zero_cost_kinds: HARD_FORK_KINDS,
        // Below 70 % of market value, 70 % of it is revenue (FAQ 2-10).
        below_market: BelowMarket::SeventyPercent,
    }),
};

/// Every country, in the order pickers list them.
pub const COUNTRIES: &[Country] = &[
    GENERIC,
    AUSTRIA,
    UNITED_STATES,
    GERMANY,
    SWITZERLAND,
    IRELAND,
    UNITED_KINGDOM,
    SPAIN,
    JAPAN,
];

/// The country stored as `code` (already normalized by the caller).
pub fn lookup(code: &str) -> Option<&'static Country> {
    COUNTRIES.iter().find(|country| country.code == code)
}

/// Whether the country stored as `code` carries basis across reviewed
/// carrying-value pairs of unlike assets (an unknown code does not).
pub fn cross_asset_carry(code: &str) -> bool {
    lookup(code).is_some_and(|country| country.cross_asset_carry)
}

/// Every country code, in picker order.
pub fn codes() -> impl Iterator<Item = &'static str> {
    COUNTRIES.iter().map(|country| country.code)
}

/// The codes as a sentence list, sorted: `at, generic`.
pub fn sorted_codes() -> String {
    let mut codes: Vec<_> = codes().collect();
    codes.sort_unstable();
    codes.join(", ")
}

impl Country {
    /// The tax year a local date belongs to in this country.
    pub fn tax_year(&self, local: calendar::Date) -> i32 {
        years::tax_year(local, self.tax_year_start)
    }

    /// The table `id` the country applies.
    pub fn tax_table(&self, id: &str) -> Option<&'static years::Schedule> {
        self.tax_tables.iter().copied().find(|table| table.id == id)
    }

    /// The engine's `rules` country of an entry with shared engine rules:
    /// its zone, its holding period, its only pool scope, its tax year's
    /// start and each feature it switches on. `None` for a country computed
    /// with its family's engine country; `Err` for a holding period counted
    /// in the profile's days, which has no calendar form.
    pub fn engine_country(&self) -> Option<Result<Value, &'static str>> {
        let rules = self.engine?;
        let holding = match self.long_term {
            LongTerm::Never => "never",
            LongTerm::MoreThanOneYear => "more_than_one_year",
            LongTerm::ProfileDays { .. } => return Some(Err("profile_days")),
        };
        let mut country = json!({"kind": "rules", "timezone": self.timezone, "holding": holding});
        if let [scope] = self.pool_scopes {
            country["pool_scope"] = json!(scope);
        }
        if self.tax_year_start != (1, 1) {
            let (month, day) = self.tax_year_start;
            country["tax_year_start"] = json!(format!("{month:02}-{day:02}"));
        }
        country
            .as_object_mut()
            .expect("an object")
            .extend(rules.engine_fields());
        Some(Ok(country))
    }

    /// The country's name in `language`.
    pub fn name(&self, language: Language) -> &'static str {
        match language {
            Language::En => self.name_en,
            Language::De => self.name_de,
            // Another language names a country in its own language only
            // where the entry has that name, else in English.
            Language::Es | Language::Ja if self.locale.language() == language => {
                self.name_local.unwrap_or(self.name_en)
            }
            Language::Es | Language::Ja => self.name_en,
        }
    }

    /// The preview notice in `language`, or `None` once reviewed.
    pub fn preview_notice(&self, language: Language) -> Option<&'static str> {
        self.preview.then_some(match language {
            Language::En => PREVIEW_NOTICE_EN,
            Language::De => PREVIEW_NOTICE_DE,
            Language::Es => PREVIEW_NOTICE_ES,
            Language::Ja => PREVIEW_NOTICE_JA,
        })
    }

    /// Whether the profile may use `method` (stored, upper case).
    pub fn allows_method(&self, method: &str) -> bool {
        self.methods.contains(&method)
    }

    /// Whether the profile may use pool `scope` (lower case).
    pub fn allows_pool_scope(&self, scope: &str) -> bool {
        self.pool_scopes.contains(&scope)
    }

    /// The machine description `commands describe`, the daemon and the
    /// desktop read.
    pub fn describe(&self) -> Value {
        let mut name = json!({"en": self.name_en, "de": self.name_de});
        if let Some(local) = self.name_local {
            name[self.locale.language().code()] = json!(local);
        }
        let mut description = json!({
            "code": self.code,
            "iso": self.iso,
            "name": name,
            "preview": self.preview,
            "preview_notice": self.preview.then(|| json!({"en": PREVIEW_NOTICE_EN, "de": PREVIEW_NOTICE_DE})),
            "rules": match self.rules {
                Rules::Generic => "generic",
                Rules::Austria => "austria",
                Rules::UnitedStates => "united_states",
                Rules::Germany => "germany",
                Rules::Switzerland => "switzerland",
                Rules::Ireland => "ireland",
                Rules::UnitedKingdom => "united_kingdom",
                Rules::Spain => "spain",
                Rules::Japan => "japan",
            },
            "fiat_currency": match self.fiat {
                Fiat::Profile => Value::Null,
                Fiat::Fixed(code) => json!(code),
            },
            "long_term": match self.long_term {
                LongTerm::ProfileDays { default } => json!({"kind": "profile_days", "default_days": default}),
                LongTerm::Never => json!({"kind": "never"}),
                LongTerm::MoreThanOneYear => json!({"kind": "more_than_one_year"}),
            },
            "methods": self.methods,
            "default_method": self.default_method,
            "pool_scopes": self.pool_scopes,
            "default_pool_scope": self.default_pool_scope,
            "locale": self.locale.code(),
            "timezone": self.timezone,
            "tax_year_start": format!("{:02}-{:02}", self.tax_year_start.0, self.tax_year_start.1),
            "cross_asset_carry": self.cross_asset_carry,
            "report_generators": self.report_generators,
            "income_categories": self.income_categories,
            "sources": self.sources.iter().map(|source| json!({
                "title": source.title,
                "url": source.url,
                "retrieved": source.retrieved,
            })).collect::<Vec<_>>(),
        });
        // Only a country with shared engine rules names them, so the other
        // descriptions keep their keys.
        if let Some(rules) = &self.engine {
            description["engine"] = Value::Object(rules.engine_fields());
        }
        if self.rail_decision_required {
            description["rail_decision_required"] = json!(true);
        }
        description
    }
}

/// Every country's description, in picker order.
pub fn describe() -> Value {
    Value::Array(COUNTRIES.iter().map(Country::describe).collect())
}

/// Where the desktop's copy of the registry lives, from the repository root.
pub const TYPESCRIPT_PATH: &str = "ui-tauri/src/lib/taxCountries.generated.ts";

/// The desktop's copy of the registry: [`describe`] as a TypeScript constant.
/// A test keeps the checked-in file equal to it.
pub fn typescript_module() -> String {
    let json = serde_json::to_string_pretty(&describe()).expect("the registry serializes");
    format!(
        "// @generated by crates/tax-countries from its registry. Do not edit by hand:\n\
         // run `KASSIBER_UPDATE_TAX_COUNTRIES_TS=1 cargo test -p kassiber-tax-countries`.\n\
         export const TAX_COUNTRY_REGISTRY = {json} as const;\n\n\
         export const TAX_COUNTRY_PREVIEW_NOTICE = {notice} as const;\n",
        notice = serde_json::to_string_pretty(
            &json!({"en": PREVIEW_NOTICE_EN, "de": PREVIEW_NOTICE_DE})
        )
        .expect("the notice serializes"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_copy_matches_the_registry() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(TYPESCRIPT_PATH);
        let expected = typescript_module();
        if std::env::var_os("KASSIBER_UPDATE_TAX_COUNTRIES_TS").is_some() {
            std::fs::write(&path, &expected).unwrap();
        }
        let actual = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            actual == expected,
            "{} is stale; rerun with KASSIBER_UPDATE_TAX_COUNTRIES_TS=1",
            TYPESCRIPT_PATH
        );
    }

    #[test]
    fn codes_are_unique_lowercase_and_generic_is_first() {
        let mut seen = std::collections::BTreeSet::new();
        for country in COUNTRIES {
            assert!(seen.insert(country.code), "{} twice", country.code);
            assert_eq!(country.code, country.code.to_lowercase());
            assert_eq!(lookup(country.code), Some(country));
        }
        assert_eq!(COUNTRIES[0].code, DEFAULT_CODE);
        assert_eq!(lookup(""), None);
        assert_eq!(lookup("AT"), None);
        assert_eq!(lookup("xx"), None);
    }

    #[test]
    fn every_policy_is_self_consistent() {
        for country in COUNTRIES {
            assert!(
                country.allows_method(country.default_method),
                "{}",
                country.code
            );
            assert!(
                country.allows_pool_scope(country.default_pool_scope),
                "{}",
                country.code
            );
            let mut methods = country.methods.to_vec();
            methods.sort_unstable();
            assert_eq!(methods, country.methods, "{} methods sorted", country.code);
            let mut scopes = country.pool_scopes.to_vec();
            scopes.sort_unstable();
            assert_eq!(
                scopes, country.pool_scopes,
                "{} scopes sorted",
                country.code
            );
            // A reviewed country names its sources; a preview country must
            // name them so its reviewer can check them.
            if country.rules != Rules::Generic {
                assert!(
                    !country.sources.is_empty(),
                    "{} has no sources",
                    country.code
                );
            }
            for source in country.sources {
                assert!(source.url.starts_with("https://"), "{}", source.url);
                assert_eq!(source.retrieved.len(), 10, "{}", source.retrieved);
            }
            assert_eq!(
                country.preview_notice(Language::En).is_some(),
                country.preview
            );
            assert!(
                SUPPORTED_ZONES.contains(&country.timezone),
                "{} zone",
                country.code
            );
            assert!(
                calendar::zone(country.timezone).is_some(),
                "{} has no calendar",
                country.code
            );
        }
    }

    #[test]
    fn the_united_states_is_a_preview_usd_country_with_wallet_pools() {
        let us = lookup("us").unwrap();
        assert!(us.preview);
        assert_eq!(us.fiat, Fiat::Fixed("USD"));
        assert_eq!(us.long_term, LongTerm::MoreThanOneYear);
        assert_eq!(us.long_term.stored_days(Some(7)), 365);
        assert_eq!(us.methods, ["FIFO", "HIFO", "LIFO", "LOFO"]);
        assert!(!us.allows_method("MOVING_AVERAGE"));
        assert_eq!(
            (us.default_pool_scope, us.pool_scopes),
            ("wallet", &["global", "wallet"][..])
        );
        assert_eq!(us.preview_notice(Language::En), Some(PREVIEW_NOTICE_EN));
        assert!(!cross_asset_carry("us"));
    }

    #[test]
    fn a_country_finds_its_tables_and_tax_years() {
        let ie = Country {
            tax_tables: &[&tables::IE_CGT_RATE, &tables::IE_ANNUAL_EXEMPTION],
            ..*lookup("us").unwrap()
        };
        assert_eq!(ie.tax_table("ie.cgt_rate"), Some(&tables::IE_CGT_RATE));
        assert_eq!(ie.tax_table("de.p23_freigrenze"), None);
        let gb = Country {
            tax_year_start: (4, 6),
            ..*lookup("us").unwrap()
        };
        let date = calendar::Date::parse("2025-04-05").unwrap();
        assert_eq!((gb.tax_year(date), ie.tax_year(date)), (2024, 2025));
        // A country's tables are its own.
        for country in COUNTRIES {
            for table in country.tax_tables {
                assert!(
                    table.id.starts_with(&format!("{}.", country.code)),
                    "{} lists {}",
                    country.code,
                    table.id
                );
                assert!(tables::TABLES.contains(table), "{}", table.id);
            }
        }
    }

    #[test]
    fn engine_rules_name_only_what_they_change() {
        assert!(EngineRules::default().engine_fields().is_empty());
        let rules = EngineRules {
            fee_units: FeeUnits::Disposal,
            ..EngineRules::default()
        };
        assert_eq!(
            Value::Object(rules.engine_fields()),
            json!({"fee_units": "disposal"})
        );
        let swaps = EngineRules {
            swap_values: SwapValues::Higher,
            ..EngineRules::default()
        };
        assert_eq!(
            Value::Object(swaps.engine_fields()),
            json!({"swap_values": "higher"})
        );
        assert_eq!(SwapValues::ReceivedGiven.code(), "received_given");
        let gifts = EngineRules {
            gifts: Gifts {
                given_at_value: true,
                donated_at_value: true,
                received_refused: true,
                losses_not_deductible: true,
            },
            ..EngineRules::default()
        };
        assert_eq!(
            Value::Object(gifts.engine_fields()),
            json!({"gifts_given": "market_value", "donations_given": "market_value",
                   "gifts_received": "quarantine", "gift_losses": "not_deductible"})
        );
        assert_eq!(FeeUnits::DisposalBeforeSale.code(), "disposal_before_sale");
        let ruled = Country {
            engine: Some(rules),
            ..*lookup("us").unwrap()
        };
        assert_eq!(ruled.describe()["engine"], json!({"fee_units": "disposal"}));
        // Only countries with shared engine rules describe them, so the
        // reviewed countries' descriptions keep their keys.
        for country in COUNTRIES {
            assert_eq!(
                country.describe().get("engine").is_some(),
                country.engine.is_some(),
                "{}",
                country.code
            );
            assert_eq!(
                country.describe().get("rail_decision_required").is_some(),
                country.rail_decision_required,
                "{}",
                country.code
            );
        }
        let refused = EngineRules {
            refused_receipts: &[RefusedReceipt {
                direction: Direction::Inbound,
                kinds: &["airdrop"],
                code: "x_required",
                detail: "why",
            }],
            ..EngineRules::default()
        };
        assert_eq!(
            Value::Object(refused.engine_fields()),
            json!({"refused_receipts": [{"kinds": ["airdrop"], "code": "x_required", "detail": "why"}]})
        );
        // An outbound refusal names its direction; inbound is the engine's
        // default and stays out.
        let review = EngineRules {
            refused_receipts: &[RefusedReceipt {
                direction: Direction::Outbound,
                kinds: &["lost"],
                code: "zz_lost_requires_review",
                detail: "why",
            }],
            zero_cost_kinds: &["hardfork"],
            below_market: BelowMarket::SeventyPercent,
            ..EngineRules::default()
        };
        assert_eq!(
            Value::Object(review.engine_fields()),
            json!({"refused_receipts": [{"direction": "outbound", "kinds": ["lost"],
                                         "code": "zz_lost_requires_review", "detail": "why"}],
                   "zero_cost_kinds": ["hardfork"], "below_market": "seventy_percent"})
        );
        assert_eq!(BelowMarket::MarketValue.code(), "market_value");
        assert_eq!(BelowMarket::Price.code(), "price");
    }

    #[test]
    fn japan_is_a_preview_jpy_country_with_its_averages() {
        let jp = lookup("jp").unwrap();
        assert!(jp.preview);
        assert_eq!((jp.rules, jp.fiat), (Rules::Japan, Fiat::Fixed("JPY")));
        assert_eq!(jp.long_term, LongTerm::Never);
        assert_eq!(jp.default_method, "JP_TOTAL_AVERAGE");
        assert!(jp.allows_method("JP_MOVING_AVERAGE") && !jp.allows_method("FIFO"));
        assert_eq!(jp.pool_scopes, ["global"]);
        assert_eq!((jp.locale, jp.timezone), (Locale::Ja, TOKYO));
        assert!(!jp.rail_decision_required);
        assert_eq!(jp.name(Language::Ja), "日本");
        assert_eq!(jp.name(Language::Es), "Japan");
        assert_eq!(lookup("es").unwrap().name(Language::Ja), "Spain");
        assert_eq!(jp.preview_notice(Language::Ja), Some(PREVIEW_NOTICE_JA));
        assert_eq!(jp.tax_table("jp.tax_regime"), Some(&tables::JP_TAX_REGIME));
        let engine = jp.engine.unwrap();
        assert_eq!(engine.swap_values, SwapValues::Received);
        assert_eq!(engine.below_market, BelowMarket::SeventyPercent);
        assert_eq!(engine.zero_cost_kinds, ["hard_fork", "hardfork"]);
        assert!(engine.gifts.given_at_value && engine.gifts.donated_at_value);
        let fields = Value::Object(engine.engine_fields());
        assert_eq!(
            fields["refused_receipts"][0]["code"],
            "jp_airdrop_requires_review"
        );
        assert_eq!(fields["refused_receipts"][3]["direction"], "outbound");
        assert_eq!(fields["zero_cost_kinds"], json!(["hard_fork", "hardfork"]));
        assert_eq!(fields["below_market"], "seventy_percent");
    }

    #[test]
    fn spain_is_a_preview_eur_country_with_global_fifo() {
        let es = lookup("es").unwrap();
        assert!(es.preview);
        assert_eq!((es.rules, es.fiat), (Rules::Spain, Fiat::Fixed("EUR")));
        assert_eq!(es.long_term, LongTerm::Never);
        assert_eq!((es.methods, es.default_method), (&["FIFO"][..], "FIFO"));
        assert_eq!(es.pool_scopes, ["global"]);
        assert_eq!((es.locale, es.timezone), (Locale::Es, MADRID));
        assert!(!es.rail_decision_required);
        assert_eq!(es.name(Language::Es), "España");
        assert_eq!(es.name(Language::De), "Spanien");
        assert_eq!(lookup("us").unwrap().name(Language::Es), "United States");
        assert_eq!(es.preview_notice(Language::Es), Some(PREVIEW_NOTICE_ES));
        assert_eq!(
            es.describe()["name"],
            json!({"en": "Spain", "de": "Spanien", "es": "España"})
        );
        let engine = es.engine.unwrap();
        assert_eq!(engine.fee_units, FeeUnits::Disposal);
        assert_eq!(engine.swap_values, SwapValues::Higher);
        assert_eq!(engine.below_market, BelowMarket::MarketValue);
        assert!(engine.gifts.given_at_value && engine.gifts.losses_not_deductible);
        assert!(!engine.gifts.donated_at_value);
        let fields = Value::Object(engine.engine_fields());
        assert_eq!(
            fields["refused_receipts"][0]["code"],
            "es_hardfork_requires_review"
        );
        assert_eq!(fields["refused_receipts"][3]["direction"], "outbound");
        assert_eq!(
            fields["refused_receipts"][3]["code"],
            "es_lost_requires_review"
        );
        assert!(fields["refused_receipts"][0].get("direction").is_none());
    }

    #[test]
    fn ireland_and_the_united_kingdom_follow_their_specifications() {
        let ie = lookup("ie").unwrap();
        assert!(ie.preview);
        assert_eq!(ie.fiat, Fiat::Fixed("EUR"));
        assert_eq!((ie.methods, ie.default_method), (&["FIFO"][..], "FIFO"));
        assert_eq!(ie.pool_scopes, ["global"]);
        assert_eq!((ie.timezone, ie.tax_year_start), (DUBLIN, (1, 1)));
        assert_eq!(ie.long_term, LongTerm::Never);
        let fields = Value::Object(ie.engine.unwrap().engine_fields());
        assert_eq!(
            (
                &fields["fee_units"],
                &fields["swap_values"],
                &fields["gifts_given"]
            ),
            (
                &json!("disposal"),
                &json!("received"),
                &json!("market_value")
            )
        );
        let codes: Vec<&str> = ie
            .engine
            .unwrap()
            .refused_receipts
            .iter()
            .map(|r| r.code)
            .collect();
        assert_eq!(
            codes,
            [
                "ie_airdrop_requires_review",
                "ie_hardfork_requires_review",
                "ie_routing_income_requires_review",
                "ie_income_requires_review"
            ]
        );
        assert_eq!(ie.tax_table("ie.cgt_rate"), Some(&tables::IE_CGT_RATE));
        let gb = lookup("gb").unwrap();
        assert!(gb.preview);
        assert_eq!(gb.fiat, Fiat::Fixed("GBP"));
        assert_eq!(gb.methods, ["UK_SHARE_MATCHING"]);
        assert_eq!(gb.pool_scopes, ["global"]);
        assert_eq!((gb.timezone, gb.tax_year_start), (LONDON, (4, 6)));
        let fields = Value::Object(gb.engine.unwrap().engine_fields());
        assert_eq!(
            (
                &fields["fee_units"],
                &fields["swap_values"],
                fields.get("gifts_given")
            ),
            (
                &json!("disposal_before_sale"),
                &json!("received_given"),
                None
            )
        );
        let codes: Vec<&str> = gb
            .engine
            .unwrap()
            .refused_receipts
            .iter()
            .map(|r| r.code)
            .collect();
        assert_eq!(
            codes,
            [
                "airdrop_classification_required",
                "hardfork_cost_split_unsupported",
                "income_kind_unsupported"
            ]
        );
        assert_eq!(lookup("uk"), None);
        // The UK fails closed on a BTC/LBTC rail pair; Ireland's is open.
        assert!(gb.rail_decision_required && !ie.rail_decision_required);
        // A kind is income or refused, never both.
        for country in [ie, gb] {
            for refused in country.engine.unwrap().refused_receipts {
                for kind in refused.kinds {
                    assert!(!country.income_categories.contains(kind), "{kind}");
                }
            }
            assert!(!country.cross_asset_carry);
        }
    }

    #[test]
    fn generic_and_austria_keep_pythons_policies() {
        // kassiber/tax_policy.py: DEFAULT_ACCOUNTING_METHODS and Austria's
        // union with them, the defaults, EUR, sys.maxsize and global pools.
        let generic = lookup("generic").unwrap();
        assert_eq!(
            generic.methods,
            ["FIFO", "HIFO", "LIFO", "LOFO", "MOVING_AVERAGE"]
        );
        assert_eq!(generic.default_method, "FIFO");
        assert_eq!(generic.fiat, Fiat::Profile);
        assert_eq!(generic.long_term.stored_days(None), 365);
        assert_eq!(generic.long_term.stored_days(Some(0)), 0);
        assert!(!generic.preview && !generic.cross_asset_carry);
        let at = lookup("at").unwrap();
        assert_eq!(at.default_method, "MOVING_AVERAGE_AT");
        assert!(
            at.allows_method("MOVING_AVERAGE_AT") && !generic.allows_method("MOVING_AVERAGE_AT")
        );
        assert_eq!(at.fiat, Fiat::Fixed("EUR"));
        assert_eq!(at.long_term.stored_days(Some(10)), i64::MAX);
        assert_eq!((at.locale, at.timezone), (Locale::DeAt, VIENNA));
        assert_eq!(
            (generic.tax_year_start, at.tax_year_start),
            ((1, 1), (1, 1))
        );
        assert!(!at.preview && at.cross_asset_carry);
        for country in [generic, at] {
            assert_eq!(country.pool_scopes, ["global"]);
        }
        assert_eq!(sorted_codes(), "at, ch, de, es, gb, generic, ie, jp, us");
        for country in [generic, at] {
            assert!(!country.rail_decision_required && country.engine.is_none());
        }
    }

    #[test]
    fn switzerland_is_a_preview_chf_country_with_informational_gains() {
        let ch = lookup("ch").unwrap();
        assert!(ch.preview);
        assert_eq!(
            (ch.rules, ch.fiat),
            (Rules::Switzerland, Fiat::Fixed("CHF"))
        );
        assert_eq!(ch.long_term, LongTerm::Never);
        assert_eq!(ch.methods, ["FIFO", "MOVING_AVERAGE"]);
        assert_eq!(
            (ch.pool_scopes, ch.default_pool_scope),
            (&["global"][..], "global")
        );
        assert_eq!(
            (ch.locale, ch.timezone, ch.locale.code()),
            (Locale::DeCh, ZURICH, "de-CH")
        );
        assert!(!ch.rail_decision_required && !ch.cross_asset_carry);
        assert!(ch.tax_tables.is_empty());
        assert_eq!(ch.report_generators, ["open_positions", "ch.wealth_income"]);
        let country = ch.engine_country().unwrap().unwrap();
        assert_eq!(
            (
                &country["timezone"],
                &country["holding"],
                &country["pool_scope"]
            ),
            (&json!("Europe/Zurich"), &json!("never"), &json!("global"))
        );
        // Only the fork refusal: fees, swaps and gifts keep the generic rules.
        assert!(country.get("fee_units").is_none() && country.get("gifts_received").is_none());
        assert_eq!(
            country["refused_receipts"],
            json!([{"kinds": ["hard_fork", "hardfork"], "code": "hardfork_treatment_unsupported",
                    "detail": ch.engine.unwrap().refused_receipts[0].detail}])
        );
        assert_eq!(ch.describe()["rules"], "switzerland");
        assert_eq!(ch.name(Language::De), "Schweiz");
    }

    #[test]
    fn germany_is_a_preview_eur_country_with_wallet_fifo_and_its_tables() {
        let de = lookup("de").unwrap();
        assert!(de.preview);
        assert_eq!((de.rules, de.fiat), (Rules::Germany, Fiat::Fixed("EUR")));
        assert_eq!(de.long_term, LongTerm::MoreThanOneYear);
        assert_eq!(de.methods, ["FIFO", "MOVING_AVERAGE_FIFO_PERIOD"]);
        assert!(!de.allows_method("LIFO") && !de.allows_method("HIFO"));
        assert_eq!(
            (de.pool_scopes, de.default_pool_scope),
            (&["wallet"][..], "wallet")
        );
        assert_eq!((de.locale, de.timezone), (Locale::DeDe, BERLIN));
        assert_eq!(de.locale.code(), "de-DE");
        assert!(de.rail_decision_required && !de.cross_asset_carry);
        assert_eq!(
            de.tax_table("de.p23_freigrenze"),
            Some(&tables::DE_P23_FREIGRENZE)
        );
        assert_eq!(
            de.tax_table("de.p22_3_freigrenze"),
            Some(&tables::DE_P22_3_FREIGRENZE)
        );
        let engine = de.engine.unwrap();
        assert_eq!(engine.fee_units, FeeUnits::DisposalBeforeSale);
        assert_eq!(engine.swap_values, SwapValues::ReceivedGiven);
        assert!(engine.gifts.received_refused && !engine.gifts.given_at_value);
        let codes: Vec<(&str, &[&str])> = engine
            .refused_receipts
            .iter()
            .map(|r| (r.code, r.kinds))
            .collect();
        assert_eq!(
            codes,
            [
                ("airdrop_classification_required", &["airdrop"][..]),
                (
                    "hardfork_cost_split_unsupported",
                    &["hard_fork", "hardfork"][..]
                ),
            ]
        );
        assert_eq!(de.describe()["rules"], "germany");
        // The engine computes it as a rules country in Berlin, one pool.
        let country = de.engine_country().unwrap().unwrap();
        assert_eq!(
            (
                &country["kind"],
                &country["timezone"],
                &country["holding"],
                &country["pool_scope"]
            ),
            (
                &json!("rules"),
                &json!("Europe/Berlin"),
                &json!("more_than_one_year"),
                &json!("wallet")
            )
        );
        assert!(country.get("tax_year_start").is_none());
        assert_eq!(country["fee_units"], "disposal_before_sale");
        assert_eq!(
            country["refused_receipts"][1]["code"],
            "hardfork_cost_split_unsupported"
        );
        assert!(lookup("us").unwrap().engine_country().is_none());
        let days = Country {
            long_term: LongTerm::ProfileDays { default: 365 },
            ..*de
        };
        assert_eq!(days.engine_country(), Some(Err("profile_days")));
        let uk_like = Country {
            tax_year_start: (4, 6),
            pool_scopes: &["global", "wallet"],
            ..*de
        };
        let country = uk_like.engine_country().unwrap().unwrap();
        assert_eq!(country["tax_year_start"], "04-06");
        assert!(country.get("pool_scope").is_none());
        assert_eq!(de.preview_notice(Language::De), Some(PREVIEW_NOTICE_DE));
    }
}
