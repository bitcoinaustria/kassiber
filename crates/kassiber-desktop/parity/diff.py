"""Tauri vs Rune parity diff (stdlib only; run with `python3 -I`).

    python3 -I parity/diff.py <tauri-dir> <rune-dir> <out-dir>

Per scene of scenes.json, given `<tauri-dir>/<scene>.png|.dom.json` and
`<rune-dir>/<scene>.png|.tree.json`:

  (a) pixels: ImageMagick fuzzed AE count (share of pixels that differ), RMSE, a red heatmap and a
      tauri | rune | heatmap side-by-side image;
  (b) text layout: DOM text items matched to Rune tree texts by normalised string (nearest first
      for duplicates); per match the left-edge offset dx, vertical-centre offset dy, width ratio
      (font size / weight / tracking proxy: Rune's tree has no font size) and colour ΔE (CIE76),
      plus texts found on one side only;
  (c) controls: interactive elements by accessible name, Tauri-only and Rune-only;
  (d) containers: large painted boxes matched by overlap, with position/size and fill deltas.

Writes `<out-dir>/report.md`, `<out-dir>/report.json` and `<out-dir>/img/<scene>.{side.jpg,heat.png}`.
"""

import json
import math
import os
import re
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import rasters  # noqa: E402  (macOS without ImageMagick)

HERE = os.path.dirname(os.path.abspath(__file__))
FUZZ = "8%"

# ---- colours -------------------------------------------------------------------------------------


def _srgb_from_linear(c):
    c = max(0.0, min(1.0, c))
    return 12.92 * c if c <= 0.0031308 else 1.055 * c ** (1 / 2.4) - 0.055


def _oklab_to_srgb(L, a, b):
    l_ = L + 0.3963377774 * a + 0.2158037573 * b
    m_ = L - 0.1055613458 * a - 0.0638541728 * b
    s_ = L - 0.0894841775 * a - 1.2914855480 * b
    l, m, s = l_ ** 3, m_ ** 3, s_ ** 3
    r = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s
    g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s
    bb = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s
    return tuple(_srgb_from_linear(v) for v in (r, g, bb))


def _num(tok, scale=1.0):
    tok = tok.strip()
    if tok.endswith("%"):
        return float(tok[:-1]) / 100.0 * scale
    if tok in ("none", ""):
        return 0.0
    return float(tok)


def parse_color(s):
    """CSS computed colour or `#rrggbb[aa]` → (r, g, b, a) in 0..1, or None."""
    if not s:
        return None
    s = s.strip()
    if s.startswith("#"):
        h = s[1:]
        if len(h) in (6, 8):
            v = [int(h[i:i + 2], 16) / 255 for i in range(0, len(h), 2)]
            return (v[0], v[1], v[2], v[3] if len(v) == 4 else 1.0)
        return None
    m = re.match(r"(\w[\w-]*)\((.*)\)$", s)
    if not m:
        return None
    fn, body = m.group(1), m.group(2).replace(",", " ")
    alpha = 1.0
    if "/" in body:
        body, a = body.split("/", 1)
        alpha = _num(a)
    parts = body.split()
    try:
        if fn in ("rgb", "rgba"):
            r, g, b = (_num(p, 255) / 255 for p in parts[:3])
            if len(parts) > 3:
                alpha = _num(parts[3])
            return (r, g, b, alpha)
        if fn == "color" and parts[0] == "srgb":
            r, g, b = (_num(p) for p in parts[1:4])
            return (r, g, b, alpha)
        if fn == "oklch":
            L, C, H = _num(parts[0]), _num(parts[1], 0.4), _num(parts[2])
            a, b = C * math.cos(math.radians(H)), C * math.sin(math.radians(H))
            return (*_oklab_to_srgb(L, a, b), alpha)
        if fn == "oklab":
            L, a, b = _num(parts[0]), _num(parts[1], 0.4), _num(parts[2], 0.4)
            return (*_oklab_to_srgb(L, a, b), alpha)
    except (ValueError, IndexError):
        return None
    return None


def _lab(rgb):
    def lin(c):
        return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4
    r, g, b = (lin(c) for c in rgb[:3])
    x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047
    y = 0.2126 * r + 0.7152 * g + 0.0722 * b
    z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883
    f = lambda t: t ** (1 / 3) if t > 0.008856 else 7.787 * t + 16 / 116
    fx, fy, fz = f(x), f(y), f(z)
    return (116 * fy - 16, 500 * (fx - fy), 200 * (fy - fz))


