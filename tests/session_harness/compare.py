"""Decide whether a candidate run agrees with a recorded reference run.

Values compare exactly except generated identities, which go through a
`Mapper` (`policy.COMPARISON_POLICY`): a uuid4, or a wall-clock value inside
the run's own window, binds on first sight to the candidate's value at the
same position, and every later occurrence must map the same way in both
directions. Values that entered through the inputs (argv, stdin, fixture
files, a restored state) are pinned to themselves first, so only what a run
generated can map. Recorded temp paths that had to move on replay map to
their new location the same way.
"""

from __future__ import annotations

import bisect
import hashlib
import io
import json
import re
import zipfile
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone

from tests.session_harness import dbdump, policy


@dataclass
class Difference:
    where: str
    category: str
    message: str
    reference: str | None = None
    candidate: str | None = None

    def as_dict(self) -> dict:
        return {key: value for key, value in self.__dict__.items() if value is not None}

    def __str__(self) -> str:
        text = f"{self.where}: [{self.category}] {self.message}"
        if self.reference is not None or self.candidate is not None:
            text += f"\n    reference: {self.reference}\n    candidate: {self.candidate}"
        return text


def _short(value, limit: int = 240) -> str:
    text = value if isinstance(value, str) else repr(value)
    return text if len(text) <= limit else text[: limit - 3] + "..."


@dataclass(frozen=True)
class Window:
    """A run's wall-clock span, in epoch seconds, with the policy's slack."""

    start: float
    end: float

    def contains(self, moment: datetime) -> bool:
        stamp = moment.timestamp()
        return self.start - policy.WINDOW_SLACK_SECONDS <= stamp <= self.end + policy.WINDOW_SLACK_SECONDS

    def contains_epoch(self, value: float) -> bool:
        return self.start - policy.WINDOW_SLACK_SECONDS <= value <= self.end + policy.WINDOW_SLACK_SECONDS

    def dates(self) -> set[str]:
        days = set()
        for moment in (self.start - policy.WINDOW_SLACK_SECONDS, self.end + policy.WINDOW_SLACK_SECONDS):
            days.add(datetime.fromtimestamp(moment, timezone.utc).date().isoformat())
            days.add(datetime.fromtimestamp(moment).date().isoformat())
        return days


@dataclass(frozen=True)
class Token:
    kind: str  # "lit", "uuid", "time", "date"
    text: str
    key: object = None
    shape: tuple = ()


_LOCAL = datetime.now().astimezone().tzinfo


def _parse_datetime(match: re.Match) -> tuple[datetime, tuple] | None:
    year, month, day, sep, hour, minute, second, fraction, zone = match.groups()
    try:
        if sep is None:
            return None
        micro = int((fraction or "0")[:6].ljust(6, "0"))
        moment = datetime(int(year), int(month), int(day), int(hour), int(minute), int(second or 0), micro)
    except ValueError:
        return None
    if zone and zone != "Z":
        sign = 1 if zone[0] == "+" else -1
        digits = zone[1:].replace(":", "")
        moment = moment.replace(tzinfo=timezone(sign * timedelta(hours=int(digits[:2]), minutes=int(digits[2:]))))
    elif zone == "Z":
        moment = moment.replace(tzinfo=timezone.utc)
    shape = ("iso", sep, second is not None, len(fraction or ""), zone[0] if zone and zone != "Z" else zone, ":" in (zone or ""))
    return moment, shape


def _parse_compact(match: re.Match) -> tuple[datetime, tuple] | None:
    day, sep, clock, zone = match.groups()
    try:
        moment = datetime.strptime(day + clock, "%Y%m%d%H%M%S")
    except ValueError:
        return None
    if zone:
        moment = moment.replace(tzinfo=timezone.utc)
    return moment, ("compact", sep, zone)


def _instant(moment: datetime) -> int:
    """Microseconds since the epoch; a naive value is read as UTC in both runs."""
    aware = moment if moment.tzinfo else moment.replace(tzinfo=timezone.utc)
    return int(aware.timestamp() * 1_000_000)


def _whole_hours_from_window(moment: datetime, window: Window) -> bool:
    aware = moment if moment.tzinfo else moment.replace(tzinfo=timezone.utc)
    stamp = aware.timestamp()
    hours = round((stamp - window.start) / 3600)
    return hours != 0 and window.contains_epoch(stamp - hours * 3600)


def _in_window(moment: datetime, window: Window) -> bool:
    if moment.tzinfo:
        return window.contains(moment)
    return window.contains(moment.replace(tzinfo=timezone.utc)) or window.contains(moment.replace(tzinfo=_LOCAL))


