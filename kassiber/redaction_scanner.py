"""Forward-only credential scanners, mirrored by the webview secretScanner.ts."""

from __future__ import annotations

import re
from functools import lru_cache
from bisect import bisect_right
from heapq import merge


_IDENTIFIER = re.compile(r"[A-Za-z0-9_.-]+")
_SEPARATORS = re.compile(r"[^A-Za-z0-9]+")
_KEY_PARTS = re.compile(r"(?<=[a-z0-9])(?=[A-Z])|(?<=[A-Z])(?=[A-Z][a-z])|(?<=[0-9])(?=[A-Za-z])")
_VERSION = re.compile(r"[_-]v[0-9]+_*$", re.IGNORECASE)
# Secret words match identifier words. A part that does not segment into known
# words is secret when it contains one outside a listed ordinary word, so
# `prune`, `dnsseed`, `bypass`, `passed` and `tokenizer` remain vocabulary.
_SECRET_WORDS = frozenset("""authorization bearer blinding cookie cookies credential credentials descriptor
macaroon mfa mnemonic otp pass passcode passphrase passwd password pin pw pwd rpcauth rune secret seed sessid
token totp twofa wif xprv""".split())
# Short names count only as a whole identifier part (`user_id`, not `author`
# or `sighash`); the shortest only as the final word (`db_pw`, not `pw_hash`),
# though a version or encoding modifier may follow (`pwold`, `pin_new`).
_PART_NAMES = frozenset("auth jwt session sid sig sign signature user".split())
_LAST_NAMES = frozenset("mfa otp pin pw sign totp twofa".split())
_MODIFIERS = frozenset("backup confirm copy enc encoded hex new old plain raw repeat value".split())
_WEAK_NAMES = frozenset(("pass", "user", "auth", "mfa", "twofa"))
# Adjacent words that name a secret together, spelled apart or concatenated.
_COMPOUNDS = {(first, second): first + second for first, second in (
    ("access", "key"), ("api", "key"), ("api", "secret"), ("api", "token"), ("auth", "code"), ("auth", "header"),
    ("auth", "key"), ("backup", "code"), ("blinding", "key"), ("encryption", "key"), ("hmac", "key"),
    ("master", "key"), ("mfa", "code"), ("otp", "code"), ("pin", "code"), ("priv", "key"), ("private", "key"),
    ("recovery", "code"), ("recovery", "phrase"), ("scan", "key"), ("scan", "private"), ("secret", "key"),
    ("seed", "phrase"), ("seed", "words"), ("session", "id"), ("session", "key"), ("signing", "key"),
    ("silent", "payment"), ("silent", "payments"), ("sp", "descriptor"), ("sp", "scan"), ("sp", "spend"),
    ("spend", "key"), ("subscription", "key"), ("totp", "code"), ("twofa", "code"), ("verification", "code"))}
_COMPOUND_WORDS = frozenset(_COMPOUNDS.values())
_METADATA = frozenset("disabled enabled fd file hint path statistics stdin".split())
_COUNT_WORDS = frozenset("count max min limit length len size ttl".split())
# Qualifiers let concatenated identifiers segment into words: `rpcpassword`,
# `dbprivkeyraw`, `masterseedwords`. Unknown pieces keep a part unsegmented.
_QUALIFIERS = frozenset("""access account admin api app aws b base bitcoin client code data db default dev
device electrum enc encoded encryption env github hash header hex id key keys lnd local login main master mfa
my mysql new old payment payments pem pg phrase plain priv private prod raw recovery refresh remote root rpc
scan server service signing silent sp spend str test text total twofa v value verification wallet web words
x azure binance bitfinex bitpanda bitstamp blockstream btcpay cln coinbase core discord electrs fulcrum gcp
gitlab google kassiber kraken lightning mempool mongo nostr postgres redis slack smtp strike stripe telegram
umbrel""".split()) | _COUNT_WORDS | _METADATA
_PLURALS = frozenset(word + "s" for word in _SECRET_WORDS | _PART_NAMES | _COMPOUND_WORDS if not word.endswith("s"))
_VOCAB = _SECRET_WORDS | _PART_NAMES | _QUALIFIERS | _COMPOUND_WORDS | _PLURALS
_SIZES = sorted({len(word) for word in _VOCAB}, reverse=True)
# Fail closed: an unknown prefix or suffix keeps a secret word secret
# (`krakenpassword`, `btcpass`, `passwordbackup`), unless the word sits inside
# one of these ordinary words (`tokenizer`, `bypass`, `prune`, `spinner`).
_ORDINARY = frozenset("""bypass compass dnsseed encompass overpass passable passage passed passenger passerby
passing passive passover passport passthrough passwordless prune secretar seeded seeder seednode spin surpass
swift tokeniz trespass underpass wifi""".split())
_EMBEDDED = sorted(_SECRET_WORDS | _COMPOUND_WORDS, key=lambda word: (-len(word), word))
_CONTAINERS = {word: tuple((ordinary, offset) for ordinary in sorted(_ORDINARY)
                           for offset in range(len(ordinary) - len(word) + 1) if ordinary.startswith(word, offset))
               for word in _EMBEDDED}
_NAMED = _SECRET_WORDS | _COMPOUND_WORDS | _PLURALS
_TWOFA = re.compile(r"2fa", re.IGNORECASE | re.ASCII)
_HINT = re.compile("|".join(sorted(_SECRET_WORDS | _PART_NAMES | {"2fa", "code", "key", "phrase", "recovery", "scan", "silent", "spend"})), re.IGNORECASE | re.ASCII)


@lru_cache(maxsize=4096)
def _segment(part: str) -> tuple[str, ...]:
    if part in _VOCAB:
        return (part,)
    if len(part) > 40:
        return _embedded_segment(part)
    # Fewest words; ties keep the longest leading word.
    best: list[tuple[int, int] | None] = [None] * len(part) + [(0, len(part))]
    for start in range(len(part) - 1, -1, -1):
        for size in _SIZES:
            end = start + size
            following = best[end] if end <= len(part) else None
            if following is not None and part[start:end] in _VOCAB and (best[start] is None or following[0] + 1 < best[start][0]):
                best[start] = (following[0] + 1, end)
    if best[0] is None:
        return _embedded_segment(part)
    words = []
    start = 0
    while start < len(part):
        end = best[start][1]
        words.append(part[start:end])
        start = end
    return tuple(words)


def _known_prefix(part: str, end: int) -> bool:
    # An ordinary word counts only at the start or after known words:
    # `pruneheight` and `mytokenizer`, not `ftprune` (`ftp` + `rune`).
    return not end or (end <= 40 and all(piece in _VOCAB for piece in _segment(part[:end])))


