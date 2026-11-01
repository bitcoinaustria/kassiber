const IDENTIFIER = /[A-Za-z0-9_.-]+/g;
const NAMES = new Set(`apikey authheader authorization blinding blindingkey cookie cookies
descriptor mnemonic passphrase password passwordhash rpcpassword rpcauth macaroon
macaroonhex recoveryphrase rune scankey scanprivate scanprivatekey spendkey secret
secretkey privatekey privkey seed seedphrase seedwords silentpayment silentpayments
silentpaymentdescriptor silentpaymentsdescriptor silentpaymentkey silentpaymentskey
silentpaymentmaterial silentpaymentsmaterial silentpaymentscan silentpaymentscan
silentpaymentscankey silentpaymentsscankey spdescriptor spscan spspend token xprv
accesstoken refreshtoken apitoken authtoken clientsecret dbpassword apisecret walletpassphrase wif rpcpass pass passwd pwd credentials auth user`.split(/\s+/));
const PROSE = new Set(`to instead flag option when requires expected missing with
without is are was were has have the a an and or for from of in on set use words
value values argument arguments deprecated reset rotated invalid expired none`.split(/\s+/));
const RECOVERY = new Set(["mnemonic", "recoveryphrase", "seed", "seedphrase", "seedwords"]);
const SCHEME = /(?<![A-Za-z0-9+.-])[A-Za-z0-9+.-]+:\/\/|(?<![^ \t\r\n\v\f=({[<>])\/\//g;
const SPACE = " \t\r\n\v\f\u0085\u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000\ufeff";
const LINES = "\r\n\u0085\u2028\u2029";
const WEAK_NAMES = new Set(["pass", "user", "auth"]);
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
  for (let end = parts.length; end > 0; end -= 1) {
    for (let count = Math.min(4, end); count > 0; count -= 1) {
      const name = parts.slice(end - count, end).join("");
      if (NAMES.has(name)) return name;
    }
  }
  return "";
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
  return [firstNewline >= 0 ? firstNewline : pos, false];
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

function redactKeyedPlain(text: string, original = text, offsets?: number[]): string {
  const offset = (pos: number) => offsets ? offsets[pos] : pos;
  const pieces: string[] = [];
  let cursor = 0;
  const nextIdentifier = /[A-Za-z0-9_.-]+/y;
  const commands = text.matchAll(COMMAND_LINE);
  let command = commands.next().value;
  let commandEnd = command ? lineEnd(text, command.index + command[0].length) : -1;
  const heads = urlHeads(text);
  const noHead: [number, number, boolean] = [-1, -1, false];
  let [headStart, headEnd, credentialContext] = heads.next().value ?? noHead;
  for (const match of text.matchAll(IDENTIFIER)) {
    if (match.index < cursor) continue;
    const key = match[0];
    while (command && commandEnd < match.index) {
      command = commands.next().value;
      commandEnd = command ? lineEnd(text, command.index + command[0].length) : -1;
    }
    const inCommand = command !== undefined && command.index <= match.index && match.index <= commandEnd;
    let name = secretName(key);
    if (["-u", "-b"].includes(key) || (inCommand && ["--user", "--cookie"].includes(key))) name = ["-b", "--cookie"].includes(key) ? "cookie" : "credentials";
    if (!name) continue;
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
        [end, closed] = close >= 0 ? [close + delimiter.length, true] : [lineEnd(text, body), false];
      } else [end, closed] = quotedEnd(text, body, delimiter);
      if (closed && end < text.length && (!valueBoundary(text, end) || ambiguousTail(text, end))) {
        while (end < text.length && !LINES.includes(text[end])) end += 1;
      }
      replacement = start === pos ? delimiter + "[redacted]" + (closed ? delimiter : "") : "[redacted]";
    } else {
      end = pos;
      const valueEnd = " \t\v\f\"'" + LINES;
      while (end < text.length && !valueEnd.includes(text[end])) end += 1;
      const token = text.slice(pos, end);
      if (WEAK_NAMES.has(name) && separator === ":" && !keyQuote && pos > 0 && SPACE.includes(text[pos - 1]) && !ambiguousTail(text, end) && (aliasProse(name, token) || (name === "pass" && /^[0-9]+$/.test(token) && /(?:^|[^A-Za-z])tests[ \t]+$/.test(text.slice(Math.max(0, match.index - 64), match.index))))) continue;
      if (separator === ":" && (["none", "expired"].includes(token.toLowerCase()) || text.startsWith("(not set)", pos))) continue;
      if (flag && ((!inCommand && (end === text.length || SPACE.includes(text[end])) && (PROSE.has(token.toLowerCase()) || /^[a-z]+[.,;!?:]$/.test(token)))
          || (name === "seedwords" && ["12", "15", "18", "21", "24"].includes(token)))) continue;
      if (["{", "["].includes(text[pos]) || text.startsWith("$(", pos)) {
        end = balancedEnd(text, pos);
        if (end < text.length && !SPACE.includes(text[end])) end = lineEnd(text, end);
      }
      else if (["|", "|-", "|+", ">", ">-", ">+"].includes(token)) end = yamlEnd(text, pos);
      const header = name === "authorization" || (["cookie", "cookies"].includes(name) && !keyQuote && !key.startsWith("-"));
      if (header || RECOVERY.has(name)) {
        while (end < text.length && !(header ? LINES + (name !== "authorization" && separator === "=" ? "\"'" : "") : ",;}{&|" + LINES + "\"'").includes(text[end])) {
          nextIdentifier.lastIndex = end;
          const nextKey = !header ? nextIdentifier.exec(text) : null;
          if (nextKey) {
            let tail = end + nextKey[0].length;
            while (tail < text.length && " \t".includes(text[tail])) tail += 1;
            if (tail < text.length && ":=".includes(text[tail])) break;
            end += nextKey[0].length;
          } else end += 1;
        }
        if (header) {
          // RFC obs-fold: each indented physical line belongs to the header.
          while (end < text.length) {
            let fold = end;
            if (text.startsWith("\r\n", fold)) fold += 2;
            else if (LINES.includes(text[fold])) fold += 1;
            else break;
            if (fold === text.length || !" \t".includes(text[fold])) break;
            end = lineEnd(text, fold);
          }
        }
        while (end > pos && " \t".includes(text[end - 1])) end -= 1;
      }
      if (ambiguousTail(text, end)) end = lineEnd(text, end);
      replacement = "[redacted]";
      if (end + 1 < text.length && "\"'".includes(text[end]) && !valueBoundary(text, end + 1)) {
        while (end < text.length && !LINES.includes(text[end])) end += 1;
      }
    }
    if (end <= start) continue;
    // Removing a keyed value must not hide its surrounding URL username.
    if (inUrl && start <= headEnd) replacement += "@";
    pieces.push(original.slice(offset(cursor), offset(start)), replacement);
    cursor = end;
  }
  pieces.push(original.slice(offset(cursor)));
  return pieces.join("");
}

function urlView(original: string): [string, number[] | undefined] {
  if (!/[%\\&ſ\uff01-\uff5e]/.test(original)) return [original, undefined];
  let text = original;
  let offsets = Array.from({ length: text.length + 1 }, (_, index) => index);
  for (let layer = 0; layer < 3; layer += 1) {
    const chars: string[] = [];
    const positions: number[] = [];
    let pos = 0;
    while (pos < text.length) {
      positions.push(offsets[pos]);
      const entity = text[pos] === "&" ? /^(?:&#([xX][0-9a-fA-F]{1,6}|[0-9]{1,7});|&(colon|sol|commat);)/.exec(text.slice(pos, pos + 14)) : null;
      if (entity) {
        const code = entity[1] ? Number.parseInt(entity[1][0].toLowerCase() === "x" ? entity[1].slice(1) : entity[1], entity[1][0].toLowerCase() === "x" ? 16 : 10) : ({ colon: 58, sol: 47, commat: 64 } as Record<string, number>)[entity[2]];
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
        } else if (end + 5 <= text.length && text[end] === "u" && /^[0-9a-fA-F]{4}$/.test(text.slice(end + 1, end + 5))) {
          chars.push(String.fromCharCode(Number.parseInt(text.slice(end + 1, end + 5), 16)));
          pos = end + 5;
        } else {
          while (pos < end) {
            chars.push(text[pos]);
            pos += 1;
            if (pos < end) positions.push(offsets[pos]);
          }
        }
      } else {
        const ch = text[pos];
        chars.push(ch >= "\uff01" && ch <= "\uff5e" ? String.fromCharCode(ch.charCodeAt(0) - 0xfee0) : ch === "ſ" ? "s" : ch);
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
  if (authority.includes(":") && !(authority.startsWith("[") || authority.includes(".") || authority.split(":", 1)[0] === "localhost")) return false;
  const tail = prefix.slice(boundary.index);
  if ((" \t\v\f" + LINES).includes(tail[0])) {
    const pos = start + boundary.index;
    if (original.slice(offset(pos), offset(pos + 1)) !== tail[0]) return false;
  }
  return !(tail.includes(":") || opaqueToken(tail) || /[\\{}[\]"'<>|]/.test(tail));
}

function urlCandidateEnd(view: string, original: string, offset: (pos: number) => number, start: number, end: number): number {
  const relativeAt = view.slice(start, end).indexOf("@");
  if (relativeAt < 0) return end;
  let gap = start + relativeAt + 1;
  while (gap < end) {
    if ((" \t\v\f" + LINES).includes(view[gap]) && original.slice(offset(gap), offset(gap + 1)) === view[gap]) {
      const tail = view.slice(start + relativeAt + 1, gap).split(/[/?#]/, 1)[0];
      return clearHost(tail) ? gap : end;
    }
    gap += 1;
  }
  return end;
}

function redactUrlCredentialsWithScheme(text: string): string {
  const [view, offsets] = urlView(text);
  const offset = (pos: number) => offsets ? offsets[pos] : pos;
  const matches = [...view.matchAll(SCHEME)];
  const pieces: string[] = [];
  let cursor = 0;
  let pending = -1;
  for (let index = 0; index < matches.length; index += 1) {
    const match = matches[index];
    let start = match.index + match[0].length;
    if (offset(start) < cursor) continue;
    const end = urlCandidateEnd(view, text, offset, start, matches[index + 1]?.index ?? view.length);
    const relativeAt = view.slice(start, end).indexOf("@");
    const firstAt = relativeAt < 0 ? -1 : relativeAt + start;
    if (firstAt < 0) {
      if (!clearHost(view.slice(start, end)) && pending < 0 && !publicUrlPrefix(view.slice(start, end), text, offset, start)) pending = start;
      continue;
    }
    const lastAt = view.slice(start, end).lastIndexOf("@") + start;
    if (publicUrlPrefix(view.slice(start, lastAt), text, offset, start)) {
      pending = -1;
      continue;
    }
    if (pending >= 0) start = pending;
    const userinfo = view.slice(start, lastAt);
    const hostTail = view.slice(lastAt + 1, end).split(/[/?#]/, 1)[0];
    const invalid = [...userinfo].some((ch) => (SPACE + '/[]{}"<>|').includes(ch)) || opaqueToken(hostTail.split(".")[0]);
    let finish = lastAt + 1;
    if (invalid) {
      while (finish < end && !(SPACE + '/"\'<>').includes(view[finish])) finish += 1;
    }
    pieces.push(text.slice(cursor, offset(start)), "[redacted-credentials]" + (invalid ? "" : "@"));
    cursor = offset(finish);
    pending = -1;
  }
  pieces.push(text.slice(cursor));
  return pieces.join("");
}

function aliasProse(name: string, value: string): boolean {
  // Lowercase passwords remain credentials; exemptions are ordinary status/user prose.
  return (name === "user" ? ["alice", "bob", "root", "anonymous"] : name === "auth" ? ["ok", "required", "enabled", "disabled", "none", "expired"] : []).includes(value);
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

function redactXml(text: string): string {
  const pieces: string[] = [];
  let cursor = 0;
  for (const tag of text.matchAll(/<(\/?)([A-Za-z][A-Za-z0-9_.:-]*)(?![A-Za-z0-9_.:-])([^<>]*)>/g)) {
    if (tag.index < cursor || tag[1]) continue;
    const key = tag[2];
    if (/^[=:-]/.test(tag[3].trimStart())) continue;
    const name = secretName(key.split(":").at(-1)!);
    const attrs = [...tag[3].matchAll(/(?<![A-Za-z0-9_.:-])([A-Za-z_:][A-Za-z0-9_.:-]*)[ \t\r\n\v\f]*=[ \t\r\n\v\f]*(["'])(.*?)\2/gs)];
    const keyed = attrs.some((a) => ["key", "name"].includes(a[1].toLowerCase()) && secretName(a[3]));
    if (keyed || name) {
      for (const value of attrs) {
        if (value[1].toLowerCase() !== "value" || (!keyed && WEAK_NAMES.has(name) && aliasProse(name, value[3]))) continue;
        const start = tag.index + 1 + key.length + value.index + value[0].length - value[3].length - 1;
        const end = start + value[3].length;
        pieces.push(text.slice(cursor, start), "[redacted]");
        cursor = end;
      }
    }
    if ((!name && !keyed) || tag[3].trimEnd().endsWith("/")) continue;
    const start = tag.index + tag[0].length;
    const closing = "</" + key;
    let end = text.indexOf(closing, start);
    while (end >= 0) {
      let tail = end + closing.length;
      while (tail < text.length && " \t\r\n\v\f".includes(text[tail])) tail += 1;
      if (tail < text.length && text[tail] === ">") break;
      end = text.indexOf(closing, tail);
    }
    if (end < 0) end = text.length;
    const value = text.slice(start, end);
    if (!keyed && WEAK_NAMES.has(name) && aliasProse(name, value.trim())) continue;
    pieces.push(text.slice(cursor, start), "[redacted]");
    cursor = end;
  }
  pieces.push(text.slice(cursor));
  return pieces.join("");
}

export function redactKeyedSecrets(text: string): string {
  text = redactXml(text);
  text = redactKeyedPlain(text);
  const [view, offsets] = urlView(text);
  if (view !== text) text = redactKeyedPlain(view, text, offsets);
  return text;
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

function redactBareUserinfo(text: string): string {
  const [view, offsets] = urlView(text);
  const offset = (pos: number) => offsets ? offsets[pos] : pos;
  const pieces: string[] = [];
  let cursor = 0;
  let start = 0;
  while (start < view.length) {
    let end = start;
    while (end < view.length && !((" \t\v\f" + LINES).includes(view[end]) && text.slice(offset(end), offset(end + 1)) === view[end])) end += 1;
    const token = view.slice(start, end);
    if (!token.includes("://") && token.includes("@")) {
      const head = /^[A-Za-z0-9+.-]+:[/\\]+/.exec(token);
      const body = head ? head[0].length : 0;
      const at = token.lastIndexOf("@");
      const colon = token.indexOf(":", body);
      if ((head || (colon > body && colon < at)) && !/[/?#=]/.test(token.slice(body, at))) {
        pieces.push(text.slice(cursor, offset(start + body)), "[redacted-credentials]@");
        cursor = offset(start + at + 1);
      }
    }
    start = end + 1;
  }
  pieces.push(text.slice(cursor));
  return pieces.join("");
}

export function redactUrlCredentials(text: string): string {
  return redactBareUserinfo(redactUrlCredentialsWithScheme(text));
}
