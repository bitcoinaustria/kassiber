"""Accessibility check over Rune `--tree out.json` dumps (stdlib only; run with `python3 -I`).

Usage: python3 -I tools/a11y-check.py tree1.json [tree2.json ...]

For each dump it lists:
  * unnamed interactive nodes in the AccessKit tree: a node whose role is interactive
    (button, link, menuitem*, checkbox, switch, tab, option, combobox, textbox, slider, radio)
    or that offers a click/focus action, but has no accessible name;
  * focusable elements (the element list) with neither a label nor text of their own or of a
    descendant;
  * element keys that appear more than once among visible elements (keys are one namespace).
Exit status is 1 when anything was found.
"""

import json
import sys

INTERACTIVE = {
    "button", "link", "menuitem", "menuitemcheckbox", "menuitemradio", "checkbox", "switch",
    "tab", "option", "combobox", "textbox", "textinput", "slider", "radio", "searchbox",
}


# rune-kit's scroll area parts (inside Menu and Combobox), keyed per instance by path.
KIT_SCROLL_PARTS = {"scroll-bar", "thumb-slot"}


def walk(node, path, out):
    role = (node.get("role") or "").lower()
    actions = node.get("actions") or []
    name = (node.get("name") or "").strip()
    here = f"{path}/{node.get('key') or role}"
    interactive = role in INTERACTIVE or "click" in actions or "focus" in actions
    if interactive and not name and not node.get("hidden"):
        out.append(f"unnamed {role or '?'} at {here} actions={actions}")
    for child in node.get("children") or []:
        walk(child, here, out)


def check(path):
    data = json.load(open(path))
    problems = []
    walk(data.get("accessibility") or {}, "", problems)
    elements = data.get("elements") or []
    by_path = {e.get("path"): e for e in elements}
    for e in elements:
        if not e.get("focusable") or e.get("inert"):
            continue
        p = e.get("path") or ""
        has = bool((e.get("label") or "").strip() or (e.get("text") or "").strip())
        if not has:
            has = any((d.get("text") or d.get("label")) for k, d in by_path.items() if k and k.startswith(p + "/"))
        if not has:
            problems.append(f"focusable without a name: {p}")
    seen = {}
    for e in elements:
        k = e.get("key")
        if not k or e.get("opacity", 1.0) == 0.0:
            continue
        seen.setdefault(k, []).append(e.get("path"))
    kit_parts = []
    for k, paths in sorted(seen.items()):
        if len(paths) > 1:
            # rune-kit keys the parts of its components with plain words (`label`, `value`,
            # `chevron`, …) and scopes them by path; only app keys (with a dash) must be unique.
            if "-" not in k or k in KIT_SCROLL_PARTS:
                kit_parts.append(k)
            else:
                problems.append(f"duplicate key {k!r} x{len(paths)}: {paths[0]} …")
    if kit_parts:
        print(f"  (rune-kit part keys repeated, path-scoped by design: {', '.join(kit_parts)})")
    return problems


def main():
    bad = 0
    for path in sys.argv[1:]:
        problems = check(path)
        bad += len(problems)
        print(f"{path}: {len(problems)} finding(s)")
        for p in problems:
            print(f"  - {p}")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
