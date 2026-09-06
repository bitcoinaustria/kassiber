"""Versioned local attribution packs; every match remains a sourced claim.

``preview_dataset(manifest, stream, format=..., adapter=...)`` validates a whole
already-open binary stream without storing it. ``import_dataset(conn, profile,
manifest, stream, expected_sha256=...)`` repeats validation while staging at most
2,000 records per commit, then atomically activates the complete version. It
requires an idle connection and never commits caller-owned work. A failed or
interrupted staging version is invisible; the previous active version survives.
Callers open local files (desktop: a reviewed staged token), supply the preview
hash, and own cancellation/progress. Neither function accepts paths or egresses.

``match_subjects`` performs indexed exact lookups for observed subjects, not a
scan of pack records. It returns label-compatible claims plus full pack/source
provenance and a dataset-state digest even when there are no matches. Public
reference packs and private book labels have separate visibility. No automatic
download, probabilistic membership, ownership propagation or accounting writes.
"""
from __future__ import annotations

from collections import defaultdict
from contextlib import contextmanager
import base64
import csv
import hashlib
import json
import re
import uuid
from urllib.parse import urlsplit

from ..errors import AppError
from ..time_utils import now_iso, parse_timestamp
from ..wallet_descriptors import normalize_network
from .address_scripts import address_to_scriptpubkey, base58check_decode

MAX_BYTES = 8 * 1024**3
MAX_RECORD_BYTES = 64 * 1024
MAX_ROWS = 50_000_000
BATCH_SIZE = 2000
MAX_SUBJECTS = 100_000
MAX_SCRIPT_BYTES = 10_000
_MANIFEST_REQUIRED = {"dataset_key", "name", "version", "chain", "network", "source", "license", "attribution_method", "visibility"}
_MANIFEST_OPTIONAL = {"source_url", "observed_at", "valid_from", "valid_until", "category", "expected_active_id"}
_CATEGORIES = {"exchange", "merchant", "mixer", "service", "self", "other"}
_CATEGORY_MAP = {"mining": "service", "payment": "service", "gambling": "service", "darknet": "service", "scam": "other", "p2p": "service", "historical": "service", "historic": "service", "unknown": "other"}


def _error(message, code="validation", **details):
    raise AppError(message, code=code, details=details or None, retryable=code == "chain_analysis_stale") from None


def _json(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False)


def _text(value, field, limit=256):
    if not isinstance(value, str) or not value.strip() or len(value) > limit or any(ord(char) < 32 for char in value):
        _error(f"{field} must be nonempty text up to {limit} characters")
    return value.strip()


def _date(value, field):
    if value in (None, ""):
        return None
    _text(value, field, 40)
    try:
        return parse_timestamp(value)
    except (AppError, ValueError, OverflowError):
        _error(f"{field} must be an ISO date or timestamp")


def _window(start, end):
    if start and end and start >= end:
        _error("valid_until must be later than valid_from")


def _url(value):
    if value in (None, ""):
        return None
    value = _text(value, "source_url", 1024)
    try:
        parsed = urlsplit(value)
        if parsed.scheme not in {"http", "https"} or not parsed.hostname or parsed.username or parsed.password or parsed.query or parsed.fragment:
            raise ValueError
    except ValueError:
        _error("source_url must be a public HTTP(S) reference without credentials, query or fragment")
    return value


def _domain(chain, network):
    if chain not in ("bitcoin", "liquid") or not isinstance(network, str) or not network:
        _error("An explicit Bitcoin or Liquid chain and network are required")
    try:
        return chain, normalize_network(chain, network)
    except (ValueError, TypeError):
        _error("Unsupported attribution network")


