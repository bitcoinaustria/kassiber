type Span = [start: number, end: number, replacement: string];

const IDENTIFIER = /[A-Za-z0-9_.-]+/g;
const NAMES = new Set(`apikey authheader authorization blinding blindingkey cookie cookies
descriptor mnemonic passphrase password passwordhash rpcpassword rpcauth macaroon
macaroonhex recoveryphrase rune scankey scanprivate scanprivatekey spendkey secret
secretkey privatekey privkey seed seedphrase seedwords silentpayment silentpayments
silentpaymentdescriptor silentpaymentsdescriptor silentpaymentkey silentpaymentskey
silentpaymentmaterial silentpaymentsmaterial silentpaymentscan silentpaymentscan
silentpaymentscankey silentpaymentsscankey spdescriptor spscan spspend token xprv
accesstoken refreshtoken apitoken authtoken clientsecret dbpassword apisecret walletpassphrase wif rpcpass pass passwd pwd credentials auth user
credential sessionid sessionkey masterkey encryptionkey signingkey bearer pw pin otp`.split(/\s+/));
// URL delimiters establish credential context regardless of scheme spelling.
const SCHEME = /(?<![A-Za-z0-9+.-])[A-Za-z0-9+.-]+:\/\/|:\/\/|\/\//g;
// Non-global twin: exec on the shared global regex would move matchAll's start.
const FIRST_SCHEME = new RegExp(SCHEME.source);
const ORDERED_NAMES = [...NAMES].sort((a, b) => b.length - a.length || (a < b ? -1 : a > b ? 1 : 0));
const SPACE = " \t\r\n\v\f\u0085\u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000\ufeff";
const LINES = "\r\n\u0085\u2028\u2029";
const WEAK_NAMES = new Set(["pass", "user", "auth"]);
// Short names count only as a whole identifier part (`user_id`, not `author`);
// the shortest only as the final part (`db_pw`, not `spinner` or `pw_hash`).
const PART_NAMES = ["user", "auth"];
const LAST_PART_NAMES = ["pw", "pin", "otp"];
const COMMAND_LINE = /(?:^|(?<=[\r\n\u0085\u2028\u2029]))[ \t]*(?:[$>][ \t]+)?(?:cmd|kassiber|bitcoind|bitcoin-cli|electrs|lncli|lightning-cli|curl|python|node|env|sudo|docker)(?=[ \t]|$)/g;

function secretName(value: string): string {
  const versionless = value.replace(/[_-]v[0-9]+_*$/i, "");
  let first = 0;
  let last = versionless.length;
  while (first < last && "_.-".includes(versionless[first])) first += 1;
  while (last > first && "_.-".includes(versionless[last - 1])) last -= 1;
  const token = versionless.slice(first, last);
  const parts = token.split(/[_.-]+|(?<=[a-z0-9])(?=[A-Z])|(?<=[A-Z])(?=[A-Z][a-z])|(?<=[0-9])(?=[A-Za-z])/).map((part) => part.toLowerCase().replace(/^[0-9]+|[0-9]+$/g, ""));
  if (["hint", "statistics", "file", "stdin", "fd"].includes(parts.at(-1)!)) return "";
  const compact = parts.join("");
  return ORDERED_NAMES.find((name) => compact.includes(name) && (!PART_NAMES.includes(name) || parts.includes(name)) && (!LAST_PART_NAMES.includes(name) || name === parts.at(-1))) ?? "";
}

function quoteAt(text: string, pos: number): [string, number] {
  let end = pos;
  while (end < text.length && text[end] === "\\") end += 1;
  if (end < text.length && "\"'".includes(text[end])) {
    return [text.slice(pos, end + 1), end + 1];
  }
  return ["", pos];
}

function quotedEnd(text: string, start: number, delimiter: string, limit = text.length): [number, boolean] {
  let slashes = 0;
  let pos = start;
  const slashCount = delimiter.length - 1;
  const period = 2 * (slashCount + (slashCount > 0 && slashCount % 2 === 0 ? 2 : 1));
  let firstNewline = -1;
  while (pos < limit) {
    const ch = text[pos];
    if (LINES.includes(ch) && firstNewline < 0) firstNewline = pos;
    if (ch === delimiter.at(-1) && slashes % period === delimiter.length - 1) return [pos + 1, true];
    slashes = ch === "\\" ? slashes + 1 : 0;
    pos += 1;
  }
  return [yamlEnd(text, firstNewline >= 0 ? firstNewline : pos), false];
}

