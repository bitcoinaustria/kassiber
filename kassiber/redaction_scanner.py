"""Forward-only credential scanners, mirrored by the webview secretScanner.ts."""

from __future__ import annotations

import re


_IDENTIFIER = re.compile(r"[A-Za-z0-9_.-]+")
_KEY_PARTS = re.compile(r"[_.-]+")
_VERSION = re.compile(r"[_-]v[0-9]+_*$", re.IGNORECASE)
_NAMES = frozenset("""apikey authheader authorization blinding blindingkey cookie
descriptor mnemonic passphrase password passwordhash rpcpassword rpcauth macaroon
macaroonhex recoveryphrase rune scankey scanprivate scanprivatekey spendkey secret
secretkey privatekey privkey seed seedphrase seedwords silentpayment silentpayments
silentpaymentdescriptor silentpaymentsdescriptor silentpaymentkey silentpaymentskey
silentpaymentmaterial silentpaymentsmaterial silentpaymentscan silentpaymentscan
silentpaymentscankey silentpaymentsscankey spdescriptor spscan spspend token xprv
accesstoken refreshtoken apitoken authtoken clientsecret dbpassword apisecret walletpassphrase wif rpcpass pass pwd credentials auth user""".split())
_PROSE = frozenset("""to instead flag option when requires expected missing with
without is are was were has have the a an and or for from of in on set use words
value values argument arguments deprecated reset rotated invalid expired none""".split())
_RECOVERY = frozenset(("mnemonic", "recoveryphrase", "seed", "seedphrase", "seedwords"))
_SCHEME = re.compile(r"(?<![A-Za-z0-9+.-])[A-Za-z][A-Za-z0-9+.-]*://")
_SPACE = " \t\r\n\v\f\u0085\u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000\ufeff"
_LINES = "\r\n\u2028\u2029"
_WEAK_NAMES = frozenset(("pass", "user", "auth"))
_COMMAND_LINE = re.compile(r"(?:^|(?<=[\r\n\u2028\u2029]))[ \t]*(?:[$>][ \t]+)?(?:cmd|kassiber|bitcoind|bitcoin-cli|electrs|lncli|lightning-cli|curl|python|node|env|sudo|docker)(?=[ \t]|$)", re.ASCII)


def _secret_name(token: str) -> str:
    token = _VERSION.sub("", token).strip("_.-").lower()
    parts = _KEY_PARTS.split(token)
    for count in range(min(4, len(parts)), 0, -1):
        name = "".join(parts[-count:]).lstrip("0123456789")
        if name.rstrip("0123456789") in _NAMES:
            return name.rstrip("0123456789")
    return ""


def _quote_at(text: str, pos: int) -> tuple[str, int]:
    end = pos
    while end < len(text) and text[end] == "\\":
        end += 1
    if end < len(text) and text[end] in "\"'":
        return text[pos:end + 1], end + 1
    return "", pos


def _quoted_end(text: str, pos: int, delimiter: str) -> tuple[int, bool]:
    slashes = 0
    slash_count = len(delimiter) - 1
    period = 2 * (slash_count + (2 if slash_count and slash_count % 2 == 0 else 1))
    first_newline = -1
    while pos < len(text):
        ch = text[pos]
        if ch in _LINES and first_newline < 0:
            first_newline = pos
        if ch == delimiter[-1] and slashes % period == len(delimiter) - 1:
            return pos + 1, True
        slashes = slashes + 1 if ch == "\\" else 0
        pos += 1
    return (first_newline if first_newline >= 0 else pos), False


def _value_boundary(text: str, pos: int) -> bool:
    if pos == len(text) or text[pos] in _SPACE + ")]}":
        return True
    if text[pos] in ",;":
        return pos + 1 == len(text) or text[pos + 1] in _SPACE + "\\\"'"
    return False


def _url_heads(text: str):
    # Preserve context even when a keyed pass masks a URL's original @.
    view, offsets = _url_view(text)
    for match in _SCHEME.finditer(view):
        end = match.end()
        while end < len(view) and view[end] not in _SPACE + "\"'<>[]/{}()?&|":
            end += 1
        yield offsets[match.end()], offsets[end]


def _line_end(text: str, pos: int) -> int:
    while pos < len(text) and text[pos] not in _LINES:
        pos += 1
    return pos


