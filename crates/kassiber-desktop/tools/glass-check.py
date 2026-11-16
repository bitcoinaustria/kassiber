#!/usr/bin/env python3
"""Glass overlay entrance check: is the backdrop blur there while an overlay fades in?

Rune draws a fading element as one isolated layer, and a `backdrop_blur` inside it sees only
what that layer has drawn: a glass surface under a fading full-window wrapper shows its tint
over a SHARP page until the fade's spring settles (~0.8 s), then the blur pops in. This opens
every glass overlay headlessly (`--script … --out frames/ --trace t.jsonl`) and measures, over
the pixels inside the glass that the settled frame shows smooth but the page behind shows
detailed, the mean horizontal gradient frame by frame:

  pre      the page before the overlay mounts (sharp)
  settled  the last frame (blurred, at rest)
  frost@N  the first frame, counted from the glass element's mount, at which the gradient is
           within 10 % of settled (90 % frosted); a fade with its blur reaches it as the fade
           does (~10–20 frames), a fade without its blur only once the trace stops animating
  rest@N   the frame the trace reports `animating: false` again
  mid      the residual (g - settled) / (pre - settled) at mount + 12 frames: ~0.2 or more
           while the tint shows over a sharp page, ~0 when the blur fades in with the surface

Run from anywhere (it uses $CARGO_TARGET_DIR's release binary unless --bin is given):

  python3 -I $CRATE/tools/glass-check.py [--bin PATH] [name …]
  python3 -I $CRATE/tools/glass-check.py --list

It only taps controls that open overlays (menus, selects, dialogs, sheets): nothing that syncs,
tests a connection or talks to a provider. Frames go to a temporary directory, removed after.
"""

import argparse
import json
import os
import shutil
import struct
import subprocess
import sys
import re
import tempfile

# Each overlay: start args, the steps that open it, and the glass element to measure.
#  steps: ("tap", key) | ("tapre", regex: the first key a pre-run shows that matches) |
#         ("key", name[, "cmd+shift"]) | ("file", path: answers the open file dialog) |
#         ("type", text) | ("wheel", x, y, dy) | ("wait", s). Each step runs 0.45 s after the last.
#  glass: a regex for the element's key (the last path segment in the trace); a leading "*"
#         measures the union of every match mounted with the latest one (the rows of a glass
#         surface that has no key or role of its own, which the trace doesn't list).
OVERLAYS = []
HERE = os.path.dirname(os.path.abspath(__file__))
CSV = os.path.join(HERE, "..", "src", "wallets", "add", "fixtures", "generic.csv")


def ov(name, args, steps, glass, note="", size="1440x900", needs=None):
    """`needs`: book data the overlay depends on; without it the opener is missing and the
    overlay is reported as not available rather than failed."""
    OVERLAYS.append({"name": name, "args": args, "steps": steps, "glass": glass, "note": note, "size": size, "needs": needs})


def R(path):
    return ["--scene", "route", "--route", path]


S = lambda name: ["--scene", name]  # noqa: E731
TX = ["--scene", "open-tx", "--tx", "{fx:full}"]  # parity/fixtures.json

