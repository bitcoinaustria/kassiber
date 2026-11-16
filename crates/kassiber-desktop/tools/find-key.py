# Lists i18n keys whose English value matches each argument (exact, then case-insensitive substring).
# Usage: python3 -I tools/find-key.py "Text one" "Text two" ...
from pathlib import Path
import json, os, sys
root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..", "assets", "i18n", "locales")
def flat(prefix, v, out):
    if isinstance(v, dict):
        for k, x in v.items():
            flat(prefix + ("" if prefix.endswith(":") else ".") + k, x, out)
    elif isinstance(v, list):
        for i, x in enumerate(v):
            flat(f"{prefix}.{i}", x, out)
    else:
        out[prefix] = str(v)
en, de = {}, {}
for lang, out in (("en", en), ("de", de)):
    d = os.path.join(root, lang)
    for f in sorted(os.listdir(d)):
        flat(f[:-5] + ":", json.loads(Path(d, f).read_text(encoding="utf-8")), out)
for q in sys.argv[1:]:
    exact = [k for k, v in en.items() if v == q]
    print(f"== {q!r}")
    hits = exact or [k for k, v in en.items() if q.lower() in v.lower()][:8]
    for k in hits:
        print(f"   {'=' if k in exact else '~'} {k}  ->  {en[k]!r}  | de: {de.get(k, 'MISSING')!r}")
