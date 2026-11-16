#!/usr/bin/env python3 -I
"""Click-through audit: press every actionable control in every scene, headless, and classify
what happened (navigates / opens overlay / sends a daemon request / changes something /
NO EFFECT / error toast / crash / egress-skipped / disabled).

Developer tooling only (stdlib Python, run with `python3 -I`). It never touches the owner's books:
each worker gets its own copy of the pristine demo book and its own preference files (offline
mode forced ON and checked before every run), a sandbox HOME, and the book and prefs are
restored from that worker's golden copy before every single run. Controls whose purpose is
network egress are never pressed (see EGRESS below); they are reported as "egress-skipped".

    source ~/rune-work/audit/env.sh
    python3 -I $CRATE/tools/clickthrough.py setup  --workers 6      # worker homes under $CT_ROOT
    python3 -I $CRATE/tools/clickthrough.py plan   [--scenes a,b]   # baseline trees -> plan.json
    python3 -I $CRATE/tools/clickthrough.py press  [--only-area A]  # presses -> results.jsonl (resumable)
    python3 -I $CRATE/tools/clickthrough.py hover                   # re-checks NO EFFECT controls for tooltips
    python3 -I $CRATE/tools/clickthrough.py snaps                   # PNGs (before/after) of flagged controls
    python3 -I $CRATE/tools/clickthrough.py report                  # summary.md + summary.json

Daemon requests are observed through a KASSIBER_BIN wrapper that records only the request
`kind` values (`ui.*`), never arguments, into a per-run file in the worker's scratch dir.
"""
from pathlib import Path
import argparse
import collections
import concurrent.futures as cf
import json
import os
import re
import shutil
import subprocess
import sys
import threading
import time

CRATE = os.environ.get("CRATE") or os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TARGET = os.environ.get("CARGO_TARGET_DIR", os.path.join(CRATE, "target"))
APP = os.path.join(TARGET, "release", "kassiber-desktop")
CT_ROOT = os.environ.get("CT_ROOT", os.path.expanduser("~/rune-work/audit/ct"))
PRISTINE = os.environ.get("CT_PRISTINE", os.path.expanduser("~/rune-parity/pristine"))
REAL_BIN = os.environ.get("KASSIBER_BIN", "")
OFFLINE = {"schema_version": 1, "enabled": True}
SIZE = "1440x900"

# Default scene per route, pressed first so shared shell controls dedupe onto them.
ROUTES = ["/overview", "/transactions", "/connections", "/imports", "/transfers", "/review",
          "/reconcile", "/reports", "/journals", "/exit-tax", "/activity", "/logs", "/egress",
          "/books", "/assistant", "/source-of-funds", "/privacy-mirror", "/chain-analysis"]
# Visual-only variants: same controls as their base scene.
VARIANT = re.compile(r"(^|-)(light|de|hidden|fiat|dark)(-|$)|^(typo|hide-amounts|locked-de)$")

# Controls whose purpose is network egress (or a write outside the disposable book): never pressed.
EGRESS = [
    (r"\bsync\b|synchron|sync-now|sh-sync-close", "sync"),
    (r"refresh(?!.*journal)|rescan|aktualisier", "refresh / rescan"),
    (r"test.?connection|check.?connection|verbindung (testen|prüfen)|-test$", "connection test"),
    (r"look ?up|lookup|graph-lookup", "on-chain lookup"),
    (r"txd-explorer-open|open explorer", "block explorer"),
    (r"check.?all|ollama-check|check ollama|check.?models|fetch.?models|asst-pick-check|modelle prüfen", "provider check"),
    (r"update.?check|check for updates|set-updates", "update check"),
    (r"set-market-live|live minute-rate", "enables live rate lookups"),
    (r"set-kraken-|kraken_csv", "Kraken rate history import"),
    (r"detect my node|add-core-detect", "local node probe"),
    (r"btcpay-discover|btcpay-authorize|exchange-key-page|check key and stores", "provider API / link"),
    (r"https?://|github|documentation|\bdocs\b|learn more|release notes|website|sh-version|sh-support-", "external link"),
    (r"(asst|chat|composer)-send|send message", "AI send"),
    (r"-speak$|read aloud|vorlesen", "speaks out loud (system voice)"),
    (r"fetch certificate|zertifikat abrufen|cert-dialog-fetch$", "fetches a server certificate"),
    (r"asst-setup-use-|^use (claude|codex|github copilot|opencode)", "AI provider selection (may start a provider check)"),
    # Prompt sends without a Send button: visual follow-up/readiness chips, starter and panel prompts,
    # launcher prompts, and every "Ask the assistant" entry point (assistant::ask_about).
    (r"asst-visual-.*-(follow|check)-|asst-suggest-|asst-panel-prompt-|asst-launch-|asst-recent-ask|"
     r"-ask(?=\s|$)|ask (the )?assistant|ask about|assistent(en)? fragen|frag(e|en)? (den|die) assistent", "AI send"),
    (r"set-ai-ns-(setup|del)-|set up (opencode|copilot|codex|claude)", "AI CLI provider setup"),
    (r"terminal.*install|install.*command|set-term-install", "writes ~/.local/bin"),
    (r"move-native|keychain|touch.?id|set-touch-", "OS keychain"),
    # Local, no network: `kassiber projects demo` writes a fresh ~60 MB demo copy under the state
    # root (KASSIBER_DEMO_ROOT redirects it). Scenes demo-live/-menu/-left exercise it instead.
    (r"onb-demo|sh-books-demo|reset demo|demo zurücksetzen|try the demo|demo(-buch)? ausprobieren", "writes a demo book copy"),
]
# Keys that match a pattern above but are local (menu triggers, cancels, form toggles).
SAFE = re.compile(r"sh-refresh-caret|-cancel$|-close$|dialog-close|add-sync-after|wd-edit-backend|^add-source-|^imports-action-|"
                  r"^[0-9a-f]{8}-[0-9a-f]{4}-|set-term-refresh|tp-menu-recheck|tp-check-again|set-ai-ns-del-copilot-x")
