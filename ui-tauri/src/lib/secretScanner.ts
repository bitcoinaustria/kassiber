type Span = [start: number, end: number, replacement: string];

const IDENTIFIER = /[A-Za-z0-9_.-]+/g;
const SEPARATORS = /[^A-Za-z0-9]+/;
const KEY_PARTS = /(?<=[a-z0-9])(?=[A-Z])|(?<=[A-Z])(?=[A-Z][a-z])|(?<=[0-9])(?=[A-Za-z])/;
const VERSION = /[_-]v[0-9]+_*$/i;
const words = (text: string) => text.split(/\s+/).filter(Boolean);
// Secret words match whole identifier words, never raw substrings, so `prune`,
// `dnsseed`, `bypass`, `passed` and `tokenizer` remain ordinary vocabulary.
const SECRET_WORDS = new Set(words(`authorization bearer blinding cookie cookies credential credentials descriptor
macaroon mnemonic otp pass passcode passphrase passwd password pin pw pwd rpcauth rune secret seed token
totp wif xprv`));
// Short names count only as a whole identifier part (`user_id`, not `author`
// or `sighash`); the shortest only as the final word (`db_pw`, not `pw_hash`).
const PART_NAMES = new Set(words("auth jwt session sid sig signature user"));
const LAST_NAMES = new Set(words("otp pin pw totp"));
const WEAK_NAMES = new Set(["pass", "user", "auth"]);
// Adjacent words that name a secret together, spelled apart or concatenated.
const COMPOUNDS = new Map(([
  ["api", "key"], ["api", "secret"], ["api", "token"], ["auth", "code"], ["auth", "header"], ["blinding", "key"],
  ["encryption", "key"], ["master", "key"], ["mfa", "code"], ["otp", "code"], ["pin", "code"], ["priv", "key"],
  ["private", "key"], ["recovery", "phrase"], ["scan", "key"], ["scan", "private"], ["secret", "key"],
  ["seed", "phrase"], ["seed", "words"], ["session", "id"], ["session", "key"], ["signing", "key"],
  ["silent", "payment"], ["silent", "payments"], ["sp", "descriptor"], ["sp", "scan"], ["sp", "spend"],
  ["spend", "key"], ["totp", "code"], ["twofa", "code"], ["verification", "code"],
] as const).map(([first, second]) => [`${first} ${second}`, first + second]));
const COMPOUND_WORDS = new Set(COMPOUNDS.values());
const METADATA = new Set(words("disabled enabled fd file hint path statistics stdin"));
const COUNT_WORDS = new Set(words("count max min limit length len size ttl"));
// Qualifiers let concatenated identifiers segment into words: `rpcpassword`,
// `dbprivkeyraw`, `masterseedwords`. Unknown pieces keep a part unsegmented.
const QUALIFIERS = new Set([...words(`access account admin api app aws b base bitcoin client code data db default dev
device electrum enc encoded encryption env github hash header hex id key keys lnd local login main master mfa
my mysql new old payment payments pem pg phrase plain priv private prod raw recovery refresh remote root rpc
scan server service signing silent sp spend str test text total twofa v value verification wallet web words
x azure binance bitfinex bitpanda bitstamp blockstream btcpay cln coinbase core discord electrs fulcrum gcp
gitlab google kassiber kraken lightning mempool mongo nostr postgres redis slack smtp strike stripe telegram
umbrel`), ...COUNT_WORDS, ...METADATA]);
const PLURALS = new Set([...SECRET_WORDS, ...PART_NAMES].filter((word) => !word.endsWith("s")).map((word) => word + "s"));
const VOCAB = new Set([...SECRET_WORDS, ...PART_NAMES, ...QUALIFIERS, ...COMPOUND_WORDS, ...PLURALS]);
const SIZES = [...new Set([...VOCAB].map((word) => word.length))].sort((a, b) => b - a);
const byLength = (a: string, b: string) => b.length - a.length || (a < b ? -1 : a > b ? 1 : 0);
// An unknown prefix before a distinctive secret word still names it
// (`krakenpassword`, `jsessionid`); an unknown suffix (`tokenizer`) does not.
const STRONG = [...new Set([...[...SECRET_WORDS].filter((word) => word.length >= 5), "pwd", "xprv", ...COMPOUND_WORDS])].sort(byLength);
const TWOFA = /2fa/gi;
const HINT = new RegExp([...new Set([...SECRET_WORDS, ...PART_NAMES, "2fa", "code", "key", "mfa", "phrase", "recovery", "scan", "silent", "spend", "twofa"])].sort().join("|"), "i");
const segments = new Map<string, string[]>();
const keyWordCache = new Map<string, [word: string, whole: boolean][][]>();