def _embedded_segment(part: str) -> tuple[str, ...]:
    """Split an unknown part around its last secret word outside ordinary words."""
    for word in _EMBEDDED:
        start = part.rfind(word)
        while start >= 0 and any(part.startswith(ordinary, start - offset) and _known_prefix(part, start - offset)
                                 for ordinary, offset in _CONTAINERS[word] if start >= offset):
            start = part.rfind(word, 0, start + len(word) - 1)
        if start < 0:
            continue
        rest = part[start + len(word):]
        head = (part[:start],) if start else ()
        if rest == "s":
            return head + (word + "s",)
        tail = (_segment(rest) if len(rest) <= 40 else (rest,)) if rest else ()
        if word in _LAST_NAMES and not all(piece in _MODIFIERS for piece in tail):
            # `rootpwdconfirm` names `pwd`, not a non-final `otp`.
            continue
        return head + (word,) + tail
    return (part,)


@lru_cache(maxsize=4096)
def _key_readings(token: str) -> tuple[tuple[tuple[str, bool], ...], ...]:
    """Lowercase words of an identifier, each marked when it is a whole part.

    Case runs are hints, not boundaries: `rpcSECRETkeyOLD` and `newPWDb64`
    split inside a word, so a chunk is also read concatenated when that
    reading segments into known words. Either reading can name a secret.
    """
    token = _TWOFA.sub("twofa", _VERSION.sub("", token).strip("_.-"))
    split: list[tuple[str, bool]] = []
    joined: list[tuple[str, bool]] = []
    for chunk in _SEPARATORS.split(token):
        parts = [part for part in (raw.lower().strip("0123456789") for raw in _KEY_PARTS.split(chunk)) if part]
        pieces = []
        for part in parts:
            segmented = _segment(part)
            pieces.extend((piece, len(segmented) == 1) for piece in segmented)
        split.extend(pieces)
        segmented = _segment("".join(parts)) if len(parts) > 1 else ()
        if segmented and (all(piece in _VOCAB for piece in segmented) or any(piece in _NAMED for piece in segmented)):
            joined.extend((piece, len(segmented) == 1) for piece in segmented)
        else:
            joined.extend(pieces)
    return (tuple(split),) if joined == split else (tuple(split), tuple(joined))


def _reading_names(words: tuple[tuple[str, bool], ...]) -> list[str]:
    if not words or words[-1][0] in _METADATA:
        return []
    names = []
    # Whether only modifiers follow each word, so a final name may carry one.
    final = [True] * len(words)
    for index in range(len(words) - 2, -1, -1):
        final[index] = final[index + 1] and words[index + 1][0] in _MODIFIERS
    for index, (word, whole) in enumerate(words):
        if index + 1 < len(words):
            second = words[index + 1][0]
            pair = _COMPOUNDS.get((word, second)) or (_COMPOUNDS.get((word, second[:-1])) if second.endswith("s") else None)
            if pair:
                names.append(pair)
        base = word[:-1] if word in _PLURALS and word not in _SECRET_WORDS else word
        if base in _COMPOUND_WORDS:
            names.append(base)
            continue
        if base in ("user", "auth") and base != word:
            continue
        if base in _PART_NAMES and not whole:
            continue
        if base in _LAST_NAMES and not final[index]:
            continue
        if base in _SECRET_WORDS or base in _PART_NAMES:
            names.append(base)
    return names


def _secret_name(token: str) -> str:
    if not _HINT.search(token):
        return ""
    names = [name for words in _key_readings(token) for name in _reading_names(words)]
    return min(names, key=lambda name: (-len(name), name)) if names else ""


_COUNT_VALUE = re.compile(r"[0-9]{1,9}(?:\.[0-9]{1,9})?")
_LITERAL = re.compile(r"-?[0-9]+(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?(?![A-Za-z0-9_.])|(?:null|true|false|None|True|False)(?![A-Za-z0-9_.])")


def _count_name(key: str, name: str) -> bool:
    # A count word or the name's plural, whole or compound (`descriptors`,
    # `disable_private_keys`).
    plural = name + "s"
    for words in _key_readings(key):
        for index, (word, _) in enumerate(words):
            if word in _COUNT_WORDS or (word == plural and word not in _SECRET_WORDS) \
                    or (index and words[index - 1][0] + word == plural):
                return True
    return False


# The delimiter establishes a URL context; spelling the scheme cannot exempt
# userinfo. The first alternative only preserves the readable scheme prefix.
_SCHEME = re.compile(r"(?<![A-Za-z0-9+.-])[A-Za-z0-9+.-]+://|://|//")
_SPACE = " \t\r\n\v\f\u0085\u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000\ufeff"
_LINES = "\r\n\u0085\u2028\u2029"
# XML and tag whitespace; both runtimes strip exactly these characters.
_XML_SPACE = " \t\r\n\v\f"
_COMMAND_LINE = re.compile(r"(?:^|(?<=[\r\n\u0085\u2028\u2029]))[ \t]*(?:[$>][ \t]+)?(?:cmd|kassiber|bitcoind|bitcoin-cli|electrs|lncli|lightning-cli|curl|python|node|env|sudo|docker)(?=[ \t]|$)", re.ASCII)
_Span = tuple[int, int, str]


def _quote_at(text: str, pos: int) -> tuple[str, int]:
    end = pos
    while end < len(text) and text[end] == "\\":
        end += 1
    if end < len(text) and text[end] in "\"'":
        return text[pos:end + 1], end + 1
    return "", pos


def _quoted_end(text: str, pos: int, delimiter: str, limit: int | None = None) -> tuple[int, bool]:
    slashes = 0
    slash_count = len(delimiter) - 1
    period = 2 * (slash_count + (2 if slash_count and slash_count % 2 == 0 else 1))
    first_newline = -1
    limit = len(text) if limit is None else limit
    while pos < limit:
        ch = text[pos]
        if ch in _LINES and first_newline < 0:
            first_newline = pos
        if ch == delimiter[-1] and slashes % period == len(delimiter) - 1:
            return pos + 1, True
        slashes = slashes + 1 if ch == "\\" else 0
        pos += 1
    return _yaml_end(text, first_newline if first_newline >= 0 else pos), False


def _value_boundary(text: str, pos: int) -> bool:
    while pos < len(text):
        if text[pos] in ")]}":
            pos += 1
        else:
            delimiter, body = _quote_at(text, pos)
            if not delimiter:
                break
            pos = body
    if pos == len(text) or text[pos] in _SPACE:
        return True
    if text[pos] in ",;":
        pos += 1
        gap = pos
        while pos < len(text) and text[pos] in _SPACE and text[pos] not in _LINES:
            pos += 1
        if pos == len(text) or text[pos] in _LINES:
            return True
        delimiter, body = _quote_at(text, pos)
        key = _IDENTIFIER.match(text, body) if delimiter else None
        if key and text.startswith(delimiter, key.end()):
            tail = key.end() + len(delimiter)
            while tail < len(text) and text[tail] in _SPACE:
                tail += 1
            return tail < len(text) and text[tail] in ":="
        prose = _IDENTIFIER.match(text, pos) if pos > gap and not delimiter else None
        if prose and re.fullmatch(r"[a-z]+", prose.group()):
            return prose.end() == len(text) or text[prose.end()] in _SPACE + ".!?"
    return False


