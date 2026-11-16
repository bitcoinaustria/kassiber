// Measures the painted bounds of every connection/rail logo the desktop draws, so the Rust side
// (src/logos.rs `BOUNDS`) can size logos by their artwork rather than their canvas: an SVG with
// a padded viewBox and a full-bleed PNG then look the same size in the same frame.
//
//   node tools/logo-bounds.mjs ../../assets/logos/integrations [--sheet out.png]
//
// Offline: each file is read from disk and handed to headless Chromium as a data URL; nothing
// is fetched. Prints one Rust line per file, `(0xFNV1A64, [x0, y0, x1, y1]), // name`: the file's
// FNV-1a hash and its painted box (alpha > 8) in units of the image's longer side.
import { readFileSync, readdirSync } from "node:fs";
import { join, extname } from "node:path";
import { tmpdir } from "node:os";
import { launch } from "../parity/cdp.mjs";

const dir = process.argv[2];
/** FNV-1a 64 of the bytes, as 16 hex digits (src/logos.rs `fnv`). */
function fnv(bytes) {
  let h = 0xcbf29ce484222325n;
  for (const b of bytes) h = ((h ^ BigInt(b)) * 0x100000001b3n) & 0xffffffffffffffffn;
  return h.toString(16).padStart(16, "0");
}
const sheet = process.argv.includes("--sheet") ? process.argv[process.argv.indexOf("--sheet") + 1] : null;
const mime = { ".svg": "image/svg+xml", ".png": "image/png", ".jpg": "image/jpeg" };
const files = readdirSync(dir).filter(f => mime[extname(f)]).sort();
const c = await launch({ port: 9471, profile: join(tmpdir(), `logo-bounds-${process.pid}`), width: 1200, height: 900 });
try {
  const out = [];
  for (const f of files) {
    const url = `data:${mime[extname(f)]};base64,${readFileSync(join(dir, f)).toString("base64")}`;
    const r = await c.eval(`new Promise((ok, bad) => {
      const img = new Image();
      img.onload = () => {
        const N = 512, cv = document.createElement("canvas");
        cv.width = cv.height = N;
        const g = cv.getContext("2d");
        const nw = img.naturalWidth || N, nh = img.naturalHeight || N;
        const k = Math.min(N / nw, N / nh), w = Math.round(nw * k), h = Math.round(nh * k);
        g.drawImage(img, 0, 0, w, h);
        const d = g.getImageData(0, 0, w, h).data;
        let x0 = w, y0 = h, x1 = -1, y1 = -1;
        for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) if (d[(y * w + x) * 4 + 3] > 8) {
          if (x < x0) x0 = x; if (x > x1) x1 = x; if (y < y0) y0 = y; if (y > y1) y1 = y;
        }
        ok([x0 / w, y0 / h, (x1 + 1) / w, (y1 + 1) / h, w, h]);
      };
      img.onerror = () => bad("decode");
      img.src = ${JSON.stringify(url)};
    })`);
    out.push([f, r]);
    const f3 = v => v.toFixed(3);
    const m = Math.max(r[4], r[5]), sx = r[4] / m, sy = r[5] / m;
    console.log(`    (0x${fnv(readFileSync(join(dir, f)))}, [${f3(r[0] * sx)}, ${f3(r[1] * sy)}, ${f3(r[2] * sx)}, ${f3(r[3] * sy)}]), // ${f}`);
  }
  if (sheet) {
    // Every logo at its canvas (dashed) with its measured bounds (red), for eyeballing.
    await c.viewport(1200, 900);
    const cells = out.map(([f, r]) => {
      const url = `data:${mime[extname(f)]};base64,${readFileSync(join(dir, f)).toString("base64")}`;
      return `<div style="display:inline-block;margin:6px;text-align:center;font:10px sans-serif;width:120px">
        <div style="position:relative;width:96px;height:96px;margin:auto;outline:1px dashed #999;background:#ddd">
          <img src="${url}" style="width:96px;height:96px;object-fit:contain">
          <div style="position:absolute;left:${r[0] * 96}px;top:${r[1] * 96}px;width:${(r[2] - r[0]) * 96}px;height:${(r[3] - r[1]) * 96}px;outline:1px solid red"></div>
        </div>${f}</div>`;
    }).join("");
    await c.eval(`document.body.innerHTML = ${JSON.stringify(cells)}; new Promise(r => setTimeout(r, 300))`);
    await c.shot(sheet);
  }
} finally {
  c.close();
}
