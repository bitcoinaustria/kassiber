"""Forward-only credential scanners, mirrored by the webview secretScanner.ts."""

from __future__ import annotations

import re
from heapq import merge


_IDENTIFIER = re.compile(r"[A-Za-z0-9_.-]+")
_KEY_PARTS = re.compile(r"[_.-]+|(?<=[a-z0-9])(?=[A-Z])|(?<=[A-Z])(?=[A-Z][a-z])|(?<=[0-9])(?=[A-Za-z])")
_VERSION = re.compile(r"[_-]v[0-9]+_*$", re.IGNORECASE)
_NAMES = frozenset("""apikey authheader authorization blinding blindingkey cookie cookies
descriptor mnemonic passphrase password passwordhash rpcpassword rpcauth macaroon
macaroonhex recoveryphrase rune scankey scanprivate scanprivatekey spendkey secret
secretkey privatekey privkey seed seedphrase seedwords silentpayment silentpayments
silentpaymentdescriptor silentpaymentsdescriptor silentpaymentkey silentpaymentskey
silentpaymentmaterial silentpaymentsmaterial silentpaymentscan silentpaymentscan
silentpaymentscankey silentpaymentsscankey spdescriptor spscan spspend token xprv
accesstoken refreshtoken apitoken authtoken clientsecret dbpassword apisecret walletpassphrase wif rpcpass pass passwd pwd credentials auth user
credential sessionid sessionkey masterkey encryptionkey signingkey bearer pw pin otp""".split())
# The delimiter establishes a URL context; spelling the scheme cannot exempt
# userinfo. The first alternative only preserves the readable scheme prefix.
_SCHEME = re.compile(r"(?<![A-Za-z0-9+.-])[A-Za-z0-9+.-]+://|://|//")
_SPACE = " \t\r\n\v\f\u0085\u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000\ufeff"
_LINES = "\r\n\u0085\u2028\u2029"
_WEAK_NAMES = frozenset(("pass", "user", "auth"))
# Short names count only as a whole identifier part (`user_id`, not `author`);
# the shortest only as the final part (`db_pw`, not `spinner` or `pw_hash`).
_PART_NAMES = frozenset(("user", "auth"))
_LAST_PART_NAMES = frozenset(("pw", "pin", "otp"))
_COMMAND_LINE = re.compile(r"(?:^|(?<=[\r\n\u0085\u2028\u2029]))[ \t]*(?:[$>][ \t]+)?(?:cmd|kassiber|bitcoind|bitcoin-cli|electrs|lncli|lightning-cli|curl|python|node|env|sudo|docker)(?=[ \t]|$)", re.ASCII)
_Span = tuple[int, int, str]
_ORDERED_NAMES = sorted(_NAMES, key=lambda name: (-len(name), name))


def _secret_name(token: str) -> str:
    token = _VERSION.sub("", token).strip("_.-")
    parts = [part.lower().strip("0123456789") for part in _KEY_PARTS.split(token)]
    if parts[-1] in ("hint", "statistics", "file", "stdin", "fd"):
        return ""
    # Identifier composition is not a security boundary (PGPASSWORD, jwtsecret).
    compact = "".join(parts)
    return next((name for name in _ORDERED_NAMES if name in compact
                 and (name not in _PART_NAMES or name in parts)
                 and (name not in _LAST_PART_NAMES or name == parts[-1])), "")


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
        limit = _url_candidate_end(view, text, offsets, start, limit)
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
    line_start = scanned = 0
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
                end, closed = (close + len(delimiter), True) if close >= 0 else (_yaml_end(text, body), False)
            else:
                end, closed = _quoted_end(text, body, delimiter)
            if closed and end < len(text) and (not _value_boundary(text, end) or _ambiguous_tail(text, end)):
                end = _yaml_end(text, end)
            replacement = delimiter + "[redacted]" + (delimiter if closed else "") if start == pos else "[redacted]"
        else:
            end = pos
            value_end = " \t\v\f\"'" + _LINES
            while end < len(text) and text[end] not in value_end:
                end += 1
            token = text[pos:end]
            # A plain value owns lines indented deeper than its key (a YAML
            # continuation), so neither exceptions nor masks stop at the first.
            line_end = _continued_end(text, pos, match.start() - line_start)
            whole = text[pos:line_end].strip(" \t")
            # Exceptions describe the entire value. A familiar first word never
            # licenses the rest of an otherwise unquoted credential.
            if separator == ":" and not key_quote and (
                whole.lower() in ("none", "expired", "(not set)")
                or (name in _WEAK_NAMES and _alias_prose(name, whole))
                or (name == "pass" and re.fullmatch(r"[0-9]+", whole)
                    and re.search(r"(?:^|[^A-Za-z])tests[ \t]+$", text[max(0, match.start() - 64):match.start()]))
            ):
                continue
            if flag and ((not in_command and _help_value(whole))
                         or (name == "seedwords" and whole in ("12", "15", "18", "21", "24"))):
                continue
            if text[pos:pos + 1] in ("{", "[", "(") or text.startswith(("$(", "${"), pos):
                end = _balanced_end(text, pos)
                if text[end:_line_end(text, end)].strip(" \t"):
                    end = _line_end(text, end)
            elif token in ("|", "|-", "|+", ">", ">-", ">+") or text[pos] in "`<":
                end = _yaml_end(text, pos)
            else:
                # Without a value opener, quotes and words are credential data,
                # including a stray quote followed by a multi-word passphrase.
                end = line_end
            header = name == "authorization" or (name in ("cookie", "cookies") and not key_quote and not key.startswith("-"))
            if header:
                end = _yaml_end(text, pos)
            while end > pos and text[end - 1] in " \t":
                end -= 1
            replacement = "[redacted]"
        if end <= start:
            continue
        spans.append((offsets[start], offsets[end], replacement))
        cursor = end
    return spans


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
                     {"ok", "required", "enabled", "disabled", "none", "expired"} if name == "auth" else
                     set())