def _record_opener(view: str, original: str, offsets: list[int] | range, quote: int) -> bool:
    if quote < 0 or original[offsets[quote]:offsets[quote + 1]] != view[quote]:
        return False
    pos = quote - 1
    while pos >= 0 and view[pos] == "\\":
        pos -= 1
    while pos >= 0 and view[pos] in _SPACE:
        pos -= 1
    if pos < 0 or view[pos] not in ":=":
        return False
    pos -= 1
    while pos >= 0 and view[pos] in _SPACE:
        pos -= 1
    if pos < 0 or view[pos] not in "\"'" or original[offsets[pos]:offsets[pos + 1]] != view[pos]:
        return False
    delimiter = view[pos]
    pos -= 1
    while pos >= 0 and view[pos] == "\\":
        pos -= 1
    end = pos
    while pos >= 0 and (view[pos] in "_.-" or (view[pos].isascii() and view[pos].isalnum())):
        pos -= 1
    return pos < end and pos >= 0 and view[pos] == delimiter and original[offsets[pos]:offsets[pos + 1]] == delimiter


def _url_record_end(view: str, original: str, offsets: list[int] | range, start: int, end: int, record: bool) -> int:
    if not record:
        return end
    quote = re.search(r"[\\\"']", view[start:end])
    if not quote:
        return end
    pos = start + quote.start()
    delimiter, body = _quote_at(view, pos)
    if not delimiter or original[offsets[pos]:offsets[body]] != delimiter or body >= end or view[body] not in ",;" or not _value_boundary(view, body):
        return end
    prefix = view[start:pos]
    for candidate in (prefix, prefix.rstrip(").>]")):
        if _clear_host(candidate) or _public_url_prefix(candidate, original, offsets, start):
            return pos
    return end


def _url_heads(text: str):
    # Preserve context even when a keyed pass masks a URL's original @.
    view, offsets = _url_view(text)
    matches = list(_SCHEME.finditer(view))
    final_at = view.rfind("@")
    quotes = (m.start() for m in re.finditer(r"[\"']", view))
    quote = next(quotes, None)
    previous_quote = checked_quote = -1
    record = False
    for index, match in enumerate(matches):
        while quote is not None and quote < match.start():
            previous_quote, quote = quote, next(quotes, None)
        if previous_quote != checked_quote:
            record = _record_opener(view, text, offsets, previous_quote)
            checked_quote = previous_quote
        start = match.end()
        segment_end = matches[index + 1].start() if index + 1 < len(matches) else len(view)
        limit = _url_record_end(view, text, offsets, start, segment_end, record)
        limit = _url_candidate_end(view, text, offsets, start, _url_line_end(view, text, offsets, start, limit))
        at = view.rfind("@", start, limit)
        credentials = at >= 0 or (limit == segment_end and index + 1 < len(matches) and final_at >= limit)
        if credentials:
            end = at + 1 if at >= 0 else limit
        else:
            end = start
            while end < limit and view[end] not in "\"'<>[]/{}()?#&|" and not (view[end] in _SPACE and text[offsets[end]:offsets[end + 1]] == view[end]):
                end += 1
        yield offsets[start], offsets[end], credentials


def _line_end(text: str, pos: int) -> int:
    while pos < len(text) and text[pos] not in _LINES:
        pos += 1
    return pos


def _keyed_spans(text: str, offsets: list[int] | range | None = None) -> list[_Span]:
    offsets = range(len(text) + 1) if offsets is None else offsets
    spans: list[_Span] = []
    cursor = 0
    commands = iter(_COMMAND_LINE.finditer(text))
    command = next(commands, None)
    command_end = _line_end(text, command.end()) if command else -1
    heads = iter(_url_heads(text))
    head_start, head_end, credential_context = next(heads, (-1, -1, False))
    previous = None
    line_start = scanned = indent = 0
    line_seen = -1
    lines: _Lines | None = None

    def table() -> _Lines:
        nonlocal lines
        if lines is None:
            lines = _Lines(text)
        return lines

    for match in _IDENTIFIER.finditer(text):
        if match.start() < cursor:
            continue
        key = match.group()
        before, previous = previous, match
        while command and command_end < match.start():
            command = next(commands, None)
            command_end = _line_end(text, command.end()) if command else -1
        in_command = command is not None and command.start() <= match.start() <= command_end
        name = _secret_name(key)
        if key in ("-u", "-b") or (in_command and key in ("--user", "--cookie")):
            name = "cookie" if key in ("-b", "--cookie") else "credentials"
        if not name and key.lower() == "key" and match.start() > 0 and text[match.start() - 1] in "?&":
            name = "apikey"
        if not name and before is not None and text[before.end():match.start()] in (" ", "\t"):
            # Spaced names (`api key:`, `"Secret Key":`) still name a secret
            # when the second word changes what the first one means.
            name = _secret_name(before.group() + "_" + key)
            name = "" if name == _secret_name(before.group()) else name
        if not name:
            continue
        while scanned < match.start():
            if text[scanned] in _LINES:
                line_start = scanned + 1
            scanned += 1
        if line_seen != line_start:
            line_seen = line_start
            indent = _line_indent(text, line_start)
        while head_end < match.start() and head_end >= 0:
            head_start, head_end, credential_context = next(heads, (-1, -1, False))
        in_url = head_start <= match.start() < head_end
        pos = match.end()
        key_quote, pos = _quote_at(text, pos)
        after_key = pos
        while pos < len(text) and text[pos] in _SPACE:
            pos += 1
        flag = False
        separator = ""
        if pos < len(text) and text[pos] in ":=":
            separator = text[pos]
            if in_url and (match.start() == head_start or name in _WEAK_NAMES) and separator == ":" and pos + 1 < len(text) and text[pos + 1] not in _SPACE:
                if credential_context or re.fullmatch(r"[0-9]+", text[pos + 1:head_end]):
                    continue
            pos += 1
            if pos < len(text) and (separator, text[pos]) in (("=", ">"), (":", "=")):
                separator = "="
                pos += 1
            while pos < len(text) and text[pos] in _SPACE:
                pos += 1
        elif text.startswith("->", pos):
            separator = "="
            pos += 2
            while pos < len(text) and text[pos] in " \t":
                pos += 1
        elif (key.startswith("-") or in_command) and pos > after_key and _flag_value_gap(text, match.start(), after_key, pos):
            flag = True
        else:
            continue
        start = pos
        while pos < len(text) and text[pos] in ":=>" and not (text[pos] == ">" and _yaml_marker(text, pos)):
            pos += 1
            while pos < len(text) and text[pos] in " \t":
                pos += 1
        if pos == len(text):
            continue
        delimiter, body = _quote_at(text, pos)
        if delimiter:
            if text.startswith(delimiter * 3, pos):
                delimiter *= 3
                body = pos + len(delimiter)
                close = text.find(delimiter, body)
                end, closed = (close + len(delimiter), True) if close >= 0 else (table().yaml_end(body), False)
            else:
                end, closed = _quoted_end(text, body, delimiter)
            if closed and end < len(text) and (not _value_boundary(text, end) or _ambiguous_tail(text, end)):
                end = table().yaml_end(end)
            replacement = delimiter + "[redacted]" + (delimiter if closed else "") if start == pos else "[redacted]"
            resume = end
        else:
            argv = key.startswith("-") or in_command
            literal = _LITERAL.match(text, pos)
            if literal and _literal_end(text, literal.end(), indent, table(), argv):
                # A bare count or boolean after a count-shaped name
                # (`max_tokens: 16`, `descriptors=true`) and JSON null,
                # booleans, sampling seeds and counts are metadata. A string
                # value never is.
                value = literal.group()
                if ((_COUNT_VALUE.fullmatch(value) or value in ("true", "false", "True", "False")) and _count_name(key, name)) or (key_quote and (
                        value in ("null", "true", "false", "None", "True", "False")
                        or name == "seed" or _count_name(key, name))):
                    cursor = literal.end()
                    continue
                if key_quote:
                    # A JSON literal ends unambiguously; later fields stay readable.
                    spans.append((offsets[start], offsets[literal.end()], "[redacted]"))
                    cursor = literal.end()
                    continue
            if key_quote and text[pos] == "{" and _count_name(key, name) and _count_object(text, pos):
                # `"prompt_tokens_details": {"cached_tokens": 0}`: each member
                # is decided on its own key.
                continue
            end = pos
            value_end = " \t\v\f\"'" + _LINES
            while end < len(text) and text[end] not in value_end:
                end += 1
            token = text[pos:end]
            token_end = end
            # A plain value owns lines indented deeper than its line (a YAML or
            # shell continuation), so neither exceptions nor masks stop at the first.
            line_end = table().continued_end(pos, indent)
            # Exceptions describe the entire value. A familiar first word never
            # licenses the rest of an otherwise unquoted credential.
            whole = text[pos:line_end].strip(" \t") if line_end - pos <= 256 else None
            if whole is not None and separator == ":" and not key_quote and (
                whole.lower() in ("none", "expired", "(not set)")
                or (name in _WEAK_NAMES and _alias_prose(name, whole))
                or (name == "pass" and re.fullmatch(r"[0-9]+", whole)
                    and re.search(r"(?:^|[^A-Za-z])tests[ \t]+$", text[max(0, match.start() - 64):match.start()]))
            ):
                continue
            if whole is not None and flag and ((not in_command and _help_value(whole))
                                               or (name == "seedwords" and whole in ("12", "15", "18", "21", "24"))):
                continue
            resume = token_end
            if text[pos:pos + 1] in ("{", "[", "(") or text.startswith(("$(", "${"), pos):
                end = _balanced_end(text, pos, table().yaml_end)
                if text[end:_line_end(text, end)].strip(" \t"):
                    end = _line_end(text, end)
                resume = end
            elif token in ("|", "|-", "|+", ">", ">-", ">+") or text[pos] in "`<":
                end = table().yaml_end(pos)
                resume = end
            elif argv and _flag_value_end(text, token_end):
                # Whitespace before a known flag ends an argv value.
                end = token_end
            else:
                # Without a value opener, quotes and words are credential data,
                # including a stray quote followed by a multi-word passphrase.
                end = line_end
            header = name == "authorization" or (name in ("cookie", "cookies") and not key_quote and not key.startswith("-"))
            if header:
                end = table().yaml_end(pos)
            while end > pos and text[end - 1] in " \t":
                end -= 1
            replacement = "[redacted]"
        if end <= start:
            continue
        spans.append((offsets[start], offsets[end], replacement))
        # Later keys on a plain value's line still establish their own blocks.
        cursor = resume
    return spans