def delta_e(c1, c2, bg=(0.1, 0.1, 0.1)):
    """ΔE (CIE76) of two RGBA colours composited over `bg`."""
    if not c1 or not c2:
        return None
    comp = lambda c: tuple(c[i] * c[3] + bg[i] * (1 - c[3]) for i in range(3))
    a, b = _lab(comp(c1)), _lab(comp(c2))
    return round(math.dist(a, b), 1)


def hexc(c):
    return "#%02x%02x%02x%s" % (*(round(v * 255) for v in c[:3]), "" if c[3] >= 0.999 else "/%.2f" % c[3]) if c else None


# ---- text normalisation ------------------------------------------------------------------------


def norm(t):
    t = (t or "").replace(" ", " ").replace(" ", " ").replace(" ", " ")
    t = re.sub(r"\s+", " ", t).strip()
    return t


# ---- loaders -----------------------------------------------------------------------------------


def load_tauri(path):
    d = json.load(open(path))
    texts = []
    for t in d["texts"]:
        n = norm(t["text"])
        if not n:
            continue
        c = parse_color(t["color"])
        if c and t.get("opacity", 1) < 1:
            c = (c[0], c[1], c[2], c[3] * t["opacity"])
        texts.append({"text": n, "rect": t["rect"], "color": c, "size": t["fontSize"], "weight": t["fontWeight"], "blur": t.get("blur", False)})
    controls = [{"name": norm(c["name"]), "role": c["role"], "rect": c["rect"]} for c in d["controls"] if not c.get("atHidden")]
    containers = [{"rect": c["rect"], "fill": parse_color(c["bg"]), "radius": c.get("radius"), "tag": c.get("tag"), "cls": c.get("cls", "")} for c in d["containers"]]
    return texts, controls, containers, d.get("viewport", [1440, 900])


INTERACTIVE = {"button", "link", "menuitem", "menuitemcheckbox", "menuitemradio", "checkbox", "switch", "tab", "option",
               "combobox", "textbox", "textinput", "slider", "radio", "searchbox", "spinbutton", "togglebutton"}


def load_rune(path, vw, vh):
    d = json.load(open(path))
    texts, containers = [], []
    for e in d["elements"]:
        r = e.get("rect") or [0, 0, 0, 0]
        if r[2] <= 0 or r[3] <= 0 or r[0] >= vw or r[1] >= vh or r[0] + r[2] <= 0 or r[1] + r[3] <= 0:
            continue
        if (e.get("opacity", 1) or 0) <= 0.01:
            continue
        if e.get("text"):
            c = parse_color(e.get("color"))
            if c and e.get("opacity", 1) < 1:
                c = (c[0], c[1], c[2], c[3] * e["opacity"])
            texts.append({"text": norm(e["text"]), "rect": r, "color": c, "key": e.get("key") or e.get("path", ""), "blur": (e.get("blur") or 0) > 0})
        f = parse_color(e.get("fill"))
        if f and f[3] > 0.02 and r[2] >= 60 and r[3] >= 24:
            containers.append({"rect": r, "fill": f, "key": e.get("key") or e.get("path", "")})
    controls = []

    def walk(n):
        role = (n.get("role") or "").lower()
        acts = n.get("actions") or []
        b = n.get("bounds") or [0, 0, 0, 0]
        if (role in INTERACTIVE or "click" in acts) and b[2] > 0 and b[3] > 0 and b[0] < vw and b[1] < vh and b[0] + b[2] > 0 and b[1] + b[3] > 0:
            controls.append({"name": norm(n.get("name") or ""), "role": role, "rect": b, "key": n.get("key")})
        for ch in n.get("children") or []:
            walk(ch)

    walk(d["accessibility"])
    # Controls behind a Rune modal are inert and absent from the accessibility tree; count them as
    # present (flagged) so "missing" means absent, and report the semantic difference separately.
    ax_keys = {c["key"] for c in controls if c.get("key")}
    for e in d["elements"]:
        r = e.get("rect") or [0, 0, 0, 0]
        if not e.get("inert") or (e.get("role") or "").lower() not in INTERACTIVE or not e.get("label"):
            continue
        if e.get("key") in ax_keys or r[2] <= 0 or r[3] <= 0 or r[0] >= vw or r[1] >= vh:
            continue
        controls.append({"name": norm(e["label"]), "role": e["role"], "rect": r, "key": e.get("key"), "inert": True})
    return texts, controls, containers