function valueBoundary(text: string, pos: number): boolean {
  while (pos < text.length) {
    if (")]}".includes(text[pos])) pos += 1;
    else {
      const [delimiter, body] = quoteAt(text, pos);
      if (!delimiter) break;
      pos = body;
    }
  }
  if (pos === text.length || SPACE.includes(text[pos])) return true;
  if (",;".includes(text[pos])) {
    pos += 1;
    const gap = pos;
    while (pos < text.length && SPACE.includes(text[pos]) && !LINES.includes(text[pos])) pos += 1;
    if (pos === text.length || LINES.includes(text[pos])) return true;
    const [delimiter, body] = quoteAt(text, pos);
    const identifier = /[A-Za-z0-9_.-]+/y;
    identifier.lastIndex = body;
    const key = delimiter ? identifier.exec(text) : null;
    if (key && text.startsWith(delimiter, body + key[0].length)) {
      let tail = body + key[0].length + delimiter.length;
      while (tail < text.length && SPACE.includes(text[tail])) tail += 1;
      return tail < text.length && ":=".includes(text[tail]);
    }
    identifier.lastIndex = pos;
    const prose = pos > gap && !delimiter ? identifier.exec(text) : null;
    if (prose && /^[a-z]+$/.test(prose[0])) {
      const end = pos + prose[0].length;
      return end === text.length || (SPACE + ".!?").includes(text[end]);
    }
  }
  return false;
}

function recordOpener(view: string, original: string, offset: (pos: number) => number, quote: number): boolean {
  if (quote < 0 || original.slice(offset(quote), offset(quote + 1)) !== view[quote]) return false;
  let pos = quote - 1;
  while (pos >= 0 && view[pos] === "\\") pos -= 1;
  while (pos >= 0 && SPACE.includes(view[pos])) pos -= 1;
  if (pos < 0 || !":=".includes(view[pos])) return false;
  pos -= 1;
  while (pos >= 0 && SPACE.includes(view[pos])) pos -= 1;
  if (pos < 0 || !"\"'".includes(view[pos]) || original.slice(offset(pos), offset(pos + 1)) !== view[pos]) return false;
  const delimiter = view[pos];
  pos -= 1;
  while (pos >= 0 && view[pos] === "\\") pos -= 1;
  const end = pos;
  while (pos >= 0 && /^[A-Za-z0-9_.-]$/.test(view[pos])) pos -= 1;
  return pos < end && pos >= 0 && view[pos] === delimiter && original.slice(offset(pos), offset(pos + 1)) === delimiter;
}

function urlRecordEnd(view: string, original: string, offset: (pos: number) => number, start: number, end: number, record: boolean): number {
  if (!record) return end;
  const quote = /[\\"']/.exec(view.slice(start, end));
  if (!quote) return end;
  const pos = start + quote.index;
  const [delimiter, body] = quoteAt(view, pos);
  if (!delimiter || original.slice(offset(pos), offset(body)) !== delimiter || body >= end || !",;".includes(view[body]) || !valueBoundary(view, body)) return end;
  const prefix = view.slice(start, pos);
  for (const candidate of [prefix, prefix.replace(/[).>\]]+$/, "")]) {
    if (clearHost(candidate) || publicUrlPrefix(candidate, original, offset, start)) return pos;
  }
  return end;
}