def canonical_subject(chain, network, subject):
    """Return (chain, canonical network, tx:/out:/script: exact subject key).

    Addresses are checksum/network checked and become their locking script.
    Canonical graph transaction/output IDs and bare txids/outpoints also work.
    Script keys require an explicit domain; they cannot establish their network.
    Wallets, Lightning records, descriptors and arbitrary identity text are not
    attribution subjects. This helper does not look up ownership.
    """
    chain, network = _domain(chain, network)
    value = _text(subject, "subject", MAX_SCRIPT_BYTES * 2 + 64)
    parts = value.split(":", 2)
    if len(parts) == 3 and parts[0] in {"bitcoin", "liquid"}:
        if _domain(parts[0], parts[1]) != (chain, network):
            _error("Attribution subject belongs to a different chain or network")
        value = parts[2]
    physical = value[3:] if value.startswith("tx:") else value[4:] if value.startswith("out:") else value
    if re.fullmatch(r"[0-9a-fA-F]{64}", physical):
        if value.startswith("out:"):
            _error("An output subject requires an output index")
        return chain, network, "tx:" + physical.lower()
    match = re.fullmatch(r"([0-9a-fA-F]{64}):(0|[1-9][0-9]{0,9})", physical)
    if match and not value.startswith("tx:") and int(match[2]) <= 0xFFFFFFFF:
        return chain, network, f"out:{match[1].lower()}:{int(match[2])}"
    if value.startswith("script:") and len(value[7:]) <= MAX_SCRIPT_BYTES * 2 and re.fullmatch(r"(?:[0-9a-fA-F]{2})+", value[7:]):
        return chain, network, value.lower()
    try:
        if chain == "bitcoin":
            script = address_to_scriptpubkey(value)
            lower = value.lower()
            if lower.startswith(("bc1", "tb1", "bcrt1")):
                expected = "bc" if network == "main" else "bcrt" if network == "regtest" else "tb"
                if lower.split("1", 1)[0] != expected:
                    raise ValueError
            elif base58check_decode(value)[0] not in ({0, 5} if network == "main" else {111, 196}):
                raise ValueError
        else:
            from embit.liquid.addresses import addr_decode
            from embit.liquid.networks import NETWORKS
            net = NETWORKS[network]
            hrp = value.lower().split("1", 1)[0]
            if hrp in {net["bech32"], net["blech32"]}:
                if value != value.lower() and value != value.upper():
                    raise ValueError
                value = value.lower()
            else:
                raw = base58check_decode(value)
                if not (raw[:1] == net["p2sh"] or raw[:2] == net["bp2sh"]):
                    raise ValueError
            decoded, _ = addr_decode(value)
            script = decoded.data
            if not script:
                raise ValueError
        return chain, network, "script:" + script.hex()
    except (AppError, ValueError, TypeError, RuntimeError, KeyError, IndexError, AssertionError):
        _error("Attribution subject must be a valid address, transaction, output or script in the selected domain")


def normalize_manifest(manifest):
    if not isinstance(manifest, dict) or _MANIFEST_REQUIRED - manifest.keys() or manifest.keys() - _MANIFEST_REQUIRED - _MANIFEST_OPTIONAL:
        _error("Unsupported or missing attribution manifest fields")
    chain, network = _domain(manifest["chain"], manifest["network"])
    result = {field: _text(manifest[field], field, 512 if field in {"source", "license"} else 256) for field in _MANIFEST_REQUIRED}
    result.update(chain=chain, network=network)
    if not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9._-]{0,127}", result["dataset_key"]):
        _error("dataset_key must be a stable identifier using letters, digits, dot, underscore or hyphen")
    if result["visibility"] not in {"public", "private"}:
        _error("Dataset visibility must be public or private")
    result["source_url"] = _url(manifest.get("source_url"))
    for field in ("observed_at", "valid_from", "valid_until"):
        result[field] = _date(manifest.get(field), field)
    _window(result["valid_from"], result["valid_until"])
    result["category"] = _text(manifest.get("category", "unknown"), "category", 64)
    result["expected_active_id"] = _text(manifest["expected_active_id"], "expected_active_id", 64) if manifest.get("expected_active_id") is not None else None
    return result


