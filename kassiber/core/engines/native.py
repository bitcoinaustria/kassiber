"""RP2-shaped calculation backend over Kassiber's native tax engine.

``kassiber/core/engines/rp2.py`` (the adapter) reaches its calculation
backend through ``_get_rp2_modules()``. When the native backend is selected,
that returns :data:`MODULES` from this module instead of RP2's classes, so
the same adapter code drives both backends. Only the subset of RP2's
interface the adapter and ``explanation.py`` use is mirrored here, with RP2's
names, check order, messages and value representation
(``docs/reference/tax-engine.md``).

Every calculation and every constructor check runs in the engine
(``kassiber_tax``); this module builds requests with ``native_request`` and
holds the values the engine returns. It never imports ``rp2``, performs no
I/O and keeps no state between calls.

Listed differences from RP2 (plan 20): errors are Kassiber classes
(``EngineValueError`` for ``RP2ValueError`` and so on, with RP2's messages),
the configuration is built from settings without a temporary INI file, and
decimal arguments must be :class:`EngineDecimal` (what ``_rp2_decimal``
returns) wherever RP2 requires an ``RP2Decimal``.
"""

from __future__ import annotations

import configparser
import sys
from dataclasses import dataclass
from datetime import date, datetime
from decimal import Decimal, DivisionByZero, InvalidOperation, Overflow
from enum import Enum
from types import SimpleNamespace
from typing import Any, Iterator, Mapping, Optional, Sequence

import kassiber_tax

from . import native_request as _request

# ---------------------------------------------------------------------------
# Errors


class EngineError(Exception):
    """An RP2 error the engine reports, with RP2's message."""

    def __init__(self, message: str) -> None:
        self.__message = message
        super().__init__(message)

    def __repr__(self) -> str:
        return self.__message

    @property
    def message(self) -> str:
        return self.__message


class EngineValueError(EngineError):
    """RP2's ``RP2ValueError``."""


class EngineTypeError(EngineError):
    """RP2's ``RP2TypeError``."""


class EngineRuntimeError(EngineError):
    """RP2's ``RP2RuntimeError``."""


class EngineContractError(RuntimeError):
    """A malformed request or an operation the engine does not implement.

    Both are caller or engine bugs, never a property of the book, so they
    block the report like a Rust panic (which the binding raises as
    ``RuntimeError``).
    """

    def __init__(self, error_class: str, message: str) -> None:
        self.error_class = error_class
        super().__init__(f"{error_class}: {message}")


_ERROR_CLASSES = {
    "ValueError": EngineValueError,
    "TypeError": EngineTypeError,
    "RuntimeError": EngineRuntimeError,
    # RP2's arithmetic raises these ``decimal`` signals unwrapped.
    "InvalidOperation": InvalidOperation,
    "DivisionByZero": DivisionByZero,
    "Overflow": Overflow,
}


def _engine_error(body: Mapping[str, Any]) -> Exception:
    error_class = str(body.get("class"))
    message = str(body.get("message"))
    exception = _ERROR_CLASSES.get(error_class)
    if exception is None:
        return EngineContractError(error_class, message)
    return exception(message)


def _call(operation: str, document: Mapping[str, Any]) -> dict[str, Any]:
    """Run one binding call; raise the engine's error as Python's."""
    function = kassiber_tax.check_entry if operation == "check_entry" else kassiber_tax.compute
    response = _request.loads(function(_request.dumps(document)))
    if response.get("ok") is False or "error" in response:
        raise _engine_error(response.get("error") or {})
    return response


# ---------------------------------------------------------------------------
# Decimals

_CRYPTO_DECIMAL_MASK = Decimal("1.0000000000000")
_PLAIN_ZERO = Decimal("0")


def _operand(other: object) -> None:
    if not isinstance(other, Decimal):
        raise EngineTypeError(f"Operand has non-Decimal value {repr(other)}")


class EngineDecimal(Decimal):
    """``RP2Decimal``'s semantics over Python's ``Decimal``.

    Comparisons round ``self - other`` half-even to 13 decimal places, so
    values within ``5E-14`` are equal. Operands must be ``Decimal``s, and
    arithmetic returns ``EngineDecimal`` at the current thread context, which
    ``kassiber/__init__.py`` pins to 32 digits. Like ``RP2Decimal`` it is
    unhashable.
    """

    def __eq__(self, other: object) -> bool:
        _operand(other)
        return Decimal.__eq__(Decimal.__sub__(self, other).quantize(_CRYPTO_DECIMAL_MASK), _PLAIN_ZERO)

    def __ne__(self, other: object) -> bool:
        return not self.__eq__(other)

    def __ge__(self, other: object) -> bool:
        _operand(other)
        return Decimal.__ge__(Decimal.__sub__(self, other).quantize(_CRYPTO_DECIMAL_MASK), _PLAIN_ZERO)

    def __gt__(self, other: object) -> bool:
        _operand(other)
        return Decimal.__gt__(Decimal.__sub__(self, other).quantize(_CRYPTO_DECIMAL_MASK), _PLAIN_ZERO)

    def __le__(self, other: object) -> bool:
        return not self.__gt__(other)

    def __lt__(self, other: object) -> bool:
        return not self.__ge__(other)

    def __add__(self, other: object) -> "EngineDecimal":
        _operand(other)
        return EngineDecimal(Decimal.__add__(self, other))

    def __sub__(self, other: object) -> "EngineDecimal":
        _operand(other)
        return EngineDecimal(Decimal.__sub__(self, other))

    def __mul__(self, other: object) -> "EngineDecimal":
        _operand(other)
        return EngineDecimal(Decimal.__mul__(self, other))

    def __truediv__(self, other: object) -> "EngineDecimal":
        _operand(other)
        return EngineDecimal(Decimal.__truediv__(self, other))

    def __floordiv__(self, other: object) -> "EngineDecimal":
        _operand(other)
        return EngineDecimal(Decimal.__floordiv__(self, other))

    def __pow__(self, other: object, modulo: object = None) -> "EngineDecimal":
        _operand(other)
        if modulo is not None and not isinstance(modulo, Decimal):
            # RP2's message names the exponent, not the modulo.
            raise EngineTypeError(f"Modulo has non-Decimal value {repr(other)}")
        return EngineDecimal(Decimal.__pow__(self, other, modulo))

    def __mod__(self, other: object) -> "EngineDecimal":
        _operand(other)
        return EngineDecimal(Decimal.__mod__(self, other))

    def __radd__(self, other: object) -> "EngineDecimal":
        _operand(other)
        return EngineDecimal(Decimal.__radd__(self, other))

    def __rsub__(self, other: object) -> "EngineDecimal":
        _operand(other)
        return EngineDecimal(Decimal.__rsub__(self, other))

    def __rmul__(self, other: object) -> "EngineDecimal":
        _operand(other)
        return EngineDecimal(Decimal.__rmul__(self, other))

    def __rtruediv__(self, other: object) -> "EngineDecimal":
        _operand(other)
        return EngineDecimal(Decimal.__rtruediv__(self, other))

    def __rfloordiv__(self, other: object) -> "EngineDecimal":
        _operand(other)
        return EngineDecimal(Decimal.__rfloordiv__(self, other))

    def __rmod__(self, other: object) -> "EngineDecimal":
        _operand(other)
        return EngineDecimal(Decimal.__rmod__(self, other))

    def __neg__(self) -> "EngineDecimal":
        return EngineDecimal(Decimal.__neg__(self))


