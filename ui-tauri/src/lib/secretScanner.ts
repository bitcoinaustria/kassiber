const IDENTIFIER = /[A-Za-z0-9_.-]+/g;
const NAMES = new Set(`apikey authheader authorization blinding blindingkey cookie
descriptor mnemonic passphrase password passwordhash rpcpassword rpcauth macaroon
macaroonhex recoveryphrase rune scankey scanprivate scanprivatekey spendkey secret
secretkey privatekey privkey seed seedphrase seedwords silentpayment silentpayments
silentpaymentdescriptor silentpaymentsdescriptor silentpaymentkey silentpaymentskey
silentpaymentmaterial silentpaymentsmaterial silentpaymentscan silentpaymentscan
silentpaymentscankey silentpaymentsscankey spdescriptor spscan spspend token xprv
accesstoken authtoken clientsecret wif rpcpass pass pwd credentials auth user`.split(/\s+/));
const PROSE = new Set(`to instead flag option when requires expected missing with
without is are was were has have the a an and or for from of in on set use words
value values argument arguments deprecated reset rotated invalid expired none`.split(/\s+/));
const RECOVERY = new Set(["mnemonic", "recoveryphrase", "seed", "seedphrase", "seedwords"]);
const SCHEME = /\b(?:https?|tcp|ssl|socks5h?|wss?|ftp|postgres|electrum):\/\//gi;
const SPACE = " \t\r\n\v\f\u0085\u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000\ufeff";
const VALUE_END = SPACE + ",;{}&|";
const COMMAND_LINE = /^[ \t]*(?:[$>][ \t]+)?(?:cmd|kassiber|bitcoind|bitcoin-cli|electrs|lncli|lightning-cli|curl|python|node|env|sudo|docker)(?=[ \t]|$)/gm;