# ---- ink boxes ---------------------------------------------------------------------------------


class Raw:
    """An RGB raster read through ImageMagick (`rgb:-`), for measuring glyph ink."""

    def __init__(self, path, w, h):
        self.w, self.h = w, h
        if rasters.HAVE_MAGICK:
            self.px = subprocess.run(["magick", path, "-resize", f"{w}x{h}!", "-depth", "8", "rgb:-"], capture_output=True, check=True).stdout
        else:
            self.px = rasters.read_rgb(path, w, h)

    def at(self, x, y):
        i = 3 * (y * self.w + x)
        return self.px[i], self.px[i + 1], self.px[i + 2]


def ink_box(img, rect, color, pad=2):
    """Bounding box of the pixels drawn in `color` inside `rect` (text ink), or None.

    The background is the most common colour on the region's border; a pixel counts as ink when
    it lies at least 35 % of the way from the background towards the text colour."""
    if not color:
        return None
    x0, y0 = max(0, int(rect[0]) - pad), max(0, int(rect[1]) - pad)
    x1, y1 = min(img.w, int(rect[0] + rect[2]) + pad + 1), min(img.h, int(rect[1] + rect[3]) + pad + 1)
    if x1 - x0 < 2 or y1 - y0 < 2 or (x1 - x0) * (y1 - y0) > 400_000:
        return None
    border = {}
    for x in range(x0, x1):
        for y in (y0, y1 - 1):
            p = img.at(x, y)
            border[p] = border.get(p, 0) + 1
    for y in range(y0, y1):
        for x in (x0, x1 - 1):
            p = img.at(x, y)
            border[p] = border.get(p, 0) + 1
    bg = max(border, key=border.get)
    a = color[3]
    tc = tuple(color[i] * 255 * a + bg[i] * (1 - a) for i in range(3))
    d = [tc[i] - bg[i] for i in range(3)]
    dd = sum(v * v for v in d)
    if dd < 30 * 30:
        return None
    bx0, by0, bx1, by1 = None, None, None, None
    for y in range(y0, y1):
        for x in range(x0, x1):
            p = img.at(x, y)
            t = sum((p[i] - bg[i]) * d[i] for i in range(3)) / dd
            if t >= 0.35:
                if bx0 is None:
                    bx0, by0, bx1, by1 = x, y, x, y
                else:
                    bx0, bx1, by1 = min(bx0, x), max(bx1, x), y
                    by0 = min(by0, y)
    if bx0 is None:
        return None
    return [bx0, by0, bx1 - bx0 + 1, by1 - by0 + 1]


# ---- matching ----------------------------------------------------------------------------------


def center(r):
    return (r[0] + r[2] / 2, r[1] + r[3] / 2)


def match_texts(tt, rt):
    by_t, by_r = {}, {}
    for i, t in enumerate(tt):
        by_t.setdefault(t["text"], []).append(i)
    for j, r in enumerate(rt):
        by_r.setdefault(r["text"], []).append(j)
    pairs, used_t, used_r = [], set(), set()
    for text, tis in by_t.items():
        rjs = by_r.get(text, [])
        cand = sorted(((math.dist(center(tt[i]["rect"]), center(rt[j]["rect"])) + (delta_e(tt[i]["color"], rt[j]["color"]) or 0) / 4, i, j) for i in tis for j in rjs))
        for dist, i, j in cand:
            if i in used_t or j in used_r:
                continue
            used_t.add(i)
            used_r.add(j)
            pairs.append((i, j))
    # Case-insensitive second pass (text-transform, label casing).
    rest_r = {}
    for j, r in enumerate(rt):
        if j not in used_r:
            rest_r.setdefault(r["text"].lower(), []).append(j)
    for i, t in enumerate(tt):
        if i in used_t:
            continue
        js = [j for j in rest_r.get(t["text"].lower(), []) if j not in used_r]
        if js:
            j = min(js, key=lambda j: math.dist(center(t["rect"]), center(rt[j]["rect"])))
            used_t.add(i)
            used_r.add(j)
            pairs.append((i, j))
    only_t = [tt[i] for i in range(len(tt)) if i not in used_t]
    only_r = [rt[j] for j in range(len(rt)) if j not in used_r]
    return pairs, only_t, only_r


def iou(a, b):
    x1, y1 = max(a[0], b[0]), max(a[1], b[1])
    x2, y2 = min(a[0] + a[2], b[0] + b[2]), min(a[1] + a[3], b[1] + b[3])
    inter = max(0, x2 - x1) * max(0, y2 - y1)
    union = a[2] * a[3] + b[2] * b[3] - inter
    return inter / union if union > 0 else 0


