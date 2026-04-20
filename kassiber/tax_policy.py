from dataclasses import dataclass

from .errors import AppError


DEFAULT_TAX_COUNTRY = "generic"
AUSTRIAN_TAX_COUNTRY = "at"
DEFAULT_LONG_TERM_DAYS = 365
DEFAULT_REPORT_GENERATORS = ("open_positions", "rp2_full_report")
DEFAULT_ACCOUNTING_METHODS = ("fifo", "lifo", "hifo", "lofo")
AUSTRIAN_ACCOUNTING_METHODS = ("fifo",)
ACTIVE_TAX_COUNTRIES = (DEFAULT_TAX_COUNTRY,)


@dataclass(frozen=True)
class TaxPolicy:
    tax_country: str
    fiat_currency: str
    long_term_days: int
    accounting_methods: tuple[str, ...]
    report_generators: tuple[str, ...]
    default_accounting_method: str = "fifo"
    generation_language: str = "en"


def normalize_tax_country(value):
    country = str(value or DEFAULT_TAX_COUNTRY).strip().lower()
    if not country:
        return DEFAULT_TAX_COUNTRY
    if country not in POLICY_BUILDERS:
        raise ValueError(f"Unsupported tax country '{value}'")
    return country


def supported_tax_countries():
    return ACTIVE_TAX_COUNTRIES


def profile_value(profile, key, default=None):
    if hasattr(profile, "keys") and key in profile.keys():
        return profile[key]
    return default


def build_tax_policy(profile):
    country = normalize_tax_country(profile_value(profile, "tax_country"))
    return POLICY_BUILDERS[country](profile)


def build_generic_policy(profile):
    long_term_days = int(profile_value(profile, "tax_long_term_days", DEFAULT_LONG_TERM_DAYS) or DEFAULT_LONG_TERM_DAYS)
    if long_term_days < 0:
        raise ValueError("tax_long_term_days cannot be negative")
    return TaxPolicy(
        tax_country=DEFAULT_TAX_COUNTRY,
        fiat_currency=str(profile_value(profile, "fiat_currency")).strip().upper(),
        long_term_days=long_term_days,
        accounting_methods=DEFAULT_ACCOUNTING_METHODS,
        report_generators=DEFAULT_REPORT_GENERATORS,
    )


def build_austrian_policy(profile):
    # Keep legacy Austrian profiles readable until the RP2-backed path lands.
    long_term_days = int(profile_value(profile, "tax_long_term_days", DEFAULT_LONG_TERM_DAYS) or DEFAULT_LONG_TERM_DAYS)
    if long_term_days < 0:
        raise ValueError("tax_long_term_days cannot be negative")
    return TaxPolicy(
        tax_country=AUSTRIAN_TAX_COUNTRY,
        fiat_currency="EUR",
        long_term_days=DEFAULT_LONG_TERM_DAYS,
        accounting_methods=AUSTRIAN_ACCOUNTING_METHODS,
        report_generators=DEFAULT_REPORT_GENERATORS,
    )


POLICY_BUILDERS = {
    DEFAULT_TAX_COUNTRY: build_generic_policy,
    AUSTRIAN_TAX_COUNTRY: build_austrian_policy,
}


def require_tax_country_supported_for_profile_mutation(value):
    country = normalize_tax_country(value)
    if country in ACTIVE_TAX_COUNTRIES:
        return country
    if country == AUSTRIAN_TAX_COUNTRY:
        raise AppError(
            "Austrian tax profiles are currently unavailable in Kassiber.",
            code="unsupported",
            hint=(
                "Use `--tax-country generic` for the active RP2-backed path. "
                "Future Austrian support will land through the Kassiber-maintained RP2 fork at "
                "`bitcoinaustria/rp2`."
            ),
            details={"tax_country": country, "planned_engine": "bitcoinaustria/rp2"},
        )
    raise AppError(
        f"Unsupported tax country '{value}'",
        code="validation",
        hint=f"Choose one of: {', '.join(sorted(ACTIVE_TAX_COUNTRIES))}",
    )


def require_tax_processing_supported(profile):
    country = normalize_tax_country(profile_value(profile, "tax_country"))
    if country == DEFAULT_TAX_COUNTRY:
        return
    if country == AUSTRIAN_TAX_COUNTRY:
        raise AppError(
            "Austrian tax processing is currently unavailable in Kassiber.",
            code="unsupported",
            hint=(
                "Use `tax_country=generic` for the active RP2-backed path. "
                "Future Austrian support will land through the Kassiber-maintained RP2 fork at "
                "`bitcoinaustria/rp2`."
            ),
            details={"tax_country": country, "planned_engine": "bitcoinaustria/rp2"},
        )
    raise AppError(
        f"Unsupported tax country '{country}'",
        code="validation",
        hint=f"Choose one of: {', '.join(sorted(ACTIVE_TAX_COUNTRIES))}",
    )