def _yaml_marker(text: str, pos: int) -> bool:
    pos += 1
    if pos < len(text) and text[pos] in "-+":
        pos += 1
    while pos < len(text) and text[pos] in _SPACE and text[pos] not in _LINES:
        pos += 1
    return pos == len(text) or text[pos] in _LINES


def _balanced_end(text: str, start: int) -> int:
    stack: list[str] = []
    # An incomplete opener owns its indented continuation, not later records.
    limit = _yaml_end(text, start)
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
_PAIR = re.compile(r"(?<![A-Za-z0-9_.-])[\"']?(?:name|key)[\"']?[ \t]*[:=][ \t]*[\"']?([A-Za-z0-9_.:-]+)[\"']?[, \t\r\n\v\f]*[\"']?value[\"']?[ \t]*[:=][ \t]*", re.IGNORECASE | re.ASCII)


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
        if tag[3].lstrip().startswith((":", "-")):
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
            plist_secret = end >= 0 and bool(_secret_name(text[tag.end():end].strip()))
            continue
        name = name or ("secret" if plist_secret else "")
        plist_secret = False
        if not name and not keyed:
            continue
        if tag[3].rstrip().endswith("/"):
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
        if not keyed and name in _WEAK_NAMES and _alias_prose(name, value.strip()):
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
            # data, unless a dotted/port host or ordinary words follow.
            literal = original[offsets[last_at]:offsets[last_at + 1]] == "@"
            following = view.find("@", gap, end)
            if _clear_host(tail) and (literal or tail.startswith("[") or any(ch in tail for ch in ".:")
                                      or not _strong_opaque(view[gap:following if following >= 0 else end])):
                return gap
        gap += 1
    return end


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
        end = _url_record_end(view, text, offsets, start, end, record)
        end = _url_candidate_end(view, text, offsets, start, end)
        first_at = view.find("@", start, end)
        if first_at < 0:
            if not _clear_host(view[start:end]) and pending < 0 and not _public_url_prefix(view[start:end], text, offsets, start):
                pending = start
            continue
        last_at = view.rfind("@", start, end)
        if _public_url_prefix(view[start:last_at], text, offsets, start):
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


def _bare_userinfo_spans(text: str) -> list[_Span]:
    view, offsets = _url_view(text)
    spans: list[_Span] = []
    cursor = 0
    start = 0
    while start < len(view):
        end = start
        while end < len(view) and not (view[end] in " \t\v\f" + _LINES and text[offsets[end]:offsets[end + 1]] == view[end]):
            end += 1
        token = view[start:end]
        # Userinfo before a later scheme is still `token:token@host`.
        scheme = _SCHEME.search(token)
        token = token[:scheme.start()] if scheme else token
        if "@" in token:
            at = token.rfind("@")
            pair = re.search(r"[^:=<>\"'({\[\]@]+:", token)
            if pair and pair.end() - 1 < at and not _secret_name(pair[0][:-1]) \
                    and not (pair[0].lower() in ("mailto:", "xmpp:", "sip:", "sips:") and ":" not in token[pair.end():at]):
                body = pair.start()
                spans.append((offsets[start + body], offsets[start + at + 1], "[redacted-credentials]@"))
                cursor = offsets[start + at + 1]
        start = end + 1
    return spans


def redact_credentials(text: str) -> str:
    """Union secret contexts before masking; one detector cannot erase another.

    Each forward scanner produces ordered original-coordinate spans. Merging a
    fixed number of streams is linear, including overlaps between decoded and
    literal syntax. Ambiguous overlaps own their whole union.
    """
    view, offsets = _url_view(text)
    streams = [_url_spans(text), _bare_userinfo_spans(text)]
    for scan in (_xml_spans, _named_pair_spans, _cli_spans, _keyed_spans):
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