# ---- shell: the anchored popups (shell::mount_popup_full), palette, settings, toasts, sync card ----
ov("shell-search", [], [("key", "k", "cmd")], "sh-search-dialog", "glass_dialog")
ov("shell-books", [], [("tap", "sh-book-crumb")], "sh-books-surface", "popover")
ov("shell-alpha", [], [("tap", "sh-alpha")], "sh-alpha-pop-surface", "popover")
ov("shell-refresh-menu", R("/transactions"), [("tap", "sh-refresh-caret")], "sh-refresh-menu-surface", "menu (only the caret; no item is pressed)")
ov("shell-theme-menu", R("/transactions"), [("tap", "sh-theme")], "sh-theme-menu-surface", "menu")
ov("shell-privacy-menu", [], [("tap", "sh-privacy")], "sh-privacy-menu-surface", "menu")
ov("shell-user-menu", [], [("tap", "sh-user")], "sh-user-menu-surface", "menu")
ov("shell-network", [], [("tap", "sh-network")], "sh-network-pop-surface", "popover (opening probes nothing)")
ov("shell-notifications", R("/transactions"), [("tap", "sh-bell")], "sh-notifications-surface", "popover")
ov("shell-settings-modal", [], [("tap", "sh-settings")], "set-backdrop", "root-level blur backdrop (its own fade)")
ov("shell-toast", S("reports-logs"), [("tap", "logs-copy")], "sh-toast1", "toast (headless clipboard)")
ov("shell-sync-card", S("sync-card"), [("tap", "sh-sync-close"), ("tap", "sh-bell"), ("tap", "sh-note-running")], "sh-sync-card", "card under a wrapper (fake sync state; nothing is sent)")
# ---- settings ----
ov("settings-select", [], [("tap", "sh-settings"), ("tap", "set-theme")], "set-theme-menu", "select")
ov("settings-dialog", [], [("tap", "sh-settings"), ("tap", "set-nav-data"), ("tap", "set-reset-welcome")], "set-confirm-dialog", "glass_dialog (never confirmed)")
# ---- overview / chart ----
ov("overview-add-connection", [], [("tap", "add-connection")], "add-connection-dialog", "dialog")
ov("chart-controls-sheet", [], [("tap", "gear-0"), ("tap", "gear-menu-0-item9")], "controls-scrim", "root-level blur scrim; the sheet slides in")
# ---- transaction detail ----
ov("txdetail-dialog", S("detail-details"), [("wheel", 1100, 600, -1500), ("tap", "txd-attach-links")], "txd-links-dialog", "dialog (never submitted)")
ov("txdetail-select", TX + ["--tab", "classify"], [("wheel", 1100, 600, -400), ("tap", "txd-label")], "txd-label-menu", "select")
ov("txdetail-add-menu", S("detail-details"), [("tap", "txd-add-surface")], "txd-add-menu", "menu")
# ---- assistant ----
ov("assistant-more-menu", R("/assistant"), [("tap", "asst-more")], "asst-more-menu", "menu")
ov("assistant-history-menu", R("/assistant"), [("tap", "asst-more"), ("tap", "asst-menu-history")], "asst-history-menu", "submenu")
ov("assistant-panel-add-menu", [], [("key", "i", "cmd"), ("tap", "asst-add-surface")], "asst-add-menu", "menu")
ov("assistant-panel-more-menu", S("assistant-thread-panel"), [("tap", "asst-panel-more")], "asst-panel-more-menu", "menu")
ov("assistant-picker", R("/assistant"), [("tap", "asst-picker-page")], "asst-pick-pop-page", "popover (cached reads only)")
ov("assistant-picker-permission", R("/assistant"), [("tap", "asst-picker-page"), ("tap", "asst-pick-perm")], "asst-pick-perm-menu", "menu")
ov("assistant-sheet", [], [("key", "i", "cmd")], "asst-sheet-scrim", "root-level blur scrim; the sheet slides in", size="1000x800")
# ---- review / reconcile ----
ov("reconcile-tools-year", R("/reconcile?mode=tools"), [("tap", "rc-tools-choose"), ("file", CSV), ("wait", 0.8), ("tap", "rc-tools-year")], "*rc-year-(all|[0-9]{4})", "popover (local cross-check)")
ov("reconcile-verify-backend", S("review-reconcile-checked"), [("tap", "rc-verify-backend")], "*rc-backend-.*", "menu", needs="two Esplora/Electrum backends")
ov("review-cause-unit-menu", R("/review"), [("tapre", "rv-cause-.*-unit")], "rv-cause-.*-unit-menu", "menu", needs="an approximate-price cause card")
ov("review-cause-dialog", R("/review"), [("tapre", "rv-cause-.*-accept")], "rv-prop-dialog", "dialog (never confirmed)", needs="a cause card with an accept action")
# ---- wallets ----
ov("wallets-detail-menu", S("wallets-detail-spending"), [("tap", "wd-more")], "wd-menu", "menu")
ov("wallets-detail-dialog", S("wallets-detail-spending"), [("tap", "wd-more"), ("tap", "wd-menu-edit")], "wd-edit", "dialog (never saved)")
ov("wallets-detail-select", S("wallets-detail-spending"), [("tap", "wd-more"), ("tap", "wd-menu-edit"), ("tap", "wd-edit-backend")], "wd-edit-backend-menu", "select")
ov("wallets-add-scanner", [], [("tap", "add-connection"), ("tap", "add-continue"), ("tap", "add-scan")], "scanner-dialog", "dialog (no camera)")
ov("wallets-add-select", [], [("tap", "add-connection"), ("tap", "add-continue"), ("tap", "connection-backend")], "connection-backend-menu", "select")
ov("wallets-node-sheet", S("wallets-node-channels"), [("tap", "node-channel-row-ch-alpha")], "node-sheet-overlay", "root-level blur overlay; the sheet slides in", needs="a Lightning node connection")
# ---- reports ----
ov("reports-activity-select", S("reports-activity"), [("tap", "act-date")], "act-date-menu", "select")
ov("reports-books-dialog", R("/books"), [("tap", "books-new-set")], "books-create-ws", "glass_dialog (never submitted)")
ov("reports-books-select", R("/books"), [("tapre", "books-new-in-.*"), ("tap", "books-sel-region")], "books-sel-region-menu", "select")
ov("reports-books-menu", R("/books"), [("tapre", "books-ws-actions-[0-9a-f-]+")], "books-ws-actions-.*-menu", "menu")
ov("reports-exit-tax-calendar", S("reports-exit-tax"), [("tap", "et-date")], "*et-(day-[0-9]+|cal-prev|cal-next)", "popover")
ov("reports-exit-tax-destination", S("reports-exit-tax"), [("tap", "et-destination")], "et-destination-menu", "select")
ov("reports-report-select", R("/reports"), [("tap", "rep-year")], "rep-year-menu", "select")