def _line_indent(text: str, start: int) -> int:
    # The line's own indent, past YAML sequence markers (`- password: x`).
    pos = start
    while True:
        while pos < len(text) and text[pos] in " \t":
            pos += 1
        if not (text.startswith("- ", pos) or text.startswith("-\t", pos)):
            return pos - start
        pos += 1


def _literal_end(text: str, pos: int, column: int, lines: _Lines, argv: bool) -> bool:
    # The literal is the whole value: closers then a record boundary, nothing
    # but whitespace through its line and any continuation, or in argv a
    # following flag or `name=` argument.
    closers = pos
    while pos < len(text) and text[pos] in ")]}":
        pos += 1
    if pos < len(text) and text[pos] in ",;":
        return _value_boundary(text, pos)
    closed = pos > closers
    gap = pos
    while pos < len(text) and text[pos] in " \t":
        pos += 1
    if pos == len(text):
        return True
    if text[pos] in _LINES:
        # A closed structure ends its literal; a bare one owns its continuation.
        return closed or not lines.continues(pos, column)
    return pos > gap and (closed or (argv and _argv_boundary(text, pos)))


def _count_object(text: str, pos: int) -> bool:
    """A flat object whose members are all numbers, null or booleans."""
    pos += 1
    while True:
        while pos < len(text) and text[pos] in _SPACE:
            pos += 1
        if text.startswith("}", pos):
            return True
        delimiter, body = _quote_at(text, pos)
        key = _IDENTIFIER.match(text, body) if delimiter else None
        if not key or not text.startswith(delimiter, key.end()):
            return False
        pos = key.end() + len(delimiter)
        while pos < len(text) and text[pos] in " \t":
            pos += 1
        if not text.startswith(":", pos):
            return False
        pos += 1
        while pos < len(text) and text[pos] in " \t":
            pos += 1
        literal = _LITERAL.match(text, pos)
        if not literal:
            return False
        pos = literal.end()
        while pos < len(text) and text[pos] in _SPACE:
            pos += 1
        if text.startswith(",", pos):
            pos += 1
        elif not text.startswith("}", pos):
            return False


# Bitcoin Core's argument-free options. A bare single-dash word can be part of
# a passphrase (`--passphrase correct -horse battery`), so only these, `-h`,
# long `--flags`, `-name=` assignments and a letter flag before a `NAME=`
# argument (`docker run -e NETWORK=mainnet`) end an argv value.
_CORE_FLAGS = """addrinfo blocksonly coinstatsindex daemon daemonwait debug disablewallet discover dns dnsseed
fixedseeds forcednsseed generate getinfo h help help-debug listen listenonion logips logsourcelocations
logthreadnames logtimestamps named natpmp netinfo nodebuglogfile peerblockfilters peerbloomfilters
persistmempool printtoconsole regtest reindex reindex-chainstate rest rpcwait server shrinkdebugfile signet
stdin stdinrpcpass stdinwalletpassphrase testnet testnet4 txindex upnp version walletbroadcast""".split()
_ARG_END = "(?=[ \t\r\n\u0085\u2028\u2029]|$)"
_NEXT_FLAG = re.compile(r"--[A-Za-z][A-Za-z0-9_.-]*(?=[= \t\r\n\u0085\u2028\u2029]|$)|-[A-Za-z][A-Za-z0-9_.-]*(?==)"
                        r"|-[A-Za-z][ \t]+(?=[A-Za-z_][A-Za-z0-9_]*=)|-(?:no)?(?:"
                        + "|".join(sorted(_CORE_FLAGS, key=lambda flag: (-len(flag), flag))) + ")" + _ARG_END)