class _Lines:
    """Bound individual records and the whole binary input before decoding."""
    def __init__(self, stream, cancelled=None):
        if not callable(getattr(stream, "readline", None)):
            _error("Dataset input must be an already-open binary stream")
        self.stream, self.byte_count, self.record_bytes, self.line_number = stream, 0, 0, 0
        self.sha256 = hashlib.sha256()
        self.cancelled = cancelled

    def __iter__(self):
        return self

    def __next__(self):
        if self.cancelled and self.cancelled():
            _error("Dataset import cancelled", "cancelled")
        raw = self.stream.readline(MAX_RECORD_BYTES + 1)
        if not isinstance(raw, bytes):
            _error("Dataset input must be a binary UTF-8 stream")
        if not raw:
            raise StopIteration
        self.byte_count += len(raw)
        self.record_bytes += len(raw)
        self.line_number += 1
        if self.byte_count > MAX_BYTES or self.record_bytes > MAX_RECORD_BYTES:
            _error("Dataset byte or record limit exceeded", "chain_analysis_dataset_limit", line=self.line_number)
        self.sha256.update(raw)
        try:
            return raw.decode("utf-8-sig" if self.line_number == 1 else "utf-8")
        except UnicodeError:
            _error("Dataset must be UTF-8", line=self.line_number)


def _records(lines, format):
    if format == "csv":
        try:
            reader = csv.DictReader(lines, strict=True)
            headers = reader.fieldnames
            if not headers or len(headers) > 64 or any(not isinstance(key, str) or not key.strip() or len(key) > 64 for key in headers):
                _error("Dataset CSV requires a bounded header")
            headers = [key.strip().lower() for key in headers]
            if len(headers) != len(set(headers)):
                _error("Dataset CSV contains duplicate column names")
            reader.fieldnames = headers
            lines.record_bytes = 0
            for row in reader:
                if None in row or any(value is None for value in row.values()):
                    _error("Dataset CSV row does not match its header", line=lines.line_number)
                yield row
                lines.record_bytes = 0
        except csv.Error:
            _error("Malformed dataset CSV", line=lines.line_number)
    else:
        for line in lines:
            if not line.strip():
                lines.record_bytes = 0
                continue
            try:
                row = json.loads(line)
            except (ValueError, RecursionError):
                _error("Malformed dataset JSONL", line=lines.line_number)
            if not isinstance(row, dict) or len(row) > 64 or any(not isinstance(key, str) or len(key) > 64 for key in row):
                _error("Dataset JSONL requires one bounded object per line", line=lines.line_number)
            lowered = {key.strip().lower(): value for key, value in row.items()}
            if len(lowered) != len(row):
                _error("Dataset JSONL contains duplicate column names", line=lines.line_number)
            row = lowered
            yield row
            lines.record_bytes = 0


def _claim(row, manifest, adapter, record_number):
    def first(*keys):
        return next((row[key] for key in keys if row.get(key) not in (None, "")), None)

    subject = first("subject", "address", "hashadd")
    category = first("category", "original_category") or manifest["category"]
    label = first("label", "entity", "name")
    if adapter == "maru92":
        fields = [key for key in ("exchange", "gambling", "mining", "service", "historic") if row.get(key) not in (None, "")]
        if len(fields) != 1:
            _error("Maru92 rows require exactly one entity category column", record_number=record_number)
        category, label = fields[0], row[fields[0]]
    if row.get("chain") not in (None, "") or row.get("network") not in (None, ""):
        if _domain(row.get("chain", manifest["chain"]), row.get("network", manifest["network"])) != (manifest["chain"], manifest["network"]):
            _error("Dataset row belongs to a different chain or network", record_number=record_number)
    _, _, key = canonical_subject(manifest["chain"], manifest["network"], subject)
    original_category = _text(category, "category", 64)
    mapped_category = original_category.lower()
    if mapped_category not in _CATEGORIES:
        mapped_category = _CATEGORY_MAP.get(mapped_category, "other")
    source_record = {"record_number": record_number, "original_subject": _text(subject, "subject", MAX_SCRIPT_BYTES * 2 + 64), "original_category": original_category}
    for field, limit in (("source", 512), ("source_id", 256), ("attribution_method", 256)):
        if row.get(field) not in (None, ""):
            source_record[field] = _text(row[field], field, limit)
    if row.get("source_url") not in (None, ""):
        source_record["source_url"] = _url(row["source_url"])
    for field in ("observed_at", "date_first_tx"):
        if row.get(field) not in (None, ""):
            source_record[field] = _date(row[field], field)
    start, end = _date(row.get("valid_from"), "valid_from"), _date(row.get("valid_until"), "valid_until")
    _window(start, end)
    return {"record_number": record_number, "subject": key, "label": _text(label, "label"), "category": mapped_category, "source_record": source_record, "valid_from": start, "valid_until": end}