def script_for(steps, tail=1.6):
    events, t = [], 1.0
    for s in steps:
        if s[0] == "tap":
            events.append({"t": round(t, 3), "type": "tap", "key": s[1]})
        elif s[0] == "key":
            e = {"t": round(t, 3), "type": "key", "name": s[1]}
            for m in (s[2].split("+") if len(s) > 2 else []):
                e[m] = True
            events.append(e)
        elif s[0] == "type":
            events.append({"t": round(t, 3), "type": "type", "text": s[1]})
        elif s[0] == "wheel":
            events.append({"t": round(t, 3), "type": "wheel", "x": s[1], "y": s[2], "dy": s[3]})
        elif s[0] == "file":
            events.append({"t": round(t, 3), "type": "file-dialog", "path": s[1]})
        elif s[0] == "wait":
            t += s[1]
            continue
        t += 0.45
    events.append({"t": round(t + tail, 3), "type": "mark", "name": "end"})
    return {"events": events}


def run(binary, o, tmp, steps, frames, scale):
    sp = os.path.join(tmp, "script.json")
    json.dump(script_for(steps, 1.6 if frames else 1.2), open(sp, "w"))
    tr = os.path.join(tmp, "t.jsonl")
    cmd = [binary, "--size", o["size"], "--scale", str(scale), "--script", sp, "--trace", tr]
    if frames:
        cmd += ["--out", os.path.join(tmp, "frames") + "/"]
    r = subprocess.run(cmd + o["args"], capture_output=True, text=True, cwd=tmp)
    if r.returncode != 0:
        raise RuntimeError("run failed: " + (r.stderr.strip().splitlines() or ["?"])[-1][:160])
    return [json.loads(line) for line in open(tr)][1:]


def resolve(binary, o, tmp, scale):
    """Replaces each ("tapre", regex) with a tap on the first matching key a pre-run shows."""
    steps = []
    for s in o["steps"]:
        if s[0] != "tapre":
            steps.append(s)
            continue
        seen = set()
        for f in run(binary, o, tmp, steps + [("wait", 1.0)], False, scale):
            for m in f.get("m", []):
                seen.add(m["nm"].rsplit("/", 1)[-1])
        hit = sorted(k for k in seen if re.fullmatch(s[1], k))
        if not hit:
            raise RuntimeError(f"no key matches {s[1]!r}")
        steps.append(("tap", hit[0]))
    return steps