_ARGV_ASSIGNMENT = re.compile(r"[A-Za-z_][A-Za-z0-9_.-]*=")


def _flag_value_end(text: str, end: int) -> bool:
    if end >= len(text) or text[end] not in " \t" or text[end - 1] == "\\":
        return False
    while end < len(text) and text[end] in " \t":
        end += 1
    return bool(_NEXT_FLAG.match(text, end))


def _argv_boundary(text: str, pos: int) -> bool:
    return bool(_NEXT_FLAG.match(text, pos) or _ARGV_ASSIGNMENT.match(text, pos))


_BREAK = re.compile("\r\n|[\r\n\u0085\u2028\u2029]")
_INDENT = re.compile(r"[ \t]*")


class _Lines:
    """One record's line facts, built once and on demand.

    A value's indented continuation and a YAML block end are lookups instead
    of scans, so an indentation staircase stays linear however many keys and
    lines share one continuation.
    """

    def __init__(self, text: str) -> None:
        self.text = text
        self.starts = [0]
        self.ends: list[int] = []
        for match in _BREAK.finditer(text):
            self.ends.append(match.start())
            self.starts.append(match.end())
        self.ends.append(len(text))
        self.depth: list[int] = []
        self.below: list[int] = []
        self.yaml: list[int] = []

    def _line(self, pos: int) -> int:
        line = bisect_right(self.starts, pos) - 1
        # The LF of a CRLF pair is its own empty line to the scanners.
        return line if pos <= self.ends[line] else -1

    def continued_end(self, pos: int, column: int) -> int:
        line = self._line(pos)
        if line < 0:
            return _continued_end(self.text, pos, column)
        if not self.depth:
            # Plain indents; a blank line (-1) ends every continuation. `below`
            # is the next shallower line, so each walk descends strictly.
            text = self.text
            for start, end in zip(self.starts, self.ends):
                indent = _INDENT.match(text, start).end()
                self.depth.append(indent - start if indent < end else -1)
            count = len(self.depth)
            self.below = [count] * count
            stack: list[int] = []
            for index in range(count - 1, -1, -1):
                while stack and self.depth[stack[-1]] >= self.depth[index]:
                    stack.pop()
                if stack:
                    self.below[index] = stack[-1]
                stack.append(index)
        following = line + 1
        while following < len(self.depth) and self.depth[following] > column:
            following = self.below[following]
        return self.ends[following - 1]

    def continues(self, pos: int, column: int) -> bool:
        return self.continued_end(pos, column) != _line_end(self.text, pos)

    def yaml_end(self, pos: int) -> int:
        line = self._line(pos)
        if line < 0:
            return _yaml_end(self.text, pos)
        if not self.yaml:
            text, starts, ends = self.text, self.starts, self.ends
            self.yaml = list(ends)
            for index in range(len(ends) - 2, -1, -1):
                start = starts[index + 1]
                if not (start < ends[index + 1] and text[start] not in " \t"):
                    self.yaml[index] = self.yaml[index + 1]
        return self.yaml[line]


def _continued_end(text: str, pos: int, column: int) -> int:
    end = _line_end(text, pos)
    while end < len(text):
        line = end + (2 if text.startswith("\r\n", end) else 1)
        indent = line
        while indent < len(text) and text[indent] in " \t":
            indent += 1
        if indent - line <= column or indent == len(text) or text[indent] in _LINES:
            break
        end = _line_end(text, indent)
    return end


def _flag_value_gap(text: str, key_start: int, after_key: int, pos: int) -> bool:
    if not any(ch in _LINES for ch in text[after_key:pos]):
        return True
    line = key_start
    while line > 0 and text[line - 1] not in _LINES:
        line -= 1
    return not text[line:key_start].strip(" \t#>$.-")


def _ambiguous_tail(text: str, end: int) -> bool:
    pos = end
    while pos < len(text) and text[pos] in " \t":
        pos += 1
    if pos == end or pos == len(text) or text[pos] in _LINES:
        return False
    tail = pos
    while tail < len(text) and text[tail] not in _SPACE:
        tail += 1
    token = text[pos:tail]
    return not re.fullmatch(r"[a-z]+[.,!?]?|-{1,2}[a-z][a-z-]*|[a-z0-9_.-]+[:=].*", token)


def _help_value(value: str) -> bool:
    return bool(re.fullmatch(
        r"(?:to set a value\.|instead\.|(?:flag|option) is (?:deprecated|available)\.|"
        r"(?:to|instead|flag|option|none|missing|the))", value, re.IGNORECASE | re.ASCII,
    ))


def _alias_prose(name: str, value: str) -> bool:
    # A single lowercase password is still a password. Only known status/user
    # prose can exempt a weak alias, and only after a spaced colon / XML tag.
    return value in ({"alice", "bob", "root", "anonymous", "alice logged in"} if name == "user" else
                     {"ok", "required", "enabled", "disabled", "none", "expired"} if name in ("auth", "mfa", "twofa") else
                     set())


def _yaml_marker(text: str, pos: int) -> bool:
    pos += 1
    if pos < len(text) and text[pos] in "-+":
        pos += 1
    while pos < len(text) and text[pos] in _SPACE and text[pos] not in _LINES:
        pos += 1
    return pos == len(text) or text[pos] in _LINES


def _balanced_end(text: str, start: int, yaml_end=None) -> int:
    stack: list[str] = []
    # An incomplete opener owns its indented continuation, not later records.
    limit = (yaml_end or (lambda pos: _yaml_end(text, pos)))(start)
    following = limit + (2 if text.startswith("\r\n", limit) else 1)
    if following < len(text) and text[following] in "}])":
        limit = _line_end(text, following)
    pos = start + (1 if text.startswith("$(", start) else 0)
    while pos < limit:
        ch = text[pos]
        if ch in "{[(":
            stack.append({"{": "}", "[": "]", "(": ")"}[ch])
        elif ch in "}])":
            if not stack or stack[-1] != ch:
                return _line_end(text, pos)
            stack.pop()
            if not stack:
                return pos + 1
        elif ch in "\"'":
            pos, closed = _quoted_end(text, pos + 1, ch, limit)
            if not closed:
                return limit
            continue
        pos += 1
    return limit


def _yaml_end(text: str, start: int) -> int:
    end = _line_end(text, start)
    while end < len(text):
        pos = end + (2 if text.startswith("\r\n", end) else 1)
        if pos < len(text) and text[pos] not in " \t" + _LINES:
            break
        end = _line_end(text, pos)
    return end


