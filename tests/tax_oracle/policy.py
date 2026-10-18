"""The oracle's tax policy (plan 20 phase 4).

The generic policy is a copy of the product's ``build_generic_policy`` with
its constants. Austria is read from the country the oracle computes with,
RP2's AT plugin, never from the product, so the product's Austrian constants
can be checked against it (``test_oracle_contract.py``).
"""

from __future__ import annotations

from importlib import import_module

from kassiber.errors import AppError
from kassiber.tax_policy import TaxPolicy

DEFAULT_TAX_COUNTRY = "generic"
AUSTRIAN_TAX_COUNTRY = "at"
DEFAULT_LONG_TERM_DAYS = 365
DEFAULT_REPORT_GENERATORS = ("open_positions", "rp2_full_report")
DEFAULT_ACCOUNTING_METHODS = ("fifo", "lifo", "hifo", "lofo", "moving_average")


def profile_value(profile, key, default=None):
    if hasattr(profile, "keys") and key in profile.keys():
        return profile[key]
    return default


def normalize_tax_country(value):
    country = str(value or DEFAULT_TAX_COUNTRY).strip().lower()
    if not country:
        return DEFAULT_TAX_COUNTRY
    if country not in POLICY_BUILDERS:
        raise ValueError(f"Unsupported tax country '{value}'")
    return country


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


def _load_austrian_country():
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
    country = _load_austrian_country()
    # The product offers the generic lot methods on Austrian books too, so
    # the allowed set is the union; the default stays the AT plugin's.
    allowed_methods = tuple(
        sorted(set(country.get_accounting_methods()) | set(DEFAULT_ACCOUNTING_METHODS))
    )
    return TaxPolicy(
        tax_country=AUSTRIAN_TAX_COUNTRY,
        fiat_currency=country.currency_iso_code.upper(),
        long_term_days=country.get_long_term_capital_gain_period(),
        accounting_methods=allowed_methods,
        report_generators=tuple(sorted(country.get_report_generators())),
        default_accounting_method=country.get_default_accounting_method(),
        generation_language=country.get_default_generation_language(),
    )


POLICY_BUILDERS = {
    DEFAULT_TAX_COUNTRY: build_generic_policy,
    AUSTRIAN_TAX_COUNTRY: build_austrian_policy,
}