EGRESS_RE = [(re.compile(p, re.I), why) for p, why in EGRESS]
# Request kinds that would contact the network (flagged if any press sends one).
NET_KIND = re.compile(r"sync|refresh|rescan|test|probe|lookup|fetch|rates\.|update\.check|explorer|models|connect|import_api|chat|send", re.I)
OVERLAY_ROLES = {"dialog", "alert-dialog", "alertdialog", "menu", "list-box", "listbox", "tooltip", "popover", "menu-bar", "sheet"}
IGNORED_STATES = {"focused", "focus-visible", "hovered", "pressed", "live-polite", "live-assertive"}
ERRORISH = re.compile(r"could not|couldn't|failed|error|invalid|refused|not allowed|offline|unable|denied|fehl|nicht|konnte", re.I)

lock = threading.Lock()


def die(msg):
    print(msg, file=sys.stderr)
    sys.exit(1)


def worker_dirs():
    return sorted(d for d in (os.path.join(CT_ROOT, n) for n in os.listdir(CT_ROOT)) if os.path.basename(d).startswith("w") and os.path.isdir(d)) if os.path.isdir(CT_ROOT) else []


def write_offline(prefs):
    os.makedirs(prefs, exist_ok=True)
    with open(os.path.join(prefs, "offline-mode.json"), "w") as f:
        json.dump(OFFLINE, f, separators=(",", ":"))


def check_offline(prefs):
    with open(os.path.join(prefs, "offline-mode.json")) as f:
        if json.load(f) != OFFLINE:
            die(f"offline mode is not ON in {prefs}; refusing to run")


def cmd_setup(a):
    if not REAL_BIN or not os.access(REAL_BIN, os.X_OK):
        die("KASSIBER_BIN is not set to an executable (source env.sh)")
    os.makedirs(CT_ROOT, exist_ok=True)
    for i in range(a.workers):
        w = os.path.join(CT_ROOT, f"w{i}")
        subprocess.run([os.path.join(CRATE, "parity", "setup-home.sh"), w, PRISTINE, REAL_BIN], check=True, stdout=subprocess.DEVNULL)
        for side in ("rune", "tauri"):
            write_offline(os.path.join(w, f"prefs-{side}"))
        for g in ("golden-book", "golden-prefs"):
            shutil.rmtree(os.path.join(w, g), ignore_errors=True)
        shutil.copytree(os.path.join(w, "book-rune"), os.path.join(w, "golden-book"), symlinks=True)
        shutil.copytree(os.path.join(w, "prefs-rune"), os.path.join(w, "golden-prefs"))
        shutil.rmtree(os.path.join(w, "book-tauri"), ignore_errors=True)
        for d in ("home", "cwd", "runs"):
            os.makedirs(os.path.join(w, d), exist_ok=True)
        wrap = os.path.join(w, "kassiber-wrap")
        with open(wrap, "w") as f:
            # Records only request kinds (dotted names), never arguments.
            f.write("#!/bin/bash\nexec \"$CT_REAL_BIN\" \"$@\" < <(tee >(grep --line-buffered -oE '\"kind\": ?\"[a-z_]+(\\.[a-z_]+)+\"' | sed -E 's/.*\"([^\"]+)\"$/\\1/' >> \"$CT_REQ_LOG\"))\n")
        os.chmod(wrap, 0o755)
    print(f"{a.workers} workers ready under {CT_ROOT}")


def restore(w):
    for name, golden in (("book-rune", "golden-book"), ("prefs-rune", "golden-prefs")):
        dst = os.path.join(w, name)
        shutil.rmtree(dst, ignore_errors=True)
        shutil.copytree(os.path.join(w, golden), dst, symlinks=True)
    write_offline(os.path.join(w, "prefs-rune"))
    check_offline(os.path.join(w, "prefs-rune"))


def run_app(w, scene, events, at, tag, snap=None, keep_tree=None):
    """One headless run on worker `w`. Returns (rc, stderr_tail, tree, request_kinds)."""
    restore(w)
    prefs = os.path.join(w, "prefs-rune")
    rdir = os.path.join(w, "runs")
    script = os.path.join(rdir, f"{tag}.script.json")
    tree = keep_tree or os.path.join(rdir, f"{tag}.tree.json")
    reqlog = os.path.join(rdir, f"{tag}.req")
    for p in (tree, reqlog):
        if os.path.exists(p):
            os.remove(p)
    Path(reqlog).write_text("")
    with open(script, "w") as f:
        json.dump({"events": events}, f)
    env = dict(os.environ)
    env.update({
        "HOME": os.path.join(w, "home"),
        "KASSIBER_BIN": os.path.join(w, "kassiber-wrap"),
        "CT_REAL_BIN": REAL_BIN,
        "CT_REQ_LOG": reqlog,
        "KASSIBER_DEMO_DATA_ROOT": os.path.join(w, "book-rune", "data"),
        "KASSIBER_OFFLINE_PREFERENCE_FILE": os.path.join(prefs, "offline-mode.json"),
        "KASSIBER_AGENT_ACCESS_FILE": os.path.join(prefs, "agent-access.json"),
        "KASSIBER_UPDATE_PREFERENCE_FILE": os.path.join(prefs, "update.json"),
        "KASSIBER_UPDATE_CACHE_FILE": os.path.join(prefs, "update-cache.json"),
        "KASSIBER_DISABLE_UPDATE_CHECK": "1",
    })
    name, extra = scene_args(scene)
    cmd = [APP, "--size", getattr(_size, "v", None) or SIZE, "--scene", name, *extra, "--script", script, "--realtime", "--tree", tree, "--at", str(at)]
    if snap:
        cmd += ["--snap", snap]
    try:
        p = subprocess.run(cmd, cwd=os.path.join(w, "cwd"), env=env, capture_output=True, text=True, timeout=60)
        rc, err = p.returncode, p.stderr
    except subprocess.TimeoutExpired:
        rc, err = -999, "timeout"
    time.sleep(0.15)  # let the tee drain
    t = None
    if os.path.exists(tree):
        with open(tree) as f:
            t = json.load(f)
    with open(reqlog) as f:
        kinds = [l.strip() for l in f if l.strip()]
    # Per-run scratch goes (the disk is small); only kept baselines stay.
    for path in (script, reqlog) + (() if keep_tree else (tree,)):
        if os.path.exists(path):
            os.remove(path)
    tail = "\n".join(l for l in err.splitlines() if not l.startswith("rune:"))[-1500:]
    return rc, tail, t, kinds