def _parse(manifest, stream, format, adapter, cancelled=None):
    if format not in ("csv", "jsonl") or adapter not in ("generic", "am_i_exposed", "maru92"):
        _error("Dataset format must be csv/jsonl and adapter generic/am_i_exposed/maru92")
    lines = _Lines(stream, cancelled)

    def claims():
        for number, row in enumerate(_records(lines, format), 1):
            if number > MAX_ROWS:
                _error("Dataset record limit exceeded", "chain_analysis_dataset_limit")
            try:
                yield _claim(row, manifest, adapter, number)
            except AppError as exc:
                if exc.details is None:
                    exc.details = {"record_number": number}
                raise

    return lines, claims()


def preview_dataset(manifest, stream, *, format="csv", adapter="generic", cancelled=None, progress=None):
    manifest = normalize_manifest(manifest)
    lines, claims = _parse(manifest, stream, format, adapter, cancelled)
    sample, count = [], 0
    for claim in claims:
        count += 1
        if len(sample) < 10:
            sample.append(claim)
        if progress and count % BATCH_SIZE == 0:
            progress({"row_count": count, "byte_count": lines.byte_count, "status": "validating"})
    if not count:
        _error("Dataset contains no attribution claims")
    return {"manifest": manifest, "format": format, "adapter": adapter, "sha256": lines.sha256.hexdigest(), "byte_count": lines.byte_count, "row_count": count, "sample_claims": sample, "validation": "complete", "accounting_authority": False}


@contextmanager
def _atomic(conn):
    name = "dataset_" + uuid.uuid4().hex
    conn.execute(f"SAVEPOINT {name}")
    try:
        yield
    except BaseException:
        conn.execute(f"ROLLBACK TO {name}")
        raise
    finally:
        conn.execute(f"RELEASE {name}")


def _pack(row):
    result = dict(row)
    result.pop("profile_id", None)
    result["manifest"] = json.loads(result.pop("manifest_json"))
    result["accounting_authority"] = False
    return result


def get_dataset(conn, profile_id, dataset_id):
    row = conn.execute("SELECT * FROM chain_analysis_datasets WHERE profile_id=? AND id=?", (profile_id, dataset_id)).fetchone()
    if row is None:
        _error("Dataset not found in this book", "not_found")
    return _pack(row)


def list_datasets(conn, profile_id, args=None):
    args = {} if args is None else args
    if not isinstance(args, dict) or args.keys() - {"limit", "cursor"}:
        _error("Unsupported dataset list arguments")
    limit = args.get("limit", 50)
    if type(limit) is not int or not 1 <= limit <= 100:
        _error("Dataset list limit must be between 1 and 100")
    params, clause = [profile_id], ""
    if args.get("cursor"):
        try:
            cursor = json.loads(base64.urlsafe_b64decode(_text(args["cursor"], "cursor", 2048)))
            if not isinstance(cursor, list) or len(cursor) != 3 or cursor[0] != profile_id or not all(isinstance(value, str) for value in cursor):
                raise ValueError
        except (ValueError, TypeError):
            _error("Invalid dataset cursor")
        clause = " AND (created_at,id)<(?,?)"
        params.extend(cursor[1:])
    rows = conn.execute(f"SELECT * FROM chain_analysis_datasets WHERE profile_id=?{clause} ORDER BY created_at DESC,id DESC LIMIT ?", (*params, limit + 1)).fetchall()
    next_cursor = base64.urlsafe_b64encode(_json([profile_id, rows[limit-1]["created_at"], rows[limit-1]["id"]]).encode()).decode() if len(rows) > limit else None
    return {"items": [_pack(row) for row in rows[:limit]], "next_cursor": next_cursor}