_SCANNERS = (
    ("uuid", policy.UUID4_RE),
    ("datetime", policy.DATETIME_RE),
    ("compact", policy.COMPACT_DATETIME_RE),
)
# Diagnostic only: what entropy injection would make equal. Any uuid version
# and any long hex run (digests and ids derived from generated values).
_ENTROPY_SCANNERS = (
    ("anyuuid", re.compile(r"\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b", re.IGNORECASE)),
    ("hex", re.compile(r"(?<![0-9a-fA-F])[0-9a-f]{16,}(?![0-9a-fA-F])")),
)
CLOCK_KINDS = frozenset({"time", "date"})
RELAXATIONS = ("clock", "entropy", "order")


class Mapper:
    """Bindings between the reference's and the candidate's generated values."""

    def __init__(self, reference: Window, candidate: Window, relocations: dict[str, str] | None = None,
                 *, relax: frozenset[str] = frozenset()):
        """`relax` is for diagnosis, never acceptance: `clock` keeps only the
        order of clock readings, `entropy` also maps any uuid and long hex
        run, and `order` compares arrays, JSON stored as text, and rows with
        generated keys in a canonical order (ties a run breaks by a clock
        reading or a random id). A run that is clean only when relaxed
        differs by what clock and entropy injection would remove.
        """
        unknown = set(relax) - set(RELAXATIONS)
        if unknown:
            raise ValueError(f"unknown relaxation: {sorted(unknown)}")
        self.relax = frozenset(relax)
        self.scanners = _SCANNERS + (_ENTROPY_SCANNERS if "entropy" in self.relax else ())
        self.windows = {"ref": reference, "cand": candidate}
        self.days = {"ref": reference.dates(), "cand": candidate.dates()}
        self.forward: dict[str, dict] = {}
        self.backward: dict[str, dict] = {}
        self.order: dict[object, list[tuple]] = {}
        # How each bound key is spelled on its side, for translating text.
        self.texts: dict[str, dict] = {"ref": {}, "cand": {}}
        # Reference prefix -> candidate prefix, longest first.
        self.relocations = sorted((relocations or {}).items(), key=lambda item: -len(item[0]))
        # Process ids of the recorded processes -> the replayed ones.
        self.pids: dict[int, int] = {}
        # Synthetic passphrases the session fed in, to read encrypted books.
        self.database_keys: list[str] = []

    def bind_pid(self, reference: int, candidate: int) -> None:
        self.pids[reference] = candidate

    def _pids_to_reference(self, text: str) -> str:
        if not self.pids:
            return text
        back = {str(cand): str(ref) for ref, cand in self.pids.items()}
        return re.sub(r"(?<![0-9])[0-9]+(?![0-9])", lambda match: back.get(match.group(0), match.group(0)), text)

    def set_candidate_window(self, window: Window) -> None:
        self.windows["cand"] = window
        self.days["cand"] = window.dates()

    def reset(self) -> None:
        self.forward.clear()
        self.backward.clear()
        self.order.clear()
        self.texts = {"ref": {}, "cand": {}}

    # Relocation -----------------------------------------------------------

    def to_candidate_paths(self, text: str) -> str:
        for ref, cand in self.relocations:
            text = text.replace(ref, cand)
        return text

    def to_reference_paths(self, text: str) -> str:
        for ref, cand in sorted(self.relocations, key=lambda item: -len(item[1])):
            text = text.replace(cand, ref)
        return text

    # Tokens ---------------------------------------------------------------

    def tokenize(self, text: str, side: str) -> list[Token]:
        if len(text) == 32 and policy.UUID4_HEX_RE.fullmatch(text):
            return [Token("uuid", text, text)]
        tokens: list[Token] = []
        literal_start = 0
        position = 0
        while True:
            best = None
            for name, pattern in self.scanners:
                match = pattern.search(text, position)
                if match and (
                    best is None
                    or match.start() < best[1].start()
                    or (match.start() == best[1].start() and match.end() > best[1].end())
                ):
                    best = (name, match)
            if best is None:
                break
            name, match = best
            token = self._token(name, match, side)
            if token is None:
                position = match.start() + 1
                continue
            if match.start() > literal_start:
                tokens.append(Token("lit", text[literal_start : match.start()]))
            tokens.append(token)
            literal_start = position = match.end()
        if literal_start < len(text):
            tokens.append(Token("lit", text[literal_start:]))
        merged: list[Token] = []
        for token in tokens:
            if token.kind == "lit" and merged and merged[-1].kind == "lit":
                merged[-1] = Token("lit", merged[-1].text + token.text)
            else:
                merged.append(token)
        return merged

    def _token(self, name: str, match: re.Match, side: str) -> Token | None:
        text = match.group(0)
        bound = self.backward if side == "cand" else self.forward
        if name == "uuid":
            return Token("uuid", text, text.lower())
        if name in ("anyuuid", "hex"):
            return Token(name, text, text.lower())
        if name == "datetime" and match.group(4) is None:
            # A bare date: generated only when it is the run's own day.
            if text in self.days[side] or text in bound.get("date", {}):
                return Token("date", text, text)
            return None
        parsed = _parse_datetime(match) if name == "datetime" else _parse_compact(match)
        if parsed is None:
            return None
        moment, shape = parsed
        key = (_instant(moment), shape)
        if _in_window(moment, self.windows[side]) or key in bound.get("time", {}):
            return Token("time", text, key, shape)
        if "clock" in self.relax and _whole_hours_from_window(moment, self.windows[side]):
            # Diagnostic: a reading shifted by whole hours, such as an expiry
            # of now plus 90 days.
            return Token("time", text, key, shape)
        return None

    def pin_text(self, text: str) -> None:
        """Mark identities that entered through the inputs as themselves."""
        for token in self.tokenize(text, "ref"):
            if token.kind == "lit":
                continue
            forward = self.forward.setdefault(token.kind, {})
            backward = self.backward.setdefault(token.kind, {})
            if token.key not in forward and token.key not in backward:
                forward[token.key] = token.key
                backward[token.key] = token.key
                self.texts["ref"][(token.kind, token.key)] = token.text
                self.texts["cand"][(token.kind, token.key)] = token.text
                self._insert_order(token.kind, token.key, token.key)

    def translate(self, text: str) -> str:
        """Reference input text as the candidate must receive it."""
        out = []
        for token in self.tokenize(text, "ref"):
            if token.kind == "lit":
                out.append(token.text)
                continue
            mapped = self.forward.get(token.kind, {}).get(token.key)
            if mapped is None or mapped == token.key:
                out.append(token.text)
            else:
                out.append(self.texts["cand"].get((token.kind, mapped), token.text))
        text = "".join(out)
        for ref_value, cand_value in self.forward.get("declared", {}).items():
            text = text.replace(ref_value, cand_value)
        return self.to_candidate_paths(text)

    def reference_text(self, kind: str, cand_key, fallback: str) -> str:
        """How the reference spelled the value a candidate key stands for."""
        ref_key = self.backward.get(kind, {}).get(cand_key)
        if ref_key is None:
            return fallback
        return self.texts["ref"].get((kind, ref_key), fallback)

    # Binding --------------------------------------------------------------

    def _insert_order(self, kind: str, ref_key, cand_key) -> None:
        group = _order_group(kind, ref_key)
        if group is None:
            return
        pairs = self.order.setdefault(group, [])
        bisect.insort(pairs, (_order_key(ref_key), _order_key(cand_key)))

    def _order_violation(self, kind: str, ref_key, cand_key) -> tuple | None:
        group = _order_group(kind, ref_key)
        if group is None:
            return None
        pairs = self.order.get(group, [])
        ref_order, cand_order = _order_key(ref_key), _order_key(cand_key)
        index = bisect.bisect_left(pairs, (ref_order, cand_order))
        if index > 0 and pairs[index - 1][1] >= cand_order:
            return pairs[index - 1]
        if index < len(pairs) and pairs[index][1] <= cand_order:
            return pairs[index]
        return None

    def bind(self, kind: str, ref_key, cand_key, where: str, ref_text: str, cand_text: str) -> Difference | None:
        forward = self.forward.setdefault(kind, {})
        backward = self.backward.setdefault(kind, {})
        if "clock" in self.relax and (kind in CLOCK_KINDS or kind.startswith("epoch")):
            return self._bind_reading(kind, ref_key, cand_key, where, ref_text, cand_text)
        if ref_key in forward:
            if forward[ref_key] != cand_key:
                return Difference(
                    where,
                    f"binding:{kind}",
                    "a generated value maps to a different candidate value than at its first occurrence",
                    ref_text,
                    cand_text,
                )
            return None
        if cand_key in backward:
            return Difference(
                where,
                f"binding:{kind}",
                "the candidate value already stands for a different reference value",
                ref_text,
                cand_text,
            )
        violation = self._order_violation(kind, ref_key, cand_key)
        if violation is not None:
            return Difference(where, f"order:{kind}", "generated values are ordered differently", ref_text, cand_text)
        forward[ref_key] = cand_key
        backward[cand_key] = ref_key
        self.texts["ref"][(kind, ref_key)] = ref_text
        self.texts["cand"][(kind, cand_key)] = cand_text
        self._insert_order(kind, ref_key, cand_key)
        return None

    def _bind_reading(self, kind, ref_key, cand_key, where, ref_text, cand_text) -> Difference | None:
        """A clock reading under the diagnostic: only its order must hold."""
        group = _order_group(kind, ref_key)
        ref_order, cand_order = _order_key(ref_key), _order_key(cand_key)
        for other_ref, other_cand in self.order.get(group, []):
            if (other_ref < ref_order and other_cand > cand_order) or (other_ref > ref_order and other_cand < cand_order):
                return Difference(where, f"order:{kind}", "clock readings are ordered differently", ref_text, cand_text)
        self.forward[kind].setdefault(ref_key, cand_key)
        self.texts["ref"][(kind, ref_key)] = ref_text
        self.texts["cand"][(kind, cand_key)] = cand_text
        bisect.insort(self.order.setdefault(group, []), (ref_order, cand_order))
        return None

    # Comparison -----------------------------------------------------------

    def canonical(self, value, side: str) -> str:
        """`value` with every mappable token blanked, for ordering ties."""
        if isinstance(value, str):
            return "".join(token.text if token.kind == "lit" else f"<{token.kind}>" for token in self.tokenize(value, side))
        if isinstance(value, dict):
            return "{" + ",".join(f"{json.dumps(key)}:{self.canonical(item, side)}" for key, item in value.items()) + "}"
        if isinstance(value, (list, tuple)):
            return "[" + ",".join(sorted(self.canonical(item, side) for item in value)) + "]"
        return json.dumps(value) if not isinstance(value, bytes) else value.hex()

    def compare_text(self, ref: str, cand: str, where: str, out: list[Difference]) -> None:
        cand = self._pids_to_reference(self.to_reference_paths(cand))
        if ref == cand and not policy.UUID4_RE.search(ref) and not any(ch.isdigit() for ch in ref):
            return
        if "order" in self.relax and ref[:1] in "[{" and cand[:1] in "[{":
            ref_doc, cand_doc = _json_or_none(ref), _json_or_none(cand)
            if ref_doc is not _NOT_JSON and cand_doc is not _NOT_JSON:
                self.compare_value(ref_doc, cand_doc, where, out)
                return
        ref_tokens = self.tokenize(ref, "ref")
        cand_tokens = self.tokenize(cand, "cand")
        if len(ref_tokens) != len(cand_tokens) or any(
            left.kind != right.kind or (left.kind == "lit" and left.text != right.text)
            for left, right in zip(ref_tokens, cand_tokens)
        ):
            out.append(Difference(where, "value", "text differs", _short(ref), _short(cand)))
            return
        for left, right in zip(ref_tokens, cand_tokens):
            if left.kind == "lit":
                continue
            if left.shape != right.shape:
                out.append(Difference(where, f"format:{left.kind}", "generated value written in another format", left.text, right.text))
                continue
            difference = self.bind(left.kind, left.key, right.key, where, left.text, right.text)
            if difference:
                out.append(difference)

    def compare_number(self, ref, cand, where: str, name: str | None, out: list[Difference]) -> None:
        if isinstance(ref, int) and ref in self.pids and cand == self.pids[ref]:
            return
        if name and policy.TIME_NAME_RE.search(name):
            for scale in (1, 1_000, 1_000_000, 1_000_000_000):
                if self.windows["ref"].contains_epoch(ref / scale) and self.windows["cand"].contains_epoch(cand / scale):
                    difference = self.bind(f"epoch{scale}", ref, cand, where, repr(ref), repr(cand))
                    if difference:
                        out.append(difference)
                    return
        if ref != cand:
            out.append(Difference(where, "value", "number differs", repr(ref), repr(cand)))

    def compare_value(self, ref, cand, where: str, out: list[Difference], name: str | None = None) -> None:
        if _json_type(ref) != _json_type(cand):
            out.append(Difference(where, "type", "value type differs", _short(json.dumps(ref)), _short(json.dumps(cand))))
            return
        if isinstance(ref, dict):
            ref_keys, cand_keys = list(ref), list(cand)
            if ref_keys != cand_keys and len(ref_keys) == len(cand_keys):
                # Keys can be generated ids themselves: map them in order.
                if "order" in self.relax:
                    ref_keys = sorted(ref_keys, key=lambda key: self.canonical(key, "ref"))
                    cand_keys = sorted(cand_keys, key=lambda key: self.canonical(key, "cand"))
                key_differences: list[Difference] = []
                for left, right in zip(ref_keys, cand_keys):
                    if left != right:
                        self.compare_text(left, right, f"{where}(key {left})", key_differences)
                if not key_differences:
                    for left, right in zip(ref_keys, cand_keys):
                        self.compare_value(ref[left], cand[right], f"{where}.{left}", out, left)
                    return
            if ref_keys != cand_keys:
                missing = [key for key in ref_keys if key not in cand]
                extra = [key for key in cand_keys if key not in ref]
                message = "keys differ" if missing or extra else "key order differs"
                out.append(Difference(where, "keys", message, _short(ref_keys), _short(cand_keys)))
            declared = _declared_keys(ref, cand)
            for key in ref_keys:
                if key not in cand:
                    continue
                if key in declared:
                    difference = self.bind("declared", ref[key], cand[key], f"{where}.{key}", ref[key], cand[key])
                    if difference:
                        out.append(difference)
                    continue
                self.compare_value(ref[key], cand[key], f"{where}.{key}", out, key)
        elif isinstance(ref, list):
            if len(ref) != len(cand):
                out.append(Difference(where, "length", "array length differs", str(len(ref)), str(len(cand))))
            if "order" in self.relax:
                ref = sorted(ref, key=lambda item: self.canonical(item, "ref"))
                cand = sorted(cand, key=lambda item: self.canonical(item, "cand"))
            for index, (left, right) in enumerate(zip(ref, cand)):
                self.compare_value(left, right, f"{where}[{index}]", out, name)
        elif isinstance(ref, str):
            self.compare_text(ref, cand, where, out)
        elif isinstance(ref, (int, float)) and not isinstance(ref, bool):
            self.compare_number(ref, cand, where, name, out)
        elif ref != cand:
            out.append(Difference(where, "value", "value differs", repr(ref), repr(cand)))

    def compare_stream(self, ref: bytes, cand: bytes, where: str, out: list[Difference], *, lines: bool = False) -> None:
        """Compare process output or a file's text."""
        try:
            ref_text, cand_text = ref.decode("utf-8"), cand.decode("utf-8")
        except UnicodeDecodeError:
            if ref != cand:
                out.append(Difference(where, "bytes", "binary content differs", _digest(ref), _digest(cand)))
            return
        if not lines:
            ref_doc, cand_doc = _json_or_none(ref_text), _json_or_none(cand_text)
            if ref_doc is not _NOT_JSON or cand_doc is not _NOT_JSON:
                if ref_doc is _NOT_JSON or cand_doc is _NOT_JSON:
                    out.append(Difference(where, "format", "only one side is a JSON document", _short(ref_text), _short(cand_text)))
                    return
                self._compare_layout(ref_text, cand_text, where, out)
                self.compare_value(ref_doc, cand_doc, where, out)
                return
        ref_lines, cand_lines = ref_text.splitlines(keepends=True), cand_text.splitlines(keepends=True)
        if len(ref_lines) != len(cand_lines):
            out.append(Difference(where, "length", "line count differs", str(len(ref_lines)), str(len(cand_lines))))
        for index, (left, right) in enumerate(zip(ref_lines, cand_lines)):
            self.compare_line(left, right, f"{where}#{index + 1}", out)

    def compare_line(self, ref: str, cand: str, where: str, out: list[Difference]) -> None:
        ref_doc, cand_doc = _json_or_none(ref), _json_or_none(cand)
        if ref_doc is not _NOT_JSON and cand_doc is not _NOT_JSON:
            self._compare_layout(ref, cand, where, out)
            self.compare_value(ref_doc, cand_doc, where, out)
        else:
            self.compare_text(ref, cand, where, out)

    def _compare_layout(self, ref: str, cand: str, where: str, out: list[Difference]) -> None:
        """Whitespace, punctuation, ASCII escaping, and the trailing newline."""
        ref_layout, cand_layout = _layout(ref), _layout(cand)
        if ref_layout != cand_layout:
            out.append(Difference(where, "layout", "JSON layout differs", _short(ref_layout), _short(cand_layout)))
        if ref.isascii() != cand.isascii():
            out.append(Difference(where, "layout", "non-ASCII escaping differs"))