ZERO = EngineDecimal("0")


def _decoded(text: Optional[str]) -> Optional[EngineDecimal]:
    """A canonical engine value as an exact ``EngineDecimal``."""
    value = _request.decode_optional_decimal(text)
    return None if value is None else EngineDecimal(value)


# ---------------------------------------------------------------------------
# Enumerations


class TransactionType(Enum):
    AIRDROP = "airdrop"
    BUY = "buy"
    DONATE = "donate"
    FEE = "fee"
    GIFT = "gift"
    HARDFORK = "hardfork"
    INCOME = "income"
    INTEREST = "interest"
    LOST = "lost"
    MINING = "mining"
    MOVE = "move"
    SELL = "sell"
    STAKING = "staking"
    WAGES = "wages"

    def is_earn_type(self) -> bool:
        return self in _EARN_TYPES


_EARN_TYPES = frozenset(
    {
        TransactionType.AIRDROP,
        TransactionType.HARDFORK,
        TransactionType.INCOME,
        TransactionType.INTEREST,
        TransactionType.MINING,
        TransactionType.STAKING,
        TransactionType.WAGES,
    }
)


class EntrySetType(Enum):
    IN = "in"
    INTRA = "intra"
    MIXED = "mixed"
    OUT = "out"


class AtDisposalCategory(Enum):
    """The Austrian category of one gain/loss, as RP2's ``at`` plugin names it."""

    INCOME_GENERAL = "income_general"
    INCOME_CAPITAL_YIELD = "income_capital_yield"
    NEU_GAIN = "neu_gain"
    NEU_LOSS = "neu_loss"
    NEU_SWAP = "neu_swap"
    ALT_SPEKULATION = "alt_spekulation"
    ALT_TAXFREE = "alt_taxfree"


# ---------------------------------------------------------------------------
# Countries

# ISO 4217 alpha-3 codes, as RP2's ``pycountry`` lookup accepts them
# (case-insensitively); ``tests/test_tax_engine_backend.py`` keeps them in step.
_ISO_4217 = frozenset(
    """
    AED AFN ALL AMD AOA ARS AUD AWG AZN BAM BBD BDT BHD BIF BMD BND BOB BOV BRL
    BSD BTN BWP BYN BZD CAD CDF CHE CHF CHW CLF CLP CNY COP COU CRC CUP CVE CZK
    DJF DKK DOP DZD EGP ERN ETB EUR FJD FKP GBP GEL GHS GIP GMD GNF GTQ GYD HKD
    HNL HTG HUF IDR ILS INR IQD IRR ISK JMD JOD JPY KES KGS KHR KMF KPW KRW KWD
    KYD KZT LAK LBP LKR LRD LSL LYD MAD MDL MGA MKD MMK MNT MOP MRU MUR MVR MWK
    MXN MXV MYR MZN NAD NGN NIO NOK NPR NZD OMR PAB PEN PGK PHP PKR PLN PYG QAR
    RON RSD RUB RWF SAR SBD SCR SDG SEK SGD SHP SLE SOS SRD SSP STN SVC SYP SZL
    THB TJS TMT TND TOP TRY TTD TWD TZS UAH UGX USD USN UYI UYU UYW UZS VED VES
    VND VUV WST XAD XAF XAG XAU XBA XBB XBC XBD XCD XCG XDR XOF XPD XPF XPT XSU
    XTS XUA XXX YER ZAR ZMW ZWG
    """.split()
)

# The country codes the adapter passes; RP2 accepts any ISO 3166-1 alpha-2.
_COUNTRY_CODES = frozenset({"generic", "at"})