def redact_keyed_secrets(text: str) -> str:
    pieces: list[str] = []
    cursor = 0
    commands = iter(_COMMAND_LINE.finditer(text))
    command = next(commands, None)
    command_end = _line_end(text, command.end()) if command else -1
    heads = iter(_url_heads(text))
    head_start, head_end = next(heads, (-1, -1))
    for match in _IDENTIFIER.finditer(text):
        if match.start() < cursor:
            continue
        key = match.group()
        while command and command_end < match.start():
            command = next(commands, None)
            command_end = _line_end(text, command.end()) if command else -1
        in_command = command is not None and command.start() <= match.start() <= command_end
        name = _secret_name(key)
        if not name:
            continue
        while head_end < match.start() and head_end >= 0:
            head_start, head_end = next(heads, (-1, -1))
        in_url = head_start <= match.start() < head_end
        pos = match.end()
        if match.start() > 0 and text[match.start() - 1] == "<" and pos < len(text) and text[pos] == ">":
            start = pos + 1
            end = text.find("</" + key + ">", start)
            if end < 0:
                end = len(text)
            pieces.extend((text[cursor:start], "[redacted]"))
            cursor = end
            continue
        key_quote, pos = _quote_at(text, pos)
        after_key = pos
        while pos < len(text) and text[pos] in _SPACE:
            pos += 1
        flag = False
        separator = ""
        if pos < len(text) and text[pos] in ":=":
            separator = text[pos]
            if in_url and match.start() == head_start and separator == ":" and pos + 1 < len(text) and text[pos + 1] not in _SPACE:
                continue
            pos += 1
            if pos < len(text) and text[pos] in "=>":
                pos += 1
            while pos < len(text) and text[pos] in _SPACE:
                pos += 1
            if name in _WEAK_NAMES and separator == ":" and not key_quote:
                before = match.start() - 1
                while before >= 0 and text[before] in " \t":
                    before -= 1
                if before >= 0 and text[before] not in _LINES + "{[,;|":
                    continue
        elif (key.startswith("-") or in_command) and pos > after_key and not any(ch in _LINES for ch in text[after_key:pos]):
            flag = True
        else:
            continue
        start = pos
        if pos == len(text):
            continue
        delimiter, body = _quote_at(text, pos)
        if delimiter:
            end, closed = _quoted_end(text, body, delimiter)
            if closed and end < len(text) and not _value_boundary(text, end):
                while end < len(text) and text[end] not in _LINES:
                    end += 1
            replacement = delimiter + "[redacted]" + (delimiter if closed else "")
        else:
            end = pos
            value_end = " \t\r\n\v\f\u2028\u2029\"'"
            while end < len(text) and text[end] not in value_end:
                end += 1
            token = text[pos:end]
            if name in _WEAK_NAMES and separator == ":" and not key_quote and re.fullmatch(r"[a-z]+", token):
                continue
            if separator == ":" and (token.lower() in ("none", "expired") or text.startswith("(not set)", pos)):
                continue
            if flag and ((not in_command and (end == len(text) or text[end] in _SPACE) and (token.lower() in _PROSE or re.fullmatch(r"[a-z]+[.,;!?:]", token)))
                         or (name == "seedwords" and token in ("12", "15", "18", "21", "24"))):
                continue
            header = name == "authorization" or (name == "cookie" and separator == ":" and not flag and not key_quote)
            if header or name in _RECOVERY:
                while end < len(text) and text[end] not in (_LINES if header else ",;}{&|" + _LINES + "\"'"):
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
        pieces.extend((text[cursor:start], replacement))
        cursor = end
    pieces.append(text[cursor:])
    return "".join(pieces)


def _url_view(text: str) -> tuple[str, list[int] | range]:
    """Decode URL syntax only, retaining an offset for every original text span.

    Three percent layers are a fixed linear envelope. JSON slash escape runs
    need no depth cap: each run is consumed once, without decoding whole JSON.
    """
    if "%" not in text and "\\/" not in text:
        return text, range(len(text) + 1)
    offsets = list(range(len(text) + 1))
    for _ in range(3):
        chars: list[str] = []
        positions: list[int] = []
        pos = 0
        while pos < len(text):
            positions.append(offsets[pos])
            if text[pos] == "%" and pos + 2 < len(text) and all(c in "0123456789abcdefABCDEF" for c in text[pos + 1:pos + 3]):
                chars.append(chr(int(text[pos + 1:pos + 3], 16)))
                pos += 3
            elif text[pos] == "\\":
                end = pos
                while end < len(text) and text[end] == "\\":
                    end += 1
                if end < len(text) and text[end] == "/":
                    chars.append("/")
                    pos = end + 1
                else:
                    chars.extend(text[pos:end])
                    positions.extend(offsets[pos + 1:end])
                    pos = end
            else:
                chars.append(text[pos])
                pos += 1
        positions.append(offsets[-1])
        decoded = "".join(chars)
        if decoded == text:
            break
        text, offsets = decoded, positions
    return text, offsets


def redact_url_credentials(text: str) -> str:
    view, offsets = _url_view(text)
    matches = list(_SCHEME.finditer(view))
    pieces: list[str] = []
    cursor = 0
    pending = -1
    token_end = 0
    for index, match in enumerate(matches):
        start = match.end()
        if offsets[start] < cursor:
            continue
        if start >= token_end:
            pending = -1
            token_end = start
            # CR/LF and non-ASCII whitespace may be malformed userinfo.
            while token_end < len(view) and view[token_end] not in " \t\v\f":
                token_end += 1
        end = min(token_end, matches[index + 1].start() if index + 1 < len(matches) else len(view))
        last_at = view.rfind("@", start, end)
        if last_at < 0:
            # A slash inside a nonnumeric password is ambiguous. Carry the
            # start to the next bounded candidate, without rescanning a suffix.
            prefix = view[start:end].split("/", 1)[0]
            if ":" in prefix:
                password = prefix.split(":", 1)[1]
                if password and not re.fullmatch(r"[0-9]+", password) and pending < 0:
                    pending = start
            continue
        start = pending if pending >= 0 else start
        userinfo = view[start:last_at]
        invalid = any(ch in _SPACE + '/[]{}"<>|' for ch in userinfo)
        finish = last_at + 1
        if invalid:
            while finish < end and view[finish] not in _SPACE + '/"\'<>':
                finish += 1
        pieces.extend((text[cursor:offsets[start]], "[redacted-credentials]" + ("" if invalid else "@")))
        cursor = offsets[finish]
        pending = -1
    pieces.append(text[cursor:])
    return "".join(pieces)
