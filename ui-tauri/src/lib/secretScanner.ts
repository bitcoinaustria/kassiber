const IDENTIFIER = /[A-Za-z0-9_.-]+/g;
const NAMES = new Set(`apikey authheader authorization blinding blindingkey cookie
descriptor mnemonic passphrase password passwordhash rpcpassword rpcauth macaroon
macaroonhex recoveryphrase rune scankey scanprivate scanprivatekey spendkey secret
secretkey privatekey privkey seed seedphrase seedwords silentpayment silentpayments
silentpaymentdescriptor silentpaymentsdescriptor silentpaymentkey silentpaymentskey
silentpaymentmaterial silentpaymentsmaterial silentpaymentscan silentpaymentscan
silentpaymentscankey silentpaymentsscankey spdescriptor spscan spspend token xprv
accesstoken refreshtoken apitoken authtoken clientsecret dbpassword apisecret walletpassphrase wif rpcpass pass pwd credentials auth user`.split(/\s+/));
const PROSE = new Set(`to instead flag option when requires expected missing with
without is are was were has have the a an and or for from of in on set use words
value values argument arguments deprecated reset rotated invalid expired none`.split(/\s+/));
const RECOVERY = new Set(["mnemonic", "recoveryphrase", "seed", "seedphrase", "seedwords"]);
const SCHEME = /(?<![A-Za-z0-9+.-])[A-Za-z][A-Za-z0-9+.-]*:\/\//g;
const SPACE = " \t\r\n\v\f\u0085\u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000\ufeff";
const LINES = "\r\n\u2028\u2029";
const WEAK_NAMES = new Set(["pass", "user", "auth"]);
const COMMAND_LINE = /(?:^|(?<=[\r\n\u2028\u2029]))[ \t]*(?:[$>][ \t]+)?(?:cmd|kassiber|bitcoind|bitcoin-cli|electrs|lncli|lightning-cli|curl|python|node|env|sudo|docker)(?=[ \t]|$)/g;