class AbstractCountry:
    """RP2's ``AbstractCountry``: codes, policy hooks, and its default hooks."""

    def __init__(self, country_iso_code: str, currency_iso_code: str) -> None:
        if not isinstance(country_iso_code, str):
            raise EngineTypeError(f"Parameter 'country_iso_code' has non-string value {repr(country_iso_code)}")
        if not isinstance(currency_iso_code, str):
            raise EngineTypeError(f"Parameter 'currency_iso_code' has non-string value {repr(currency_iso_code)}")
        if country_iso_code not in _COUNTRY_CODES:
            raise EngineValueError(
                f"Parameter 'country_iso_code' has non-ISO-3166-1 alpha-2 format value {country_iso_code}."
            )
        if currency_iso_code.upper() not in _ISO_4217:
            raise EngineValueError(f"Parameter 'currency_iso_code' has non-ISO-4217 format value {currency_iso_code}.")
        self.__country_iso_code = country_iso_code
        self.__currency_iso_code = currency_iso_code

    @property
    def country_iso_code(self) -> str:
        return self.__country_iso_code

    @property
    def currency_iso_code(self) -> str:
        return self.__currency_iso_code

    def get_long_term_capital_gain_period(self) -> int:
        raise NotImplementedError("Abstract function")

    def get_default_accounting_method(self) -> str:
        raise NotImplementedError("Abstract function")

    def get_accounting_methods(self) -> set[str]:
        raise NotImplementedError("Abstract function")

    def get_report_generators(self) -> set[str]:
        raise NotImplementedError("Abstract function")

    def get_default_generation_language(self) -> str:
        raise NotImplementedError("Abstract function")

    def get_default_application_method(self) -> str:
        return "universal"

    def get_application_methods(self) -> set[str]:
        return {"universal"}

    def validate_input_data(self, input_data_list: Sequence["InputData"]) -> None:
        return None

    def compute_tax_for_assets(
        self,
        configuration: "Configuration",
        accounting_engine: "AccountingEngine",
        asset_to_input_data: Mapping[str, "InputData"],
    ) -> Optional[dict[str, "ComputedData"]]:
        return None


def _country_spec(country: AbstractCountry) -> dict[str, Any]:
    if isinstance(country, AT):
        return _request.country_spec("at")
    return _request.country_spec("generic", country.get_long_term_capital_gain_period())


class AT(AbstractCountry):
    """Austria, with RP2's ``at`` plugin policy; its hooks run in the engine."""

    def __init__(self) -> None:
        super().__init__("at", "eur")

    def get_long_term_capital_gain_period(self) -> int:
        return sys.maxsize

    def get_default_accounting_method(self) -> str:
        return "moving_average_at"

    def get_accounting_methods(self) -> set[str]:
        return {"fifo", "moving_average", "moving_average_at"}

    def get_report_generators(self) -> set[str]:
        return {"open_positions"}

    def get_default_generation_language(self) -> str:
        return "en"

    def validate_input_data(self, input_data_list: Sequence["InputData"]) -> None:
        """RP2's swap-link pairing checks over every asset, in list order."""
        input_data_list = list(input_data_list)
        for input_data in input_data_list:
            InputData.type_check("input_data", input_data)
        _call(
            "compute",
            _request.compute_request(
                method=self.get_default_accounting_method(),
                assets=_assets_request(input_data_list),
                country=_request.country_spec("at"),
                operation="validate",
            ),
        )

    def compute_tax_for_assets(
        self,
        configuration: "Configuration",
        accounting_engine: "AccountingEngine",
        asset_to_input_data: Mapping[str, "InputData"],
    ) -> Optional[dict[str, "ComputedData"]]:
        """The Austrian multi-asset runner; ``None`` without swap pairs."""
        Configuration.type_check("configuration", configuration)
        AccountingEngine.type_check("accounting_engine", accounting_engine)
        input_data_list = list(asset_to_input_data.values())
        for input_data in input_data_list:
            InputData.type_check("input_data", input_data)
        response = _call(
            "compute",
            _request.compute_request(
                method=accounting_engine.method_name(),
                assets=_assets_request(input_data_list),
                country=_request.country_spec("at"),
                operation="compute_multi",
            ),
        )
        if response.get("handled") is False:
            return None
        outputs = {str(document["asset"]): document for document in response.get("assets", ())}
        return {
            asset: ComputedData(input_data, outputs[input_data.asset])
            for asset, input_data in asset_to_input_data.items()
            if input_data.asset in outputs
        }


def classify_disposal(gain_loss: "GainLoss") -> AtDisposalCategory:
    """The Austrian category the engine assigned to ``gain_loss``.

    RP2 classifies when the adapter asks, so a classification error (for
    example an empty ``at_swap_link=`` marker on a Neu disposal) is raised
    here, not while computing.
    """
    if not isinstance(gain_loss, GainLoss):
        raise EngineTypeError(f"Parameter 'gain_loss' is not of type GainLoss: {gain_loss}")
    if gain_loss._at_category_error is not None:
        raise _engine_error(gain_loss._at_category_error)
    if gain_loss._at_category is None:
        raise EngineRuntimeError("Internal error: the engine returned no Austrian category for this gain/loss")
    try:
        return AtDisposalCategory(gain_loss._at_category)
    except ValueError:
        raise EngineRuntimeError(f"Internal error: unknown Austrian category {gain_loss._at_category!r}") from None


# ---------------------------------------------------------------------------
# Configuration


def _string_set(field: str, section: configparser.SectionProxy) -> frozenset[str]:
    # RP2 reads these fields from an INI file with a default ConfigParser:
    # ``%`` interpolation applies and each comma-separated element is
    # stripped. The same section is parsed from memory so the same values
    # pass or fail the same way; messages lack RP2's file-path prefix.
    text = section[field]
    if not text or not text.strip():
        raise EngineValueError(f"field '{field}' in section '{section.name}' cannot be empty")
    result: set[str] = set()
    for element in (value.strip() for value in text.split(",")):
        if not element:
            raise EngineValueError(f"field '{field}' in section '{section.name}' cannot contain empty elements")
        if element in result:
            raise EngineValueError(f"field '{field}' in section '{section.name}' contains duplicate elements: {element}")
        result.add(element)
    return frozenset(result)


def _type_check_string(name: str, value: Any) -> str:
    if not isinstance(value, str):
        raise EngineTypeError(f"Parameter '{name}' has non-string value {repr(value)}")
    return value