def _check_replacement(conn, profile_id, manifest):
    active = conn.execute("SELECT id FROM chain_analysis_datasets WHERE profile_id=? AND dataset_key=? AND status='active'", (profile_id, manifest["dataset_key"])).fetchone()
    if (active["id"] if active else None) != manifest["expected_active_id"]:
        _error("Active dataset changed; preview the replacement again", "chain_analysis_stale")
    reused = conn.execute("SELECT 1 FROM chain_analysis_datasets WHERE profile_id=? AND dataset_key=? AND version=? AND status IN ('active','superseded','revoked') LIMIT 1", (profile_id, manifest["dataset_key"], manifest["version"])).fetchone()
    if reused:
        _error("Dataset versions are immutable; choose a new version")


def import_dataset(conn, profile_id, manifest, stream, *, format="csv", adapter="generic", expected_sha256=None, progress=None, cancelled=None):
    """Import on an idle dedicated connection; activate only a fully valid pack.

    Progress receives bounded {id,row_count,byte_count,status} dictionaries after
    committed batches. Raising from the callback cancels; already staged rows
    remain invisible under a failed pack. Incomplete packs can be explicitly
    discarded with ``discard_dataset`` (also chunked), never auto-resumed.
    """
    manifest = normalize_manifest(manifest)
    if conn.in_transaction:
        _error("Dataset imports require an idle connection; finish caller-owned writes first", "chain_analysis_dataset_busy")
    if expected_sha256 is not None and (not isinstance(expected_sha256, str) or not re.fullmatch(r"[0-9a-f]{64}", expected_sha256)):
        _error("expected_sha256 must be a lowercase SHA-256 digest")
    if conn.execute("SELECT 1 FROM profiles WHERE id=?", (profile_id,)).fetchone() is None:
        _error("Book not found", "not_found")
    lines, claims = _parse(manifest, stream, format, adapter, cancelled)
    dataset_id, timestamp = str(uuid.uuid4()), now_iso()
    with _atomic(conn):
        _check_replacement(conn, profile_id, manifest)
        conn.execute("INSERT INTO chain_analysis_datasets(id,profile_id,dataset_key,version,chain,network,visibility,status,manifest_json,created_at,updated_at) VALUES(?,?,?,?,?,?,?,'staging',?,?,?)", (dataset_id, profile_id, manifest["dataset_key"], manifest["version"], manifest["chain"], manifest["network"], manifest["visibility"], _json({**manifest, "format": format, "adapter": adapter}), timestamp, timestamp))
    count, batch = 0, []
    claims_hash = hashlib.sha256()

    def flush():
        with _atomic(conn):
            # A reset or explicit discard while the file was being read cannot
            # resurrect the book/pack. Foreign keys enforce the same boundary.
            row = conn.execute("SELECT status FROM chain_analysis_datasets WHERE id=? AND profile_id=?", (dataset_id, profile_id)).fetchone()
            if row is None or row["status"] != "staging":
                _error("Staged dataset changed during import", "chain_analysis_stale")
            conn.executemany("INSERT INTO chain_analysis_dataset_claims VALUES(?,?,?,?,?,?,?,?)", batch)
            conn.execute("UPDATE chain_analysis_datasets SET row_count=?,byte_count=?,updated_at=? WHERE id=?", (count, lines.byte_count, now_iso(), dataset_id))
        batch.clear()
        if progress:
            progress({"id": dataset_id, "row_count": count, "byte_count": lines.byte_count, "status": "staging"})

    try:
        for claim in claims:
            count += 1
            claims_hash.update((_json(claim) + "\n").encode())
            batch.append((dataset_id, count, claim["subject"], claim["label"], claim["category"], _json(claim["source_record"]), claim["valid_from"], claim["valid_until"]))
            if len(batch) >= BATCH_SIZE:
                flush()
        if not count:
            _error("Dataset contains no attribution claims")
        if batch:
            flush()
        content_hash = lines.sha256.hexdigest()
        if expected_sha256 is not None and content_hash != expected_sha256:
            _error("Dataset content changed since preview", "chain_analysis_stale")
        if cancelled and cancelled():
            _error("Dataset import cancelled before activation", "chain_analysis_cancelled")
        with _atomic(conn):
            _check_replacement(conn, profile_id, manifest)
            if manifest["expected_active_id"]:
                conn.execute("UPDATE chain_analysis_datasets SET status='superseded',revision=revision+1,updated_at=? WHERE id=?", (now_iso(), manifest["expected_active_id"]))
            changed = conn.execute("UPDATE chain_analysis_datasets SET status='active',content_sha256=?,claims_sha256=?,byte_count=?,updated_at=? WHERE id=? AND profile_id=? AND status='staging'", (content_hash, claims_hash.hexdigest(), lines.byte_count, now_iso(), dataset_id, profile_id)).rowcount
            if changed != 1:
                _error("Staged dataset changed before activation", "chain_analysis_stale")
        return get_dataset(conn, profile_id, dataset_id)
    except BaseException:
        with _atomic(conn):
            conn.execute("UPDATE chain_analysis_datasets SET status='failed',updated_at=? WHERE id=? AND profile_id=? AND status='staging'", (now_iso(), dataset_id, profile_id))
        raise