function secretName(value: string): string {
  const versionless = value.replace(/[_-]v[0-9]+_*$/i, "");
  let first = 0;
  let last = versionless.length;
  while (first < last && "_.-".includes(versionless[first])) first += 1;
  while (last > first && "_.-".includes(versionless[last - 1])) last -= 1;
  const token = versionless.slice(first, last).toLowerCase();
  const parts = token.split(/[_.-]+/);
  for (let count = 1; count <= Math.min(4, parts.length); count += 1) {
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
  if (end < text.length && "\"'".includes(text[end]) && [0, 1, 2, 3, 6, 7, 14].includes(end - pos)) {
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
    if ("\r\n".includes(ch) && firstNewline < 0) firstNewline = pos;
    if (ch === delimiter.at(-1) && slashes % period === delimiter.length - 1) return [pos + 1, true];
    slashes = ch === "\\" ? slashes + 1 : 0;
    pos += 1;
  }
  return [firstNewline >= 0 ? firstNewline : pos, false];
}

function* urlHeads(text: string): Generator<[number, number]> {
  let scanned = 0;
  for (const match of text.matchAll(SCHEME)) {
    if (match.index < scanned) continue;
    let end = match.index + match[0].length;
    while (end < text.length && !(SPACE + "\"'<>[]/{}()&|").includes(text[end])) end += 1;
    scanned = end;
    yield [match.index + match[0].length, end];
  }
}

export function redactKeyedSecrets(text: string): string {
  const pieces: string[] = [];
  let cursor = 0;
  const nextIdentifier = /[A-Za-z0-9_.-]+/y;
  const commands = text.matchAll(COMMAND_LINE);
  let command = commands.next().value;
  let commandEnd = command ? text.indexOf("\n", command.index + command[0].length) : -1;
  if (command && commandEnd < 0) commandEnd = text.length;
  const heads = urlHeads(text);
  let [headStart, headEnd] = heads.next().value ?? [-1, -1];
  for (const match of text.matchAll(IDENTIFIER)) {
    if (match.index < cursor) continue;
    const key = match[0];
    while (command && commandEnd < match.index) {
      command = commands.next().value;
      commandEnd = command ? text.indexOf("\n", command.index + command[0].length) : -1;
      if (command && commandEnd < 0) commandEnd = text.length;
    }
    const inCommand = command !== undefined && command.index <= match.index && match.index <= commandEnd;
    const name = secretName(key);
    if (!name) continue;
    while (headEnd < match.index && headEnd >= 0) [headStart, headEnd] = heads.next().value ?? [-1, -1];
    const inUrl = headStart <= match.index && match.index < headEnd;
    let pos = match.index + key.length;
    [, pos] = quoteAt(text, pos);
    const afterKey = pos;
    while (pos < text.length && SPACE.includes(text[pos])) pos += 1;
    let flag = false;
    let separator = "";
    if (pos < text.length && ":=".includes(text[pos])) {
      separator = text[pos];
      if (inUrl && separator === ":" && pos + 1 < text.length && !SPACE.includes(text[pos + 1])) continue;
      pos += 1;
      while (pos < text.length && SPACE.includes(text[pos])) pos += 1;
    } else if ((key.startsWith("-") || inCommand) && pos > afterKey && !/[\r\n]/.test(text.slice(afterKey, pos))) {
      flag = true;
    } else continue;
    const start = pos;
    if (pos === text.length || VALUE_END.includes(text[pos]) || (flag && text[pos] === "-")) continue;
    const [delimiter, body] = quoteAt(text, pos);
    let end: number;
    let replacement: string;
    if (delimiter) {
      let closed: boolean;
      [end, closed] = quotedEnd(text, body, delimiter);
      if (closed && end < text.length && !(VALUE_END + "])").includes(text[end])) {
        while (end < text.length && !"\r\n".includes(text[end])) end += 1;
      }
      replacement = delimiter + "[redacted]" + (closed ? delimiter : "");
    } else {
      end = pos;
      const valueEnd = inUrl ? SPACE + "\"'<>[]/{}()" : VALUE_END + "\"'";
      while (end < text.length && !valueEnd.includes(text[end])) end += 1;
      const token = text.slice(pos, end);
      if (separator === ":" && (["none", "expired"].includes(token.toLowerCase()) || text.startsWith("(not set)", pos))) continue;
      if (flag && ((!inCommand && (PROSE.has(token.toLowerCase()) || /[.,;!?:]$/.test(token)))
          || (name === "seedwords" && ["12", "15", "18", "21", "24"].includes(token)))) continue;
      if (name === "authorization" || RECOVERY.has(name)) {
        while (end < text.length && !(name === "authorization" ? "\r\n" : ",;}{&|\r\n\"'").includes(text[end])) {
          nextIdentifier.lastIndex = end;
          const nextKey = name !== "authorization" ? nextIdentifier.exec(text) : null;
          if (nextKey && name !== "authorization") {
            let tail = end + nextKey[0].length;
            while (tail < text.length && " \t".includes(text[tail])) tail += 1;
            if (tail < text.length && ":=".includes(text[tail])) break;
            end += nextKey[0].length;
          } else end += 1;
        }
        while (end > pos && " \t".includes(text[end - 1])) end -= 1;
      }
      replacement = "[redacted]";
      if (end + 1 < text.length && "\"'".includes(text[end]) && !(VALUE_END + "])").includes(text[end + 1])) {
        while (end < text.length && !"\r\n".includes(text[end])) end += 1;
      }
    }
    if (end <= start) continue;
    pieces.push(text.slice(cursor, start), replacement);
    cursor = end;
  }
  pieces.push(text.slice(cursor));
  return pieces.join("");
}

export function redactUrlCredentials(text: string): string {
  const pieces: string[] = [];
  let cursor = 0;
  let scanned = 0;
  for (const match of text.matchAll(SCHEME)) {
    if (match.index < scanned) continue;
    const start = match.index + match[0].length;
    let pos = start;
    let lastAt = -1;
    let invalid = false;
    while (pos < text.length && !(SPACE + "/[]{}").includes(text[pos])) {
      const ch = text[pos];
      if ("\"'<>()&|".includes(ch)) {
        if (pos + 1 === text.length || (",;}:" + SPACE).includes(text[pos + 1])) break;
        invalid = true;
      }
      if (ch === "@") lastAt = pos;
      pos += 1;
    }
    scanned = pos;
    if (pos < text.length && text[pos] === "/") {
      let tokenEnd = pos;
      while (tokenEnd < text.length && !(SPACE + "\"'<>[]{}()&|").includes(text[tokenEnd])) tokenEnd += 1;
      scanned = tokenEnd;
      const prefix = text.slice(start, pos);
      if (lastAt < 0 && prefix.includes(":")) {
        const password = prefix.slice(prefix.indexOf(":") + 1);
        if (password && !/^[0-9]+$/.test(password)) {
          const relativeAt = text.slice(pos, tokenEnd).lastIndexOf("@");
          const tailAt = pos + relativeAt;
          if (relativeAt >= 0) {
            lastAt = tailAt;
            invalid = true;
            pos = tokenEnd;
          }
        }
      }
    } else if (pos < text.length && "[]()&|".includes(text[pos]) && pos > start && lastAt < 0) {
      let tokenEnd = pos;
      while (tokenEnd < text.length && !(SPACE + "\"'<>{}").includes(text[tokenEnd])) tokenEnd += 1;
      const relativeAt = text.slice(pos, tokenEnd).lastIndexOf("@");
      scanned = tokenEnd;
      if (relativeAt >= 0) {
        lastAt = pos + relativeAt;
        invalid = true;
        pos = tokenEnd;
      }
    }
    if (lastAt < 0) continue;
    const end = invalid ? pos : lastAt + 1;
    pieces.push(text.slice(cursor, start), "[redacted-credentials]" + (invalid ? "" : "@"));
    cursor = end;
  }
  pieces.push(text.slice(cursor));
  return pieces.join("");
}