function secretName(value: string): string {
  const versionless = value.replace(/[_-]v[0-9]+_*$/i, "");
  let first = 0;
  let last = versionless.length;
  while (first < last && "_.-".includes(versionless[first])) first += 1;
  while (last > first && "_.-".includes(versionless[last - 1])) last -= 1;
  const token = versionless.slice(first, last).toLowerCase();
  const parts = token.split(/[_.-]+/);
  for (let count = Math.min(4, parts.length); count > 0; count -= 1) {
    const candidate = parts.slice(-count).join("");
    let begin = 0;
    let end = candidate.length;
    while (begin < end && "0123456789".includes(candidate[begin])) begin += 1;
    while (end > begin && "0123456789".includes(candidate[end - 1])) end -= 1;
    const name = candidate.slice(begin, end);
    if (NAMES.has(name)) return name;
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

function quotedEnd(text: string, start: number, delimiter: string): [number, boolean] {
  let slashes = 0;
  let pos = start;
  const slashCount = delimiter.length - 1;
  const period = 2 * (slashCount + (slashCount > 0 && slashCount % 2 === 0 ? 2 : 1));
  let firstNewline = -1;
  while (pos < text.length) {
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

function* urlHeads(text: string): Generator<[number, number]> {
  // Preserve context even when a keyed pass masks a URL's original @.
  const [view, offsets] = urlView(text);
  const offset = (pos: number) => offsets ? offsets[pos] : pos;
  for (const match of view.matchAll(SCHEME)) {
    let end = match.index + match[0].length;
    while (end < view.length && !(SPACE + "\"'<>[]/{}()?&|").includes(view[end])) end += 1;
    yield [offset(match.index + match[0].length), offset(end)];
  }
}

function lineEnd(text: string, start: number): number {
  let pos = start;
  while (pos < text.length && !LINES.includes(text[pos])) pos += 1;
  return pos;
}

export function redactKeyedSecrets(text: string): string {
  const pieces: string[] = [];
  let cursor = 0;
  const nextIdentifier = /[A-Za-z0-9_.-]+/y;
  const commands = text.matchAll(COMMAND_LINE);
  let command = commands.next().value;
  let commandEnd = command ? lineEnd(text, command.index + command[0].length) : -1;
  const heads = urlHeads(text);
  let [headStart, headEnd] = heads.next().value ?? [-1, -1];
  for (const match of text.matchAll(IDENTIFIER)) {
    if (match.index < cursor) continue;
    const key = match[0];
    while (command && commandEnd < match.index) {
      command = commands.next().value;
      commandEnd = command ? lineEnd(text, command.index + command[0].length) : -1;
    }
    const inCommand = command !== undefined && command.index <= match.index && match.index <= commandEnd;
    const name = secretName(key);
    if (!name) continue;
    while (headEnd < match.index && headEnd >= 0) [headStart, headEnd] = heads.next().value ?? [-1, -1];
    const inUrl = headStart <= match.index && match.index < headEnd;
    let pos = match.index + key.length;
    if (match.index > 0 && text[match.index - 1] === "<" && text[pos] === ">") {
      const start = pos + 1;
      let end = text.indexOf("</" + key + ">", start);
      if (end < 0) end = text.length;
      pieces.push(text.slice(cursor, start), "[redacted]");
      cursor = end;
      continue;
    }
    const [keyQuote, keyEnd] = quoteAt(text, pos);
    pos = keyEnd;
    const afterKey = pos;
    while (pos < text.length && SPACE.includes(text[pos])) pos += 1;
    let flag = false;
    let separator = "";
    if (pos < text.length && ":=".includes(text[pos])) {
      separator = text[pos];
      if (inUrl && match.index === headStart && separator === ":" && pos + 1 < text.length && !SPACE.includes(text[pos + 1])) continue;
      pos += 1;
      if ((separator === "=" && text[pos] === ">") || (separator === ":" && text[pos] === "=")) {
        separator = "=";
        pos += 1;
      }
      while (pos < text.length && SPACE.includes(text[pos])) pos += 1;
      if (WEAK_NAMES.has(name) && separator === ":" && !keyQuote) {
        let before = match.index - 1;
        while (before >= 0 && " \t".includes(text[before])) before -= 1;
        if (before >= 0 && !(LINES + "{[,;|").includes(text[before])) continue;
      }
    } else if ((key.startsWith("-") || inCommand) && pos > afterKey && !/[\r\n\u2028\u2029]/.test(text.slice(afterKey, pos))) {
      flag = true;
    } else continue;
    const start = pos;
    if (pos === text.length) continue;
    const [delimiter, body] = quoteAt(text, pos);
    let end: number;
    let replacement: string;
    if (delimiter) {
      let closed: boolean;
      [end, closed] = quotedEnd(text, body, delimiter);
      if (closed && end < text.length && !valueBoundary(text, end)) {
        while (end < text.length && !LINES.includes(text[end])) end += 1;
      }
      replacement = delimiter + "[redacted]" + (closed ? delimiter : "");
    } else {
      end = pos;
      const valueEnd = " \t\r\n\v\f\u2028\u2029\"'";
      while (end < text.length && !valueEnd.includes(text[end])) end += 1;
      const token = text.slice(pos, end);
      if (WEAK_NAMES.has(name) && separator === ":" && !keyQuote && /^[a-z]+$/.test(token)) continue;
      if (separator === ":" && (["none", "expired"].includes(token.toLowerCase()) || text.startsWith("(not set)", pos))) continue;
      if (flag && ((!inCommand && (end === text.length || SPACE.includes(text[end])) && (PROSE.has(token.toLowerCase()) || /^[a-z]+[.,;!?:]$/.test(token)))
          || (name === "seedwords" && ["12", "15", "18", "21", "24"].includes(token)))) continue;
      const header = name === "authorization" || (name === "cookie" && separator === ":" && !flag && !keyQuote);
      if (header || RECOVERY.has(name)) {
        while (end < text.length && !(header ? LINES : ",;}{&|" + LINES + "\"'").includes(text[end])) {
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
      replacement = "[redacted]";
      if (end + 1 < text.length && "\"'".includes(text[end]) && !valueBoundary(text, end + 1)) {
        while (end < text.length && !LINES.includes(text[end])) end += 1;
      }
    }
    if (end <= start) continue;
    // Removing a keyed value must not hide its surrounding URL username.
    if (inUrl && start <= headEnd) replacement += "@";
    pieces.push(text.slice(cursor, start), replacement);
    cursor = end;
  }
  pieces.push(text.slice(cursor));
  return pieces.join("");
}

function urlView(original: string): [string, number[] | undefined] {
  if (!original.includes("%") && !original.includes("\\/")) return [original, undefined];
  let text = original;
  let offsets = Array.from({ length: text.length + 1 }, (_, index) => index);
  for (let layer = 0; layer < 3; layer += 1) {
    const chars: string[] = [];
    const positions: number[] = [];
    let pos = 0;
    while (pos < text.length) {
      positions.push(offsets[pos]);
      if (text[pos] === "%" && /^[0-9a-fA-F]{2}$/.test(text.slice(pos + 1, pos + 3))) {
        chars.push(String.fromCharCode(Number.parseInt(text.slice(pos + 1, pos + 3), 16)));
        pos += 3;
      } else if (text[pos] === "\\") {
        let end = pos;
        while (end < text.length && text[end] === "\\") end += 1;
        if (end < text.length && text[end] === "/") {
          chars.push("/");
          pos = end + 1;
        } else {
          while (pos < end) {
            chars.push(text[pos]);
            pos += 1;
            if (pos < end) positions.push(offsets[pos]);
          }
        }
      } else {
        chars.push(text[pos]);
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

export function redactUrlCredentials(text: string): string {
  const [view, offsets] = urlView(text);
  const offset = (pos: number) => offsets ? offsets[pos] : pos;
  const matches = [...view.matchAll(SCHEME)];
  const pieces: string[] = [];
  let cursor = 0;
  let pending = -1;
  let tokenEnd = 0;
  for (let index = 0; index < matches.length; index += 1) {
    const match = matches[index];
    let start = match.index + match[0].length;
    if (offset(start) < cursor) continue;
    if (start >= tokenEnd) {
      pending = -1;
      tokenEnd = start;
      // CR/LF and non-ASCII whitespace may be malformed userinfo.
      while (tokenEnd < view.length && !" \t\v\f".includes(view[tokenEnd])) tokenEnd += 1;
    }
    const end = Math.min(tokenEnd, matches[index + 1]?.index ?? view.length);
    const relativeAt = view.slice(start, end).lastIndexOf("@");
    if (relativeAt < 0) {
      const prefix = view.slice(start, end).split("/", 1)[0];
      if (prefix.includes(":")) {
        const password = prefix.slice(prefix.indexOf(":") + 1);
        if (password && !/^[0-9]+$/.test(password) && pending < 0) pending = start;
      }
      continue;
    }
    const lastAt = start + relativeAt;
    if (pending >= 0) start = pending;
    const userinfo = view.slice(start, lastAt);
    const invalid = [...userinfo].some((ch) => (SPACE + '/[]{}"<>|').includes(ch));
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