class Configuration:
    """RP2's run configuration, built from settings instead of an INI file.

    ``assets``, ``exchanges`` (the wallet labels) and ``holders`` (the
    profile label) are the names RP2's membership checks accept. RP2's
    defaults apply: no date window and no negative balances.
    """

    def __init__(
        self,
        country: AbstractCountry,
        *,
        assets: Sequence[str],
        exchanges: Sequence[str],
        holders: Sequence[str],
    ) -> None:
        if not isinstance(country, AbstractCountry):
            raise EngineTypeError(f"Parameter 'country' is not of type AbstractCountry: {country}")
        self.__country = country
        parser = configparser.ConfigParser()
        parser.read_string(
            "[general]\n"
            f"assets = {', '.join(assets)}\n"
            f"exchanges = {', '.join(exchanges)}\n"
            f"holders = {', '.join(holders)}\n"
        )
        general = parser["general"]
        self.__assets = _string_set("assets", general)
        self.__exchanges = _string_set("exchanges", general)
        self.__holders = _string_set("holders", general)

    @classmethod
    def type_check(cls, name: str, instance: Any) -> "Configuration":
        if not isinstance(instance, cls):
            raise EngineTypeError(f"Parameter '{name}' is not of type {cls.__name__}: {instance}")
        return instance

    @property
    def country(self) -> AbstractCountry:
        return self.__country

    @property
    def assets(self) -> frozenset[str]:
        return self.__assets

    @property
    def exchanges(self) -> frozenset[str]:
        return self.__exchanges

    @property
    def holders(self) -> frozenset[str]:
        return self.__holders

    @property
    def from_date(self) -> date:
        return date(1970, 1, 1)

    @property
    def to_date(self) -> date:
        return date(9999, 12, 31)

    def type_check_asset(self, name: str, value: Any) -> str:
        _type_check_string(name, value)
        if value not in self.__assets:
            raise EngineValueError(f"Parameter '{name}' value is not known: {value}")
        return value

    def names(self) -> dict[str, list[str]]:
        """The membership names, as a ``check_entry`` request carries them."""
        return {
            "assets": sorted(self.__assets),
            "exchanges": sorted(self.__exchanges),
            "holders": sorted(self.__holders),
        }


# ---------------------------------------------------------------------------
# Transactions


def _check_decimals(values: Mapping[str, Any]) -> None:
    for name, value in values.items():
        if value is None:
            continue
        if not isinstance(value, EngineDecimal):
            raise EngineTypeError(f"Parameter '{name}' has non-RP2Decimal value {repr(value)}")
        if not value.is_finite():
            # RP2's positivity check signals InvalidOperation on NaN and infinity.
            raise InvalidOperation([InvalidOperation])


class AbstractTransaction:
    """One engine entry: the constructor's arguments and its derived values."""

    _KIND = ""
    _DECIMALS: tuple[str, ...] = ()

    def __init__(self, configuration: Any, build: Any, arguments: dict[str, Any]) -> None:
        Configuration.type_check("configuration", configuration)
        _type_check_string("asset", arguments["asset"])
        decimals = {name: arguments[name] for name in self._DECIMALS if name in arguments}
        _check_decimals(decimals)
        if arguments.get("row") is None:
            arguments["row"] = id(self)
        try:
            entry = build(**arguments)
        except TypeError as exc:
            raise EngineTypeError(str(exc)) from None
        derived = _call("check_entry", _request.check_entry_request(entry, **configuration.names()))["entry"]
        self.__configuration = configuration
        self.__text = _request.entry_text(entry, derived)
        self.__entry = _request.with_text(entry, self.__text)
        self.__derived = derived
        self.__timestamp = _request.parse_timestamp(entry["ts"]["raw"])
        self.__transaction_type = TransactionType(derived["transaction_type"])

    @classmethod
    def type_check(cls, name: str, instance: Any) -> "AbstractTransaction":
        if not isinstance(instance, cls):
            raise EngineTypeError(f"Parameter '{name}' is not of type {cls.__name__}: {instance}")
        return instance

    def _value(self, name: str) -> Optional[EngineDecimal]:
        return _decoded(self.__derived[name])

    @property
    def configuration(self) -> Configuration:
        return self.__configuration

    @property
    def request_entry(self) -> dict[str, Any]:
        """The engine entry, with its ``str(transaction)`` text."""
        return self.__entry

    @property
    def kind(self) -> str:
        return self._KIND

    @property
    def row(self) -> int:
        return int(self.__derived["row"])

    @property
    def internal_id(self) -> str:
        return str(self.__derived["row"])

    @property
    def unique_id(self) -> str:
        return str(self.__derived["unique_id"])

    @property
    def notes(self) -> str:
        return str(self.__derived["notes"])

    @property
    def asset(self) -> str:
        return str(self.__entry["asset"])

    @property
    def timestamp(self) -> datetime:
        return self.__timestamp

    @property
    def transaction_type(self) -> TransactionType:
        return self.__transaction_type

    @property
    def spot_price(self) -> EngineDecimal:
        return self._value("spot_price")

    @property
    def crypto_fee(self) -> EngineDecimal:
        return self._value("crypto_fee")

    @property
    def fiat_fee(self) -> EngineDecimal:
        return self._value("fiat_fee")

    @property
    def crypto_balance_change(self) -> EngineDecimal:
        return self._value("crypto_balance_change")

    @property
    def crypto_taxable_amount(self) -> EngineDecimal:
        return self._value("crypto_taxable_amount")

    @property
    def fiat_taxable_amount(self) -> EngineDecimal:
        return self._value("fiat_taxable_amount")

    def is_taxable(self) -> bool:
        taxable = self.__derived["is_taxable"]
        if taxable is None:
            # RP2 evaluates taxability lazily; its 13-place comparison of an
            # out-of-range fee signals InvalidOperation.
            raise InvalidOperation([InvalidOperation])
        return bool(taxable)

    def is_earning(self) -> bool:
        return bool(self.__derived["is_earn"])

    def __str__(self) -> str:
        return self.__text

    def __repr__(self) -> str:
        return f"{type(self).__name__}(id={self.internal_id}, unique_id={self.unique_id!r}, asset={self.asset!r})"

    def __eq__(self, other: object) -> bool:
        if not other:
            return False
        if not isinstance(other, AbstractTransaction):
            raise EngineTypeError(f"Operand has non-AbstractTransaction value {repr(other)}")
        return self.internal_id == other.internal_id

    def __ne__(self, other: object) -> bool:
        return not self.__eq__(other)

    def __hash__(self) -> int:
        return hash(self.internal_id)