def scene_args(scene):
    if scene.startswith("route:"):
        return "route", ["--route", scene[6:]]
    return scene, []


# ---- tree helpers -------------------------------------------------------------------------

def walk(node, parents=()):
    yield node, parents
    for c in node.get("children", []) or []:
        yield from walk(c, parents + (node,))


def sig(n):
    st = sorted(s for s in n.get("states", []) or [] if s not in IGNORED_STATES)
    return f"{n.get('role','')}|{n.get('key','')}|{n.get('name','')}|{','.join(st)}|{n.get('value','')}"


def page_title(tree):
    for n, par in walk(tree["accessibility"]):
        if n.get("role") == "heading" and any(p.get("key") == "sh-crumbs" for p in par):
            return n.get("name")
    return None


def toasts(tree):
    out = []
    for n, par in walk(tree["accessibility"]):
        if n.get("role") in ("status", "alert") and any(p.get("key") == "sh-toasts" for p in par):
            out.append(n.get("name", ""))
    return out


def overlays(tree):
    out = set()
    for n, _ in walk(tree["accessibility"]):
        if n.get("role") in OVERLAY_ROLES:
            out.add(f"{n.get('role')}:{n.get('key') or n.get('name') or ''}")
    return out


def expanded(tree):
    return {n.get("key") or n.get("name") for n, _ in walk(tree["accessibility"]) if "expanded" in (n.get("states") or [])}