function* urlHeads(text: string): Generator<[number, number, boolean]> {
  // Preserve context even when a keyed pass masks a URL's original @.
  const [view, offsets] = urlView(text);
  const offset = (pos: number) => offsets ? offsets[pos] : pos;
  const matches = [...view.matchAll(SCHEME)];
  const finalAt = view.lastIndexOf("@");
  const quotes = view.matchAll(/["']/g);
  let quote = quotes.next().value;
  let previousQuote = -1;
  let checkedQuote = -1;
  let record = false;
  for (let index = 0; index < matches.length; index += 1) {
    const match = matches[index];
    while (quote && quote.index < match.index) {
      previousQuote = quote.index;
      quote = quotes.next().value;
    }
    if (previousQuote !== checkedQuote) {
      record = recordOpener(view, text, offset, previousQuote);
      checkedQuote = previousQuote;
    }
    const start = match.index + match[0].length;
    const segmentEnd = matches[index + 1]?.index ?? view.length;
    const limit = urlCandidateEnd(view, text, offset, start, urlRecordEnd(view, text, offset, start, segmentEnd, record));
    const at = view.slice(start, limit).lastIndexOf("@");
    const credentials = at >= 0 || (limit === segmentEnd && index + 1 < matches.length && finalAt >= limit);
    let end = at >= 0 ? start + at + 1 : limit;
    if (!credentials) {
      end = start;
      while (end < limit && !"\"'<>[]/{}()?#&|".includes(view[end]) && !(SPACE.includes(view[end]) && text.slice(offset(end), offset(end + 1)) === view[end])) end += 1;
    }
    yield [offset(start), offset(end), credentials];
  }
}

function lineEnd(text: string, start: number): number {
  let pos = start;
  while (pos < text.length && !LINES.includes(text[pos])) pos += 1;
  return pos;
}

function keyedSpans(text: string, offsets?: number[]): Span[] {
  const offset = (pos: number) => offsets ? offsets[pos] : pos;
  const spans: Span[] = [];
  let cursor = 0;
  const commands = text.matchAll(COMMAND_LINE);
  let command = commands.next().value;
  let commandEnd = command ? lineEnd(text, command.index + command[0].length) : -1;
  const heads = urlHeads(text);
  const noHead: [number, number, boolean] = [-1, -1, false];
  let [headStart, headEnd, credentialContext] = heads.next().value ?? noHead;
  let previous: RegExpExecArray | undefined;
  let lineStart = 0;
  let scanned = 0;
  for (const match of text.matchAll(IDENTIFIER)) {
    if (match.index < cursor) continue;
    const key = match[0];
    const before = previous;
    previous = match;
    while (command && commandEnd < match.index) {
      command = commands.next().value;
      commandEnd = command ? lineEnd(text, command.index + command[0].length) : -1;
    }
    const inCommand = command !== undefined && command.index <= match.index && match.index <= commandEnd;
    let name = secretName(key);
    if (["-u", "-b"].includes(key) || (inCommand && ["--user", "--cookie"].includes(key))) name = ["-b", "--cookie"].includes(key) ? "cookie" : "credentials";
    if (!name && before && [" ", "\t"].includes(text.slice(before.index + before[0].length, match.index))) {
      // Spaced names (`api key:`, `"Secret Key":`) still name a secret
      // when the second word changes what the first one means.
      name = secretName(before[0] + "_" + key);
      if (name === secretName(before[0])) name = "";
    }
    if (!name) continue;
    for (; scanned < match.index; scanned += 1) if (LINES.includes(text[scanned])) lineStart = scanned + 1;
    while (headEnd < match.index && headEnd >= 0) [headStart, headEnd, credentialContext] = heads.next().value ?? noHead;
    const inUrl = headStart <= match.index && match.index < headEnd;
    let pos = match.index + key.length;
    const [keyQuote, keyEnd] = quoteAt(text, pos);
    pos = keyEnd;
    const afterKey = pos;
    while (pos < text.length && SPACE.includes(text[pos])) pos += 1;
    let flag = false;
    let separator = "";
    if (pos < text.length && ":=".includes(text[pos])) {
      separator = text[pos];
      if (inUrl && (match.index === headStart || WEAK_NAMES.has(name)) && separator === ":" && pos + 1 < text.length && !SPACE.includes(text[pos + 1])) {
        if (credentialContext || /^[0-9]+$/.test(text.slice(pos + 1, headEnd))) continue;
      }
      pos += 1;
      if ((separator === "=" && text[pos] === ">") || (separator === ":" && text[pos] === "=")) {
        separator = "=";
        pos += 1;
      }
      while (pos < text.length && SPACE.includes(text[pos])) pos += 1;
    } else if (text.startsWith("->", pos)) {
      separator = "=";
      pos += 2;
      while (pos < text.length && " \t".includes(text[pos])) pos += 1;
    } else if ((key.startsWith("-") || inCommand) && pos > afterKey && flagValueGap(text, match.index, afterKey, pos)) {
      flag = true;
    } else continue;
    const start = pos;
    while (pos < text.length && ":=>".includes(text[pos]) && !(text[pos] === ">" && yamlMarker(text, pos))) {
      pos += 1;
      while (pos < text.length && " \t".includes(text[pos])) pos += 1;
    }
    if (pos === text.length) continue;
    let [delimiter, body] = quoteAt(text, pos);
    let end: number;
    let replacement: string;
    if (delimiter) {
      let closed: boolean;
      if (text.startsWith(delimiter.repeat(3), pos)) {
        delimiter = delimiter.repeat(3);
        body = pos + delimiter.length;
        const close = text.indexOf(delimiter, body);
        [end, closed] = close >= 0 ? [close + delimiter.length, true] : [yamlEnd(text, body), false];
      } else [end, closed] = quotedEnd(text, body, delimiter);
      if (closed && end < text.length && (!valueBoundary(text, end) || ambiguousTail(text, end))) {
        end = yamlEnd(text, end);
      }
      replacement = start === pos ? delimiter + "[redacted]" + (closed ? delimiter : "") : "[redacted]";
    } else {
      end = pos;
      const valueEnd = " \t\v\f\"'" + LINES;
      while (end < text.length && !valueEnd.includes(text[end])) end += 1;
      const token = text.slice(pos, end);
      // A plain value owns lines indented deeper than its key (a YAML
      // continuation), so neither exceptions nor masks stop at the first.
      const line = continuedEnd(text, pos, match.index - lineStart);
      const whole = text.slice(pos, line).replace(/^[ \t]+|[ \t]+$/g, "");
      if (separator === ":" && !keyQuote && (
        ["none", "expired", "(not set)"].includes(whole.toLowerCase())
        || (WEAK_NAMES.has(name) && aliasProse(name, whole))
        || (name === "pass" && /^[0-9]+$/.test(whole) && /(?:^|[^A-Za-z])tests[ \t]+$/.test(text.slice(Math.max(0, match.index - 64), match.index)))
      )) continue;
      if (flag && ((!inCommand && helpValue(whole)) || (name === "seedwords" && ["12", "15", "18", "21", "24"].includes(whole)))) continue;
      if (["{", "[", "("].includes(text[pos]) || text.startsWith("$(", pos) || text.startsWith("${", pos)) {
        end = balancedEnd(text, pos);
        if (text.slice(end, lineEnd(text, end)).replace(/[ \t]/g, "")) end = lineEnd(text, end);
      } else if (["|", "|-", "|+", ">", ">-", ">+"].includes(token) || "`<".includes(text[pos])) end = yamlEnd(text, pos);
      else end = line;
      const header = name === "authorization" || (["cookie", "cookies"].includes(name) && !keyQuote && !key.startsWith("-"));
      if (header) end = yamlEnd(text, pos);
      while (end > pos && " \t".includes(text[end - 1])) end -= 1;
      replacement = "[redacted]";
    }
    if (end <= start) continue;
    spans.push([offset(start), offset(end), replacement]);
    cursor = end;
  }
  return spans;
}

function urlView(original: string): [string, number[] | undefined] {
  if (!/[%\\&ſ﹫\uff01-\uff5e]/.test(original)) return [original, undefined];
  let text = original;
  let offsets = Array.from({ length: text.length + 1 }, (_, index) => index);
  for (let layer = 0; layer < 3; layer += 1) {
    const chars: string[] = [];
    const positions: number[] = [];
    let pos = 0;
    while (pos < text.length) {
      positions.push(offsets[pos]);
      const entity = text[pos] === "&" ? /^(?:&#([xX][0-9a-fA-F]{1,6}|[0-9]{1,7});|&(colon|sol|commat|quot|apos|lt|gt|amp|equals|Tab|NewLine);)/.exec(text.slice(pos, pos + 14)) : null;
      if (entity) {
        const code = entity[1] ? Number.parseInt(entity[1][0].toLowerCase() === "x" ? entity[1].slice(1) : entity[1], entity[1][0].toLowerCase() === "x" ? 16 : 10) : ({ colon: 58, sol: 47, commat: 64, quot: 34, apos: 39, lt: 60, gt: 62, amp: 38, equals: 61, Tab: 9, NewLine: 10 } as Record<string, number>)[entity[2]];
        chars.push(String.fromCodePoint(code <= 0x10ffff ? code : 0xfffd));
        // Supplementary characters occupy two UTF-16 offsets in the view.
        if (code > 0xffff && code <= 0x10ffff) positions.push(offsets[pos]);
        pos += entity[0].length;
      } else if (text[pos] === "%" && /^[0-9a-fA-F]{2}$/.test(text.slice(pos + 1, pos + 3))) {
        chars.push(String.fromCharCode(Number.parseInt(text.slice(pos + 1, pos + 3), 16)));
        pos += 3;
      } else if (text[pos] === "\\") {
        let end = pos;
        while (end < text.length && text[end] === "\\") end += 1;
        if (end < text.length && text[end] === "/") {
          chars.push("/");
          pos = end + 1;
        } else if (end < text.length && "ux".includes(text[end]) && end + (text[end] === "u" ? 5 : 3) <= text.length && /^[0-9a-fA-F]+$/.test(text.slice(end + 1, end + (text[end] === "u" ? 5 : 3)))) {
          const escapeEnd = end + (text[end] === "u" ? 5 : 3);
          chars.push(String.fromCharCode(Number.parseInt(text.slice(end + 1, escapeEnd), 16)));
          pos = escapeEnd;
        } else {
          while (pos < end) {
            chars.push(text[pos]);
            pos += 1;
            if (pos < end) positions.push(offsets[pos]);
          }
        }
      } else {
        const ch = text[pos];
        chars.push(ch >= "\uff01" && ch <= "\uff5e" ? String.fromCharCode(ch.charCodeAt(0) - 0xfee0) : ch === "ſ" ? "s" : ch === "﹫" ? "@" : ch);
        pos += 1;
      }
    }
    positions.push(offsets[offsets.length - 1]);
    const decoded = chars.join("");
    if (decoded === text) break;
    text = decoded;
    offsets = positions;
  }
  return [text, offsets];
}

function opaqueToken(text: string): boolean {
  if (/[A-Za-z0-9_]{10,}|[A-Z]{4,}/.test(text)) return true;
  return [...text.matchAll(/[A-Za-z0-9_]{6,}/g)].some((part) => /[A-Za-z]/.test(part[0]) && /[0-9]/.test(part[0]));
}

function strongOpaque(text: string): boolean {
  // Long lowercase words are ordinary paths; capitals and mixed runs are not.
  return /[A-Z]{4,}/.test(text) || [...text.matchAll(/[A-Za-z0-9_]{6,}/g)].some((part) => /[A-Za-z]/.test(part[0]) && /[0-9]/.test(part[0]));
}

function clearHost(authority: string): boolean {
  if (/^\[[0-9A-Fa-f:.%a-z]+\](?::[0-9]*)?$/.test(authority)) return true;
  const colon = authority.lastIndexOf(":");
  const host = colon < 0 ? authority : authority.slice(0, colon);
  const port = colon < 0 ? "" : authority.slice(colon + 1);
  if (colon >= 0 && !/^[0-9]+$/.test(port)) return false;
  return /^[A-Za-z0-9](?:[A-Za-z0-9.-]*[A-Za-z0-9])?$/.test(host) && !opaqueToken(host.split(".")[0]);
}

function publicUrlPrefix(prefix: string, original: string, offset: (pos: number) => number, start: number): boolean {
  const boundary = /[/?# \t\r\n\v\f\u0085\u2028\u2029]/.exec(prefix);
  if (!boundary || !clearHost(prefix.slice(0, boundary.index))) return false;
  const authority = prefix.slice(0, boundary.index);
  const tail = prefix.slice(boundary.index);
  // Opaque path words and the words before the later @ stay ambiguous; named
  // query values are left to the keyed scanner.
  const gap = /[ \t\v\f\r\n\u0085\u2028\u2029]/.exec(tail);
  const gapAt = start + boundary.index + (gap ? gap.index : 0);
  if (gap && "/?#".includes(tail[0]) && original.slice(offset(gapAt), offset(gapAt + 1)) === gap[0]) {
    const path = tail.slice(0, gap.index);
    const after = tail.slice(gap.index);
    const words = path.replace(/([?&;][A-Za-z_][A-Za-z0-9_.-]*=)[^&;#]*/g, "$1");
    return !(path.includes(":") || /[\\{}[\]"'<>|]/.test(path) || strongOpaque(words) || strongOpaque(after));
  }
  if ((" \t\v\f" + LINES).includes(tail[0])) {
    const pos = start + boundary.index;
    if (original.slice(offset(pos), offset(pos + 1)) !== tail[0]) return false;
  }
  if (authority.includes(":") && !authority.startsWith("[") && !("?#".includes(tail[0]) && (authority.includes(".") || authority.split(":", 1)[0] === "localhost")) && !(LINES.includes(tail[0]) && !opaqueToken(tail) && !tail.includes(":"))) return false;
  return !(tail.includes(":") || opaqueToken(tail) || /[\\{}[\]"'<>|]/.test(tail));
}

function urlCandidateEnd(view: string, original: string, offset: (pos: number) => number, start: number, end: number): number {
  const relativeAt = view.slice(start, end).indexOf("@");
  if (relativeAt < 0) return end;
  let lastAt = start + relativeAt;
  let checkedAt = -1;
  let gap = lastAt + 1;
  while (gap < end) {
    if (view[gap] === "@") lastAt = gap;
    else if (lastAt !== checkedAt && (" \t\v\f" + LINES).includes(view[gap]) && original.slice(offset(gap), offset(gap + 1)) === view[gap]) {
      checkedAt = lastAt;
      const tail = view.slice(lastAt + 1, gap).split(/[/?#]/, 1)[0];
      // An encoded @ before a bare word (`%40q (ZK…@h`) may be userinfo
      // data, unless a dotted/port host or ordinary words follow.
      const literal = original.slice(offset(lastAt), offset(lastAt + 1)) === "@";
      let following = gap;
      while (following < end && view[following] !== "@") following += 1;
      if (clearHost(tail) && (literal || tail.startsWith("[") || /[.:]/.test(tail) || !strongOpaque(view.slice(gap, following)))) return gap;
    }
    gap += 1;
  }
  return end;
}

function urlSpans(text: string): Span[] {
  const [view, offsets] = urlView(text);
  const offset = (pos: number) => offsets ? offsets[pos] : pos;
  const matches = [...view.matchAll(SCHEME)];
  const spans: Span[] = [];
  let cursor = 0;
  let pending = -1;
  const quotes = view.matchAll(/["']/g);
  let quote = quotes.next().value;
  let previousQuote = -1;
  let checkedQuote = -1;
  let record = false;
  for (let index = 0; index < matches.length; index += 1) {
    const match = matches[index];
    while (quote && quote.index < match.index) {
      previousQuote = quote.index;
      quote = quotes.next().value;
    }
    if (previousQuote !== checkedQuote) {
      record = recordOpener(view, text, offset, previousQuote);
      checkedQuote = previousQuote;
    }
    let start = match.index + match[0].length;
    if (offset(start) < cursor) continue;
    const end = urlCandidateEnd(view, text, offset, start, urlRecordEnd(view, text, offset, start, matches[index + 1]?.index ?? view.length, record));
    const relativeAt = view.slice(start, end).indexOf("@");
    const firstAt = relativeAt < 0 ? -1 : relativeAt + start;
    if (firstAt < 0) {
      if (!clearHost(view.slice(start, end)) && pending < 0 && !publicUrlPrefix(view.slice(start, end), text, offset, start)) pending = start;
      continue;
    }
    const lastAt = view.slice(start, end).lastIndexOf("@") + start;
    if (publicUrlPrefix(view.slice(start, lastAt), text, offset, start)) {
      pending = -1;
      // A public @handle path cannot license a `handle:password` pair.
      const handle = new RegExp(`^[^/?#${SPACE}]*`).exec(view.slice(lastAt + 1, end))![0];
      if (handle.includes(":") && !/^(?:[^:]*:[0-9]*|sha(?:1|256|384|512):[0-9A-Fa-f]+)$/.test(handle)) {
        spans.push([offset(lastAt + 1), offset(lastAt + 1 + handle.length), "[redacted-credentials]"]);
        cursor = offset(lastAt + 1 + handle.length);
      }
      continue;
    }
    if (pending >= 0) start = pending;
    const hostTail = view.slice(lastAt + 1, end).split(/[/?#]/, 1)[0];
    let invalid = false;
    for (let pos = start; !invalid && pos < lastAt; pos += 1) invalid = (SPACE + '/[]{}"<>|').includes(view[pos]);
    invalid ||= opaqueToken(hostTail.split(".")[0]);
    let finish = lastAt + 1;
    if (invalid) {
      while (finish < end && !(SPACE + '/"\'<>').includes(view[finish])) finish += 1;
    }
    spans.push([offset(start), offset(finish), "[redacted-credentials]" + (invalid ? "" : "@")]);
    cursor = offset(finish);
    pending = -1;
    // A delimiter glued to an unfinished authority (`a@b:#x//@h`) is more
    // userinfo, not a new URL: the next @ extends this span.
    let glued = index + 1 < matches.length && end === matches[index + 1].index && !clearHost(view.slice(finish, end).replace(/[,;|&]+$/, ""));
    for (let gap = finish; glued && gap < end; gap += 1) if (SPACE.includes(view[gap]) && text.slice(offset(gap), offset(gap + 1)) === view[gap]) glued = false;
    if (glued) pending = start;
  }
  return spans;
}

function helpValue(value: string): boolean {
  return /^(?:to set a value\.|instead\.|(?:flag|option) is (?:deprecated|available)\.|(?:to|instead|flag|option|none|missing|the))$/i.test(value);
}

function aliasProse(name: string, value: string): boolean {
  // Lowercase passwords remain credentials; exemptions are ordinary status/user prose.
  return (name === "user" ? ["alice", "bob", "root", "anonymous", "alice logged in"] : name === "auth" ? ["ok", "required", "enabled", "disabled", "none", "expired"] : []).includes(value);
}

function yamlMarker(text: string, pos: number): boolean {
  pos += 1;
  if (pos < text.length && "-+".includes(text[pos])) pos += 1;
  while (pos < text.length && SPACE.includes(text[pos]) && !LINES.includes(text[pos])) pos += 1;
  return pos === text.length || LINES.includes(text[pos]);
}

function balancedEnd(text: string, start: number): number {
  const stack: string[] = [];
  let limit = yamlEnd(text, start);
  const following = limit + (text.startsWith("\r\n", limit) ? 2 : 1);
  if (following < text.length && "}])".includes(text[following])) limit = lineEnd(text, following);
  let pos = start + (text.startsWith("$(", start) ? 1 : 0);
  while (pos < limit) {
    const ch = text[pos];
    if ("{[(".includes(ch)) stack.push(({ "{": "}", "[": "]", "(": ")" } as Record<string, string>)[ch]);
    else if ("}])".includes(ch)) {
      if (!stack.length || stack.at(-1) !== ch) return lineEnd(text, pos);
      stack.pop();
      if (!stack.length) return pos + 1;
    } else if ("\"'".includes(ch)) {
      let closed: boolean;
      [pos, closed] = quotedEnd(text, pos + 1, ch, limit);
      if (!closed) return limit;
      continue;
    }
    pos += 1;
  }
  return limit;
}

function yamlEnd(text: string, start: number): number {
  let end = lineEnd(text, start);
  while (end < text.length) {
    const pos = end + (text.startsWith("\r\n", end) ? 2 : 1);
    if (pos < text.length && !(" \t" + LINES).includes(text[pos])) break;
    end = lineEnd(text, pos);
  }
  return end;
}

function namedPairSpans(text: string, offsets?: number[]): Span[] {
  const offset = (pos: number) => offsets ? offsets[pos] : pos;
  const spans: Span[] = [];
  let cursor = 0;
  const pairs = /(?<![A-Za-z0-9_.-])["']?(?:name|key)["']?[ \t]*[:=][ \t]*["']?([A-Za-z0-9_.:-]+)["']?[, \t\r\n\v\f]*["']?value["']?[ \t]*[:=][ \t]*/gi;
  for (const pair of text.matchAll(pairs)) {
    if (pair.index < cursor || !secretName(pair[1])) continue;
    const start = pair.index + pair[0].length;
    const [delimiter, body] = quoteAt(text, start);
    let end: number;
    let replacement: string;
    if (delimiter) {
      const [finish, closed] = quotedEnd(text, body, delimiter);
      end = finish;
      replacement = delimiter + "[redacted]" + (closed ? delimiter : "");
    } else {
      end = yamlEnd(text, start);
      replacement = "[redacted]";
    }
    spans.push([offset(start), offset(end), replacement]);
    cursor = end;
  }
  return spans;
}

const VOID_TAGS = new Set("area base br col embed hr img input link meta param source track wbr".split(" "));

function xmlSpans(text: string, offsets?: number[]): Span[] {
  const offset = (pos: number) => offsets ? offsets[pos] : pos;
  const spans: Span[] = [];
  let cursor = 0;
  let plistSecret = false;
  for (const tag of text.matchAll(/<(\/?)([A-Za-z][A-Za-z0-9_.:-]*)(?![A-Za-z0-9_.:-])([^<>]*)>/g)) {
    if (tag.index < cursor || tag[1]) continue;
    const key = tag[2];
    if (/^[:-]/.test(tag[3].trimStart())) continue;
    let name = secretName(key.split(":").at(-1)!);
    const attrs = [...tag[3].matchAll(/(?<![A-Za-z0-9_.:-])([A-Za-z_:][A-Za-z0-9_.:-]*)[ \t\r\n\v\f]*=[ \t\r\n\v\f]*(?:"([^"]*)"|'([^']*)'|([^ \t\r\n\v\f>]+))/g)].map((attr) => {
      const group = [2, 3, 4].find((i) => attr[i] !== undefined)!;
      const end = attr.index + attr[0].length - (group === 4 ? 0 : 1);
      return { key: attr[1].split(":").at(-1)!.toLowerCase(), value: attr[group], start: end - attr[group].length, end };
    });
    let keyed = attrs.some((a) => ["key", "name", "type"].includes(a.key) && secretName(a.value));
    if (tag[3].startsWith("=")) {
      const selector = /^=["']([^"']+)["']/.exec(tag[3]);
      keyed ||= !!selector && !!secretName(selector[1]);
    }
    if (keyed || name) {
      for (const attr of attrs) {
        if (!["value", "content"].includes(attr.key) || (!keyed && WEAK_NAMES.has(name) && aliasProse(name, attr.value))) continue;
        const base = tag.index + 1 + key.length;
        spans.push([offset(base + attr.start), offset(base + attr.end), "[redacted]"]);
        cursor = base + attr.end;
      }
    }
    const start = tag.index + tag[0].length;
    const closing = "</" + key;
    if (key === "key") {
      const end = text.indexOf(closing, start);
      plistSecret = end >= 0 && !!secretName(text.slice(start, end).trim());
      continue;
    }
    name ||= plistSecret ? "secret" : "";
    plistSecret = false;
    if ((!name && !keyed) || tag[3].trimEnd().endsWith("/")) continue;
    let end = text.indexOf(closing, start);
    while (end >= 0) {
      let tail = end + closing.length;
      while (tail < text.length && " \t\r\n\v\f".includes(text[tail])) tail += 1;
      if (tail < text.length && text[tail] === ">") break;
      end = text.indexOf(closing, tail);
    }
    if (end < 0) {
      // HTML void elements such as <input> and <meta> have no body.
      if (VOID_TAGS.has(key.toLowerCase())) continue;
      end = yamlEnd(text, start);
    }
    const value = text.slice(start, end);
    if (!keyed && WEAK_NAMES.has(name) && aliasProse(name, value.trim())) continue;
    spans.push([offset(start), offset(end), "[redacted]"]);
    cursor = end;
  }
  return spans;
}

function cliSpans(text: string, offsets?: number[]): Span[] {
  const offset = (pos: number) => offsets ? offsets[pos] : pos;
  const spans: Span[] = [];
  let cursor = 0;
  const commands = /(?<![A-Za-z0-9_])(?:curl[ \t]+-u[ \t]*|sshpass[ \t]+-p[ \t]*|mysql[ \t]+-u[ \t]+[^ \t\r\n]+[ \t]+-p)/g;
  for (const match of text.matchAll(commands)) {
    if (match.index < cursor) continue;
    const start = match.index + match[0].length;
    const [delimiter, body] = quoteAt(text, start);
    let end: number;
    let replacement: string;
    if (delimiter) {
      const [finish, closed] = quotedEnd(text, body, delimiter);
      end = finish;
      replacement = delimiter + "[redacted]" + (closed ? delimiter : "");
    } else {
      end = lineEnd(text, start);
      replacement = "[redacted]";
    }
    spans.push([offset(start), offset(end), replacement]);
    cursor = end;
  }
  return spans;
}


function continuedEnd(text: string, pos: number, column: number): number {
  let end = lineEnd(text, pos);
  while (end < text.length) {
    const line = end + (text.startsWith("\r\n", end) ? 2 : 1);
    let indent = line;
    while (indent < text.length && " \t".includes(text[indent])) indent += 1;
    if (indent - line <= column || indent === text.length || LINES.includes(text[indent])) break;
    end = lineEnd(text, indent);
  }
  return end;
}

function flagValueGap(text: string, keyStart: number, afterKey: number, pos: number): boolean {
  if (![...text.slice(afterKey, pos)].some((ch) => LINES.includes(ch))) return true;
  let line = keyStart;
  while (line > 0 && !LINES.includes(text[line - 1])) line -= 1;
  return [...text.slice(line, keyStart)].every((ch) => " \t#>$.-".includes(ch));
}

function ambiguousTail(text: string, end: number): boolean {
  let pos = end;
  while (pos < text.length && " \t".includes(text[pos])) pos += 1;
  if (pos === end || pos === text.length || LINES.includes(text[pos])) return false;
  let tail = pos;
  while (tail < text.length && !SPACE.includes(text[tail])) tail += 1;
  return !/^(?:[a-z]+[.,!?]?|-{1,2}[a-z][a-z-]*|[a-z0-9_.-]+[:=].*)$/.test(text.slice(pos, tail));
}

function bareUserinfoSpans(text: string): Span[] {
  const [view, offsets] = urlView(text);
  const offset = (pos: number) => offsets ? offsets[pos] : pos;
  const spans: Span[] = [];
  let start = 0;
  while (start < view.length) {
    let end = start;
    while (end < view.length && !((" \t\v\f" + LINES).includes(view[end]) && text.slice(offset(end), offset(end + 1)) === view[end])) end += 1;
    // Userinfo before a later scheme is still `token:token@host`.
    const scheme = FIRST_SCHEME.exec(view.slice(start, end));
    const token = view.slice(start, scheme ? start + scheme.index : end);
    if (token.includes("@")) {
      const at = token.lastIndexOf("@");
      const pair = /[^:=<>"'({[\]@]+:/.exec(token);
      if (pair && pair.index + pair[0].length - 1 < at && !secretName(pair[0].slice(0, -1))
          && !(["mailto:", "xmpp:", "sip:", "sips:"].includes(pair[0].toLowerCase()) && !token.slice(pair.index + pair[0].length, at).includes(":"))) {
        spans.push([offset(start + pair.index), offset(start + at + 1), "[redacted-credentials]@"]);
      }
    }
    start = end + 1;
  }
  return spans;
}

export function redactCredentials(text: string): string {
  // Collect against the original record, so no detector can erase the context
  // needed by another. A fixed number of ordered streams keeps merging linear.
  const [view, offsets] = urlView(text);
  const streams = [urlSpans(text), bareUserinfoSpans(text)];
  for (const scan of [xmlSpans, namedPairSpans, cliSpans, keyedSpans]) {
    streams.push(scan(text));
    if (view !== text) streams.push(scan(view, offsets));
  }
  const indices = streams.map(() => 0);
  const pieces: string[] = [];
  let cursor = 0;
  let pending: Span | undefined;
  for (;;) {
    let selected = -1;
    let next: Span | undefined;
    for (let i = 0; i < streams.length; i += 1) {
      const span = streams[i][indices[i]];
      if (span && (!next || span[0] < next[0] || (span[0] === next[0] && span[1] > next[1]))) {
        selected = i;
        next = span;
      }
    }
    if (!next) break;
    indices[selected] += 1;
    if (pending && next[0] < pending[1]) {
      if (next[1] > pending[1]) pending = [pending[0], next[1], "[redacted]"];
      continue;
    }
    if (pending) {
      pieces.push(text.slice(cursor, pending[0]), pending[2]);
      cursor = pending[1];
    }
    pending = next;
  }
  if (pending) {
    pieces.push(text.slice(cursor, pending[0]), pending[2]);
    cursor = pending[1];
  }
  pieces.push(text.slice(cursor));
  return pieces.join("");
}