class InTransaction(AbstractTransaction):
    """RP2's ``InTransaction``: an acquisition lot, taxable when earn-typed."""

    _KIND = "in"
    _DECIMALS = ("spot_price", "crypto_in", "crypto_fee", "fiat_in_no_fee", "fiat_in_with_fee", "fiat_fee")

    def __init__(
        self,
        configuration: Configuration,
        timestamp: str,
        asset: str,
        exchange: str,
        holder: str,
        transaction_type: str,
        spot_price: EngineDecimal,
        crypto_in: EngineDecimal,
        crypto_fee: Optional[EngineDecimal] = None,
        fiat_in_no_fee: Optional[EngineDecimal] = None,
        fiat_in_with_fee: Optional[EngineDecimal] = None,
        fiat_fee: Optional[EngineDecimal] = None,
        row: Optional[int] = None,
        unique_id: Optional[str] = None,
        notes: Optional[str] = None,
    ) -> None:
        super().__init__(
            configuration,
            _request.in_entry,
            dict(
                timestamp=timestamp,
                asset=asset,
                exchange=exchange,
                holder=holder,
                transaction_type=transaction_type,
                spot_price=spot_price,
                crypto_in=crypto_in,
                crypto_fee=crypto_fee,
                fiat_in_no_fee=fiat_in_no_fee,
                fiat_in_with_fee=fiat_in_with_fee,
                fiat_fee=fiat_fee,
                row=row,
                unique_id=unique_id,
                notes=notes,
            ),
        )

    @property
    def exchange(self) -> str:
        return str(self.request_entry["exchange"])

    @property
    def holder(self) -> str:
        return str(self.request_entry["holder"])

    @property
    def crypto_in(self) -> EngineDecimal:
        return self._value("crypto_in")

    @property
    def fiat_in_no_fee(self) -> EngineDecimal:
        return self._value("fiat_in_no_fee")

    @property
    def fiat_in_with_fee(self) -> EngineDecimal:
        return self._value("fiat_in_with_fee")

    @property
    def cost_basis_timestamp(self) -> datetime:
        return self.timestamp

    @property
    def from_lot(self) -> None:
        return None


class OutTransaction(AbstractTransaction):
    """RP2's ``OutTransaction``: a disposal."""

    _KIND = "out"
    _DECIMALS = ("spot_price", "crypto_out_no_fee", "crypto_fee", "crypto_out_with_fee", "fiat_out_no_fee", "fiat_fee")

    def __init__(
        self,
        configuration: Configuration,
        timestamp: str,
        asset: str,
        exchange: str,
        holder: str,
        transaction_type: str,
        spot_price: EngineDecimal,
        crypto_out_no_fee: EngineDecimal,
        crypto_fee: EngineDecimal,
        crypto_out_with_fee: Optional[EngineDecimal] = None,
        fiat_out_no_fee: Optional[EngineDecimal] = None,
        fiat_fee: Optional[EngineDecimal] = None,
        row: Optional[int] = None,
        unique_id: Optional[str] = None,
        notes: Optional[str] = None,
    ) -> None:
        super().__init__(
            configuration,
            _request.out_entry,
            dict(
                timestamp=timestamp,
                asset=asset,
                exchange=exchange,
                holder=holder,
                transaction_type=transaction_type,
                spot_price=spot_price,
                crypto_out_no_fee=crypto_out_no_fee,
                crypto_fee=crypto_fee,
                crypto_out_with_fee=crypto_out_with_fee,
                fiat_out_no_fee=fiat_out_no_fee,
                fiat_fee=fiat_fee,
                row=row,
                unique_id=unique_id,
                notes=notes,
            ),
        )

    @property
    def exchange(self) -> str:
        return str(self.request_entry["exchange"])

    @property
    def holder(self) -> str:
        return str(self.request_entry["holder"])

    @property
    def crypto_out_no_fee(self) -> EngineDecimal:
        return self._value("crypto_out_no_fee")

    @property
    def crypto_out_with_fee(self) -> EngineDecimal:
        return self._value("crypto_out_with_fee")

    @property
    def fiat_out_no_fee(self) -> EngineDecimal:
        return self._value("fiat_out_no_fee")

    @property
    def fiat_out_with_fee(self) -> EngineDecimal:
        return self._value("fiat_out_with_fee")


class IntraTransaction(AbstractTransaction):
    """RP2's ``IntraTransaction``: a move; its fee can be a taxable event."""

    _KIND = "intra"
    _DECIMALS = ("spot_price", "crypto_sent", "crypto_received")

    def __init__(
        self,
        configuration: Configuration,
        timestamp: str,
        asset: str,
        from_exchange: str,
        from_holder: str,
        to_exchange: str,
        to_holder: str,
        spot_price: Optional[EngineDecimal],
        crypto_sent: EngineDecimal,
        crypto_received: EngineDecimal,
        row: Optional[int] = None,
        unique_id: Optional[str] = None,
        notes: Optional[str] = None,
    ) -> None:
        super().__init__(
            configuration,
            _request.intra_entry,
            dict(
                timestamp=timestamp,
                asset=asset,
                from_exchange=from_exchange,
                from_holder=from_holder,
                to_exchange=to_exchange,
                to_holder=to_holder,
                spot_price=spot_price,
                crypto_sent=crypto_sent,
                crypto_received=crypto_received,
                row=row,
                unique_id=unique_id,
                notes=notes,
            ),
        )

    @property
    def from_exchange(self) -> str:
        return str(self.request_entry["from_exchange"])

    @property
    def from_holder(self) -> str:
        return str(self.request_entry["from_holder"])

    @property
    def to_exchange(self) -> str:
        return str(self.request_entry["to_exchange"])

    @property
    def to_holder(self) -> str:
        return str(self.request_entry["to_holder"])

    @property
    def crypto_sent(self) -> EngineDecimal:
        return self._value("crypto_sent")

    @property
    def crypto_received(self) -> EngineDecimal:
        return self._value("crypto_received")