_VOID_TAGS = frozenset("area base br col embed hr img input link meta param source track wbr".split())
_TAG = re.compile(r"<(/?)([A-Za-z][A-Za-z0-9_.:-]*)(?![A-Za-z0-9_.:-])([^<>]*)>")
_ATTR = re.compile(r"(?<![A-Za-z0-9_.:-])([A-Za-z_:][A-Za-z0-9_.:-]*)[ \t\r\n\v\f]*=[ \t\r\n\v\f]*(?:\"([^\"]*)\"|'([^']*)'|([^ \t\r\n\v\f>]+))", re.ASCII)
_PAIR = re.compile(r"(?<![A-Za-z0-9_.-])[\"']?(?:name|key)[\"']?[ \t]*[:=][ \t]*[\"']?([A-Za-z0-9_.-]+(?::[A-Za-z0-9_.-]+)?)[\"']?[, \t\r\n\v\f\u0085\u2028\u2029]*[\"']?value[\"']?[ \t]*[:=][ \t]*", re.IGNORECASE | re.ASCII)


def _named_pair_spans(text: str, offsets: list[int] | range | None = None) -> list[_Span]:
    offsets = range(len(text) + 1) if offsets is None else offsets
    spans: list[_Span] = []
    cursor = 0
    for pair in _PAIR.finditer(text):
        if pair.start() < cursor or not _secret_name(pair[1]):
            continue
        start = pair.end()
        delimiter, body = _quote_at(text, start)
        if delimiter:
            end, closed = _quoted_end(text, body, delimiter)
            replacement = delimiter + "[redacted]" + (delimiter if closed else "")
        else:
            end = _yaml_end(text, start)
            replacement = "[redacted]"
        spans.append((offsets[start], offsets[end], replacement))
        cursor = end
    return spans


def _xml_spans(text: str, offsets: list[int] | range | None = None) -> list[_Span]:
    offsets = range(len(text) + 1) if offsets is None else offsets
    spans: list[_Span] = []
    cursor = 0
    plist_secret = False
    for tag in _TAG.finditer(text):
        if tag.start() < cursor or tag[1]:
            continue
        key = tag[2]
        if tag[3].lstrip(_XML_SPACE).startswith((":", "-")):
            continue
        name = _secret_name(key.split(":")[-1])
        attrs = []
        for attr in _ATTR.finditer(tag[3]):
            group = next(i for i in (2, 3, 4) if attr[i] is not None)
            attrs.append((attr[1].split(":")[-1].lower(), attr[group], attr.start(group), attr.end(group)))
        keyed = any(k in ("key", "name", "type") and _secret_name(v) for k, v, _, _ in attrs)
        # Even malformed XML like <entrykey="password"value="..."> declares
        # a secret context; spacing mistakes cannot exempt the value.
        if tag[3].startswith("="):
            selector = re.match(r"=[\"']([^\"']+)[\"']", tag[3])
            keyed = keyed or bool(selector and _secret_name(selector[1]))
        if keyed or name:
            for attr_key, value, value_start, value_end in attrs:
                if attr_key not in ("value", "content") or (not keyed and name in _WEAK_NAMES and _alias_prose(name, value)):
                    continue
                start = tag.start(3) + value_start
                end = tag.start(3) + value_end
                spans.append((offsets[start], offsets[end], "[redacted]"))
                cursor = end
        closing = "</" + key
        if key == "key":
            end = text.find(closing, tag.end())
            plist_secret = end >= 0 and bool(_secret_name(text[tag.end():end].strip(_XML_SPACE)))
            continue
        name = name or ("secret" if plist_secret else "")
        plist_secret = False
        if not name and not keyed:
            continue
        if tag[3].rstrip(_XML_SPACE).endswith("/"):
            continue
        end = text.find(closing, tag.end())
        while end >= 0:
            tail = end + len(closing)
            while tail < len(text) and text[tail] in " \t\r\n\v\f":
                tail += 1
            if tail < len(text) and text[tail] == ">":
                break
            end = text.find(closing, tail)
        if end < 0:
            # HTML void elements such as <input> and <meta> have no body.
            if key.lower() in _VOID_TAGS:
                continue
            end = _yaml_end(text, tag.end())
        value = text[tag.end():end]
        if not keyed and name in _WEAK_NAMES and _alias_prose(name, value.strip(_XML_SPACE)):
            continue
        spans.append((offsets[tag.end()], offsets[end], "[redacted]"))
        cursor = end
    return spans


def _cli_spans(text: str, offsets: list[int] | range | None = None) -> list[_Span]:
    offsets = range(len(text) + 1) if offsets is None else offsets
    spans: list[_Span] = []
    cursor = 0
    for match in re.finditer(r"(?<![A-Za-z0-9_])(?:curl[ \t]+-u[ \t]*|sshpass[ \t]+-p[ \t]*|mysql[ \t]+-u[ \t]+[^ \t\r\n]+[ \t]+-p)", text):
        if match.start() < cursor:
            continue
        start = match.end()
        delimiter, body = _quote_at(text, start)
        if delimiter:
            end, closed = _quoted_end(text, body, delimiter)
            replacement = delimiter + "[redacted]" + (delimiter if closed else "")
        else:
            end = _line_end(text, start)
            replacement = "[redacted]"
        spans.append((offsets[start], offsets[end], replacement))
        cursor = end
    return spans



def _url_view(text: str) -> tuple[str, list[int] | range]:
    """Decode credential syntax, retaining an offset for every original span.

    Three percent layers are a fixed linear envelope. JSON slash escape runs
    need no depth cap: each run is consumed once, without decoding whole JSON.
    """
    if not any(ch in text for ch in "%\\&ſ﹫：ｈｔｐ") and not any("\uff01" <= ch <= "\uff5e" for ch in text):
        return text, range(len(text) + 1)
    offsets = list(range(len(text) + 1))
    for _ in range(3):
        chars: list[str] = []
        positions: list[int] = []
        pos = 0
        while pos < len(text):
            positions.append(offsets[pos])
            entity = re.match(r"&#([xX][0-9a-fA-F]{1,6}|[0-9]{1,7});|&(colon|sol|commat|quot|apos|lt|gt|amp|equals|Tab|NewLine);", text[pos:pos + 14]) if text[pos] == "&" else None
            if entity:
                number = entity[1]
                code = int(number[1:], 16) if number and number[0] in "xX" else int(number) if number else {"colon": 58, "sol": 47, "commat": 64, "quot": 34, "apos": 39, "lt": 60, "gt": 62, "amp": 38, "equals": 61, "Tab": 9, "NewLine": 10}[entity[2]]
                chars.append(chr(code) if code <= 0x10ffff else "\ufffd")
                pos += entity.end()
            elif text[pos] == "%" and pos + 2 < len(text) and all(c in "0123456789abcdefABCDEF" for c in text[pos + 1:pos + 3]):
                chars.append(chr(int(text[pos + 1:pos + 3], 16)))
                pos += 3
            elif text[pos] == "\\":
                end = pos
                while end < len(text) and text[end] == "\\":
                    end += 1
                if end < len(text) and text[end] == "/":
                    chars.append("/")
                    pos = end + 1
                elif end < len(text) and text[end] in "ux" and end + (5 if text[end] == "u" else 3) <= len(text) and all(ch in "0123456789abcdefABCDEF" for ch in text[end + 1:end + (5 if text[end] == "u" else 3)]):
                    escape_end = end + (5 if text[end] == "u" else 3)
                    chars.append(chr(int(text[end + 1:escape_end], 16)))
                    pos = escape_end
                else:
                    chars.extend(text[pos:end])
                    positions.extend(offsets[pos + 1:end])
                    pos = end
            else:
                ch = text[pos]
                chars.append(chr(ord(ch) - 0xfee0) if "\uff01" <= ch <= "\uff5e" else "s" if ch == "ſ" else "@" if ch == "﹫" else ch)
                pos += 1
        positions.append(offsets[-1])
        decoded = "".join(chars)
        if decoded == text:
            break
        text, offsets = decoded, positions
    return text, offsets


