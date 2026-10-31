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
accesstoken authtoken clientsecret wif rpcpass pass pwd credentials auth user""".split())
_PROSE = frozenset("""to instead flag option when requires expected missing with
without is are was were has have the a an and or for from of in on set use words
value values argument arguments deprecated reset rotated invalid expired none""".split())
_RECOVERY = frozenset(("mnemonic", "recoveryphrase", "seed", "seedphrase", "seedwords"))
_SCHEME = re.compile(r"\b(?:https?|tcp|ssl|socks5h?|wss?|ftp|postgres|electrum)://", re.IGNORECASE)
_SPACE = " \t\r\n\v\f\u0085\u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000\ufeff"
_VALUE_END = _SPACE + ",;{}&|"
_COMMAND_LINE = re.compile(r"^[ \t]*(?:[$>][ \t]+)?(?:cmd|kassiber|bitcoind|bitcoin-cli|electrs|lncli|lightning-cli|curl|python|node|env|sudo|docker)(?=[ \t]|$)", re.MULTILINE)


def _secret_name(token: str) -> str:
    token = _VERSION.sub("", token).strip("_.-").lower()
    parts = _KEY_PARTS.split(token)
    for count in range(1, min(4, len(parts)) + 1):
        name = "".join(parts[-count:]).lstrip("0123456789")
        if name.rstrip("0123456789") in _NAMES:
            return name.rstrip("0123456789")
    return ""


def _quote_at(text: str, pos: int) -> tuple[str, int]:
    end = pos
    while end < len(text) and text[end] == "\\":
        end += 1
    if end < len(text) and text[end] in "\"'" and end - pos in (0, 1, 2, 3, 6, 7, 14):
        return text[pos:end + 1], end + 1
    return "", pos


def _quoted_end(text: str, pos: int, delimiter: str) -> tuple[int, bool]:
    slashes = 0
    slash_count = len(delimiter) - 1
    period = 2 * (slash_count + (2 if slash_count and slash_count % 2 == 0 else 1))
    first_newline = -1
    while pos < len(text):
        ch = text[pos]
        if ch in "\r\n" and first_newline < 0:
            first_newline = pos
        if ch == delimiter[-1] and slashes % period == len(delimiter) - 1:
            return pos + 1, True
        slashes = slashes + 1 if ch == "\\" else 0
        pos += 1
    return (first_newline if first_newline >= 0 else pos), False


def _url_heads(text: str):
    scanned = 0
    for match in _SCHEME.finditer(text):
        if match.start() < scanned:
            continue
        end = match.end()
        while end < len(text) and text[end] not in _SPACE + "\"'<>[]/{}()&|":
            end += 1
        scanned = end
        yield match.end(), end


def redact_keyed_secrets(text: str) -> str:
    pieces: list[str] = []
    cursor = 0
    commands = iter(_COMMAND_LINE.finditer(text))
    command = next(commands, None)
    command_end = text.find("\n", command.end()) if command else -1
    if command and command_end < 0:
        command_end = len(text)
    heads = iter(_url_heads(text))
    head_start, head_end = next(heads, (-1, -1))
    for match in _IDENTIFIER.finditer(text):
        if match.start() < cursor:
            continue
        key = match.group()
        while command and command_end < match.start():
            command = next(commands, None)
            command_end = text.find("\n", command.end()) if command else -1
            if command and command_end < 0:
                command_end = len(text)
        in_command = command is not None and command.start() <= match.start() <= command_end
        name = _secret_name(key)
        if not name:
            continue
        while head_end < match.start() and head_end >= 0:
            head_start, head_end = next(heads, (-1, -1))
        in_url = head_start <= match.start() < head_end
        pos = match.end()
        _, pos = _quote_at(text, pos)
        after_key = pos
        while pos < len(text) and text[pos] in _SPACE:
            pos += 1
        flag = False
        separator = ""
        if pos < len(text) and text[pos] in ":=":
            separator = text[pos]
            if in_url and separator == ":" and pos + 1 < len(text) and text[pos + 1] not in _SPACE:
                continue
            pos += 1
            while pos < len(text) and text[pos] in _SPACE:
                pos += 1
        elif (key.startswith("-") or in_command) and pos > after_key and "\n" not in text[after_key:pos] and "\r" not in text[after_key:pos]:
            flag = True
        else:
            continue
        start = pos
        if pos == len(text) or text[pos] in _VALUE_END or (flag and text[pos] == "-"):
            continue
        delimiter, body = _quote_at(text, pos)
        if delimiter:
            end, closed = _quoted_end(text, body, delimiter)
            if closed and end < len(text) and text[end] not in _VALUE_END + "])":
                while end < len(text) and text[end] not in "\r\n":
                    end += 1
            replacement = delimiter + "[redacted]" + (delimiter if closed else "")
        else:
            end = pos
            value_end = (_SPACE + "\"'<>[]/{}()" if in_url else _VALUE_END + "\"'")
            while end < len(text) and text[end] not in value_end:
                end += 1
            token = text[pos:end]
            if separator == ":" and (token.lower() in ("none", "expired") or text.startswith("(not set)", pos)):
                continue
            if flag and ((not in_command and (token.lower() in _PROSE or token.endswith(tuple(".,;!?:"))))
                         or (name == "seedwords" and token in ("12", "15", "18", "21", "24"))):
                continue
            if name == "authorization" or name in _RECOVERY:
                while end < len(text) and text[end] not in ("\r\n" if name == "authorization" else ",;}{&|\r\n\"'"):
                    next_key = _IDENTIFIER.match(text, end) if name != "authorization" else None
                    if next_key and name != "authorization":
                        tail = next_key.end()
                        while tail < len(text) and text[tail] in " \t":
                            tail += 1
                        if tail < len(text) and text[tail] in ":=":
                            break
                        end = next_key.end()
                    else:
                        end += 1
                while end > pos and text[end - 1] in " \t":
                    end -= 1
            replacement = "[redacted]"
            if end + 1 < len(text) and text[end] in "\"'" and text[end + 1] not in _VALUE_END + "])":
                while end < len(text) and text[end] not in "\r\n":
                    end += 1
        if end <= start:
            continue
        pieces.extend((text[cursor:start], replacement))
        cursor = end
    pieces.append(text[cursor:])
    return "".join(pieces)


def redact_url_credentials(text: str) -> str:
    pieces: list[str] = []
    cursor = scanned = 0
    for match in _SCHEME.finditer(text):
        if match.start() < scanned:
            continue
        start = pos = match.end()
        last_at = -1
        invalid = False
        while pos < len(text) and text[pos] not in _SPACE + "/[]{}":
            ch = text[pos]
            if ch in "\"'<>()&|":
                if pos + 1 == len(text) or text[pos + 1] in ",;}:" + _SPACE:
                    break
                invalid = True
            if ch == "@":
                last_at = pos
            pos += 1
        scanned = pos
        if pos < len(text) and text[pos] == "/":
            token_end = pos
            while token_end < len(text) and text[token_end] not in _SPACE + "\"'<>[]{}()&|":
                token_end += 1
            scanned = token_end
            prefix = text[start:pos]
            if last_at < 0 and ":" in prefix:
                password = prefix.split(":", 1)[1]
                if password and not re.fullmatch(r"[0-9]+", password):
                    tail_at = text.rfind("@", pos, token_end)
                    if tail_at >= 0:
                        last_at = tail_at
                        invalid = True
                        pos = token_end
        elif pos < len(text) and text[pos] in "[]()&|" and pos > start and last_at < 0:
            token_end = pos
            while token_end < len(text) and text[token_end] not in _SPACE + "\"'<>{}":
                token_end += 1
            tail_at = text.rfind("@", pos, token_end)
            scanned = token_end
            if tail_at >= 0:
                last_at = tail_at
                invalid = True
                pos = token_end
        if last_at < 0:
            continue
        end = pos if invalid else last_at + 1
        pieces.extend((text[cursor:start], "[redacted-credentials]" + ("" if invalid else "@")))
        cursor = end
    pieces.append(text[cursor:])
    return "".join(pieces)