def revoke_dataset(conn, profile_id, dataset_id, expected_revision):
    with _atomic(conn):
        old = get_dataset(conn, profile_id, dataset_id)
        if type(expected_revision) is not int or old["revision"] != expected_revision:
            _error("Dataset changed; reload before revoking", "chain_analysis_stale")
        if old["status"] not in {"active", "superseded"}:
            _error("Only a complete dataset can be revoked")
        conn.execute("UPDATE chain_analysis_datasets SET status='revoked',revision=revision+1,updated_at=? WHERE id=? AND profile_id=?", (now_iso(), dataset_id, profile_id))
        return get_dataset(conn, profile_id, dataset_id)


def discard_dataset(conn, profile_id, dataset_id, *, progress=None, cancelled=None):
    """Remove only incomplete imports in bounded commits, on an idle connection."""
    if conn.in_transaction:
        _error("Dataset cleanup requires an idle connection", "chain_analysis_dataset_busy")
    with _atomic(conn):
        old = get_dataset(conn, profile_id, dataset_id)
        if old["status"] not in {"staging", "failed"}:
            _error("Complete versions are retained as provenance; revoke them instead")
        conn.execute("UPDATE chain_analysis_datasets SET status='failed' WHERE id=? AND profile_id=?", (dataset_id, profile_id))
    removed = 0
    while True:
        if cancelled and cancelled():
            _error("Dataset cleanup cancelled", "chain_analysis_cancelled")
        with _atomic(conn):
            deleted = conn.execute("DELETE FROM chain_analysis_dataset_claims WHERE dataset_id=? AND record_number IN (SELECT record_number FROM chain_analysis_dataset_claims WHERE dataset_id=? LIMIT ?)", (dataset_id, dataset_id, BATCH_SIZE)).rowcount
        removed += deleted
        if progress:
            progress({"rows_deleted": removed, "status": "discarding"})
        if not deleted:
            break
    with _atomic(conn):
        conn.execute("DELETE FROM chain_analysis_datasets WHERE id=? AND profile_id=? AND status='failed'", (dataset_id, profile_id))
    return {"id": dataset_id, "discarded": True}


def _matched_claim(pack, row):
    manifest = pack["manifest"]
    source_record = json.loads(row["source_record_json"])
    return {"id": f"dataset:{pack['id']}:{row['record_number']}", "revision": 1,
            "dataset_id": pack["id"], "dataset_status": pack["status"],
            "chain": pack["chain"], "network": pack["network"],
            "subject": row["subject"], "label": row["label"], "category": row["category"],
            "source": source_record.get("source", manifest["source"]), "confidence": "imported",
            "cluster_defining": False, "visibility": pack["visibility"],
            "attribution_method": source_record.get("attribution_method", manifest["attribution_method"]),
            "source_record": source_record, "valid_from": row["valid_from"], "valid_until": row["valid_until"],
            "dataset_version": pack["version"], "content_sha256": pack["content_sha256"],
            "license": manifest["license"], "evidence_level": "sourced_claim", "accounting_authority": False}