function cached<T>(cache: Map<string, T>, key: string, compute: () => T): T {
  let value = cache.get(key);
  if (value === undefined) {
    if (cache.size >= 4096) cache.clear();
    value = compute();
    cache.set(key, value);
  }
  return value;
}

function segment(part: string): string[] {
  return cached(segments, part, () => {
    if (VOCAB.has(part)) return [part];
    if (part.length > 40) return strongSegment(part);
    // Fewest words; ties keep the longest leading word.
    const best: ([count: number, end: number] | undefined)[] = Array.from({ length: part.length + 1 }, () => undefined);
    best[part.length] = [0, part.length];
    for (let start = part.length - 1; start >= 0; start -= 1) {
      for (const size of SIZES) {
        const end = start + size;
        const following = end <= part.length ? best[end] : undefined;
        const current = best[start];
        if (following && VOCAB.has(part.slice(start, end)) && (!current || following[0] + 1 < current[0])) best[start] = [following[0] + 1, end];
      }
    }
    if (!best[0]) return strongSegment(part);
    const pieces: string[] = [];
    for (let start = 0; start < part.length; start = best[start]![1]) pieces.push(part.slice(start, best[start]![1]));
    return pieces;
  });
}

function strongSegment(part: string): string[] {
  for (const word of STRONG) {
    const start = part.lastIndexOf(word);
    const rest = part.slice(start + word.length);
    if (start <= 0 || rest.length > 40) continue;
    if (rest === "s") return [part.slice(0, start), word + "s"];
    const tail = rest ? segment(rest) : [];
    if (tail.every((piece) => VOCAB.has(piece))) return [part.slice(0, start), word, ...tail];
  }
  return [part];
}

type Word = [word: string, whole: boolean];

function keyReadings(value: string): Word[][] {
  // Lowercase words of an identifier, each marked when it is a whole part.
  // Case runs are hints, not boundaries: `rpcSECRETkeyOLD` and `newPWDb64`
  // split inside a word, so a chunk is also read concatenated when that
  // reading segments into known words. Either reading can name a secret.
  return cached(keyWordCache, value, () => {
    const token = value.replace(VERSION, "").replace(/^[_.-]+|[_.-]+$/g, "").replace(TWOFA, "twofa");
    const split: Word[] = [];
    const joined: Word[] = [];
    for (const chunk of token.split(SEPARATORS)) {
      const parts = chunk.split(KEY_PARTS).map((raw) => raw.toLowerCase().replace(/^[0-9]+|[0-9]+$/g, "")).filter(Boolean);
      const pieces: Word[] = parts.flatMap((part) => {
        const segmented = segment(part);
        return segmented.map((piece): Word => [piece, segmented.length === 1]);
      });
      // Identifiers can hold 100,000 parts: append without spreading arguments.
      for (const piece of pieces) split.push(piece);
      const segmented = parts.length > 1 ? segment(parts.join("")) : [];
      if (segmented.length && segmented.every((piece) => VOCAB.has(piece))) for (const piece of segmented) joined.push([piece, segmented.length === 1]);
      else for (const piece of pieces) joined.push(piece);
    }
    const same = joined.length === split.length && joined.every(([word, whole], index) => word === split[index][0] && whole === split[index][1]);
    return same ? [split] : [split, joined];
  });
}

function readingNames(words: Word[]): string[] {
  if (!words.length || METADATA.has(words.at(-1)![0])) return [];
  const names: string[] = [];
  words.forEach(([word, whole], index) => {
    if (index + 1 < words.length) {
      const second = words[index + 1][0];
      const pair = COMPOUNDS.get(`${word} ${second}`) ?? (second.endsWith("s") ? COMPOUNDS.get(`${word} ${second.slice(0, -1)}`) : undefined);
      if (pair) names.push(pair);
    }
    if (COMPOUND_WORDS.has(word)) {
      names.push(word);
      return;
    }
    const base = PLURALS.has(word) && !SECRET_WORDS.has(word) ? word.slice(0, -1) : word;
    if (["user", "auth"].includes(base) && base !== word) return;
    if (PART_NAMES.has(base) && !whole) return;
    if (LAST_NAMES.has(base) && index + 1 < words.length) return;
    if (SECRET_WORDS.has(base) || PART_NAMES.has(base)) names.push(base);
  });
  return names;
}