def crop_frames(paths, rect, tmp):
    x, y, w, h = rect
    out = os.path.join(tmp, "crops")
    os.makedirs(out, exist_ok=True)
    subprocess.run(["sips", "-s", "format", "bmp", "-c", str(h), str(w), "--cropOffset", str(y), str(x)] + paths + ["--out", out], capture_output=True, check=True)
    imgs = []
    for p in paths:
        data = open(os.path.join(out, os.path.basename(p)[:-4] + ".bmp"), "rb").read()
        off, = struct.unpack_from("<I", data, 10)
        bw, bh, _, bpp = struct.unpack_from("<iiHH", data, 18)
        step = bpp // 8
        stride = (abs(bw) * step + 3) & ~3
        imgs.append((data, off, abs(bw), abs(bh), step, stride, bh < 0))
    return imgs


def px_offsets(img, coords):
    _, off, bw, bh, step, stride, top_down = img
    res = []
    for (cx, cy) in coords:
        row = cy if top_down else bh - 1 - cy
        res.append(off + row * stride + cx * step)
    return res


def grad_at(img, offs):
    data, step = img[0], img[4]
    s = 0
    for o in offs:
        s += abs(data[o] - data[o + step]) + abs(data[o + 1] - data[o + 1 + step]) + abs(data[o + 2] - data[o + 2 + step])
    return s / max(1, len(offs))