def query_claims(conn, profile_id, args):
    """Inspect exact subject or exact entity label, using bounded keyset pages.

    Subject queries require chain/network; absent dataset_id searches active
    packs. Label queries require dataset_id. A supplied pack may be superseded or
    revoked: this is historical inspection, so validity/status remain explicit
    and do not silently filter stored source records. No substring scan exists.
    Cursors bind profile, observer, query and dataset metadata state.
    """
    if not isinstance(args, dict) or args.keys() - {"subject", "chain", "network", "label", "dataset_id", "limit", "cursor", "observer"}:
        _error("Unsupported attribution query arguments")
    observer, limit = args.get("observer", "owner"), args.get("limit", 50)
    if observer not in ("public", "owner", "disclosed") or type(limit) is not int or not 1 <= limit <= 100:
        _error("Invalid attribution query observer or limit")
    has_subject, has_label = "subject" in args, "label" in args
    if has_subject == has_label or (has_label and (not args.get("dataset_id") or "chain" in args or "network" in args)):
        _error("Query an exact subject with chain/network, or an exact label with dataset_id")
    if has_subject:
        chain, network, value = canonical_subject(args.get("chain"), args.get("network"), args["subject"])
        query = {"chain": chain, "network": network, "subject": value}
        column, index = "subject", "idx_chain_analysis_dataset_subject"
    else:
        value = _text(args["label"], "label")
        query = {"label": value}
        column, index = "label", "idx_chain_analysis_dataset_entity"
    query.update(dataset_id=args.get("dataset_id"), observer=observer)
    with _atomic(conn):
        if args.get("dataset_id"):
            pack = get_dataset(conn, profile_id, _text(args["dataset_id"], "dataset_id", 64))
            if observer == "public" and pack["visibility"] != "public":
                _error("Dataset not found in this view", "not_found")
            if pack["status"] in {"staging", "failed"}:
                _error("An incomplete dataset cannot be queried")
            packs = [pack] if not has_subject or (pack["chain"], pack["network"]) == (chain, network) else []
        else:
            visible = " AND visibility='public'" if observer == "public" else ""
            packs = [_pack(row) for row in conn.execute(f"SELECT * FROM chain_analysis_datasets WHERE profile_id=? AND chain=? AND network=? AND status='active'{visible} ORDER BY id", (profile_id, chain, network))]
        packs.sort(key=lambda row: row["id"])
        binding = hashlib.sha256(_json([profile_id, query, packs]).encode()).hexdigest()
        after_pack, after_record = "", 0
        if args.get("cursor"):
            try:
                decoded = json.loads(base64.urlsafe_b64decode(_text(args["cursor"], "cursor", 2048)))
                if not isinstance(decoded, list) or len(decoded) != 3 or decoded[0] != binding or not isinstance(decoded[1], str) or type(decoded[2]) is not int or decoded[2] < 0:
                    raise ValueError
                after_pack, after_record = decoded[1], decoded[2]
            except (ValueError, TypeError):
                _error("Attribution cursor changed or belongs to another query", "chain_analysis_stale")
        items, position = [], []
        for pack in packs:
            if pack["id"] < after_pack:
                continue
            record = after_record if pack["id"] == after_pack else 0
            rows = conn.execute(f"SELECT * FROM chain_analysis_dataset_claims INDEXED BY {index} WHERE dataset_id=? AND {column}=? AND record_number>? ORDER BY record_number LIMIT ?", (pack["id"], value, record, limit + 1 - len(items)))
            for row in rows:
                items.append(_matched_claim(pack, row))
                position.append((pack["id"], row["record_number"]))
            if len(items) > limit:
                break
        next_cursor = base64.urlsafe_b64encode(_json([binding, *position[limit-1]]).encode()).decode() if len(items) > limit else None
        return {"items": items[:limit], "datasets": packs, "query": query,
                "next_cursor": next_cursor, "dataset_state_digest": binding,
                "coverage": {"scope": "exact_local_attribution_claims", "historical_inspection": True, "accounting_authority": False}}