def _declared_keys(ref: dict, cand: dict) -> set[str]:
    kind = ref.get("kind")
    if not isinstance(kind, str) or kind != cand.get("kind"):
        return set()
    return {
        key
        for prefix, key in policy.DECLARED_IDENTITIES
        if kind.startswith(prefix) and isinstance(ref.get(key), str) and isinstance(cand.get(key), str)
    }


def _order_group(kind: str, ref_key):
    """Values whose order must survive the mapping: one clock, one spelling."""
    if kind == "time":
        return ("time", ref_key[1])
    if kind == "date" or kind.startswith("epoch"):
        return (kind,)
    return None


def _order_key(key):
    return key[0] if isinstance(key, tuple) else key


_NOT_JSON = object()
_JSON_SCALAR = re.compile(r'"(?:[^"\\]|\\.)*"|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?|true|false|null')


def _json_or_none(text: str):
    stripped = text.strip()
    if not stripped or stripped[0] not in "{[":
        return _NOT_JSON
    try:
        return json.loads(stripped)
    except json.JSONDecodeError:
        return _NOT_JSON


def _layout(text: str) -> str:
    return _JSON_SCALAR.sub(lambda match: '"' if match.group(0).startswith('"') else "0", text)


def _json_type(value) -> str:
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "bool"
    if isinstance(value, int):
        return "int"
    if isinstance(value, float):
        return "float"
    return type(value).__name__