_SET_CLASSES = {EntrySetType.IN: InTransaction, EntrySetType.OUT: OutTransaction, EntrySetType.INTRA: IntraTransaction}
_MIN_DATE = date(1970, 1, 1)


def _entry_set_type(name: str, value: Any) -> EntrySetType:
    _type_check_string(name, value)
    try:
        return EntrySetType(value.lower())
    except ValueError:
        raise EngineValueError(f"Parameter '{name}' has invalid entry set type value: {value}") from None


class TransactionSet:
    """RP2's ``TransactionSet``: one asset's entries of one kind.

    Entries keep insertion order for the engine; iteration follows RP2's
    stable sort by instant and skips entries dated before 1970 in their own
    offset.
    """

    def __init__(self, configuration: Configuration, entry_set_type: str, asset: str) -> None:
        Configuration.type_check("configuration", configuration)
        self.__entry_set_type = _entry_set_type("entry_set_type", entry_set_type)
        self.__asset = configuration.type_check_asset("asset", asset)
        self.__entries: list[AbstractTransaction] = []
        self.__members: set[AbstractTransaction] = set()

    @classmethod
    def type_check(
        cls,
        name: str,
        instance: Any,
        entry_set_type: EntrySetType,
        asset: str,
        allow_empty: bool = False,
    ) -> "TransactionSet":
        if not isinstance(instance, cls):
            raise EngineTypeError(f"Parameter is not of type {cls.__name__}: {instance}")
        if instance.entry_set_type != entry_set_type:
            raise EngineValueError(f"entry_set_type {instance.entry_set_type} != {entry_set_type}: {instance}")
        if instance.asset != asset:
            raise EngineValueError(f"asset {instance.asset} != {asset}: {instance}")
        if not allow_empty and instance.is_empty():
            raise EngineValueError(f"IN transaction set is empty: {instance}")
        return instance

    @property
    def entry_set_type(self) -> EntrySetType:
        return self.__entry_set_type

    @property
    def asset(self) -> str:
        return self.__asset

    @property
    def count(self) -> int:
        return len(self.__entries)

    def is_empty(self) -> bool:
        return self.count == 0

    def add_entry(self, entry: AbstractTransaction) -> None:
        AbstractTransaction.type_check("entry", entry)
        if entry.asset != self.asset:
            raise EngineValueError(f"Attempting to add a {entry.asset} entry to a {self.asset} set")
        expected = _SET_CLASSES.get(self.__entry_set_type)
        if expected is not None and not isinstance(entry, expected):
            raise EngineTypeError(
                f"Attempting to add a {entry.__class__.__name__} to a set of type {self.__entry_set_type.name}"
            )
        if entry in self.__members:
            raise EngineValueError(f"Entry already added: {entry}")
        self.__entries.append(entry)
        self.__members.add(entry)

    def entries(self) -> list[AbstractTransaction]:
        """The entries in insertion order."""
        return list(self.__entries)

    def __iter__(self) -> Iterator[AbstractTransaction]:
        ordered = sorted(self.__entries, key=lambda entry: entry.timestamp)
        return iter([entry for entry in ordered if entry.timestamp.date() >= _MIN_DATE])

    def __str__(self) -> str:
        lines = [
            f"{type(self).__name__}:",
            f"  entry_set_type={self.__entry_set_type}",
            f"  asset={self.__asset}",
            "  entries=",
        ]
        lines.extend(f"    {entry}" for entry in self)
        return "\n".join(lines)


class InputData:
    """RP2's ``InputData``: one asset's IN (non-empty), OUT, and INTRA sets."""

    def __init__(
        self,
        asset: str,
        unfiltered_in_transaction_set: TransactionSet,
        unfiltered_out_transaction_set: TransactionSet,
        unfiltered_intra_transaction_set: TransactionSet,
    ) -> None:
        self.__asset = _type_check_string("asset", asset)
        self.__in = TransactionSet.type_check(
            "in_transaction_set", unfiltered_in_transaction_set, EntrySetType.IN, asset, False
        )
        self.__out = TransactionSet.type_check(
            "out_transaction_set", unfiltered_out_transaction_set, EntrySetType.OUT, asset, True
        )
        self.__intra = TransactionSet.type_check(
            "intra_transaction_set", unfiltered_intra_transaction_set, EntrySetType.INTRA, asset, True
        )

    @classmethod
    def type_check(cls, name: str, instance: Any) -> "InputData":
        if not isinstance(instance, cls):
            raise EngineTypeError(f"Parameter '{name}' is not of type {cls.__name__}: {instance}")
        return instance

    @property
    def asset(self) -> str:
        return self.__asset

    @property
    def unfiltered_in_transaction_set(self) -> TransactionSet:
        return self.__in

    @property
    def unfiltered_out_transaction_set(self) -> TransactionSet:
        return self.__out

    @property
    def unfiltered_intra_transaction_set(self) -> TransactionSet:
        return self.__intra

    # The adapter passes no date window, so the filtered sets are the
    # unfiltered ones.
    filtered_in_transaction_set = unfiltered_in_transaction_set
    filtered_out_transaction_set = unfiltered_out_transaction_set
    filtered_intra_transaction_set = unfiltered_intra_transaction_set

    def entries(self) -> list[AbstractTransaction]:
        """Every entry: the IN, OUT, then INTRA set, each in insertion order."""
        return [*self.__in.entries(), *self.__out.entries(), *self.__intra.entries()]