function secretName(token: string): string {
  if (!HINT.test(token)) return "";
  const names = keyReadings(token).flatMap(readingNames);
  return names.length ? names.sort(byLength)[0] : "";
}

const COUNT_VALUE = /^[0-9]{1,9}(?:\.[0-9]{1,9})?$/;
const LITERAL = /-?[0-9]+(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?(?![A-Za-z0-9_.])|(?:null|true|false|None|True|False)(?![A-Za-z0-9_.])/y;

function countName(key: string, name: string): boolean {
  return keyReadings(key).some((words) => words.some(([word]) => COUNT_WORDS.has(word) || (word === name + "s" && !SECRET_WORDS.has(word))));
}

// URL delimiters establish credential context regardless of scheme spelling.
const SCHEME = /(?<![A-Za-z0-9+.-])[A-Za-z0-9+.-]+:\/\/|:\/\/|\/\//g;
// Non-global twin: exec on the shared global regex would move matchAll's start.
const FIRST_SCHEME = new RegExp(SCHEME.source);
const SPACE = " \t\r\n\v\f\u0085\u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000\ufeff";
const LINES = "\r\n\u0085\u2028\u2029";
// XML and tag whitespace; both runtimes strip exactly these characters.
const XML_SPACE = " \t\r\n\v\f";
const COMMAND_LINE = /(?:^|(?<=[\r\n\u0085\u2028\u2029]))[ \t]*(?:[$>][ \t]+)?(?:cmd|kassiber|bitcoind|bitcoin-cli|electrs|lncli|lightning-cli|curl|python|node|env|sudo|docker)(?=[ \t]|$)/g;

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
    const limit = urlCandidateEnd(view, text, offset, start, urlLineEnd(view, text, offset, start, urlRecordEnd(view, text, offset, start, segmentEnd, record)));
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
  // Per-line facts, computed once however many keys share the line.
  let lineSeen = -1;
  let keyLineEnd = 0;
  let indent = 0;
  let continued = 0;
  let continuedSeen = -1;
  let yaml = 0;
  let yamlSeen = -1;
  const lineYamlEnd = (pos: number): number => {
    if (pos > keyLineEnd) return yamlEnd(text, pos);
    if (yamlSeen !== lineStart) {
      yaml = yamlEnd(text, pos);
      yamlSeen = lineStart;
    }
    return yaml;
  };
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
    if (!name && key.toLowerCase() === "key" && match.index > 0 && "?&".includes(text[match.index - 1])) name = "apikey";
    if (!name && before && [" ", "\t"].includes(text.slice(before.index + before[0].length, match.index))) {
      // Spaced names (`api key:`, `"Secret Key":`) still name a secret
      // when the second word changes what the first one means.
      name = secretName(before[0] + "_" + key);
      if (name === secretName(before[0])) name = "";
    }
    if (!name) continue;
    for (; scanned < match.index; scanned += 1) if (LINES.includes(text[scanned])) lineStart = scanned + 1;
    if (lineSeen !== lineStart) {
      lineSeen = lineStart;
      keyLineEnd = lineEnd(text, match.index + key.length);
      indent = lineIndent(text, lineStart);
    }
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
    let resume: number;
    if (delimiter) {
      let closed: boolean;
      if (text.startsWith(delimiter.repeat(3), pos)) {
        delimiter = delimiter.repeat(3);
        body = pos + delimiter.length;
        const close = text.indexOf(delimiter, body);
        [end, closed] = close >= 0 ? [close + delimiter.length, true] : [lineYamlEnd(body), false];
      } else [end, closed] = quotedEnd(text, body, delimiter);
      if (closed && end < text.length && (!valueBoundary(text, end) || ambiguousTail(text, end))) {
        end = lineYamlEnd(end);
      }
      replacement = start === pos ? delimiter + "[redacted]" + (closed ? delimiter : "") : "[redacted]";
      resume = end;
    } else {
      LITERAL.lastIndex = pos;
      const literal = LITERAL.exec(text);
      const literalEnd = literal ? pos + literal[0].length : -1;
      if (literal && literalEndsValue(text, literalEnd, indent)) {
        // A bare count after a count-shaped name (`max_tokens: 16`,
        // `token_ttl=3600`) and JSON null, booleans, sampling seeds and
        // counts are metadata. A string value never is.
        const value = literal[0];
        if ((COUNT_VALUE.test(value) && countName(key, name)) || (keyQuote && (
          ["null", "true", "false", "None", "True", "False"].includes(value)
          || name === "seed" || countName(key, name)))) {
          cursor = literalEnd;
          continue;
        }
        if (keyQuote) {
          // A JSON literal ends unambiguously; later fields stay readable.
          spans.push([offset(start), offset(literalEnd), "[redacted]"]);
          cursor = literalEnd;
          continue;
        }
      }
      // `"prompt_tokens_details": {"cached_tokens": 0}`: each member is
      // decided on its own key.
      if (keyQuote && text[pos] === "{" && countName(key, name) && countObject(text, pos)) continue;
      end = pos;
      const valueEnd = " \t\v\f\"'" + LINES;
      while (end < text.length && !valueEnd.includes(text[end])) end += 1;
      const token = text.slice(pos, end);
      const tokenEnd = end;
      // A plain value owns lines indented deeper than its line (a YAML or
      // shell continuation), so neither exceptions nor masks stop at the first.
      let line: number;
      if (pos > keyLineEnd) line = continuedEnd(text, pos, indent);
      else {
        if (continuedSeen !== lineStart) {
          continued = continuedEnd(text, pos, indent);
          continuedSeen = lineStart;
        }
        line = continued;
      }
      // Exceptions describe the entire value. A familiar first word never
      // licenses the rest of an otherwise unquoted credential.
      const whole = line - pos <= 256 ? text.slice(pos, line).replace(/^[ \t]+|[ \t]+$/g, "") : undefined;
      if (whole !== undefined && separator === ":" && !keyQuote && (
        ["none", "expired", "(not set)"].includes(whole.toLowerCase())
        || (WEAK_NAMES.has(name) && aliasProse(name, whole))
        || (name === "pass" && /^[0-9]+$/.test(whole) && /(?:^|[^A-Za-z])tests[ \t]+$/.test(text.slice(Math.max(0, match.index - 64), match.index)))
      )) continue;
      if (whole !== undefined && flag && ((!inCommand && helpValue(whole)) || (name === "seedwords" && ["12", "15", "18", "21", "24"].includes(whole)))) continue;
      resume = tokenEnd;
      if (["{", "[", "("].includes(text[pos]) || text.startsWith("$(", pos) || text.startsWith("${", pos)) {
        end = balancedEnd(text, pos, lineYamlEnd);
        if (text.slice(end, lineEnd(text, end)).replace(/[ \t]/g, "")) end = lineEnd(text, end);
        resume = end;
      } else if (["|", "|-", "|+", ">", ">-", ">+"].includes(token) || "`<".includes(text[pos])) {
        end = lineYamlEnd(pos);
        resume = end;
      } else if ((key.startsWith("-") || inCommand) && flagValueEnd(text, tokenEnd)) {
        // Whitespace before a new `-flag` ends an argv value.
        end = tokenEnd;
      } else end = line;
      const header = name === "authorization" || (["cookie", "cookies"].includes(name) && !keyQuote && !key.startsWith("-"));
      if (header) end = lineYamlEnd(pos);
      while (end > pos && " \t".includes(text[end - 1])) end -= 1;
      replacement = "[redacted]";
    }
    if (end <= start) continue;
    spans.push([offset(start), offset(end), replacement]);
    // Later keys on a plain value's line still establish their own blocks.
    cursor = resume;
  }
  return spans;
}

