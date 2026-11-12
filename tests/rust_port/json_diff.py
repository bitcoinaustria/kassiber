"""Print the paths where two JSON values differ (debugging aid for parity tests)."""


def json_diff(left, right, path="$", out=None, limit=40):
    out = [] if out is None else out
    if len(out) >= limit:
        return out
    if isinstance(left, dict) and isinstance(right, dict):
        for key in sorted(set(left) | set(right), key=str):
            if key not in left or key not in right:
                out.append(f"{path}.{key}: {left.get(key, '<missing>')!r} != {right.get(key, '<missing>')!r}"[:400])
            else:
                json_diff(left[key], right[key], f"{path}.{key}", out, limit)
    elif isinstance(left, list) and isinstance(right, list) and len(left) == len(right):
        for index, (a, b) in enumerate(zip(left, right)):
            json_diff(a, b, f"{path}[{index}]", out, limit)
    elif left != right:
        out.append(f"{path}: {left!r} != {right!r}"[:400])
    return out