def _digest(data: bytes) -> str:
    return f"{len(data)} bytes sha256:{hashlib.sha256(data).hexdigest()[:16]}"


# Databases -------------------------------------------------------------------

_TYPE_RANK = {type(None): 0, int: 1, float: 1, str: 2, bytes: 3}


def _sqlite_order(value):
    return (_TYPE_RANK.get(type(value), 4), value if not isinstance(value, str) else value.encode("utf-8"))


def compare_dumps(ref: dbdump.Dump, cand: dbdump.Dump, mapper: Mapper, where: str, out: list[Difference]) -> None:
    for name, value in ref.pragmas.items():
        if cand.pragmas.get(name) != value:
            out.append(Difference(f"{where}/pragma {name}", "schema", "pragma differs", repr(value), repr(cand.pragmas.get(name))))
    if ref.schema != cand.schema:
        ref_objects = {(row[0], row[1]): row for row in ref.schema}
        cand_objects = {(row[0], row[1]): row for row in cand.schema}
        for key in sorted(set(ref_objects) | set(cand_objects)):
            if ref_objects.get(key) != cand_objects.get(key):
                out.append(
                    Difference(
                        f"{where}/schema {key[0]} {key[1]}",
                        "schema",
                        "schema object differs",
                        _short(ref_objects.get(key)),
                        _short(cand_objects.get(key)),
                    )
                )
    tables = [name for name in ref.tables if name in cand.tables]
    # Generated primary keys bind in insertion order before any row compares,
    # so references from other tables resolve whatever order rows sort in.
    for name in tables:
        _bind_generated_keys(ref.tables[name], cand.tables[name], mapper, f"{where}/{name}", out)
    for name in tables:
        _compare_table(ref.tables[name], cand.tables[name], mapper, f"{where}/{name}", out)