def match_subjects(conn, profile_id, subjects, *, observer="owner", limit=5000, as_of=None):
    """Match only provided exact keys; preserve disagreements and source rows.

    ``as_of`` defaults to now. Validity is [valid_from, valid_until). Expired or
    not-yet-valid claims stay stored but do not count as present attribution.
    ``limit`` applies independently to public/private claim populations, so
    private knowledge cannot starve public observer analysis. Owner/disclosed
    calls return at most twice that limit; public calls return at most limit.
    Coverage reports each visibility bound; absence is never proof of anonymity.
    ``datasets`` includes active visible manifests even when nothing matches.
    """
    if observer not in ("public", "owner", "disclosed") or type(limit) is not int or not 1 <= limit <= 20_000:
        _error("Invalid dataset observer or match limit")
    instant = _date(as_of, "as_of") if as_of is not None else now_iso()
    grouped = defaultdict(set)
    for number, value in enumerate(subjects, 1):
        if number > MAX_SUBJECTS:
            _error("Too many observed attribution subjects", "chain_analysis_dataset_limit")
        if not isinstance(value, (tuple, list)) or len(value) != 3:
            _error("Observed subjects must be chain, network, subject triples")
        chain, network, subject = canonical_subject(*value)
        grouped[(chain, network)].add(subject)
    clause = " AND visibility='public'" if observer == "public" else ""
    with _atomic(conn):
        packs = [_pack(row) for row in conn.execute(f"SELECT * FROM chain_analysis_datasets WHERE profile_id=? AND status='active'{clause} ORDER BY id", (profile_id,))]
        state = hashlib.sha256(_json(packs).encode()).hexdigest()
        # Ownership knowledge must not consume an outside observer's claim
        # budget. Otherwise a large private pack encountered first would hide a
        # public attribution when a canonical owner snapshot is projected later.
        visibilities = ("public",) if observer == "public" else ("public", "private")
        found = {visibility: [] for visibility in visibilities}
        coverage_by_visibility = {visibility: {"match_count": 0, "active_dataset_count": 0,
                                  "inactive_by_date_count": 0, "lookup_queries": 0,
                                  "match_limit": limit, "truncated": False} for visibility in visibilities}
        for pack in packs:
            visibility = pack["visibility"]
            partition = coverage_by_visibility[visibility]
            partition["active_dataset_count"] += 1
            manifest = pack["manifest"]
            if (manifest["valid_from"] and instant < manifest["valid_from"]) or (manifest["valid_until"] and instant >= manifest["valid_until"]):
                partition["inactive_by_date_count"] += 1
                continue
            if partition["truncated"]:
                continue
            keys = sorted(grouped.get((pack["chain"], pack["network"]), ()))
            matches = found[visibility]
            for offset in range(0, len(keys), 200):
                chunk = keys[offset:offset+200]
                marks = ",".join("?" for _ in chunk)
                partition["lookup_queries"] += 1
                rows = conn.execute(f"SELECT * FROM chain_analysis_dataset_claims INDEXED BY idx_chain_analysis_dataset_subject WHERE dataset_id=? AND subject IN ({marks}) AND (valid_from IS NULL OR valid_from<=?) AND (valid_until IS NULL OR valid_until>?) ORDER BY subject,record_number LIMIT ?", (pack["id"], *chunk, instant, instant, limit + 1 - len(matches))).fetchall()
                matches.extend(_matched_claim(pack, row) for row in rows)
                if len(matches) > limit:
                    partition["truncated"] = True
                    break
        for visibility in visibilities:
            coverage_by_visibility[visibility]["match_count"] = min(limit, len(found[visibility]))
        claims = [claim for visibility in visibilities for claim in found[visibility][:limit]]
        coverage = {"subject_count": sum(len(keys) for keys in grouped.values()), "match_count": len(claims),
                    "active_dataset_count": len(packs), "inactive_by_date_count": sum(row["inactive_by_date_count"] for row in coverage_by_visibility.values()),
                    "lookup_queries": sum(row["lookup_queries"] for row in coverage_by_visibility.values()), "match_limit": limit,
                    "match_limit_per_visibility": True,
                    "truncated": any(row["truncated"] for row in coverage_by_visibility.values()),
                    "visibility_coverage": coverage_by_visibility, "as_of": instant, "complete_chain_coverage": False,
                    "authority": "imported_attribution_claims"}
    return {"claims": claims, "datasets": packs, "dataset_state_digest": state, "coverage": coverage}
