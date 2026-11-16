"""Which Rune font-smoothing mode draws text closest to headless Chromium on Linux?

    python3 -I parity/font_smoothing.py <tauri-dir> <out-dir> [scene]

Renders `scene` (default overview-dark) from the Rune build once per `--font-smoothing` mode,
matches texts against the Tauri capture (same as diff.py), and for every matched text whose ink
is found on both sides and whose ink boxes differ by at most 1 pt in size compares the glyphs:
  * ink coverage ratio (Rune / Chromium summed ink weight): stroke weight, 1.0 is equal;
  * RMSE of the two ink crops aligned on their top-left corner.
Needs the Rune env (CARGO_TARGET_DIR, KASSIBER_BIN, KASSIBER_DEMO_DATA_ROOT…).
"""

from pathlib import Path
import json
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import diff  # noqa: E402

MODES = ["default", "on", "off", "windfoil", "windfoil-chromium", "slug", "hinted-gpu", "hinted"]


def coverage(img, box, color):
    """Summed ink weight inside an ink box (0..1 per pixel, against the box's border colour)."""
    x, y, w, h = box
    bg = img.at(max(0, x - 2), max(0, y - 2))
    d = [color[i] * 255 - bg[i] for i in range(3)]
    dd = sum(v * v for v in d) or 1
    total = 0.0
    for yy in range(y, y + h):
        for xx in range(x, x + w):
            p = img.at(xx, yy)
            total += max(0.0, min(1.0, sum((p[i] - bg[i]) * d[i] for i in range(3)) / dd))
    return total


def crop_rmse(tp, rp, a, b, tmp):
    w, h = max(a[2], b[2]) + 2, max(a[3], b[3]) + 2
    ta, rb = os.path.join(tmp, "a.png"), os.path.join(tmp, "b.png")
    subprocess.run(["magick", tp, "-crop", f"{w}x{h}+{a[0] - 1}+{a[1] - 1}", "+repage", "-colorspace", "gray", ta], check=True)
    subprocess.run(["magick", rp, "-crop", f"{w}x{h}+{b[0] - 1}+{b[1] - 1}", "+repage", "-colorspace", "gray", rb], check=True)
    r = subprocess.run(["magick", "compare", "-metric", "RMSE", ta, rb, "null:"], capture_output=True, text=True)
    try:
        return float(r.stderr.split("(")[1].split(")")[0])
    except (IndexError, ValueError):
        return None


def main():
    tdir, out = sys.argv[1], sys.argv[2]
    scene = sys.argv[3] if len(sys.argv) > 3 else "overview-dark"
    rune_scene = {"overview-dark": "dark"}.get(scene, scene)
    os.makedirs(out, exist_ok=True)
    binary = os.path.join(os.environ["CARGO_TARGET_DIR"], "release", "kassiber-desktop")
    tp = os.path.join(tdir, f"{scene}.png")
    tt, _, _, _ = diff.load_tauri(os.path.join(tdir, f"{scene}.dom.json"))
    timg = diff.Raw(tp, 1440, 900)
    rows = []
    for mode in MODES:
        png, tree = os.path.join(out, f"{mode}.png"), os.path.join(out, f"{mode}.tree.json")
        extra = [] if mode == "default" else ["--font-smoothing", mode]
        for flag, path in (("--snap", png), ("--tree", tree)):
            subprocess.run([binary, "--scene", rune_scene, "--size", "1440x900", "--scale", "1", "--at", "0.5", *extra, flag, path], cwd="/tmp", capture_output=True, check=True)
        rt, _, _ = diff.load_rune(tree, 1440, 900)
        rimg = diff.Raw(png, 1440, 900)
        pairs, _, _ = diff.match_texts(tt, rt)
        covs, rmses, widths, n = [], [], [], 0
        for i, j in pairs:
            a, b = tt[i], rt[j]
            if a["blur"] or b["blur"] or len(a["text"]) < 3:
                continue
            ia, ib = diff.ink_box(timg, a["rect"], a["color"]), diff.ink_box(rimg, b["rect"], b["color"])
            if not ia or not ib or abs(ia[3] - ib[3]) > 1 or abs(ia[2] - ib[2]) > max(2, 0.08 * ia[2]):
                continue
            ca, cb = coverage(timg, ia, a["color"]), coverage(rimg, ib, b["color"])
            if ca <= 0:
                continue
            n += 1
            covs.append(cb / ca)
            widths.append(ib[2] / ia[2])
            r = crop_rmse(tp, png, ia, ib, out)
            if r is not None:
                rmses.append(r)
        med = lambda v: sorted(v)[len(v) // 2] if v else None
        rows.append({"mode": mode, "texts": n, "coverage_ratio_median": round(med(covs), 3) if covs else None,
                     "width_ratio_median": round(med(widths), 3) if widths else None, "glyph_rmse_mean": round(sum(rmses) / len(rmses), 4) if rmses else None})
        print(rows[-1], flush=True)
    Path(out, "font-smoothing.json").write_text(json.dumps(rows, indent=1), encoding="utf-8")


if __name__ == "__main__":
    main()