def _bind_generated_keys(ref: dbdump.Table, cand: dbdump.Table, mapper: Mapper, where: str, out) -> None:
    if not ref.has_rowid or not ref.pk or ref.columns != cand.columns:
        return
    positions = [ref.columns.index(column) for column in ref.pk]
    if "order" in mapper.relax:
        ref_rows = sorted(ref.rows, key=lambda row: mapper.canonical(row[1], "ref"))
        cand_rows = sorted(cand.rows, key=lambda row: mapper.canonical(row[1], "cand"))
    else:
        ref_rows = sorted(ref.rows, key=lambda row: row[0])
        cand_rows = sorted(cand.rows, key=lambda row: row[0])
    for (_, left), (_, right) in zip(ref_rows, cand_rows):
        for position in positions:
            ref_value, cand_value = left[position], right[position]
            if not isinstance(ref_value, str) or not isinstance(cand_value, str):
                continue
            ref_tokens = mapper.tokenize(ref_value, "ref")
            cand_tokens = mapper.tokenize(mapper.to_reference_paths(cand_value), "cand")
            # Keys such as `observed:<id>` bind part by part when the rest agrees.
            if len(ref_tokens) != len(cand_tokens) or any(
                token.kind != other.kind or (token.kind == "lit" and token.text != other.text)
                for token, other in zip(ref_tokens, cand_tokens)
            ):
                continue
            for token, other in zip(ref_tokens, cand_tokens):
                if token.kind == "lit":
                    continue
                if token.key in mapper.forward.get(token.kind, {}) or other.key in mapper.backward.get(token.kind, {}):
                    continue
                mapper.bind(token.kind, token.key, other.key, where, token.text, other.text)


