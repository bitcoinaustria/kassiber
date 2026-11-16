// Minimal Chrome DevTools Protocol driver (node 24 global WebSocket; no packages). Adapted from the pilot's w-tauri tooling.
import { spawn } from "node:child_process";
import { existsSync, readdirSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

export async function launch({ port = 9333, profile, width = 1440, height = 900 }) {
  const proc = spawn(chromiumPath(), [
    "--headless=new", `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`,
    `--window-size=${width},${height}`, "--no-first-run", "--no-default-browser-check",
    "--hide-scrollbars", "--force-color-profile=srgb", "--enable-precise-memory-info", "about:blank",
  ], { stdio: "ignore" });
  let targets;
  for (let i = 0; i < 100; i++) {
    try { targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json(); if (targets.some(t => t.type === "page")) break; } catch {}
    await sleep(100);
  }
  const page = targets.find(t => t.type === "page");
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise(r => ws.addEventListener("open", r, { once: true }));
  let id = 0; const waiters = new Map(); const listeners = [];
  ws.addEventListener("message", ev => {
    const msg = JSON.parse(ev.data);
    if (msg.id && waiters.has(msg.id)) { const w = waiters.get(msg.id); waiters.delete(msg.id); msg.error ? w.reject(new Error(JSON.stringify(msg.error))) : w.resolve(msg.result); }
    else if (msg.method) listeners.forEach(l => l(msg));
  });
  const send = (method, params = {}) => new Promise((resolve, reject) => { const i = ++id; waiters.set(i, { resolve, reject }); ws.send(JSON.stringify({ id: i, method, params })); });
  const c = {
    proc, send, on: f => listeners.push(f),
    async eval(expr) {
      const r = await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true });
      if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
      return r.result.value;
    },
    async viewport(w, h) { await send("Emulation.setDeviceMetricsOverride", { width: w, height: h, deviceScaleFactor: 1, mobile: false }); },
    async shot(path) { const r = await send("Page.captureScreenshot", { format: "png" }); writeFileSync(path, Buffer.from(r.data, "base64")); },
    async waitFor(expr, timeout = 30000) {
      const t0 = Date.now();
      while (Date.now() - t0 < timeout) { try { if (await c.eval(expr)) return true; } catch {} await sleep(100); }
      throw new Error(`timeout waiting for ${expr}`);
    },
    async mouse(type, x, y, extra = {}) { await send("Input.dispatchMouseEvent", { type, x, y, button: "left", clickCount: type === "mouseMoved" ? 0 : 1, ...extra }); },
    async click(x, y) { await c.mouse("mouseMoved", x, y); await c.mouse("mousePressed", x, y); await c.mouse("mouseReleased", x, y); },
    async key(key, code, mods = 0, vk) { for (const type of ["keyDown", "keyUp"]) await send("Input.dispatchKeyEvent", { type, key, code, modifiers: mods, windowsVirtualKeyCode: vk }); },
    async rect(js) { return c.eval(`(()=>{const e=(${js}); if(!e) return null; e.scrollIntoView?.({block:"nearest"}); const r=e.getBoundingClientRect(); return {x:r.x+r.width/2,y:r.y+r.height/2,w:r.width,h:r.height,l:r.left,t:r.top};})()`); },
    async clickEl(js) { const r = await c.rect(js); if (!r) throw new Error(`no element: ${js}`); await c.click(r.x, r.y); return r; },
    close() { try { ws.close(); } catch {} proc.kill("SIGTERM"); },
  };
  await send("Page.enable"); await send("Runtime.enable"); await send("Performance.enable");
  return c;
}
export const sleep = ms => new Promise(r => setTimeout(r, ms));

/** `$CHROMIUM`, else Linux's /usr/bin/chromium, else the newest Playwright Chromium (macOS). */
function chromiumPath() {
  if (process.env.CHROMIUM) return process.env.CHROMIUM;
  if (existsSync("/usr/bin/chromium")) return "/usr/bin/chromium";
  const cache = join(homedir(), "Library/Caches/ms-playwright");
  const dirs = existsSync(cache) ? readdirSync(cache).filter(d => /^chromium-\d+$/.test(d)).sort((a, b) => Number(b.split("-")[1]) - Number(a.split("-")[1])) : [];
  for (const d of dirs) {
    for (const arch of ["chrome-mac-arm64", "chrome-mac"]) {
      const p = join(cache, d, arch, "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing");
      if (existsSync(p)) return p;
    }
  }
  throw new Error("no Chromium found; set CHROMIUM");
}