function lineIndent(text: string, start: number): number {
  // The line's own indent, past YAML sequence markers (`- password: x`).
  let pos = start;
  for (;;) {
    while (pos < text.length && " \t".includes(text[pos])) pos += 1;
    if (!(text.startsWith("- ", pos) || text.startsWith("-\t", pos))) return pos - start;
    pos += 1;
  }
}

function literalEndsValue(text: string, start: number, indent: number): boolean {
  // The literal is the whole value: closers then a record boundary, or
  // nothing but whitespace through its line and any continuation.
  let pos = start;
  while (pos < text.length && ")]}".includes(text[pos])) pos += 1;
  if (pos < text.length && ",;".includes(text[pos])) return valueBoundary(text, pos);
  while (pos < text.length && " \t".includes(text[pos])) pos += 1;
  return (pos === text.length || LINES.includes(text[pos])) && continuedEnd(text, pos, indent) === pos;
}

function countObject(text: string, start: number): boolean {
  // A flat object whose members are all numbers, null or booleans.
  let pos = start + 1;
  const identifier = /[A-Za-z0-9_.-]+/y;
  for (;;) {
    while (pos < text.length && SPACE.includes(text[pos])) pos += 1;
    if (text.startsWith("}", pos)) return true;
    const [delimiter, body] = quoteAt(text, pos);
    identifier.lastIndex = body;
    const key = delimiter ? identifier.exec(text) : null;
    if (!key || !text.startsWith(delimiter, body + key[0].length)) return false;
    pos = body + key[0].length + delimiter.length;
    while (pos < text.length && " \t".includes(text[pos])) pos += 1;
    if (!text.startsWith(":", pos)) return false;
    pos += 1;
    while (pos < text.length && " \t".includes(text[pos])) pos += 1;
    LITERAL.lastIndex = pos;
    const literal = LITERAL.exec(text);
    if (!literal) return false;
    pos += literal[0].length;
    while (pos < text.length && SPACE.includes(text[pos])) pos += 1;
    if (text.startsWith(",", pos)) pos += 1;
    else if (!text.startsWith("}", pos)) return false;
  }
}