def _opaque_token(text: str) -> bool:
    if re.search(r"[A-Za-z0-9_]{10,}|[A-Z]{4,}", text):
        return True
    return any(re.search(r"[A-Za-z]", part) and re.search(r"[0-9]", part)
               for part in re.findall(r"[A-Za-z0-9_]{6,}", text))


_INVALID_USERINFO = re.compile("[" + re.escape(_SPACE + '/[]{}"<>|') + "]")
_QUERY_VALUE = re.compile(r"([?&;][A-Za-z_][A-Za-z0-9_.-]*=)[^&;#]*")


def _strong_opaque(text: str) -> bool:
    # Long lowercase words are ordinary paths; capitals and mixed runs are not.
    return bool(re.search(r"[A-Z]{4,}", text)) or any(
        re.search(r"[A-Za-z]", part) and re.search(r"[0-9]", part)
        for part in re.findall(r"[A-Za-z0-9_]{6,}", text))


def _clear_host(authority: str) -> bool:
    if re.fullmatch(r"\[[0-9A-Fa-f:.%a-z]+\](?::[0-9]*)?", authority):
        return True
    host, colon, port = authority.rpartition(":")
    if not colon:
        host = authority
    elif not re.fullmatch(r"[0-9]+", port):
        return False
    return bool(re.fullmatch(r"[A-Za-z0-9](?:[A-Za-z0-9.-]*[A-Za-z0-9])?", host)
                and not _opaque_token(host.split(".")[0]))


def _public_url_prefix(prefix: str, original: str, offsets: list[int] | range, start: int) -> bool:
    # An @ in an ordinary path, handle, package reference or following email
    # cannot create userinfo. Opaque tokens and user:pass shapes stay ambiguous.
    boundary = re.search(r"[/?# \t\r\n\v\f\u0085\u2028\u2029]", prefix)
    if not boundary or not _clear_host(prefix[:boundary.start()]):
        return False
    authority = prefix[:boundary.start()]
    tail = prefix[boundary.start():]
    # A literal gap following a complete public URL ends that URL, regardless
    # of the length of its path words. Encoded gaps remain credential data.
    # Opaque path words and the words before the later @ stay ambiguous; named
    # query values are left to the keyed scanner.
    gap = re.search(r"[ \t\v\f" + _LINES + r"]", tail)
    pos = start + boundary.start() + (gap.start() if gap else 0)
    if gap and tail[0] in "/?#" and original[offsets[pos]:offsets[pos + 1]] == gap[0]:
        path, after = tail[:gap.start()], tail[gap.start():]
        words = _QUERY_VALUE.sub(r"\1", path)
        return not (":" in path or any(ch in path for ch in "\\{}[]\"'<>|")
                    or _strong_opaque(words) or _strong_opaque(after))
    if tail[0] in " \t\v\f" + _LINES:
        pos = start + boundary.start()
        if original[offsets[pos]:offsets[pos + 1]] != tail[0]:
            return False
    if ":" in authority and not authority.startswith("[") and not (tail[0] in "?#" and ("." in authority or authority.split(":", 1)[0] == "localhost")) and not (tail[0] in _LINES and not _opaque_token(tail) and ":" not in tail):
        return False
    return not (":" in tail or _opaque_token(tail)
                or any(ch in tail for ch in "\\{}[]\"'<>|"))


def _url_line_end(view: str, original: str, offsets: list[int] | range, start: int, end: int) -> int:
    # A literal line break ends a URL after its path starts, unless the next
    # line's first token continues the userinfo with an `@`.
    pos = start
    authority = True
    while pos < end:
        literal = original[offsets[pos]:offsets[pos + 1]] == view[pos]
        if view[pos] in "/?#" and literal:
            authority = False
        elif view[pos] in _LINES and literal and not authority:
            following = pos
            while following < end and view[following] in _SPACE:
                following += 1
            token = following
            while token < end and view[token] != "@" and not (view[token] in _SPACE and original[offsets[token]:offsets[token + 1]] == view[token]):
                token += 1
            if token == end or view[token] != "@":
                return pos
            pos = following
            continue
        pos += 1
    return end


def _url_candidate_end(view: str, original: str, offsets: list[int] | range, start: int, end: int) -> int:
    first_at = view.find("@", start, end)
    if first_at < 0:
        return end
    last_at = first_at
    checked_at = -1
    gap = first_at + 1
    while gap < end:
        if view[gap] == "@":
            last_at = gap
        elif last_at != checked_at and view[gap] in " \t\v\f" + _LINES and original[offsets[gap]:offsets[gap + 1]] == view[gap]:
            checked_at = last_at
            tail = re.split(r"[/?#]", view[last_at + 1:gap], maxsplit=1)[0]
            # An encoded @ before a bare word (`%40q (ZK…@h`) may be userinfo
            # data, unless a dotted/port host or ordinary words follow. A line
            # whose first token reaches an @ continues a wrapped password.
            literal = original[offsets[last_at]:offsets[last_at + 1]] == "@"
            following = view.find("@", gap, end)
            if _clear_host(tail) and (literal or tail.startswith("[") or any(ch in tail for ch in ".:")
                                      or not (_strong_opaque(view[gap:following if following >= 0 else end])
                                              or _wrapped_userinfo(view, original, offsets, gap, end))):
                return gap
        gap += 1
    return end


def _wrapped_userinfo(view: str, original: str, offsets: list[int] | range, gap: int, end: int) -> bool:
    if view[gap] not in _LINES:
        return False
    pos = gap
    while pos < end and view[pos] in _SPACE:
        pos += 1
    while pos < end and view[pos] != "@" and not (view[pos] in _SPACE and original[offsets[pos]:offsets[pos + 1]] == view[pos]):
        pos += 1
    return pos < end and view[pos] == "@"


def _fragment_userinfo(prefix: str) -> bool:
    # `https://Pw9#Leak@host`: a `#` after a bare token, not a dotted, port or
    # bracketed host, is credential data before the later @.
    boundary = re.search(r"[/?#]", prefix)
    return bool(boundary) and boundary[0] == "#" and not any(ch in prefix[:boundary.start()] for ch in ".:[")


