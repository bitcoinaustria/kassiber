import spec from "../../../tests/fixtures/secret_redaction_generator.json";

export { spec };

export function* generatedCases() {
  let state = spec.seed;
  function pick(name: "schemes" | "hosts" | "ports" | "users" | "userinfo_separators" | "delimiters" | "keys" | "helps" | "nonsecrets" | "flag_prefixes" | "malformed_separators"): string {
    state = (Math.imul(1664525, state) + 1013904223) >>> 0;
    return spec[name][(state >>> 8) % spec[name].length];
  }
  const quoted = JSON.stringify;
  for (let index = 0; index < spec.count; index += 1) {
    let sentinel = "QZX" + [...index.toString(16).padStart(8, "0")].map((ch) => "BCDFGJKLMNQRVXYZ"[Number.parseInt(ch, 16)]).join("") + "ZVQJK";
    const left = sentinel.slice(0, 8);
    const right = sentinel.slice(8);
    const scheme = pick("schemes"), host = pick("hosts"), port = pick("ports");
    const user = pick("users"), separator = pick("userinfo_separators");
    const delimiter = pick("delimiters"), key = pick("keys");
    const help = pick("helps"), keep = pick("nonsecrets");
    const flagPrefix = pick("flag_prefixes");
    const malformed = pick("malformed_separators");
    const url = `${scheme}://${host}${port}`;
    const secret = left + "@" + right + ['\\"9', "'007", "\\\\", "1234\n"][Math.floor(index / 20) % 4];
    const q = quoted(secret);
    const shape = index % 20;
    let text: string;
    if (shape === 0) text = `${scheme}://${user}:${left}${separator}${right}@${host}${port}/${delimiter}`;
    else if (shape === 1) text = '{"error":' + quoted("cannot reach (" + url + delimiter) + ',"' + key + '":' + q + '}';
    else if (shape === 2) text = key + "=" + q;
    else if (shape === 3) text = '{"' + key + '":' + q + ',"url":' + quoted(url) + '}';
    else if (shape === 4) text = key + ": " + q;
    else if (shape === 5) text = "--" + key + "=" + q;
    else if (shape === 6) text = "cmd " + flagPrefix + key + " " + q;
    else if (shape === 7) text = "Authorization: " + ["Basic", "Bearer", "Token", "Digest", "Negotiate"][Math.floor(index / 20) % 5] + " " + sentinel + "\n";
    else if ([8, 9, 16].includes(shape)) {
      text = '{"backend":{"url":' + quoted(url) + ',"' + key + '":' + q + '}}';
      for (let layer = 0; layer < Math.floor(index / 20) % 4; layer += 1) text = quoted(text);
      if (shape === 9) text = "'" + text.replaceAll("\\", "\\\\").replaceAll("'", "\\'") + "'";
    } else if (shape === 10) text = `${scheme}://u:${left}${malformed}${right}@${host}${port}/`;
    else if (shape === 11) text = "url=" + url + "|" + key + "=" + q + "|status=ready";
    else if (shape === 12) { text = help; sentinel = ""; }
    else if (shape === 13) text = "cmd " + flagPrefix + key + " " + sentinel + (Math.floor(index / 20) % 2 ? "." : "") + " --verbose";
    else if (shape === 14) text = key + "=" + left + "@" + right + delimiter;
    else if (shape === 15) text = "url=" + url + "&rpcpassword=" + left + "@" + right;
    else if (shape === 17) text = key + "=" + quoted(secret + "\\");
    else if (shape === 18) text = `${scheme}://u:${left}@ss@${right}@${host}${port}/`;
    else text = "{'" + key + "': '" + secret.replaceAll("\\", "\\\\").replaceAll("'", "\\'") + "'}";
    yield { index, input: text + "\n" + keep + "\n" + help, sentinel, preserved: [keep, help] };
  }
}