def measure(binary, o, scale, keep):
    tmp = tempfile.mkdtemp(prefix="glass-check-")
    W, H = (int(v) for v in o["size"].split("x"))
    try:
        try:
            steps = resolve(binary, o, tmp, scale)
            frames = run(binary, o, tmp, steps, True, scale)
        except RuntimeError as err:
            return {"error": str(err)}
        if keep:
            dst = os.path.join(keep, o["name"])
            shutil.rmtree(dst, ignore_errors=True)
            shutil.copytree(os.path.join(tmp, "frames"), dst)
            shutil.copy(os.path.join(tmp, "t.jsonl"), dst)
        union = o["glass"].startswith("*")
        pat = o["glass"].lstrip("*")
        # Trace ids are stable per element path: a remount shows as the id coming back in "e"
        # after it went in "g", not as a new "m" entry. Only elements with a role or key are
        # traced, so a glass surface without either is measured as the union of its rows ("*").
        names, rects, mounts, gone = {}, {}, {}, set()
        for i, f in enumerate(frames):
            gone.update(f.get("g", []))
            for m in f.get("m", []):
                names[m["id"]] = m["nm"]
                gone.discard(m["id"])
                mounts[m["id"]] = i
            for e in f.get("e", []):
                rects.setdefault(e[0], {})[i] = e
                if e[0] in gone:
                    gone.discard(e[0])
                    mounts[e[0]] = i
        hits = {c: i for c, i in mounts.items() if re.fullmatch(pat, names.get(c, "").rsplit("/", 1)[-1])}
        if not hits:
            return {"error": f"glass element {o['glass']!r} never mounted"}
        first = max(hits.values())  # the latest mount: the one the last step opened
        ids = [c for c, i in hits.items() if i == first] if union else [max((c for c, i in hits.items() if i == first))]
        boxes = []
        for c in ids:
            r = {i: e for i, e in rects.get(c, {}).items() if i >= first}
            if r:
                boxes.append(r[max(r)])
        if not boxes:
            return {"error": "glass element has no geometry"}
        x0, y0 = min(e[1] for e in boxes), min(e[2] for e in boxes)
        x1, y1 = max(e[1] + e[3] for e in boxes), max(e[2] + e[4] for e in boxes)
        rects = {first: [0, x0, y0, x1 - x0, y1 - y0]}
        last = len(frames) - 1
        rest = next((i for i in range(first + 1, len(frames)) if not frames[i]["quality"]["animating"]), None)
        e = rects[max(rects)]
        ins = min(14.0, e[3] / 6, e[4] / 6) * scale
        x, y, w, h = (int(e[1] * scale + ins), int(e[2] * scale + ins), int(e[3] * scale - 2 * ins), int(e[4] * scale - 2 * ins))
        x, y = max(0, x), max(0, y)
        w, h = min(w, W * scale - x - 1), min(h, H * scale - y)
        if w < 8 or h < 8:
            return {"error": "glass element too small to measure"}
        end = min(last, (rest or last) + 3, first + 120)
        idx = list(range(max(0, first - 1), end + 1))
        if last not in idx:
            idx.append(last)
        paths = [os.path.join(tmp, "frames", "frame_%04d.png" % i) for i in idx]
        imgs = crop_frames(paths, (x, y, w, h), tmp)
        pre, fin = imgs[0], imgs[-1]
        # Pixels the settled glass shows smooth over a page that has detail there.
        cand = [(cx, cy) for cy in range(0, h, 2) for cx in range(0, w - 1, 2)]
        po, fo = px_offsets(pre, cand), px_offsets(fin, cand)
        mask = []
        for c, a, b in zip(cand, po, fo):
            gp = sum(abs(pre[0][a + k] - pre[0][a + k + pre[4]]) for k in range(3))
            gf = sum(abs(fin[0][b + k] - fin[0][b + k + fin[4]]) for k in range(3))
            if gp >= 9 and gf <= 3:
                mask.append(c)
        if len(mask) > 6000:
            mask = mask[:: len(mask) // 6000 + 1]
        res = {"first": first, "rest": rest, "mask": len(mask), "t0": frames[first]["t"]}
        if len(mask) < 40:
            res["note"] = "no page detail behind the glass to blur (not measurable)"
            return res
        g = [grad_at(im, px_offsets(im, mask)) for im in imgs]
        gpre, gset = g[0], g[-1]
        span = max(1e-6, gpre - gset)
        series = list(zip(idx, g))[1:-1] if idx[-1] == last else list(zip(idx, g))[1:]
        frost = next((i for i, v in series if i >= first and v <= gset + 0.1 * span), None)
        mid_i = first + 12
        mid = next(((v - gset) / span for i, v in series if i == mid_i), None)
        res.update({"pre": gpre, "settled": gset, "frost": frost, "mid": mid})
        return res
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bin", default=os.path.join(os.environ.get("CARGO_TARGET_DIR", "target"), "release", "kassiber-desktop"))
    ap.add_argument("--scale", type=int, default=1, help="render scale (1 is quicker; 2 is the app's)")
    ap.add_argument("--keep", help="copy each overlay's frames and trace here")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("names", nargs="*", help="overlay names or prefixes (default: all)")
    a = ap.parse_args()
    if a.list:
        for o in OVERLAYS:
            print(f"{o['name']:30} {o['glass']:28} {' '.join(o['args'])} {o['steps']}  {o['note']}")
        return
    sel = [o for o in OVERLAYS if not a.names or any(o["name"] == n or o["name"].startswith(n) for n in a.names)]
    print(f"{'overlay':34} {'mount':>5} {'frost':>9} {'rest':>9} {'mid':>6} {'pre':>6} {'settled':>7}  verdict")
    bad = 0
    for o in sel:
        r = measure(a.bin, o, a.scale, a.keep)
        if "error" in r:
            missing = "no element with key" in r["error"] or "no key matches" in r["error"]
            if o["needs"] and missing:
                print(f"{o['name']:34} n/a   needs {o['needs']} in the book")
                continue
            print(f"{o['name']:34} ERROR {r['error']}")
            bad += 1
            continue
        if "frost" not in r:
            print(f"{o['name']:34} {r['first']:>5} {'-':>9} {('@' + str(r['rest'] - r['first'])) if r['rest'] else '-':>9}   {r['note']}")
            continue
        fr = (r["frost"] - r["first"]) if r["frost"] is not None else None
        rs = (r["rest"] - r["first"]) if r["rest"] is not None else None
        ok = fr is not None and fr <= 24 and (r["mid"] is None or r["mid"] < 0.15)
        bad += 0 if ok else 1
        mid = f"{r['mid']:.2f}" if r["mid"] is not None else "-"
        frs = f"+{fr} ({fr * 1000 // 60}ms)" if fr is not None else "never"
        print(f"{o['name']:34} {r['first']:>5} {frs:>9} {('+' + str(rs)) if rs is not None else '-':>9} {mid:>6} {r['pre']:6.1f} {r['settled']:7.1f}  {'ok' if ok else 'BLUR LATE'}")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
