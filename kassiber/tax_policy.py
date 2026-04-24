from dataclasses import dataclass
from importlib import import_module

from .errors import AppError


DEFAULT_TAX_COUNTRY = "generic"
AUSTRIAN_TAX_COUNTRY = "at"
DEFAULT_LONG_TERM_DAYS = 365
DEFAULT_REPORT_GENERATORS = ("open_positions", "rp2_full_report")
DEFAULT_ACCOUNTING_METHODS = ("fifo", "lifo", "hifo", "lofo")
ACTIVE_TAX_COUNTRIES = (DEFAULT_TAX_COUNTRY, AUSTRIAN_TAX_COUNTRY)


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


def _load_rp2_austrian_country():
    try:
        module = import_module("rp2.plugin.country.at")
    except ModuleNotFoundError as exc:
        raise AppError(
            "Austrian tax support requires rp2 with the `at` country plugin.",
            code="unsupported",
            hint=(
                "Install the Kassiber-maintained rp2 fork from `bitcoinaustria/rp2` "
                "with the Austrian country plugin."
            ),
            details={"missing_module": "rp2.plugin.country.at"},
        ) from exc
    return module.AT()


def build_austrian_policy(profile):
    country = _load_rp2_austrian_country()
    return TaxPolicy(
        tax_country=AUSTRIAN_TAX_COUNTRY,
        fiat_currency=country.currency_iso_code.upper(),
        long_term_days=country.get_long_term_capital_gain_period(),
        accounting_methods=tuple(sorted(country.get_accounting_methods())),
        report_generators=tuple(sorted(country.get_report_generators())),
        default_accounting_method=country.get_default_accounting_method(),
        generation_language=country.get_default_generation_language(),
    )


POLICY_BUILDERS = {
    DEFAULT_TAX_COUNTRY: build_generic_policy,
    AUSTRIAN_TAX_COUNTRY: build_austrian_policy,
}


def require_tax_country_supported_for_profile_mutation(value):
    country = normalize_tax_country(value)
    if country in ACTIVE_TAX_COUNTRIES:
        return country
    raise AppError(
        f"Unsupported tax country '{value}'",
        code="validation",
        hint=f"Choose one of: {', '.join(sorted(ACTIVE_TAX_COUNTRIES))}",
    )


def require_tax_processing_supported(profile):
    country = normalize_tax_country(profile_value(profile, "tax_country"))
    if country in ACTIVE_TAX_COUNTRIES:
        return
    raise AppError(
        f"Unsupported tax country '{country}'",
        code="validation",
        hint=f"Choose one of: {', '.join(sorted(ACTIVE_TAX_COUNTRIES))}",
    )