def _assets_request(input_data_list: Sequence[InputData]) -> dict[str, list[dict[str, Any]]]:
    assets: dict[str, list[dict[str, Any]]] = {}
    for input_data in input_data_list:
        if input_data.asset in assets:
            raise EngineRuntimeError(f"Internal error: asset {input_data.asset} appears twice")
        assets[input_data.asset] = [entry.request_entry for entry in input_data.entries()]
    return assets


# ---------------------------------------------------------------------------
# Accounting methods and the engine holder

ACCOUNTING_METHODS = ("fifo", "lifo", "hifo", "lofo", "moving_average", "moving_average_at")


class AccountingMethod:
    """One accounting method, named as RP2's plugin."""

    def __init__(self, name: str) -> None:
        self.name = name


def accounting_method_module(name: str) -> Any:
    """Stands in for ``rp2.plugin.accounting_method.<name>``."""
    if name not in ACCOUNTING_METHODS:
        raise ModuleNotFoundError(f"the native tax engine has no accounting method {name!r}")
    return SimpleNamespace(AccountingMethod=lambda: AccountingMethod(name))


class AVLTree:
    """Holds the year-to-method assignment, as RP2's ``AVLTree`` does."""

    def __init__(self) -> None:
        self.__nodes: dict[int, Any] = {}

    def insert_node(self, key: int, value: Any) -> None:
        self.__nodes[key] = value

    def nodes(self) -> dict[int, Any]:
        return dict(self.__nodes)


class AccountingEngine:
    """Holds the accounting method. The adapter assigns one method from 1970."""

    def __init__(self, years_2_methods: AVLTree) -> None:
        self.__years_2_methods = years_2_methods

    @classmethod
    def type_check(cls, name: str, instance: Any) -> "AccountingEngine":
        if not isinstance(instance, cls):
            raise EngineTypeError(f"Parameter '{name}' is not of type {cls.__name__}: {instance}")
        return instance

    @property
    def years_2_methods(self) -> AVLTree:
        return self.__years_2_methods

    def method_name(self) -> str:
        nodes = self.__years_2_methods.nodes()
        method = nodes.get(1970)
        if len(nodes) != 1 or not isinstance(method, AccountingMethod):
            # RP2 switches methods by year; Kassiber never configures that.
            raise EngineRuntimeError("Internal error: the native tax engine takes one accounting method assigned from 1970")
        return method.name


# ---------------------------------------------------------------------------
# Results


class GainLoss:
    """One realized fragment: an event's amount matched to a lot (or income)."""

    def __init__(
        self,
        document: Mapping[str, Any],
        taxable_event: AbstractTransaction,
        acquired_lot: Optional[InTransaction],
    ) -> None:
        self.__taxable_event = taxable_event
        self.__acquired_lot = acquired_lot
        self.__crypto_amount = _decoded(document["crypto_amount"])
        self.__fiat_cost_basis = _decoded(document["fiat_cost_basis"])
        self.__proceeds = _decoded(document["proceeds"])
        self.__fiat_gain = _decoded(document["fiat_gain"])
        self.__unit_cost_basis_override = _decoded(document["unit_cost_basis_override"])
        self.__long_term = bool(document["long_term"])
        self._at_category: Optional[str] = document.get("at_category")
        self._at_category_error: Optional[Mapping[str, Any]] = document.get("at_category_error")

    @property
    def taxable_event(self) -> AbstractTransaction:
        return self.__taxable_event

    @property
    def acquired_lot(self) -> Optional[InTransaction]:
        return self.__acquired_lot

    @property
    def asset(self) -> str:
        return self.__taxable_event.asset

    @property
    def timestamp(self) -> datetime:
        return self.__taxable_event.timestamp

    @property
    def crypto_amount(self) -> EngineDecimal:
        return self.__crypto_amount

    @property
    def fiat_cost_basis(self) -> EngineDecimal:
        return self.__fiat_cost_basis

    @property
    def taxable_event_fiat_amount_with_fee_fraction(self) -> EngineDecimal:
        return self.__proceeds

    @property
    def fiat_gain(self) -> EngineDecimal:
        return self.__fiat_gain

    @property
    def unit_cost_basis_override(self) -> Optional[EngineDecimal]:
        return self.__unit_cost_basis_override

    def is_long_term_capital_gains(self) -> bool:
        return self.__long_term


@dataclass(frozen=True)
class YearlyGainLoss:
    year: int
    asset: str
    transaction_type: TransactionType
    is_long_term_capital_gains: bool
    crypto_amount: EngineDecimal
    fiat_amount: EngineDecimal
    fiat_cost_basis: EngineDecimal
    fiat_gain_loss: EngineDecimal


@dataclass(frozen=True)
class Balance:
    asset: str
    exchange: str
    holder: str
    final_balance: EngineDecimal
    acquired_balance: EngineDecimal
    sent_balance: EngineDecimal
    received_balance: EngineDecimal