def _url_spans(text: str) -> list[_Span]:
    view, offsets = _url_view(text)
    matches = list(_SCHEME.finditer(view))
    spans: list[_Span] = []
    cursor = 0
    pending = -1
    quotes = iter(re.finditer(r"[\"']", view))
    quote = next(quotes, None)
    previous_quote = checked_quote = -1
    record = False
    for index, match in enumerate(matches):
        while quote is not None and quote.start() < match.start():
            previous_quote, quote = quote.start(), next(quotes, None)
        if previous_quote != checked_quote:
            record = _record_opener(view, text, offsets, previous_quote)
            checked_quote = previous_quote
        start = match.end()
        if offsets[start] < cursor:
            continue
        end = matches[index + 1].start() if index + 1 < len(matches) else len(view)
        record_end = _url_record_end(view, text, offsets, start, end, record)
        line_end = _url_line_end(view, text, offsets, start, record_end)
        end = _url_candidate_end(view, text, offsets, start, line_end)
        first_at = view.find("@", start, end)
        if first_at < 0:
            # A URL ended by its line cannot merge with a later credential URL.
            if line_end == record_end and not _clear_host(view[start:end]) and pending < 0 and not _public_url_prefix(view[start:end], text, offsets, start):
                pending = start
            continue
        last_at = view.rfind("@", start, end)
        if _public_url_prefix(view[start:last_at], text, offsets, start) and not _fragment_userinfo(view[start:last_at]):
            pending = -1
            # A public @handle path cannot license a `handle:password` pair.
            handle = re.match(r"[^/?#" + _SPACE + r"]*", view[last_at + 1:end])[0]
            if ":" in handle and not re.fullmatch(r"[^:]*:[0-9]*|sha(?:1|256|384|512):[0-9A-Fa-f]+", handle):
                spans.append((offsets[last_at + 1], offsets[last_at + 1 + len(handle)], "[redacted-credentials]"))
                cursor = offsets[last_at + 1 + len(handle)]
            continue
        start = pending if pending >= 0 else start
        host_tail = re.split(r"[/?#]", view[last_at + 1:end], maxsplit=1)[0]
        invalid = bool(_INVALID_USERINFO.search(view, start, last_at)) or _opaque_token(host_tail.split(".")[0])
        finish = last_at + 1
        if invalid:
            while finish < end and view[finish] not in _SPACE + '/"\'<>':
                finish += 1
        spans.append((offsets[start], offsets[finish], "[redacted-credentials]" + ("" if invalid else "@")))
        cursor = offsets[finish]
        pending = -1
        # A delimiter glued to an unfinished authority (`a@b:#x//@h`) is more
        # userinfo, not a new URL: the next @ extends this span.
        rest = view[finish:end]
        if index + 1 < len(matches) and end == matches[index + 1].start() and not _clear_host(rest.rstrip(",;|&")) \
                and not any(view[gap] in _SPACE and text[offsets[gap]:offsets[gap + 1]] == view[gap] for gap in range(finish, end)):
            pending = start
    return spans


_PAIR_STOPS = re.compile(r"[:=<>\"'({\[\]@]")


def _userinfo_pair(token: str, at: int) -> tuple[int, int] | None:
    # The first `name:` before the last @, found by one forward scan.
    run = 0
    for stop in _PAIR_STOPS.finditer(token, 0, at):
        if stop.group() == ":" and stop.start() > run:
            return run, stop.end()
        run = stop.end()
    return None


def _bare_userinfo_spans(text: str) -> list[_Span]:
    view, offsets = _url_view(text)
    spans: list[_Span] = []
    start = 0
    while start < len(view):
        end = start
        while end < len(view) and not (view[end] in " \t\v\f" + _LINES and text[offsets[end]:offsets[end + 1]] == view[end]):
            end += 1
        token = view[start:end]
        # Userinfo before a later scheme is still `token:token@host`.
        scheme = _SCHEME.search(token)
        token = token[:scheme.start()] if scheme else token
        at = token.rfind("@")
        pair = _userinfo_pair(token, at) if at > 0 else None
        if pair and not _secret_name(token[pair[0]:pair[1] - 1]) \
                and not (token[pair[0]:pair[1]].lower() in ("mailto:", "xmpp:", "sip:", "sips:") and ":" not in token[pair[1]:at]):
            spans.append((offsets[start + pair[0]], offsets[start + at + 1], "[redacted-credentials]@"))
        start = end + 1
    return spans


# Bot and webhook tokens travel in URL paths. Their extent is ambiguous, so
# the mask runs to the next literal whitespace.
_PATH_TOKEN = re.compile(r"/bot(?=[0-9]{3,}:)|hooks\.slack\.com/services/[^/ \t\r\n\v\f]+/[^/ \t\r\n\v\f]+/|hooks\.slack\.com/(?:services|workflows|triggers)/"
                         r"|discord(?:app)?\.com/api/(?:v[0-9]+/)?webhooks/[0-9]+/|(?<![A-Za-z0-9_])(?=xox[a-z]-|xapp-)", re.IGNORECASE | re.ASCII)


def _path_token_spans(text: str, offsets: list[int] | range | None = None) -> list[_Span]:
    offsets = range(len(text) + 1) if offsets is None else offsets
    spans: list[_Span] = []
    cursor = 0
    for match in _PATH_TOKEN.finditer(text):
        if match.start() < cursor:
            continue
        end = match.end()
        while end < len(text) and text[end] not in _SPACE:
            end += 1
        if end > match.end():
            spans.append((offsets[match.end()], offsets[end], "[redacted]"))
        cursor = end
    return spans


# Longer records are masked whole: no scanner runs on an unbounded input.
REDACTION_LIMIT = 1_000_000


def redact_credentials(text: str) -> str:
    """Union secret contexts before masking; one detector cannot erase another.

    Each forward scanner produces ordered original-coordinate spans. Merging a
    fixed number of streams is linear, including overlaps between decoded and
    literal syntax. Ambiguous overlaps own their whole union.
    """
    if len(text) > REDACTION_LIMIT:
        return "[redacted-oversized]"
    view, offsets = _url_view(text)
    streams = [_url_spans(text), _bare_userinfo_spans(text)]
    for scan in (_xml_spans, _named_pair_spans, _cli_spans, _path_token_spans, _keyed_spans):
        streams.append(scan(text))
        if view != text:
            streams.append(scan(view, offsets))
    pieces = []
    cursor = 0
    pending: _Span | None = None
    for start, end, replacement in merge(*streams, key=lambda span: (span[0], -span[1])):
        if pending is not None and start < pending[1]:
            if end > pending[1]:
                pending = (pending[0], end, "[redacted]")
            continue
        if pending is not None:
            pieces.extend((text[cursor:pending[0]], pending[2]))
            cursor = pending[1]
        pending = start, end, replacement
    if pending is not None:
        pieces.extend((text[cursor:pending[0]], pending[2]))
        cursor = pending[1]
    pieces.append(text[cursor:])
    return "".join(pieces)