def norm_key(k):
    k = re.sub(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", "{uuid}", k)
    k = re.sub(r"[0-9a-f]{16,}", "{hex}", k)
    k = re.sub(r"\d+", "#", k)
    return k


def actionables(tree):
    vw, vh = tree["viewport"]
    out = []
    for n, par in walk(tree["accessibility"]):
        acts = n.get("actions") or []
        if "click" not in acts:
            continue
        b = n.get("bounds") or [0, 0, 0, 0]
        # The nearest clipping ancestor (a scroll view, or any element that scrolls or clips)
        # decides whether the control's centre is on screen.
        clip = [0, 0, vw, vh]
        scroller = None
        for p in reversed(par):
            if p.get("scroll") is not None or p.get("role") == "scroll-view" or "clips" in (p.get("states") or []):
                clip = p.get("bounds") or clip
                scroller = p
                break
        cx, cy = b[0] + b[2] / 2, b[1] + b[3] / 2
        visible = clip[0] <= cx <= clip[0] + clip[2] and clip[1] <= cy <= clip[1] + clip[3] and 0 <= cx <= vw and 0 <= cy <= vh
        wheel = None
        if not visible and scroller is not None and b[2] > 0:
            sb = list(scroller.get("bounds"))
            # Only the part of the scroller inside the window can take the wheel.
            x0, y0 = max(sb[0], 0), max(sb[1], 0)
            x1, y1 = min(sb[0] + sb[2], vw), min(sb[1] + sb[3], vh)
            sb = [x0, y0, max(1, x1 - x0), max(1, y1 - y0)]
            mx, my = sb[0] + sb[2] / 2, sb[1] + sb[3] / 2
            dx = 0 if sb[0] <= cx <= sb[0] + sb[2] else cx - mx
            dy = 0 if sb[1] <= cy <= sb[1] + sb[3] else cy - my
            wheel = {"x": max(1, min(vw - 1, mx)), "y": max(1, min(vh - 1, my)), "dx": -dx, "dy": -dy}
        out.append({
            "key": n.get("key"), "role": n.get("role"), "name": n.get("name", ""),
            "description": n.get("description", ""), "states": n.get("states") or [],
            "bounds": b, "visible": visible, "wheel": wheel,
            "in_overlay": any(p.get("role") in OVERLAY_ROLES for p in par),
        })
    return out


def area_of(scene):
    s = scene[6:] if scene.startswith("route:") else scene
    s = s.lstrip("/")
    table = [
        ("settings", "settings"), ("wallets", "wallets"), ("connections", "wallets"), ("imports", "wallets"),
        ("review", "review"), ("links", "review"), ("reconcile", "review"),
        ("reports", "reports"), ("journals", "reports"), ("exit-tax", "reports"), ("activity", "reports"),
        ("logs", "reports"), ("egress", "reports"), ("books", "reports"),
        ("assistant", "assistant"), ("custody", "custody"), ("transfers", "custody"),
        ("source-of-funds", "provenance"), ("privacy-mirror", "provenance"), ("chain-analysis", "provenance"),
        ("detail", "txdetail (in progress)"), ("graph", "txdetail (in progress)"), ("open-tx", "txdetail (in progress)"),
        ("tx-detail", "txdetail (in progress)"), ("chart", "chart (in progress)"),
        ("transactions", "transactions"), ("overview", "overview"), ("add-connection", "overview"),
    ]
    for p, a in table:
        if s.startswith(p):
            return a
    return "shell"


def control_area(scene, key):
    k = key or ""
    if k.startswith("sh-"):
        return "shell"
    if re.match(r"(legend-|dot-|range-|year-|logx-|logy-|fit-|gear-|brush-|chart-)", k):
        return "chart (in progress)"
    if re.match(r"(td-|detail-|graph-|txd-)", k):
        return "txdetail (in progress)"
    return area_of(scene)


def egress_reason(c):
    if c.get("key") and SAFE.search(c["key"]):
        return None
    text = " ".join(str(x) for x in (c.get("key"), c.get("name"), c.get("description")) if x)
    for rx, why in EGRESS_RE:
        if rx.search(text):
            return why
    return None


# ---- commands -----------------------------------------------------------------------------

def list_scenes():
    p = subprocess.run([APP, "--list-scenes"], capture_output=True, text=True)
    return [s.strip() for s in p.stdout.splitlines() if s.strip()]


def pool(fn, items, workers):
    ws = worker_dirs()
    if not ws:
        die("no workers; run setup first")
    ws = ws[:workers] if workers else ws
    free = list(ws)
    cv = threading.Condition()

    def job(item):
        with cv:
            while not free:
                cv.wait()
            w = free.pop()
        try:
            return fn(w, item)
        finally:
            with cv:
                free.append(w)
                cv.notify()

    with cf.ThreadPoolExecutor(len(ws)) as ex:
        yield from ex.map(job, items)


def baseline_events():
    return [{"t": 0.3, "type": "move", "x": -40, "y": -40}, {"t": 0.9, "type": "mark", "name": "end"}]


AT = 2.2


def cmd_plan(a):
    scenes = a.scenes.split(",") if a.scenes else ["route:" + r for r in ROUTES] + [s for s in list_scenes() if s not in ("route", "open-tx") and not VARIANT.search(s)]
    out = os.path.join(CT_ROOT, "plan.json")
    old = {}
    if a.reuse and os.path.exists(out):
        with open(out) as f:
            old = json.load(f)["scenes"]
    base_dir = os.path.join(CT_ROOT, "base")
    os.makedirs(base_dir, exist_ok=True)

    def one(w, scene):
        tag = "base-" + re.sub(r"[^a-z0-9]+", "_", scene.lower())
        res = []
        for i in range(2):  # twice, to find signatures that change on their own
            keep = os.path.join(base_dir, f"{tag}.{i}.json")
            if a.reuse and os.path.exists(keep) and old.get(scene):
                with open(keep) as f:
                    res.append((0, "", json.load(f), old[scene]["kinds" if i == 0 else "kinds2"]))
                continue
            rc, err, t, kinds = run_app(w, scene, baseline_events(), AT, tag, keep_tree=keep)
            res.append((rc, err, t, kinds))
        with lock:
            print(f"  base {scene}: rc={res[0][0]} {len(actionables(res[0][2])) if res[0][2] else '-'} controls", flush=True)
        return scene, res

    plan = {"scenes": {}, "controls": []}
    seen = {}
    for scene, res in pool(one, scenes, a.workers):
        (rc, err, t, kinds), (rc2, _, t2, kinds2) = res
        if not t:
            plan["scenes"][scene] = {"error": f"rc={rc} {err[-300:]}"}
            continue
        s1 = collections.Counter(sig(n) for n, _ in walk(t["accessibility"]))
        s2 = collections.Counter(sig(n) for n, _ in walk(t2["accessibility"])) if t2 else s1
        unstable = sorted(set((s1 - s2) + (s2 - s1)))
        plan["scenes"][scene] = {"title": page_title(t), "kinds": kinds, "kinds2": kinds2, "unstable": unstable,
                                 "tree": os.path.join(base_dir, "base-" + re.sub(r"[^a-z0-9]+", "_", scene.lower()) + ".0.json")}
        for c in actionables(t):
            nk = norm_key(c["key"]) if c["key"] else f"~{c['role']}|{c['name']}|{scene}"
            if nk in seen:
                seen[nk]["instances"] += 1
                if scene not in seen[nk]["scenes"]:
                    seen[nk]["scenes"].append(scene)
                continue
            c.update({"scene": scene, "norm": nk, "instances": 1, "scenes": [scene], "area": control_area(scene, c["key"])})
            seen[nk] = c
            plan["controls"].append(c)
    with open(out, "w") as f:
        json.dump(plan, f, indent=1, ensure_ascii=False)
    print(f"plan: {len(plan['scenes'])} scenes, {len(plan['controls'])} unique controls -> {out}")


def wheel_events(ev, w, t):
    """Scroll a clipped control into view: wheel steps of at most 400 pt over its scroller."""
    dx, dy = w.get("dx", 0.0), w.get("dy", 0.0)
    n = max(1, int(max(abs(dx), abs(dy)) // 400) + 1)
    for i in range(n):
        ev.append({"t": round(t + 0.02 * i, 3), "type": "wheel", "x": w["x"], "y": w["y"], "dx": dx / n, "dy": dy / n})
    return t + 0.02 * n + 0.3


def press_events(c):
    ev = [{"t": 0.3, "type": "move", "x": -40, "y": -40}]
    t = 0.45
    if c.get("wheel"):
        t = wheel_events(ev, c["wheel"], t)
    if c["key"]:
        ev.append({"t": t, "type": "tap", "key": c["key"]})
    else:
        b = c["bounds"]
        ev.append({"t": t, "type": "tap", "x": b[0] + b[2] / 2, "y": b[1] + b[3] / 2})
    ev.append({"t": t + 0.2, "type": "move", "x": -40, "y": -40})
    ev.append({"t": 0.9, "type": "mark", "name": "end"})
    return ev


def classify(c, base, t, kinds, rc, err, plan_scene):
    flags = {}
    if "no element with key" in err:
        return "key not found", {"rc": rc, "stderr": err[-300:]}
    if rc != 0 or "panicked" in err:
        return "crash", {"rc": rc, "stderr": err[-600:]}
    if t is None:
        return "crash", {"rc": rc, "stderr": "no tree written " + err[-400:]}
    unstable = set(plan_scene.get("unstable", []))
    b = collections.Counter(sig(n) for n, _ in walk(base["accessibility"]))
    a = collections.Counter(sig(n) for n, _ in walk(t["accessibility"]))
    added = sorted(s for s in (a - b) if s not in unstable)
    removed = sorted(s for s in (b - a) if s not in unstable)
    bt, at = page_title(base), page_title(t)
    bo, ao = overlays(base), overlays(t)
    be, ae = expanded(base), expanded(t)
    new_toasts = [x for x in toasts(t) if x not in toasts(base)]
    bk = collections.Counter(plan_scene.get("kinds", [])) | collections.Counter(plan_scene.get("kinds2", []))
    extra = sorted((collections.Counter(kinds) - bk).elements())
    flags.update({"added": added[:12], "removed": removed[:12], "n_added": len(added), "n_removed": len(removed),
                  "requests": extra, "toasts": new_toasts})
    net = [k for k in extra if NET_KIND.search(k)]
    if net:
        flags["net_kinds"] = net
    if new_toasts and any(ERRORISH.search(x) for x in new_toasts):
        return "error toast", flags
    if (ao - bo) or (ae - be):
        flags["overlay"] = sorted((ao - bo) | {f"expanded:{x}" for x in (ae - be)})
        return "opens overlay", flags
    if bt and at and bt != at:
        flags["route"] = f"{bt} -> {at}"
        return "navigates", flags
    if new_toasts:
        return "toast", flags
    if (bo - ao) or (be - ae):
        flags["closed"] = sorted((bo - ao) | {f"collapsed:{x}" for x in (be - ae)})
        return "closes overlay", flags
    if added or removed:
        return "changes something", flags
    if extra:
        return "daemon request", flags
    return "NO EFFECT", flags


def load_results(path):
    done = {}
    if os.path.exists(path):
        with open(path) as f:
            for line in f:
                r = json.loads(line)
                done[r["norm"]] = r
    return done


def cmd_press(a):
    with open(os.path.join(CT_ROOT, "plan.json")) as f:
        plan = json.load(f)
    path = os.path.join(CT_ROOT, "results.jsonl")
    done = load_results(path)
    todo = []
    with open(path, "a") as out:
        for c in plan["controls"]:
            if c["norm"] in done or (a.only_area and a.only_area not in c["area"]):
                continue
            r = dict(c)
            why = egress_reason(c)
            if "disabled" in c["states"]:
                r["class"], r["flags"] = "disabled", {}
            elif why:
                r["class"], r["flags"] = "egress-skipped", {"why": why}
            elif not c["visible"] and not c.get("wheel"):
                r["class"], r["flags"] = "off-screen (not pressed)", {}
            else:
                todo.append(c)
                continue
            out.write(json.dumps(r, ensure_ascii=False) + "\n")
    print(f"pressing {len(todo)} controls ({len(done)} done)", flush=True)
    base_cache = {}

    def one(w, c):
        tag = "press-" + str(abs(hash(c["norm"])))
        rc, err, t, kinds = run_app(w, c["scene"], press_events(c), AT, tag)
        sc = plan["scenes"][c["scene"]]
        if c["scene"] not in base_cache:
            with open(sc["tree"]) as f:
                base_cache[c["scene"]] = json.load(f)
        cls, flags = classify(c, base_cache[c["scene"]], t, kinds, rc, err, sc)
        r = dict(c)
        r["class"], r["flags"] = cls, flags
        return r

    n = 0
    with open(path, "a") as out:
        for r in pool(one, todo, a.workers):
            n += 1
            out.write(json.dumps(r, ensure_ascii=False) + "\n")
            out.flush()
            print(f"[{n}/{len(todo)}] {r['class']:18} {r['scene']:28} {r['key'] or r['name']}", flush=True)


def cmd_hover(a):
    """NO EFFECT controls: hover without pressing; a new tooltip makes them info-only."""
    path = os.path.join(CT_ROOT, "results.jsonl")
    res = load_results(path)
    with open(os.path.join(CT_ROOT, "plan.json")) as f:
        plan = json.load(f)
    todo = [r for r in res.values() if r["class"] == "NO EFFECT" and "hover" not in r["flags"]]

    def one(w, r):
        ev = [{"t": 0.3, "type": "move", "x": -40, "y": -40}]
        if r.get("wheel"):
            wheel_events(ev, r["wheel"], 0.45)
        b = r["bounds"]
        ev.append({"t": 0.7, "type": "move", **({"key": r["key"]} if r["key"] else {"x": b[0] + b[2] / 2, "y": b[1] + b[3] / 2})})
        ev.append({"t": 0.9, "type": "mark", "name": "end"})
        rc, err, t, kinds = run_app(w, r["scene"], ev, AT, "hover-" + str(abs(hash(r["norm"]))))
        with open(plan["scenes"][r["scene"]]["tree"]) as f:
            base = json.load(f)
        unstable = set(plan["scenes"][r["scene"]].get("unstable", []))
        bsig = collections.Counter(sig(n) for n, _ in walk(base["accessibility"]))
        asig = collections.Counter(sig(n) for n, _ in walk(t["accessibility"])) if t else bsig
        added = [s for s in (asig - bsig) if s not in unstable]
        r["flags"]["hover"] = added[:6]
        return r

    for r in pool(one, todo, a.workers):
        res[r["norm"]] = r
        print(f"hover {r['scene']} {r['key']}: {r['flags']['hover'][:2]}", flush=True)
    with open(path, "w") as f:
        for r in res.values():
            f.write(json.dumps(r, ensure_ascii=False) + "\n")


def cmd_snaps(a):
    res = load_results(os.path.join(CT_ROOT, "results.jsonl"))
    flagged = [r for r in res.values() if r["class"] in ("NO EFFECT", "error toast", "crash") and not r["flags"].get("hover")]
    img = os.path.join(CT_ROOT, "img")
    os.makedirs(img, exist_ok=True)

    def one(w, r):
        name = re.sub(r"[^A-Za-z0-9._-]+", "_", f"{r['scene']}__{r['key'] or r['name']}")[:120]
        run_app(w, r["scene"], press_events(r), AT, "snap", snap=os.path.join(img, name + ".after.png"))
        return name

    for name in pool(one, flagged, a.workers):
        print("snap", name, flush=True)


def cmd_report(a):
    res = list(load_results(os.path.join(CT_ROOT, "results.jsonl")).values())
    for r in res:
        if r["class"] == "crash" and "no element with key" in r["flags"].get("stderr", ""):
            r["class"] = "key not found"
    with open(os.path.join(CT_ROOT, "plan.json")) as f:
        plan = json.load(f)
    for r in res:
        if r["class"] == "NO EFFECT" and r["flags"].get("hover"):
            r["class"] = "tooltip only"
    by_area = collections.defaultdict(collections.Counter)
    for r in res:
        by_area[r["area"]][r["class"]] += 1
    classes = ["navigates", "opens overlay", "closes overlay", "daemon request", "changes something", "toast",
               "tooltip only", "NO EFFECT", "error toast", "crash", "key not found", "egress-skipped", "disabled", "off-screen (not pressed)"]
    lines = ["# Click-through audit", "", f"{len(plan['scenes'])} scenes, {len(res)} unique controls "
             f"({sum(r['instances'] for r in res)} instances incl. repeats).", "",
             "| area | " + " | ".join(classes) + " | total |", "|" + "---|" * (len(classes) + 2)]
    for area in sorted(by_area):
        c = by_area[area]
        lines.append(f"| {area} | " + " | ".join(str(c.get(k, 0) or "") for k in classes) + f" | {sum(c.values())} |")
    tot = collections.Counter(r["class"] for r in res)
    lines.append("| **all** | " + " | ".join(str(tot.get(k, 0)) for k in classes) + f" | {len(res)} |")
    for cls in ("crash", "key not found", "error toast", "NO EFFECT", "egress-skipped", "off-screen (not pressed)"):
        rows = [r for r in res if r["class"] == cls]
        if not rows:
            continue
        lines += ["", f"## {cls} ({len(rows)})", "", "| area | scene | key | role | label | detail |", "|---|---|---|---|---|---|"]
        for r in sorted(rows, key=lambda r: (r["area"], r["scene"])):
            d = r["flags"].get("why") or "; ".join(r["flags"].get("toasts", [])) or r["flags"].get("stderr", "")[:120]
            lines.append(f"| {r['area']} | {r['scene']} | `{r['key'] or ''}` | {r['role']} | {r['name'][:60]} | {d[:120]} |")
    net = [r for r in res if r["flags"].get("net_kinds")]
    if net:
        lines += ["", "## network-shaped request kinds sent by a press", ""]
        lines += [f"- {r['scene']} `{r['key']}` ({r['name']}): {r['flags']['net_kinds']}" for r in net]
    broken = [s for s, v in plan["scenes"].items() if "error" in v]
    if broken:
        lines += ["", "## scenes that failed to render", ""] + [f"- {s}: {plan['scenes'][s]['error'][:200]}" for s in broken]
    md = os.path.join(CT_ROOT, "summary.md")
    with open(md, "w") as f:
        f.write("\n".join(lines) + "\n")
    with open(os.path.join(CT_ROOT, "summary.json"), "w") as f:
        json.dump({"by_area": by_area, "total": tot}, f, indent=1)
    print("\n".join(lines[:40]))
    print(f"... full report: {md}")


# ---- dropdown / menu battery ----------------------------------------------------------------

ITEM_ROLES = {"list-box-option", "option", "menu-item", "menu-item-radio", "menu-item-check-box", "menuitem", "menuitemradio", "menuitemcheckbox"}
LIST_ROLES = {"menu", "list-box", "listbox"}


def nodes(tree):
    return [n for n, _ in walk(tree["accessibility"])] if tree else []


def find_key(tree, key):
    for n in nodes(tree):
        if n.get("key") == key:
            return n
    return None


def focused(tree):
    for n in nodes(tree):
        if "focused" in (n.get("states") or []):
            return n
    return None


def menu_items(tree, base_keys):
    """Items of menus/listboxes that weren't in the scene's baseline (the popup's rows)."""
    out = []
    for n, par in walk(tree["accessibility"]) if tree else []:
        if n.get("role") in ITEM_ROLES and (n.get("key") or n.get("name")) not in base_keys:
            out.append(n)
    return out


def lists(tree, base_keys):
    out = []
    for n, _ in walk(tree["accessibility"]) if tree else []:
        if n.get("role") in LIST_ROLES | {"dialog"} and (n.get("key") or f"{n.get('role')}:{n.get('name')}") not in base_keys:
            out.append(n)
    return out


def ident(n):
    return (n.get("key") or n.get("name")) if n else None


def tap_ev(c, t):
    if c["key"]:
        return {"t": t, "type": "tap", "key": c["key"]}
    b = c["bounds"]
    return {"t": t, "type": "tap", "x": b[0] + b[2] / 2, "y": b[1] + b[3] / 2}


def seq(c, steps):
    ev = [{"t": 0.3, "type": "move", "x": -40, "y": -40}]
    t = 0.45
    if c.get("wheel"):
        t = wheel_events(ev, c["wheel"], t)
    for s in steps:
        if s == "tap":
            ev.append(tap_ev(c, t))
        elif s.startswith("type:"):
            ev.append({"t": t, "type": "type", "text": s[5:]})
        elif s.startswith("hover:"):
            ev.append({"t": t, "type": "move", "key": s[6:]})
        elif s.startswith("at:"):
            x, y = s[3:].split(",")
            ev.append({"t": t, "type": "tap", "x": float(x), "y": float(y)})
        elif s.startswith("wait:"):
            t += float(s[5:])
            continue
        else:
            ev.append({"t": t, "type": "key", "name": s})
        t += 0.2
    ev.append({"t": max(0.9, t), "type": "mark", "name": "end"})
    return ev, max(AT, t + 1.0)


SEQS = {
    "click": ["tap"],
    "esc": ["tap", "Escape"],
    "enter": ["tap", "Escape", "Enter"],
    "space": ["tap", "Escape", "type: "],
    "down_trigger": ["tap", "Escape", "Down"],
    "down2": ["tap", "Down", "Down"],
    "end": ["tap", "End"],
    "home": ["tap", "End", "Home"],
    "select": ["tap", "Down", "Enter"],
    "tab": ["tap", "Tab"],
}


def cmd_dropdowns(a):
    with open(os.path.join(CT_ROOT, "plan.json")) as f:
        plan = json.load(f)
    trig = [c for c in plan["controls"] if (c["role"] == "combobox" or "collapsed" in c["states"] or "expanded" in c["states"])
            and "disabled" not in c["states"] and (c["visible"] or c.get("wheel")) and not egress_reason(c)]
    if a.only:
        trig = [c for c in trig if re.search(a.only, c["key"] or "")]
    if a.keys:
        want = a.keys.split(",")
        bykey = {}
        for c in plan["controls"]:
            if c["key"] in want and c["key"] not in bykey:
                bykey[c["key"]] = c
        trig = [bykey[k] for k in want if k in bykey]
        missing = [k for k in want if k not in bykey]
        if missing:
            print("not in plan:", missing)
    out_path = os.path.join(CT_ROOT, "dropdowns-quick.jsonl" if a.quick else "dropdowns.jsonl")
    done = {}
    if os.path.exists(out_path) and not a.fresh:
        with open(out_path) as f:
            for l in f:
                r = json.loads(l)
                done[r["norm"]] = r
    trig = [c for c in trig if c["norm"] not in done]
    print(f"{len(trig)} triggers", flush=True)
    base_cache = {}

    def base_keys(scene):
        if scene not in base_cache:
            with open(plan["scenes"][scene]["tree"]) as f:
                t = json.load(f)
            base_cache[scene] = {(n.get("key") or n.get("name")) for n in nodes(t)} | {n.get("key") or f"{n.get('role')}:{n.get('name')}" for n in nodes(t)}
        return base_cache[scene]

    def run_seq(w, c, name, steps, size=None):
        ev, at = seq(c, steps)
        tag = f"dd-{abs(hash(c['norm']))}-{name}"
        rc, err, t, kinds = run_app_sized(w, c["scene"], ev, at, tag, size)
        return t, rc, err

    def one(w, c):
        bk = base_keys(c["scene"])
        r = {k: c[k] for k in ("key", "role", "name", "scene", "norm", "area", "states")}
        t, rc, err = run_seq(w, c, "click", SEQS["click"])
        items = menu_items(t, bk)
        if rc != 0 or not items:
            r["kind"] = "crash" if rc != 0 else "not a list popup"
            r["opened"] = [f"{n.get('role')}:{ident(n)}" for n in lists(t, bk)]
            return r
        r["kind"] = "dropdown"
        lst = lists(t, bk)
        r["list_roles"] = sorted({n.get("role") for n in lst})
        r["item_roles"] = sorted({n.get("role") for n in items})
        r["n_items"] = len(items)
        r["disabled_items"] = [ident(n) for n in items if "disabled" in (n.get("states") or [])]
        r["checked_items"] = [ident(n) for n in items if {"checked", "selected"} & set(n.get("states") or [])]
        r["sub_items"] = [n.get("key") for n in items if "expand" in (n.get("actions") or []) and n.get("key")]
        trg = find_key(t, c["key"]) if c["key"] else None
        r["trigger_role"] = c["role"]
        r["trigger_states_open"] = (trg or {}).get("states")
        f = focused(t)
        r["focus_on_open"] = ident(f)
        r["focus_on_open_is_item"] = bool(f) and f.get("role") in ITEM_ROLES
        r["focus_on_open_is_checked"] = bool(f) and bool({"checked", "selected"} & set(f.get("states") or []))
        vw, vh = t["viewport"]
        ib = [n["bounds"] for n in items if n.get("bounds")]
        r["items_bounds"] = [min(b[1] for b in ib), max(b[1] + b[3] for b in ib)] if ib else None
        r["fits"] = all(b[0] >= -1 and b[1] >= -1 and b[0] + b[2] <= vw + 1 and b[1] + b[3] <= vh + 1 for b in ib)
        order = [ident(n) for n in items]
        res = {}
        for name, steps in SEQS.items():
            if name == "click":
                continue
            tt, rc2, _ = run_seq(w, c, name, steps)
            fi = focused(tt)
            open_now = bool(menu_items(tt, bk))
            trg2 = find_key(tt, c["key"]) if (tt and c["key"]) else None
            res[name] = {"open": open_now, "focus": ident(fi), "focus_role": (fi or {}).get("role"),
                         "trigger_value": (trg2 or {}).get("value"), "trigger_name": (trg2 or {}).get("name"),
                         "trigger_states": (trg2 or {}).get("states"), "rc": rc2}
        trg_value0 = (trg or {}).get("value")
        # Typeahead: the first letter of an item other than the one focused on open.
        target = next((n for n in items if n.get("name") and ident(n) != r["focus_on_open"] and n.get("name")[0].isalpha()
                       and "disabled" not in (n.get("states") or [])), None)
        if target:
            tt, _, _ = run_seq(w, c, "type", ["tap", "type:" + target["name"][0].lower()])
            fi = focused(tt)
            res["type"] = {"letter": target["name"][0], "focus": ident(fi), "focus_name": (fi or {}).get("name"),
                           "ok": bool(fi) and (fi.get("name") or "").lower().startswith(target["name"][0].lower())}
        # Outside press: a point far from the trigger and the list.
        tb = c["bounds"]
        ox = 1400 if tb[0] < 700 else 300
        tt, _, _ = run_seq(w, c, "outside", ["tap", f"at:{ox},880"])
        res["outside"] = {"open": bool(menu_items(tt, bk)), "title": page_title(tt) if tt else None, "base_title": plan["scenes"][c["scene"]].get("title")}
        # Long list: End must keep the focused (last) item inside its scroll view.
        if "end" in res:
            tt, _, _ = run_seq(w, c, "end_vis", ["tap", "End"])
            fi = focused(tt)
            vis = None
            if fi and fi.get("bounds"):
                fb = fi["bounds"]
                # The nearest scroll view around the item.
                for n, par in walk(tt["accessibility"]):
                    if n is fi:
                        sv = next((p for p in reversed(par) if p.get("role") == "scroll-view"), None)
                        clip = sv["bounds"] if sv else [0, 0, vw, vh]
                        vis = fb[1] >= clip[1] - 1 and fb[1] + fb[3] <= clip[1] + clip[3] + 1 and fb[1] + fb[3] <= vh
                        r["list_scrolls"] = bool(sv and sv.get("scroll") and sv["scroll"][2] > 0)
                        break
            res["end_vis"] = {"visible": vis}
        # Short window: the list must stay inside it (flip / max height).
        tt, _, _ = run_seq(w, c, "edge", ["tap"], size="1440x420")
        its = menu_items(tt, bk)
        if tt:
            vw2, vh2 = tt["viewport"]
            ib2 = [n["bounds"] for n in its if n.get("bounds")]
            res["edge"] = {"open": bool(its), "fits": all(b[1] >= -1 and b[1] + b[3] <= vh2 + 1 for b in ib2) if ib2 else None,
                           "items_y": [min(b[1] for b in ib2), max(b[1] + b[3] for b in ib2)] if ib2 else None}
        # Submenus: hover opens, Right opens, Left closes.
        if r["sub_items"]:
            sk = r["sub_items"][0]
            before = len(items)
            tt, _, _ = run_seq(w, c, "sub_hover", ["tap", "hover:" + sk, "wait:0.6"])
            res["sub_hover"] = {"opened": len(menu_items(tt, bk)) > before}
            tt, _, _ = run_seq(w, c, "sub_right", ["tap", "hover:" + sk, "Right"])
            res["sub_right"] = {"opened": len(menu_items(tt, bk)) > before, "focus": ident(focused(tt))}
            tt, _, _ = run_seq(w, c, "sub_left", ["tap", "hover:" + sk, "Right", "Left"])
            res["sub_left"] = {"closed": len(menu_items(tt, bk)) <= before, "focus": ident(focused(tt)), "parent_open": bool(menu_items(tt, bk))}
        r["trigger_value0"] = trg_value0
        r["order"] = order
        r["res"] = res
        return r

    def quick(w, c):
        bk = base_keys(c["scene"])
        r = {k: c[k] for k in ("key", "role", "name", "scene", "norm", "area", "states")}
        t, rc, err = run_seq(w, c, "click", SEQS["click"])
        items = menu_items(t, bk)
        r["kind"] = "dropdown" if items else ("crash" if rc != 0 else "not a list popup")
        r["opened"] = [f"{n.get('role')}:{ident(n)}" for n in lists(t, bk)]
        if items:
            f = focused(t)
            r["list_roles"] = sorted({n.get("role") for n in lists(t, bk)})
            r["item_roles"] = sorted({n.get("role") for n in items})
            r["n_items"] = len(items)
            r["checked_items"] = [ident(n) for n in items if {"checked", "selected"} & set(n.get("states") or [])]
            r["focus_on_open"] = ident(f)
            r["focus_role"] = (f or {}).get("role")
            trg = find_key(t, c["key"]) if c["key"] else None
            r["trigger_states_open"] = (trg or {}).get("states")
            vw, vh = t["viewport"]
            r["fits"] = all(n["bounds"][1] >= -1 and n["bounds"][1] + n["bounds"][3] <= vh + 1 for n in items if n.get("bounds"))
        return r

    if a.quick:
        one = quick

    with open(out_path, "a") as out:
        n = 0
        for r in pool(one, trig, a.workers):
            n += 1
            out.write(json.dumps(r, ensure_ascii=False) + "\n")
            out.flush()
            print(f"[{n}/{len(trig)}] {r['kind']:16} {r['scene']:30} {r['key']}", flush=True)


def run_app_sized(w, scene, events, at, tag, size):
    if not size:
        return run_app(w, scene, events, at, tag)
    # SIZE is read inside run_app; pass it through a thread-local override.
    _size.v = size
    try:
        return run_app(w, scene, events, at, tag)
    finally:
        _size.v = None


_size = threading.local()


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("setup"); s.add_argument("--workers", type=int, default=6)
    s = sub.add_parser("plan"); s.add_argument("--scenes"); s.add_argument("--reuse", action="store_true"); s.add_argument("--workers", type=int, default=0)
    s = sub.add_parser("press"); s.add_argument("--only-area"); s.add_argument("--workers", type=int, default=0)
    s = sub.add_parser("hover"); s.add_argument("--workers", type=int, default=0)
    s = sub.add_parser("snaps"); s.add_argument("--workers", type=int, default=0)
    sub.add_parser("report")
    s = sub.add_parser("dropdowns"); s.add_argument("--only"); s.add_argument("--keys"); s.add_argument("--quick", action="store_true"); s.add_argument("--fresh", action="store_true"); s.add_argument("--workers", type=int, default=0)
    a = ap.parse_args()
    {"setup": cmd_setup, "plan": cmd_plan, "press": cmd_press, "hover": cmd_hover, "snaps": cmd_snaps, "report": cmd_report, "dropdowns": cmd_dropdowns}[a.cmd](a)


if __name__ == "__main__":
    main()