def _candidate_key(row: tuple, positions: list[int], mapper: Mapper) -> tuple:
    key = []
    for position in positions:
        value = row[position]
        if isinstance(value, str):
            value = mapper.to_reference_paths(value)
            parts = []
            for token in mapper.tokenize(value, "cand"):
                if token.kind == "lit":
                    parts.append(token.text)
                    continue
                parts.append(mapper.reference_text(token.kind, token.key, token.text))
            value = "".join(parts)
        key.append(_sqlite_order(value))
    return tuple(key)


def _compare_table(ref: dbdump.Table, cand: dbdump.Table, mapper: Mapper, where: str, out: list[Difference]) -> None:
    if ref.columns != cand.columns:
        out.append(Difference(where, "schema", "columns differ", _short(ref.columns), _short(cand.columns)))
        return
    if len(ref.rows) != len(cand.rows):
        out.append(Difference(where, "rows", "row count differs", str(len(ref.rows)), str(len(cand.rows))))
    if ref.pk:
        positions = [ref.columns.index(column) for column in ref.pk]
        ref_rows = [(tuple(_sqlite_order(row[1][p]) for p in positions), row) for row in ref.rows]
        cand_rows = sorted(((_candidate_key(row[1], positions, mapper), row) for row in cand.rows), key=lambda item: item[0])
        pairs, missing, extra = _merge(ref_rows, cand_rows)
    else:
        pairs = list(zip(sorted(ref.rows), sorted(cand.rows)))
        missing = [row for row in sorted(ref.rows)[len(pairs):]]
        extra = [row for row in sorted(cand.rows)[len(pairs):]]
    for row in missing[:5]:
        out.append(Difference(f"{where}{_row_label(ref, row)}", "rows", "row missing from the candidate", _short(row[1])))
    for row in extra[:5]:
        out.append(Difference(f"{where}{_row_label(cand, row)}", "rows", "row only in the candidate", None, _short(row[1])))
    for left, right in pairs:
        label = f"{where}{_row_label(ref, left)}"
        for index, column in enumerate(ref.columns):
            _compare_cell(left[1][index], right[1][index], f"{label}.{column}", column, mapper, out)