# ---- pixels ------------------------------------------------------------------------------------


def pixel_diff(tp, rp, img_dir, name, w, h):
    heat = os.path.join(img_dir, f"{name}.heat.png")
    side = os.path.join(img_dir, f"{name}.side.jpg")
    if not rasters.HAVE_MAGICK:
        ae_n, rmse = rasters.compare(tp, rp, heat, side, w, h, float(FUZZ.rstrip("%")) / 100)
        return round(100 * ae_n / (w * h), 2), round(rmse, 4)
    # Normalise both to the scene size (the Tauri PNG can be a few px off at fractional sizes).
    tn, rn = os.path.join(img_dir, f".{name}.t.png"), os.path.join(img_dir, f".{name}.r.png")
    subprocess.run(["magick", tp, "-resize", f"{w}x{h}!", tn], check=True)
    subprocess.run(["magick", rp, "-resize", f"{w}x{h}!", rn], check=True)
    ae = subprocess.run(["magick", "compare", "-metric", "AE", "-fuzz", FUZZ, "-highlight-color", "red", "-lowlight-color", "#00000040", tn, rn, heat], capture_output=True, text=True)
    rm = subprocess.run(["magick", "compare", "-metric", "RMSE", tn, rn, "null:"], capture_output=True, text=True)
    try:
        ae_n = float(ae.stderr.split()[0])
    except (ValueError, IndexError):
        ae_n = float("nan")
    m = re.search(r"\(([\d.e-]+)\)", rm.stderr)
    rmse = float(m.group(1)) if m else float("nan")
    subprocess.run(["magick", tn, rn, heat, "+append", "-resize", "2700x", "-quality", "70", side], check=True)
    for f in (tn, rn):
        os.remove(f)
    return round(100 * ae_n / (w * h), 2), round(rmse, 4)


# ---- per scene ---------------------------------------------------------------------------------