class ComputedData:
    """The ``ComputedData`` values the adapter reads, from one engine asset.

    Entry objects are the ones the adapter built, mapped back by row; every
    decimal keeps the engine's exact coefficient and exponent.
    """

    def __init__(self, input_data: InputData, document: Mapping[str, Any]) -> None:
        asset = str(document["asset"])
        if asset != input_data.asset:
            raise EngineRuntimeError(f"Internal error: engine returned asset {asset} for {input_data.asset}")
        by_ref: dict[tuple[str, int], AbstractTransaction] = {
            (entry.kind, entry.row): entry for entry in input_data.entries()
        }

        def lot(row: int) -> InTransaction:
            entry = by_ref.get(("in", int(row)))
            if not isinstance(entry, InTransaction):
                raise EngineRuntimeError(f"Internal error: engine returned unknown lot row {row}")
            return entry

        def event(ref: Mapping[str, Any]) -> AbstractTransaction:
            entry = by_ref.get((str(ref["kind"]), int(ref["row"])))
            if entry is None:
                raise EngineRuntimeError(f"Internal error: engine returned unknown event {ref}")
            return entry

        self.__asset = asset
        self.__in_transactions = tuple(lot(row) for row in document["in_transactions"])
        self.__fiat_in_with_fee = {
            lot_entry.row: _decoded(value)
            for lot_entry, value in zip(self.__in_transactions, document["in_fiat_in_with_fee"])
        }
        self.__gain_losses = tuple(
            GainLoss(
                item,
                event(item["event"]),
                None if item["lot"] is None else lot(item["lot"]),
            )
            for item in document["gain_losses"]
        )
        self.__yearly = [
            YearlyGainLoss(
                year=int(item["year"]),
                asset=asset,
                transaction_type=TransactionType(item["transaction_type"]),
                is_long_term_capital_gains=bool(item["long_term"]),
                crypto_amount=_decoded(item["crypto_amount"]),
                fiat_amount=_decoded(item["fiat_amount"]),
                fiat_cost_basis=_decoded(item["fiat_cost_basis"]),
                fiat_gain_loss=_decoded(item["fiat_gain_loss"]),
            )
            for item in document["yearly"]
        ]
        self.__open_positions = tuple(lot(item["row"]) for item in document["open_positions"])
        self.__sold_percentage = {
            int(item["row"]): _decoded(item["sold_percentage"]) for item in document["open_positions"]
        }
        self.__open_fiat_in_with_fee = {
            int(item["row"]): _decoded(item["fiat_in_with_fee"]) for item in document["open_positions"]
        }
        self.__balances = tuple(
            Balance(
                asset=asset,
                exchange=str(item["exchange"]),
                holder=str(item["holder"]),
                final_balance=_decoded(item["final_balance"]),
                acquired_balance=_decoded(item["acquired_balance"]),
                sent_balance=_decoded(item["sent_balance"]),
                received_balance=_decoded(item["received_balance"]),
            )
            for item in document["balances"]
        )

    @property
    def asset(self) -> str:
        return self.__asset

    @property
    def in_transaction_set(self) -> tuple[InTransaction, ...]:
        return self.__in_transactions

    @property
    def gain_loss_set(self) -> tuple[GainLoss, ...]:
        return self.__gain_losses

    @property
    def yearly_gain_loss_list(self) -> list[YearlyGainLoss]:
        return list(self.__yearly)

    @property
    def open_position_in_transaction_set(self) -> tuple[InTransaction, ...]:
        return self.__open_positions

    @property
    def balance_set(self) -> tuple[Balance, ...]:
        return self.__balances

    def get_in_transaction_fiat_in_with_fee(self, in_transaction: InTransaction) -> EngineDecimal:
        InTransaction.type_check("in_transaction", in_transaction)
        return self.__fiat_in_with_fee.get(in_transaction.row, in_transaction.fiat_in_with_fee)

    def get_open_position_in_lot_sold_percentage(self, in_transaction: InTransaction) -> EngineDecimal:
        InTransaction.type_check("in_transaction", in_transaction)
        return self.__sold_percentage.get(in_transaction.row, ZERO)

    def get_open_position_in_transaction_fiat_in_with_fee(self, in_transaction: InTransaction) -> EngineDecimal:
        InTransaction.type_check("in_transaction", in_transaction)
        return self.__open_fiat_in_with_fee.get(
            in_transaction.row, self.get_in_transaction_fiat_in_with_fee(in_transaction)
        )


def compute_tax(
    configuration: Configuration,
    accounting_engine: AccountingEngine,
    input_data: InputData,
) -> ComputedData:
    """RP2's ``compute_tax`` for one asset, as one engine call."""
    Configuration.type_check("configuration", configuration)
    AccountingEngine.type_check("accounting_engine", accounting_engine)
    InputData.type_check("input_data", input_data)
    response = _call(
        "compute",
        _request.compute_request(
            method=accounting_engine.method_name(),
            assets=_assets_request([input_data]),
            country=_country_spec(configuration.country),
        ),
    )
    assets = response.get("assets") or []
    if len(assets) != 1:
        raise EngineRuntimeError(f"Internal error: engine returned {len(assets)} assets for one")
    return ComputedData(input_data, assets[0])


#: What ``_get_rp2_modules()`` returns for the native backend.
MODULES: dict[str, Any] = {
    "AVLTree": AVLTree,
    "AbstractCountry": AbstractCountry,
    "AccountingEngine": AccountingEngine,
    "Configuration": Configuration,
    "InputData": InputData,
    "InTransaction": InTransaction,
    "IntraTransaction": IntraTransaction,
    "OutTransaction": OutTransaction,
    "TransactionSet": TransactionSet,
    "compute_tax": compute_tax,
    "RP2Decimal": EngineDecimal,
}

__all__ = [
    "ACCOUNTING_METHODS",
    "AT",
    "AVLTree",
    "AbstractCountry",
    "AbstractTransaction",
    "AccountingEngine",
    "AccountingMethod",
    "AtDisposalCategory",
    "Balance",
    "ComputedData",
    "Configuration",
    "EngineContractError",
    "EngineDecimal",
    "EngineError",
    "EngineRuntimeError",
    "EngineTypeError",
    "EngineValueError",
    "EntrySetType",
    "GainLoss",
    "InTransaction",
    "InputData",
    "IntraTransaction",
    "MODULES",
    "OutTransaction",
    "TransactionSet",
    "TransactionType",
    "YearlyGainLoss",
    "ZERO",
    "accounting_method_module",
    "classify_disposal",
    "compute_tax",
]