def _merge(ref_rows, cand_rows):
    pairs, missing, extra = [], [], []
    ref_sorted = sorted(ref_rows, key=lambda item: item[0])
    cand_map: dict[tuple, list] = {}
    for key, row in cand_rows:
        cand_map.setdefault(key, []).append(row)
    for key, row in ref_sorted:
        bucket = cand_map.get(key)
        if bucket:
            pairs.append((row, bucket.pop(0)))
        else:
            missing.append(row)
    for bucket in cand_map.values():
        extra.extend(bucket)
    return pairs, missing, extra


def _row_label(table: dbdump.Table, row) -> str:
    if table.pk:
        values = [row[1][table.columns.index(column)] for column in table.pk]
        return f"[{', '.join(_short(value, 80) for value in values)}]"
    return f"[rowid {row[0]}]"


def _compare_cell(ref, cand, where: str, column: str, mapper: Mapper, out: list[Difference]) -> None:
    if type(ref) is not type(cand):
        out.append(Difference(where, "type", "stored type differs", _short(ref), _short(cand)))
        return
    if isinstance(ref, str):
        mapper.compare_text(ref, cand, where, out)
    elif isinstance(ref, (int, float)):
        mapper.compare_number(ref, cand, where, column, out)
    elif ref != cand:
        out.append(Difference(where, "value", "stored value differs", _short(ref), _short(cand)))


# Files and trees -------------------------------------------------------------

def compare_file(ref: bytes, cand: bytes, where: str, mapper: Mapper, out: list[Difference], *, side_files=None) -> None:
    ref_db, cand_db = dbdump.is_sqlite(ref), dbdump.is_sqlite(cand)
    if mapper.database_keys and where.endswith(dbdump.DATABASE_SUFFIXES) and ref and cand:
        # Encrypted books carry no header; the recorded passphrases open them.
        ref_db = cand_db = True
    if ref_db or cand_db:
        if not (ref_db and cand_db):
            out.append(Difference(where, "format", "only one side is a SQLite database"))
            return
        ref_side, cand_side = side_files or ({}, {})
        try:
            ref_dump = dbdump.dump_files(ref, ref_side, keys=mapper.database_keys)
            cand_dump = dbdump.dump_files(cand, cand_side, keys=mapper.database_keys)
        except dbdump.UnreadableDatabase as exc:
            out.append(Difference(where, "error", f"database could not be read: {exc}"))
            return
        compare_dumps(ref_dump, cand_dump, mapper, where, out)
        return
    if ref.startswith(b"PK\x03\x04") and cand.startswith(b"PK\x03\x04"):
        try:
            _compare_zip(ref, cand, where, mapper, out)
            return
        except zipfile.BadZipFile:
            pass
    if "clock" in mapper.relax and ref.startswith(b"%PDF") and cand.startswith(b"%PDF"):
        # Diagnostic: the creation stamps and the document id derived from them.
        ref, cand = _PDF_CLOCK.sub(b"", ref), _PDF_CLOCK.sub(b"", cand)
    if b"\x00" in ref or b"\x00" in cand or ref.startswith(b"%PDF"):
        if ref != cand:
            out.append(Difference(where, "bytes", "binary content differs", _digest(ref), _digest(cand)))
        return
    mapper.compare_stream(ref, cand, where, out)


_PDF_CLOCK = re.compile(rb"/(?:CreationDate|ModDate) \(D:[^)]*\)|/ID\s*\[\s*<[0-9A-Fa-f]*>\s*<[0-9A-Fa-f]*>\s*\]")


def _compare_zip(ref: bytes, cand: bytes, where: str, mapper: Mapper, out: list[Difference]) -> None:
    with zipfile.ZipFile(io.BytesIO(ref)) as left, zipfile.ZipFile(io.BytesIO(cand)) as right:
        ref_members, cand_members = left.infolist(), right.infolist()
        ref_names = [info.filename for info in ref_members]
        cand_names = [mapper.to_reference_paths(info.filename) for info in cand_members]
        if len(ref_names) != len(cand_names):
            out.append(Difference(where, "length", "archive member count differs", _short(ref_names), _short(cand_names)))
        for ref_info, cand_info in zip(ref_members, cand_members):
            member = f"{where}!{ref_info.filename}"
            mapper.compare_text(ref_info.filename, cand_info.filename, f"{member}(name)", out)
            stamp = "%04d-%02d-%02dT%02d:%02d:%02d"
            mapper.compare_text(stamp % ref_info.date_time, stamp % cand_info.date_time, f"{member}(date_time)", out)
            if ref_info.compress_type != cand_info.compress_type:
                out.append(Difference(member, "value", "compression differs", str(ref_info.compress_type), str(cand_info.compress_type)))
            compare_file(left.read(ref_info), right.read(cand_info), member, mapper, out)


