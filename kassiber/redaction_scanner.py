"""Forward-only credential scanners, mirrored by the webview secretScanner.ts."""

from __future__ import annotations

import re


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
accesstoken refreshtoken apitoken authtoken clientsecret dbpassword apisecret walletpassphrase wif rpcpass pass passwd pwd credentials auth user""".split())
_PROSE = frozenset("""to instead flag option when requires expected missing with
without is are was were has have the a an and or for from of in on set use words
value values argument arguments deprecated reset rotated invalid expired none""".split())
_RECOVERY = frozenset(("mnemonic", "recoveryphrase", "seed", "seedphrase", "seedwords"))
_SCHEME = re.compile(r"(?<![A-Za-z0-9+.-])[A-Za-z0-9+.-]+://|(?<![^ \t\r\n\v\f=({\[<>])//")
_SPACE = " \t\r\n\v\f\u0085\u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000\ufeff"
_LINES = "\r\n\u0085\u2028\u2029"
_WEAK_NAMES = frozenset(("pass", "user", "auth"))
_COMMAND_LINE = re.compile(r"(?:^|(?<=[\r\n\u0085\u2028\u2029]))[ \t]*(?:[$>][ \t]+)?(?:cmd|kassiber|bitcoind|bitcoin-cli|electrs|lncli|lightning-cli|curl|python|node|env|sudo|docker)(?=[ \t]|$)", re.ASCII)


def _secret_name(token: str) -> str:
    token = _VERSION.sub("", token).strip("_.-")
    parts = [part.lower().strip("0123456789") for part in _KEY_PARTS.split(token)]
    if parts[-1] in ("hint", "statistics", "file", "stdin", "fd"):
        return ""
    for end in range(len(parts), 0, -1):
        for count in range(min(4, end), 0, -1):
            name = "".join(parts[end - count:end])
            if name in _NAMES:
                return name
    return ""


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
    return (first_newline if first_newline >= 0 else pos), False


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


def _redact_keyed_plain(text: str, original: str | None = None, offsets: list[int] | range | None = None) -> str:
    source = text if original is None else original
    offsets = range(len(text) + 1) if offsets is None else offsets
    pieces: list[str] = []
    cursor = 0
    commands = iter(_COMMAND_LINE.finditer(text))
    command = next(commands, None)
    command_end = _line_end(text, command.end()) if command else -1
    heads = iter(_url_heads(text))
    head_start, head_end, credential_context = next(heads, (-1, -1, False))
    for match in _IDENTIFIER.finditer(text):
        if match.start() < cursor:
            continue
        key = match.group()
        while command and command_end < match.start():
            command = next(commands, None)
            command_end = _line_end(text, command.end()) if command else -1
        in_command = command is not None and command.start() <= match.start() <= command_end
        name = _secret_name(key)
        if key in ("-u", "-b") or (in_command and key in ("--user", "--cookie")):
            name = "cookie" if key in ("-b", "--cookie") else "credentials"
        if not name:
            continue
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
                end, closed = (close + len(delimiter), True) if close >= 0 else (_line_end(text, body), False)
            else:
                end, closed = _quoted_end(text, body, delimiter)
            if closed and end < len(text) and (not _value_boundary(text, end) or _ambiguous_tail(text, end)):
                while end < len(text) and text[end] not in _LINES:
                    end += 1
            replacement = delimiter + "[redacted]" + (delimiter if closed else "") if start == pos else "[redacted]"
        else:
            end = pos
            value_end = " \t\v\f\"'" + _LINES
            while end < len(text) and text[end] not in value_end:
                end += 1
            token = text[pos:end]
            if name in _WEAK_NAMES and separator == ":" and not key_quote and pos > 0 and text[pos - 1] in _SPACE and not _ambiguous_tail(text, end) and (_alias_prose(name, token) or (name == "pass" and re.fullmatch(r"[0-9]+", token) and re.search(r"(?:^|[^A-Za-z])tests[ \t]+$", text[max(0, match.start() - 64):match.start()]))):
                continue
            if separator == ":" and (token.lower() in ("none", "expired") or text.startswith("(not set)", pos)):
                continue
            if flag and ((not in_command and (end == len(text) or text[end] in _SPACE) and (token.lower() in _PROSE or re.fullmatch(r"[a-z]+[.,;!?:]", token)))
                         or (name == "seedwords" and token in ("12", "15", "18", "21", "24"))):
                continue
            if text[pos:pos + 1] in ("{", "[") or text.startswith("$(", pos):
                end = _balanced_end(text, pos)
                if end < len(text) and text[end] not in _SPACE:
                    end = _line_end(text, end)
            elif token in ("|", "|-", "|+", ">", ">-", ">+"):
                end = _yaml_end(text, pos)
            header = name == "authorization" or (name in ("cookie", "cookies") and not key_quote and not key.startswith("-"))
            if header or name in _RECOVERY:
                while end < len(text) and text[end] not in ((_LINES + ("\"'" if name != "authorization" and separator == "=" else "")) if header else ",;}{&|" + _LINES + "\"'"):
                    next_key = _IDENTIFIER.match(text, end) if not header else None
                    if next_key:
                        tail = next_key.end()
                        while tail < len(text) and text[tail] in " \t":
                            tail += 1
                        if tail < len(text) and text[tail] in ":=":
                            break
                        end = next_key.end()
                    else:
                        end += 1
                if header:
                    # RFC obs-fold: each indented physical line belongs to the header.
                    while end < len(text):
                        fold = end
                        if text.startswith("\r\n", fold):
                            fold += 2
                        elif text[fold] in _LINES:
                            fold += 1
                        else:
                            break
                        if fold == len(text) or text[fold] not in " \t":
                            break
                        end = _line_end(text, fold)
                while end > pos and text[end - 1] in " \t":
                    end -= 1
            if _ambiguous_tail(text, end):
                end = _line_end(text, end)
            replacement = "[redacted]"
            if end + 1 < len(text) and text[end] in "\"'" and not _value_boundary(text, end + 1):
                while end < len(text) and text[end] not in _LINES:
                    end += 1
        if end <= start:
            continue
        if in_url and start <= head_end:
            # Retain a synthetic userinfo terminator: removing a keyed value
            # must not hide the surrounding username from the URL floor.
            replacement += "@"
        pieces.extend((source[offsets[cursor]:offsets[start]], replacement))
        cursor = end
    pieces.append(source[offsets[cursor]:])
    return "".join(pieces)


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


def _alias_prose(name: str, value: str) -> bool:
    # A single lowercase password is still a password. Only known status/user
    # prose can exempt a weak alias, and only after a spaced colon / XML tag.
    return value in ({"alice", "bob", "root", "anonymous"} if name == "user" else
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


_TAG = re.compile(r"<(/?)([A-Za-z][A-Za-z0-9_.:-]*)(?![A-Za-z0-9_.:-])([^<>]*)>")
_ATTR = re.compile(r"(?<![A-Za-z0-9_.:-])([A-Za-z_:][A-Za-z0-9_.:-]*)\s*=\s*([\"'])(.*?)\2", re.ASCII | re.DOTALL)


def _redact_xml(text: str) -> str:
    pieces = []
    cursor = 0
    for tag in _TAG.finditer(text):
        if tag.start() < cursor or tag[1]:
            continue
        key = tag[2]
        if tag[3].lstrip().startswith(("=", ":", "-")):
            continue
        name = _secret_name(key.split(":")[-1])
        attrs = list(_ATTR.finditer(tag[3]))
        keyed = any(a[1].lower() in ("key", "name") and _secret_name(a[3]) for a in attrs)
        if keyed or name:
            for value in attrs:
                if value[1].lower() != "value" or (not keyed and name in _WEAK_NAMES and _alias_prose(name, value[3])):
                    continue
                start = tag.start(3) + value.start(3)
                end = tag.start(3) + value.end(3)
                pieces.extend((text[cursor:start], "[redacted]"))
                cursor = end
        if not name and not keyed:
            continue
        if tag[3].rstrip().endswith("/"):
            continue
        closing = "</" + key
        end = text.find(closing, tag.end())
        while end >= 0:
            tail = end + len(closing)
            while tail < len(text) and text[tail] in " \t\r\n\v\f":
                tail += 1
            if tail < len(text) and text[tail] == ">":
                break
            end = text.find(closing, tail)
        if end < 0:
            end = len(text)
        value = text[tag.end():end]
        if not keyed and name in _WEAK_NAMES and _alias_prose(name, value.strip()):
            continue
        pieces.extend((text[cursor:tag.end()], "[redacted]"))
        cursor = end
    pieces.append(text[cursor:])
    return "".join(pieces)


def redact_keyed_secrets(text: str) -> str:
    text = _redact_xml(text)
    text = _redact_keyed_plain(text)
    view, offsets = _url_view(text)
    if view != text:
        text = _redact_keyed_plain(view, text, offsets)
    return text


def _url_view(text: str) -> tuple[str, list[int] | range]:
    """Decode credential syntax, retaining an offset for every original span.

    Three percent layers are a fixed linear envelope. JSON slash escape runs
    need no depth cap: each run is consumed once, without decoding whole JSON.
    """
    if not any(ch in text for ch in "%\\&ſ：ｈｔｐ") and not any("\uff01" <= ch <= "\uff5e" for ch in text):
        return text, range(len(text) + 1)
    offsets = list(range(len(text) + 1))
    for _ in range(3):
        chars: list[str] = []
        positions: list[int] = []
        pos = 0
        while pos < len(text):
            positions.append(offsets[pos])
            entity = re.match(r"&#([xX][0-9a-fA-F]{1,6}|[0-9]{1,7});|&(colon|sol|commat);", text[pos:pos + 14]) if text[pos] == "&" else None
            if entity:
                number = entity[1]
                code = int(number[1:], 16) if number and number[0] in "xX" else int(number) if number else {"colon": 58, "sol": 47, "commat": 64}[entity[2]]
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
                elif end + 5 <= len(text) and text[end] == "u" and all(ch in "0123456789abcdefABCDEF" for ch in text[end + 1:end + 5]):
                    chars.append(chr(int(text[end + 1:end + 5], 16)))
                    pos = end + 5
                else:
                    chars.extend(text[pos:end])
                    positions.extend(offsets[pos + 1:end])
                    pos = end
            else:
                ch = text[pos]
                chars.append(chr(ord(ch) - 0xfee0) if "\uff01" <= ch <= "\uff5e" else "s" if ch == "ſ" else ch)
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
    if ":" in authority and not (authority.startswith("[") or "." in authority or authority.split(":", 1)[0] == "localhost"):
        return False
    tail = prefix[boundary.start():]
    if tail[0] in " \t\v\f" + _LINES:
        pos = start + boundary.start()
        if original[offsets[pos]:offsets[pos + 1]] != tail[0]:
            return False
    return not (":" in tail or _opaque_token(tail)
                or any(ch in tail for ch in "\\{}[]\"'<>|"))


def _url_candidate_end(view: str, original: str, offsets: list[int] | range, start: int, end: int) -> int:
    first_at = view.find("@", start, end)
    if first_at < 0:
        return end
    gap = first_at + 1
    while gap < end:
        if view[gap] in " \t\v\f" + _LINES and original[offsets[gap]:offsets[gap + 1]] == view[gap]:
            tail = re.split(r"[/?#]", view[first_at + 1:gap], maxsplit=1)[0]
            return gap if _clear_host(tail) else end
        gap += 1
    return end


def _redact_url_credentials(text: str) -> str:
    view, offsets = _url_view(text)
    matches = list(_SCHEME.finditer(view))
    pieces: list[str] = []
    cursor = 0
    pending = -1
    for index, match in enumerate(matches):
        start = match.end()
        if offsets[start] < cursor:
            continue
        end = matches[index + 1].start() if index + 1 < len(matches) else len(view)
        end = _url_candidate_end(view, text, offsets, start, end)
        first_at = view.find("@", start, end)
        if first_at < 0:
            if not _clear_host(view[start:end]) and pending < 0 and not _public_url_prefix(view[start:end], text, offsets, start):
                pending = start
            continue
        last_at = view.rfind("@", start, end)
        if _public_url_prefix(view[start:last_at], text, offsets, start):
            pending = -1
            continue
        start = pending if pending >= 0 else start
        userinfo = view[start:last_at]
        host_tail = re.split(r"[/?#]", view[last_at + 1:end], maxsplit=1)[0]
        invalid = any(ch in _SPACE + '/[]{}"<>|' for ch in userinfo) or _opaque_token(host_tail.split(".")[0])
        finish = last_at + 1
        if invalid:
            while finish < end and view[finish] not in _SPACE + '/"\'<>':
                finish += 1
        pieces.extend((text[cursor:offsets[start]], "[redacted-credentials]" + ("" if invalid else "@")))
        cursor = offsets[finish]
        pending = -1
    pieces.append(text[cursor:])
    return "".join(pieces)


def _redact_bare_userinfo(text: str) -> str:
    view, offsets = _url_view(text)
    pieces = []
    cursor = 0
    start = 0
    while start < len(view):
        end = start
        while end < len(view) and not (view[end] in " \t\v\f" + _LINES and text[offsets[end]:offsets[end + 1]] == view[end]):
            end += 1
        token = view[start:end]
        if "://" not in token and "@" in token:
            head = re.match(r"[A-Za-z0-9+.-]+:[/\\]+", token)
            body = head.end() if head else 0
            at = token.rfind("@")
            colon = token.find(":", body)
            if (head or body < colon < at) and not any(ch in token[body:at] for ch in "/?#="):
                pieces.extend((text[cursor:offsets[start + body]], "[redacted-credentials]@"))
                cursor = offsets[start + at + 1]
        start = end + 1
    pieces.append(text[cursor:])
    return "".join(pieces)


def redact_url_credentials(text: str) -> str:
    return _redact_bare_userinfo(_redact_url_credentials(text))