def scene_report(sc, tdir, rdir, img_dir, W, H):
    name = sc["name"]
    w, h = sc.get("width", W), sc.get("height", H)
    tp, tj = os.path.join(tdir, f"{name}.png"), os.path.join(tdir, f"{name}.dom.json")
    rp, rj = os.path.join(rdir, f"{name}.png"), os.path.join(rdir, f"{name}.tree.json")
    missing = [p for p in (tp, tj, rp, rj) if not os.path.exists(p)]
    if missing:
        return {"name": name, "error": "missing capture: " + ", ".join(os.path.basename(m) for m in missing)}
    px, rmse = pixel_diff(tp, rp, img_dir, name, w, h)
    tt, tc, tbox, vp = load_tauri(tj)
    rt, rc, rbox = load_rune(rj, w, h)
    pairs, only_t, only_r = match_texts(tt, rt)
    timg, rimg = Raw(tp, w, h), Raw(rp, w, h)
    deltas = []
    for i, j in pairs:
        a, b = tt[i], rt[j]
        ia = None if a["blur"] else ink_box(timg, a["rect"], a["color"])
        ib = None if b["blur"] else ink_box(rimg, b["rect"], b["color"])
        if ia and ib:
            # Glyph ink in both screenshots: left edge, vertical centre, width ratio, height delta.
            dx = ib[0] - ia[0]
            dy = (ib[1] + ib[3] / 2) - (ia[1] + ia[3] / 2)
            wr = ib[2] / ia[2] if ia[2] > 0 else None
            dh = ib[3] - ia[3]
            basis = "ink"
        else:
            # Layout boxes (Rune's can be stretched to its container: x and centre only).
            dx = b["rect"][0] - a["rect"][0]
            dy = center(b["rect"])[1] - center(a["rect"])[1]
            wr, dh, basis = None, None, "layout"
        de = delta_e(a["color"], b["color"])
        deltas.append({"text": a["text"][:80], "tauri": [round(v, 1) for v in (ia or a["rect"])], "rune": [round(v, 1) for v in (ib or b["rect"])],
                       "basis": basis, "dh": dh,
                       "dx": round(dx, 1), "dy": round(dy, 1), "width_ratio": round(wr, 3) if wr else None,
                       "tauri_size": a["size"], "tauri_weight": a["weight"], "dE": de,
                       "tauri_color": hexc(a["color"]), "rune_color": hexc(b["color"]), "rune_key": b["key"][-80:],
                       "blur": [a["blur"], b["blur"]]})
    for d in deltas:
        wr = d["width_ratio"] or 1
        d["score"] = round(abs(d["dx"]) + abs(d["dy"]) + 40 * abs(wr - 1) + 2 * abs(d["dh"] or 0) + (d["dE"] or 0) / 2, 1)
    deltas.sort(key=lambda d: -d["score"])
    pos = [math.hypot(d["dx"], d["dy"]) for d in deltas]
    # Controls by name (multiset).
    from collections import Counter
    cn_t = Counter(c["name"].lower() for c in tc if c["name"])
    cn_r = Counter(c["name"].lower() for c in rc if c["name"])
    rest_t = list((cn_t - cn_r).elements())
    rest_r = list((cn_r - cn_t).elements())
    # Fuzzy second pass: one name a prefix of the other ("back" / "back (alt+left)"), or word
    # sets overlapping by 60 % (row links whose names add amounts).
    words = lambda n: set(re.findall(r"[\w₿€.,:+-]+", n))
    ctl_only_t, fuzzy = [], []
    for n in rest_t:
        best = None
        for m in rest_r:
            wn, wm = words(n), words(m)
            j = len(wn & wm) / max(1, len(wn | wm))
            if n.startswith(m) or m.startswith(n) or j >= 0.6:
                best = m
                break
        if best is None:
            ctl_only_t.append(n)
        else:
            rest_r.remove(best)
            fuzzy.append([n, best])
    ctl_only_r = sorted(rest_r)
    ctl_only_t.sort()
    unnamed_t = sum(1 for c in tc if not c["name"])
    # Containers: greedy by IoU, report big ones.
    big_t = sorted((b for b in tbox if b["rect"][2] * b["rect"][3] >= 15000), key=lambda b: -b["rect"][2] * b["rect"][3])[:60]
    cont, used = [], set()
    for b in big_t:
        best = max(((iou(b["rect"], r["rect"]), k) for k, r in enumerate(rbox) if k not in used), default=(0, None))
        if best[0] >= 0.5:
            used.add(best[1])
            r = rbox[best[1]]
            d = [round(r["rect"][k] - b["rect"][k], 1) for k in range(4)]
            de = delta_e(b["fill"], r["fill"])
            if max(abs(v) for v in d) >= 2 or (de or 0) >= 3:
                cont.append({"tauri": [round(v, 1) for v in b["rect"]], "rune": [round(v, 1) for v in r["rect"]], "d": d, "dE": de,
                             "tauri_fill": hexc(b["fill"]), "rune_fill": hexc(r["fill"]), "tauri_radius": b["radius"], "tauri_cls": b["cls"][:80], "rune_key": r["key"][-60:]})
        else:
            cont.append({"tauri": [round(v, 1) for v in b["rect"]], "rune": None, "tauri_fill": hexc(b["fill"]), "tauri_radius": b["radius"], "tauri_cls": b["cls"][:80], "best_iou": round(best[0], 2)})
    return {
        "name": name, "pixel_diff_pct": px, "rmse": rmse,
        "texts_tauri": len(tt), "texts_rune": len(rt), "matched": len(pairs),
        "pos_mean": round(sum(pos) / len(pos), 1) if pos else None, "pos_max": round(max(pos), 1) if pos else None,
        "pos_p90": round(sorted(pos)[int(0.9 * (len(pos) - 1))], 1) if pos else None,
        "text_deltas": deltas, "texts_only_tauri": [{"text": t["text"][:100], "rect": [round(v) for v in t["rect"]]} for t in only_t],
        "texts_only_rune": [{"text": t["text"][:100], "rect": [round(v) for v in t["rect"]], "key": t["key"][-60:]} for t in only_r],
        "controls_tauri": len(tc), "controls_rune": len(rc), "controls_only_tauri": ctl_only_t, "controls_only_rune": ctl_only_r, "controls_renamed": fuzzy, "controls_inert_rune": sum(1 for c in rc if c.get("inert")), "controls_unnamed_tauri": unnamed_t,
        "containers": cont,
    }