def _shape(path: str, mapper: Mapper, side: str) -> str:
    return "".join(token.text if token.kind == "lit" else f"<{token.kind}>" for token in mapper.tokenize(path, side))


def compare_tree(ref_tree: dict, read_ref, cand_tree: dict, read_cand, mapper: Mapper, where: str, out: list[Difference]) -> None:
    """Compare two captured trees; `read_*` return a file's bytes by path."""
    if ref_tree.get("exists") != cand_tree.get("exists"):
        out.append(Difference(where, "missing" if ref_tree.get("exists") else "extra", "root existence differs",
                              str(ref_tree.get("exists")), str(cand_tree.get("exists"))))
        return
    if not ref_tree.get("exists"):
        return
    if ref_tree.get("mode") != cand_tree.get("mode"):
        out.append(Difference(where, "mode", "root mode differs", oct(ref_tree.get("mode") or 0), oct(cand_tree.get("mode") or 0)))
    ref_entries, cand_entries = ref_tree["entries"], cand_tree["entries"]
    pairs = _align_paths(list(ref_entries), list(cand_entries), mapper, where, out)
    side_suffixes = dbdump.SIDE_SUFFIXES + ("-shm",)
    for ref_path, cand_path in pairs:
        if ref_path.endswith(side_suffixes) and ref_path.rsplit("-", 1)[0] in ref_entries:
            continue
        left, right = ref_entries[ref_path], cand_entries[cand_path]
        label = f"{where}/{ref_path}"
        if left["type"] != right["type"]:
            out.append(Difference(label, "type", "entry type differs", left["type"], right["type"]))
            continue
        if left.get("mode") != right.get("mode"):
            out.append(Difference(label, "mode", "permissions differ", oct(left.get("mode") or 0), oct(right.get("mode") or 0)))
        if left["type"] == "symlink":
            mapper.compare_text(left["target"], right["target"], f"{label}(link)", out)
        elif left["type"] == "file":
            ref_bytes, cand_bytes = read_ref(ref_path), read_cand(cand_path)
            if ref_bytes == cand_bytes and not ref_path.endswith(dbdump.DATABASE_SUFFIXES):
                # Identical bytes: still pin what they contain for later rows.
                if b"\x00" not in ref_bytes:
                    try:
                        mapper.compare_text(ref_bytes.decode("utf-8"), cand_bytes.decode("utf-8"), label, out)
                    except UnicodeDecodeError:
                        pass
                continue
            sides = None
            if ref_bytes.startswith(dbdump.SQLITE_HEADER) or ref_path.endswith(dbdump.DATABASE_SUFFIXES):
                sides = (
                    {suffix: read_ref(ref_path + suffix) for suffix in dbdump.SIDE_SUFFIXES if ref_path + suffix in ref_entries},
                    {suffix: read_cand(cand_path + suffix) for suffix in dbdump.SIDE_SUFFIXES if cand_path + suffix in cand_entries},
                )
            compare_file(ref_bytes, cand_bytes, label, mapper, out, side_files=sides)


def _align_paths(ref_paths, cand_paths, mapper: Mapper, where: str, out: list[Difference]) -> list[tuple[str, str]]:
    translated = {}
    for path in cand_paths:
        text = mapper.to_reference_paths(path)
        translated[path] = "".join(
            token.text if token.kind == "lit" else mapper.reference_text(token.kind, token.key, token.text)
            for token in mapper.tokenize(text, "cand")
        )
    by_translation = {}
    for path, text in translated.items():
        by_translation.setdefault(text, []).append(path)
    pairs, unmatched_ref = [], []
    for path in ref_paths:
        bucket = by_translation.get(path)
        if bucket:
            pairs.append((path, bucket.pop(0)))
        else:
            unmatched_ref.append(path)
    unmatched_cand = [path for bucket in by_translation.values() for path in bucket]
    # Names with unbound generated parts pair by shape, in sorted order.
    ref_by_shape, cand_by_shape = {}, {}
    for path in unmatched_ref:
        ref_by_shape.setdefault(_shape(path, mapper, "ref"), []).append(path)
    for path in unmatched_cand:
        cand_by_shape.setdefault(_shape(mapper.to_reference_paths(path), mapper, "cand"), []).append(path)
    for shape in sorted(set(ref_by_shape) | set(cand_by_shape)):
        lefts, rights = sorted(ref_by_shape.get(shape, [])), sorted(cand_by_shape.get(shape, []))
        for left, right in zip(lefts, rights):
            before = len(out)
            mapper.compare_text(left, mapper.to_reference_paths(right), f"{where}/{left}(name)", out)
            if len(out) == before:
                pairs.append((left, right))
        for left in lefts[len(rights):]:
            out.append(Difference(f"{where}/{left}", "missing", "entry missing from the candidate"))
        for right in rights[len(lefts):]:
            out.append(Difference(f"{where}/{right}", "extra", "entry only in the candidate"))
    return sorted(pairs)
