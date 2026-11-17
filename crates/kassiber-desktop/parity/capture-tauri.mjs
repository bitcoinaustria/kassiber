// Captures every scene of scenes.json from the React desktop (production bundle, `vite preview`
// with the daemon bridge, headless Chromium over CDP): a PNG and a DOM dump per scene.
//
//   node parity/capture-tauri.mjs <out-dir>        (ONLY=a,b to limit scenes)
//
// Env (see parity/run.sh): PARITY_DIST (built bundle), KASSIBER_BIN, KASSIBER_DEMO_DATA_ROOT and
// the isolated KASSIBER_*_FILE preference paths. The preview server and the browser are started
// here and stopped on exit.
import { spawn, spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { launch, sleep } from "./cdp.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, "../../..");
const out = process.argv[2];
if (!out) throw new Error("usage: capture-tauri.mjs <out-dir>");
mkdirSync(out, { recursive: true });
const cfg = JSON.parse(readFileSync(join(here, "scenes.json"), "utf8"));
// One extra file per area (scenes.d/<area>.json), so parallel workers never edit the shared list.
try {
  for (const f of readdirSync(join(here, "scenes.d")).filter(f => f.endsWith(".json")).sort())
    cfg.scenes.push(...JSON.parse(readFileSync(join(here, "scenes.d", f), "utf8")).scenes);
} catch {}
const ONLY = process.env.ONLY ? process.env.ONLY.split(",") : null;
// Demo-book fixtures (parity/fixtures.json): `{fx:name}` / `{fx:name.field}` in paths and steps,
// and named book variants a scene asks for with `"book": "<variant>"`.
const FIXTURES = JSON.parse(readFileSync(join(here, "fixtures.json"), "utf8"));
const expand = (s, saved = {}) => s.replace(/\{fx:([a-z_]+)(?:\.([a-z_]+))?\}/g, (_, n, f) => {
  const v = n === "saved" ? saved[f] : FIXTURES.fixtures[n]?.[f || "id"];
  if (v === undefined) throw new Error(`unknown fixture {fx:${n}${f ? "." + f : ""}}`);
  return v;
});
// Builds a variant on a copy of the Tauri book under the run directory (the book itself is never
// written): the variant's local CLI steps, with egress switched off for them.
function buildBook(variant) {
  const book = FIXTURES.books[variant];
  if (!book) throw new Error(`no books.${variant} in fixtures.json`);
  const root = join(out, ".books", variant, "data");
  rmSync(join(out, ".books", variant), { recursive: true, force: true });
  mkdirSync(root, { recursive: true });
  for (const f of ["kassiber.sqlite3", "kassiber.sqlite3-wal"]) {
    const from = join(process.env.KASSIBER_DEMO_DATA_ROOT, f);
    if (existsSync(from)) copyFileSync(from, join(root, f));
  }
  const saved = {};
  for (const st of book.steps) {
    const args = st.args.map(a => expand(a, saved));
    const r = spawnSync(process.env.KASSIBER_BIN, ["--data-root", root, "--machine", ...args], { env: { ...process.env, KASSIBER_NO_EGRESS: "1" }, encoding: "utf8" });
    if (r.status !== 0) throw new Error(`book ${variant}: kassiber ${args[0]} failed: ${(r.stderr || "").trim().split("\n").pop()}`);
    if (st.save) saved[st.save] = JSON.parse(r.stdout).data.id;
  }
  return root;
}
const PORT = Number(process.env.PARITY_PORT || 5399);
const BASE = `http://127.0.0.1:${PORT}`;
const SEED_ASSET = readdirSync(join(process.env.PARITY_DIST, "assets")).find(f => f.endsWith(".png"));
const IDENTITY = { name: "Full Accounting", workspace: "Regtest Demo", profile: "Full Accounting", country: "Generic", encrypted: false, databaseMode: "plaintext" };

// ---- preview server ---------------------------------------------------------------------------
const viteArgs = ["exec", "vite", "preview", "--outDir", process.env.PARITY_DIST, "--port", String(PORT), "--strictPort", "--host", "127.0.0.1"];
const hasMise = spawnSync("sh", ["-c", "command -v mise"]).status === 0;
// One server per data root: a scene on a book variant restarts it on that copy.
let server = null, serverRoot = null, serverLog = "";
async function startServer(dataRoot) {
  server = spawn(hasMise ? "mise" : "pnpm", hasMise ? ["exec", "pnpm@10.33.0", "node@24", "--", "pnpm", ...viteArgs] : viteArgs, {
    cwd: join(repo, "ui-tauri"),
    env: { ...process.env, VITE_DAEMON: "bridge", KASSIBER_DEV_DATA_ROOT: dataRoot },
    stdio: ["ignore", "pipe", "pipe"],
    detached: true,
  });
  serverRoot = dataRoot;
  server.stdout.on("data", d => (serverLog += d));
  server.stderr.on("data", d => (serverLog += d));
  for (let i = 0; i < 200; i++) {
    // Bounded probes: a server going down can leave a socket that never answers.
    try { if ((await fetch(BASE, { signal: AbortSignal.timeout(2000) })).ok) break; } catch {}
    await sleep(150);
  }
}
async function stopServer() {
  try { process.kill(-server.pid, "SIGTERM"); } catch {}
  // strictPort: wait until the port is free before the next server binds it.
  for (let i = 0; i < 100; i++) {
    try { await fetch(BASE, { signal: AbortSignal.timeout(1000) }); } catch { break; }
    await sleep(100);
  }
  server = null;
}
const stopAll = () => {
  try { if (server) process.kill(-server.pid, "SIGTERM"); } catch {}
  try { c?.close(); } catch {}
};
process.on("exit", stopAll);
process.on("SIGINT", () => process.exit(130));
await startServer(process.env.KASSIBER_DEMO_DATA_ROOT);

// ---- browser ----------------------------------------------------------------------------------
const profile = join(out, ".chrome-profile");
var c = await launch({ port: Number(process.env.PARITY_CDP_PORT || 9399), profile, width: cfg.width, height: cfg.height });
// Count bridge requests in flight, so "settled" means the daemon answered everything.
await c.send("Page.addScriptToEvaluateOnNewDocument", {
  source: `(() => { const of = window.fetch; window.__kbPending = 0; window.__kbDone = 0;
    window.fetch = async function (i, init) { const u = typeof i === "string" ? i : i.url; const b = u.includes("/__kassiber__/");
      if (b) window.__kbPending++; try { return await of.apply(this, arguments); } finally { if (b) { window.__kbPending--; window.__kbDone++; } } }; })();`,
});

// The shipped macOS app draws its own title bar beside the traffic lights (windowChrome.ts:
// 40 px row, 80 px leading inset) only inside Tauri; a browser has neither. Rune draws the macOS
// chrome, so on macOS the capture forces the same insets the Tauri shell sets (PARITY_MAC_CHROME=0
// turns it off). `!important` in a stylesheet beats WindowFrame's inline custom properties.
if ((process.env.PARITY_MAC_CHROME ?? (process.platform === "darwin" ? "1" : "0")) === "1") {
  await c.send("Page.addScriptToEvaluateOnNewDocument", {
    source: `document.addEventListener("DOMContentLoaded", () => { const s = document.createElement("style");
      s.textContent = ":root { --kb-traffic-light-inset: 80px !important; --kb-native-titlebar-height: 40px !important; }";
      document.head.appendChild(s); });`,
  });
}

async function settle(ready, timeout = 45000) {
  const t0 = Date.now();
  const cond = {
    overview: `document.body.innerText.includes("Recent Transactions") || document.body.innerText.includes("Letzte Transaktionen")`,
    transactions: `document.querySelectorAll('#transactions-table [role=row], #transactions-table tbody tr').length > 3`,
    detail: `!!document.querySelector('[role=tab]') && document.querySelectorAll('[role=tab]').length >= 5`,
  }[ready] || "true";
  await c.waitFor(cond, timeout).catch(() => {});
  await c.eval(`document.fonts.ready.then(() => true)`);
  // Quiet for 700 ms: nothing in flight and no new requests.
  let quietSince = Date.now(), last = -1;
  while (Date.now() - t0 < timeout) {
    const s = await c.eval(`[window.__kbPending, window.__kbDone]`);
    if (s[0] > 0 || s[1] !== last) { quietSince = Date.now(); last = s[1]; }
    if (Date.now() - quietSince > 700) break;
    await sleep(100);
  }
}

// Element lookup for steps: label / labelPrefix / text / textIncludes (+ sel, nth) / css.
function finder(t) {
  const nth = t.nth || 0;
  if (t.css) return `[...document.querySelectorAll(${JSON.stringify(t.css)})].filter(v)[${nth}]`;
  if (t.label) return `[...document.querySelectorAll('[aria-label=${JSON.stringify(t.label)}]')].filter(v)[${nth}]`;
  if (t.labelPrefix) return `[...document.querySelectorAll('[aria-label^=${JSON.stringify(t.labelPrefix)}]')].filter(v)[${nth}]`;
  const sel = JSON.stringify(t.sel || "*");
  if (t.text) return `[...document.querySelectorAll(${sel})].filter(v).filter(e => (e.innerText ?? e.textContent ?? "").trim() === ${JSON.stringify(t.text)})[${nth}]`;
  if (t.textIncludes) return `[...document.querySelectorAll(${sel})].filter(v).filter(e => (e.innerText ?? e.textContent ?? "").includes(${JSON.stringify(t.textIncludes)}))[${nth}]`;
  throw new Error("bad target " + JSON.stringify(t));
}
const VIS = `const v = e => { const r = e.getBoundingClientRect(); const s = getComputedStyle(e); return r.width > 0 && r.height > 0 && s.visibility !== "hidden" && s.display !== "none"; };`;
async function rectOf(t) {
  return c.eval(`(() => { ${VIS} const e = ${finder(t)}; if (!e) return null; e.scrollIntoView?.({ block: "nearest" }); const r = e.getBoundingClientRect(); return { l: r.left, t: r.top, w: r.width, h: r.height }; })()`);
}
const MODS = { alt: 1, ctrl: 2, meta: 4, shift: 8 };
async function step(s) {
  if (s.wait) return sleep(s.wait);
  if (s.waitText) return c.waitFor(`document.body.innerText.includes(${JSON.stringify(s.waitText)})`, 15000);
  if (s.waitCss) return c.waitFor(`!!document.querySelector(${JSON.stringify(s.waitCss)})`, 15000);
  // Scrolls every scroller inside the matched element back to the top: clicks scroll their
  // target into view (rectOf), which Rune's scenes don't; this restores the opening position.
  if (s.scrollTop) return c.eval(`document.querySelectorAll(${JSON.stringify(s.scrollTop)}).forEach(r => [r, ...r.querySelectorAll("*")].forEach(e => { if (e.scrollTop) e.scrollTop = 0; }))`);
  if (s.key) {
    const mods = (s.mods || []).reduce((m, k) => m | MODS[k], 0);
    const code = s.key.length === 1 ? `Key${s.key.toUpperCase()}` : s.key;
    const vk = s.key.length === 1 ? s.key.toUpperCase().charCodeAt(0) : undefined;
    return c.key(s.key, code, mods, vk);
  }
  const t = s.click || s.hover;
  let x, y;
  if (t.xy) [x, y] = t.xy;
  else {
    const r = await rectOf(t);
    if (!r) throw new Error("no element for " + JSON.stringify(t));
    x = r.l + r.w * (t.fx ?? 0.5);
    y = r.t + r.h * (t.fy ?? 0.5);
  }
  if (s.hover) return c.mouse("mouseMoved", x, y);
  return c.click(x, y);
}

// The DOM dump the diff reads: visible texts, interactive elements, big containers.
const DUMP = `(() => { ${VIS}
  const rgb = s => s; const out = { texts: [], controls: [], containers: [], viewport: [innerWidth, innerHeight] };
  const inView = r => r.right > 0 && r.bottom > 0 && r.left < innerWidth && r.top < innerHeight;
  // Text: each element with its own non-empty text node, rendered text (text-transform applied).
  for (const el of document.querySelectorAll("body *")) {
    if (!v(el) || ["SCRIPT", "STYLE", "NOSCRIPT"].includes(el.tagName)) continue;
    const own = [...el.childNodes].filter(n => n.nodeType === 3 && n.textContent.trim());
    if (!own.length) continue;
    const range = document.createRange(); range.selectNodeContents(el);
    const rr = [...range.getClientRects()].filter(r => r.width > 0);
    if (!rr.length) continue;
    const l = Math.min(...rr.map(r => r.left)), t = Math.min(...rr.map(r => r.top)), rt = Math.max(...rr.map(r => r.right)), b = Math.max(...rr.map(r => r.bottom));
    const box = { left: l, top: t, right: rt, bottom: b };
    if (!inView(box)) continue;
    const cs = getComputedStyle(el);
    if (parseFloat(cs.opacity) === 0) continue;
    let o = el, op = 1; while (o && o !== document.body) { op *= parseFloat(getComputedStyle(o).opacity); o = o.parentElement; }
    out.texts.push({ text: (el.innerText || el.textContent).trim(), rect: [l, t, rt - l, b - t], fontSize: parseFloat(cs.fontSize), fontWeight: Number(cs.fontWeight) || 400,
      fontFamily: cs.fontFamily.split(",")[0].replace(/["']/g, ""), color: (el instanceof SVGElement && cs.fill && cs.fill !== "none" && !cs.fill.startsWith("url")) ? cs.fill : cs.color, opacity: +op.toFixed(3), letterSpacing: cs.letterSpacing, lineHeight: cs.lineHeight,
      textTransform: cs.textTransform, blur: cs.filter.includes("blur") });
  }
  // Interactive elements and their accessible names.
  const nameOf = e => { const lb = e.getAttribute("aria-labelledby"); const byId = lb ? lb.split(/\\s+/).map(i => document.getElementById(i)?.innerText || "").join(" ").trim() : "";
    return (e.getAttribute("aria-label") || byId || (e.innerText || "").trim() || e.getAttribute("title") || e.getAttribute("placeholder") || e.value || "").replace(/\\s+/g, " ").trim(); };
  const sel = 'button, a[href], input, select, textarea, [role=button], [role=link], [role=menuitem], [role=menuitemcheckbox], [role=menuitemradio], [role=tab], [role=checkbox], [role=switch], [role=radio], [role=option], [role=combobox], [role=slider], [tabindex]:not([tabindex="-1"])';
  for (const e of document.querySelectorAll(sel)) {
    if (!v(e)) continue; const r = e.getBoundingClientRect(); if (!inView(r)) continue;
    out.controls.push({ role: e.getAttribute("role") || e.tagName.toLowerCase(), name: nameOf(e), rect: [r.left, r.top, r.width, r.height],
      disabled: e.disabled || e.getAttribute("aria-disabled") === "true", checked: e.getAttribute("aria-checked") || e.getAttribute("aria-pressed") || e.getAttribute("data-state"),
      expanded: e.getAttribute("aria-expanded"),
      // Hidden from assistive technology behind an open modal (Radix hideOthers / inert), as Rune's
      // modal makes the background inert: the diff compares only what AT can reach.
      atHidden: !!e.closest('[aria-hidden="true"], [inert]') });
  }
  // Containers: painted boxes (background, border or shadow) big enough to matter.
  for (const e of document.querySelectorAll("body *")) {
    if (!v(e)) continue; const r = e.getBoundingClientRect(); if (r.width < 60 || r.height < 24 || !inView(r)) continue;
    const cs = getComputedStyle(e); const bg = cs.backgroundColor; const bw = parseFloat(cs.borderTopWidth) + parseFloat(cs.borderLeftWidth);
    const painted = (bg && !bg.endsWith(", 0)") && bg !== "transparent") || bw > 0 || cs.boxShadow !== "none";
    if (!painted) continue;
    out.containers.push({ tag: e.tagName.toLowerCase(), cls: (e.className && e.className.baseVal === undefined ? String(e.className) : "").slice(0, 120), rect: [r.left, r.top, r.width, r.height],
      bg, border: bw > 0 ? cs.borderTopColor : null, borderWidth: bw / 2, radius: cs.borderTopLeftRadius, shadow: cs.boxShadow !== "none" });
    if (out.containers.length > 600) break;
  }
  return out; })()`;

const results = {};
// Scenes on the plain book first, then each book variant's scenes together (one server start each).
const order = cfg.scenes.filter(sc => !ONLY || ONLY.includes(sc.name)).map((sc, i) => [sc, i]).sort((a, b) => (a[0].book || "").localeCompare(b[0].book || "") || a[1] - b[1]).map(([sc]) => sc);
const books = {};
for (const sc of order) {
  const t = { ...sc.tauri, path: expand(sc.tauri.path), steps: JSON.parse(expand(JSON.stringify(sc.tauri.steps || []))) };
  const width = sc.width || cfg.width, height = sc.height || cfg.height;
  try {
    const root = sc.book ? (books[sc.book] ??= buildBook(sc.book)) : process.env.KASSIBER_DEMO_DATA_ROOT;
    if (root !== serverRoot) {
      await stopServer();
      await startServer(root);
    }
    await c.viewport(width, height);
    // Seed localStorage from a static same-origin file: no app code runs there, so nothing can
    // persist its own defaults over the seeded state (the SPA at "/" races and wins otherwise).
    await c.send("Page.navigate", { url: `${BASE}/assets/${SEED_ASSET}` });
    await c.waitFor(`document.readyState === "complete"`, 10000);
    const state = { identity: IDENTITY, theme: "dark", currency: "btc", privacyMode: "normal", navCollapsed: false, preAlphaBannerVisible: true, lang: "en", ...t.state };
    await c.eval(`localStorage.clear(); localStorage.setItem("kb.ui", JSON.stringify(${JSON.stringify({ state, version: 5 })}))`);
    await c.send("Page.navigate", { url: `${BASE}${t.path}` });
    await settle(t.ready || "overview");
    for (const s of t.steps) await step(s);
    await settle("none", 8000);
    await sleep(350);
    await c.shot(join(out, `${sc.name}.png`));
    writeFileSync(join(out, `${sc.name}.dom.json`), JSON.stringify(await c.eval(DUMP)));
    results[sc.name] = "ok";
  } catch (e) {
    results[sc.name] = "FAILED: " + String(e.message || e).slice(0, 300);
  }
  console.log(sc.name, results[sc.name]);
}
writeFileSync(join(out, "capture-tauri.json"), JSON.stringify(results, null, 1));
await stopServer();
rmSync(join(out, ".books"), { recursive: true, force: true });
if (Object.values(results).some(r => r !== "ok")) console.log("server log tail:\n" + serverLog.slice(-1500));
process.exit(0);