const NEXT_FLAG = /--?[A-Za-z][A-Za-z0-9_.-]*(?=[= \t\r\n\u0085\u2028\u2029]|$)/y;

function flagValueEnd(text: string, start: number): boolean {
  if (start >= text.length || !" \t".includes(text[start]) || text[start - 1] === "\\") return false;
  let pos = start;
  while (pos < text.length && " \t".includes(text[pos])) pos += 1;
  NEXT_FLAG.lastIndex = pos;
  return NEXT_FLAG.test(text);
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

function urlLineEnd(view: string, original: string, offset: (pos: number) => number, start: number, end: number): number {
  // A literal line break ends a URL after its path starts, unless the next
  // line's first token continues the userinfo with an `@`.
  let pos = start;
  let authority = true;
  while (pos < end) {
    const literal = original.slice(offset(pos), offset(pos + 1)) === view[pos];
    if ("/?#".includes(view[pos]) && literal) authority = false;
    else if (LINES.includes(view[pos]) && literal && !authority) {
      let following = pos;
      while (following < end && SPACE.includes(view[following])) following += 1;
      let token = following;
      while (token < end && view[token] !== "@" && !(SPACE.includes(view[token]) && original.slice(offset(token), offset(token + 1)) === view[token])) token += 1;
      if (token === end || view[token] !== "@") return pos;
      pos = following;
      continue;
    }
    pos += 1;
  }
  return end;
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
    const recordEnd = urlRecordEnd(view, text, offset, start, matches[index + 1]?.index ?? view.length, record);
    const lineCut = urlLineEnd(view, text, offset, start, recordEnd);
    const end = urlCandidateEnd(view, text, offset, start, lineCut);
    const relativeAt = view.slice(start, end).indexOf("@");
    const firstAt = relativeAt < 0 ? -1 : relativeAt + start;
    if (firstAt < 0) {
      // A URL ended by its line cannot merge with a later credential URL.
      if (lineCut === recordEnd && !clearHost(view.slice(start, end)) && pending < 0 && !publicUrlPrefix(view.slice(start, end), text, offset, start)) pending = start;
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

function balancedEnd(text: string, start: number, lineYamlEnd = (pos: number) => yamlEnd(text, pos)): number {
  const stack: string[] = [];
  // An incomplete opener owns its indented continuation, not later records.
  let limit = lineYamlEnd(start);
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
  const pairs = /(?<![A-Za-z0-9_.-])["']?(?:name|key)["']?[ \t]*[:=][ \t]*["']?([A-Za-z0-9_.-]+(?::[A-Za-z0-9_.-]+)?)["']?[, \t\r\n\v\f\u0085\u2028\u2029]*["']?value["']?[ \t]*[:=][ \t]*/gi;
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

function stripChars(value: string, chars: string, left = true, right = true): string {
  let first = 0;
  let last = value.length;
  while (left && first < last && chars.includes(value[first])) first += 1;
  while (right && last > first && chars.includes(value[last - 1])) last -= 1;
  return value.slice(first, last);
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
    if (/^[:-]/.test(stripChars(tag[3], XML_SPACE, true, false))) continue;
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
      plistSecret = end >= 0 && !!secretName(stripChars(text.slice(start, end), XML_SPACE));
      continue;
    }
    name ||= plistSecret ? "secret" : "";
    plistSecret = false;
    if ((!name && !keyed) || stripChars(tag[3], XML_SPACE, false, true).endsWith("/")) continue;
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
    if (!keyed && WEAK_NAMES.has(name) && aliasProse(name, stripChars(value, XML_SPACE))) continue;
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

function userinfoPair(token: string, at: number): [number, number] | undefined {
  // The first `name:` before the last @, found by one forward scan.
  let run = 0;
  for (let pos = 0; pos < at; pos += 1) {
    const ch = token[pos];
    if (ch === ":" && pos > run) return [run, pos + 1];
    if (":=<>\"'({[]@".includes(ch)) run = pos + 1;
  }
  return undefined;
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
    const at = token.lastIndexOf("@");
    const pair = at > 0 ? userinfoPair(token, at) : undefined;
    if (pair && !secretName(token.slice(pair[0], pair[1] - 1))
        && !(["mailto:", "xmpp:", "sip:", "sips:"].includes(token.slice(pair[0], pair[1]).toLowerCase()) && !token.slice(pair[1], at).includes(":"))) {
      spans.push([offset(start + pair[0]), offset(start + at + 1), "[redacted-credentials]@"]);
    }
    start = end + 1;
  }
  return spans;
}

// Bot and webhook tokens travel in URL paths. Their extent is ambiguous, so
// the mask runs to the next literal whitespace.
const PATH_TOKEN = /\/bot(?=[0-9]{3,}:)|hooks\.slack\.com\/services\/[^/ \t\r\n\v\f]+\/[^/ \t\r\n\v\f]+\/|discord(?:app)?\.com\/api\/webhooks\/[0-9]+\/|(?<![A-Za-z0-9_])(?=xox[abposr]-)/gi;

function pathTokenSpans(text: string, offsets?: number[]): Span[] {
  const offset = (pos: number) => offsets ? offsets[pos] : pos;
  const spans: Span[] = [];
  let cursor = 0;
  for (const match of text.matchAll(PATH_TOKEN)) {
    if (match.index < cursor) continue;
    const start = match.index + match[0].length;
    let end = start;
    while (end < text.length && !SPACE.includes(text[end])) end += 1;
    if (end > start) spans.push([offset(start), offset(end), "[redacted]"]);
    cursor = end;
  }
  return spans;
}

// Longer records are masked whole: no scanner runs on an unbounded input.
export const REDACTION_LIMIT = 1_000_000;

function codePointsExceed(text: string, limit: number): boolean {
  if (text.length <= limit) return false;
  let count = 0;
  for (let pos = 0; pos < text.length; pos += 1) {
    const unit = text.charCodeAt(pos);
    if (unit >= 0xd800 && unit <= 0xdbff && pos + 1 < text.length) {
      const next = text.charCodeAt(pos + 1);
      if (next >= 0xdc00 && next <= 0xdfff) pos += 1;
    }
    count += 1;
    if (count > limit) return true;
  }
  return false;
}

export function redactCredentials(text: string): string {
  // Collect against the original record, so no detector can erase the context
  // needed by another. A fixed number of ordered streams keeps merging linear.
  // Python counts code points; so does the limit.
  if (codePointsExceed(text, REDACTION_LIMIT)) return "[redacted-oversized]";
  const [view, offsets] = urlView(text);
  const streams = [urlSpans(text), bareUserinfoSpans(text)];
  for (const scan of [xmlSpans, namedPairSpans, cliSpans, pathTokenSpans, keyedSpans]) {
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