def main():
    tdir, rdir, out = sys.argv[1:4]
    only = [s for s in os.environ.get("ONLY", "").split(",") if s]
    cfg = json.load(open(os.path.join(HERE, "scenes.json")))
    extra = os.path.join(HERE, "scenes.d")
    for f in sorted(os.listdir(extra)) if os.path.isdir(extra) else []:
        if f.endswith(".json"):
            cfg["scenes"] += json.load(open(os.path.join(extra, f)))["scenes"]
    img = os.path.join(out, "img")
    os.makedirs(img, exist_ok=True)
    reps = []
    for sc in cfg["scenes"]:
        if only and sc["name"] not in only:
            continue
        rep = scene_report(sc, tdir, rdir, img, cfg["width"], cfg["height"])
        reps.append(rep)
        print(rep["name"], rep.get("error") or f'{rep["pixel_diff_pct"]}% px, {rep["matched"]}/{rep["texts_tauri"]} texts, pos mean {rep["pos_mean"]} max {rep["pos_max"]}, -{len(rep["texts_only_tauri"])} texts, -{len(rep["controls_only_tauri"])} controls', flush=True)
    json.dump(reps, open(os.path.join(out, "report.json"), "w"), indent=1, ensure_ascii=False)
    write_md(reps, os.path.join(out, "report.md"))


def write_md(reps, path):
    L = ["# Tauri vs Rune parity report", "",
         ("Pixel diff = share of pixels differing beyond an 8 % fuzz. Text offsets are matched-text distances in points "
          "(left edge, vertical centre). Width ratio = Rune text width / Tauri text width. ΔE = CIE76 colour distance."), "",
         "| Scene | Pixel diff | RMSE | Texts matched | Pos mean / p90 / max | Only Tauri texts | Only Rune texts | Missing controls | Extra controls |",
         "| --- | --- | --- | --- | --- | --- | --- | --- | --- |"]
    for r in reps:
        if r.get("error"):
            L.append(f'| {r["name"]} | {r["error"]} | | | | | | | |')
            continue
        L.append(f'| {r["name"]} | {r["pixel_diff_pct"]} % | {r["rmse"]} | {r["matched"]}/{r["texts_tauri"]} | {r["pos_mean"]} / {r["pos_p90"]} / {r["pos_max"]} | '
                 f'{len(r["texts_only_tauri"])} | {len(r["texts_only_rune"])} | {len(r["controls_only_tauri"])} | {len(r["controls_only_rune"])} |')
    for r in reps:
        if r.get("error"):
            continue
        L += ["", f'## {r["name"]}', "", f'![side](img/{r["name"]}.side.jpg)', "", "Worst text deltas:", "",
              "| Text | basis | Tauri x,y,w,h | Rune x,y,w,h | dx | dy | width ratio | dh | Tauri size/weight | ΔE (tauri → rune) |", "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |"]
        for d in r["text_deltas"][:20]:
            L.append(f'| {d["text"][:50]} | {d["basis"]} | {d["tauri"]} | {d["rune"]} | {d["dx"]} | {d["dy"]} | {d["width_ratio"]} | {d["dh"]} | {d["tauri_size"]}/{d["tauri_weight"]} | {d["dE"]} ({d["tauri_color"]} → {d["rune_color"]}) |')
        if r["texts_only_tauri"]:
            L += ["", "Only in Tauri: " + "; ".join(f'`{t["text"][:60]}` @{t["rect"][:2]}' for t in r["texts_only_tauri"][:25])]
        if r["texts_only_rune"]:
            L += ["", "Only in Rune: " + "; ".join(f'`{t["text"][:60]}` @{t["rect"][:2]}' for t in r["texts_only_rune"][:25])]
        if r["controls_only_tauri"]:
            L += ["", "Controls missing in Rune: " + "; ".join(f"`{c}`" for c in r["controls_only_tauri"][:30])]
        if r["controls_only_rune"]:
            L += ["", "Controls only in Rune: " + "; ".join(f"`{c}`" for c in r["controls_only_rune"][:30])]
        if r["containers"]:
            L += ["", "Containers (Tauri → Rune; d = dx, dy, dw, dh):", ""]
            for c in r["containers"][:12]:
                if c.get("rune"):
                    L.append(f'- {c["tauri"]} → {c["rune"]} d={c["d"]} fill {c["tauri_fill"]} → {c["rune_fill"]} (ΔE {c["dE"]}), radius {c["tauri_radius"]} `{c["tauri_cls"][:50]}`')
                else:
                    L.append(f'- {c["tauri"]} fill {c["tauri_fill"]} radius {c["tauri_radius"]}: no Rune box (best IoU {c["best_iou"]}) `{c["tauri_cls"][:50]}`')
    open(path, "w").write("\n".join(L) + "\n")


if __name__ == "__main__":
    main()
