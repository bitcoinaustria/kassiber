"""Direct deriver witnesses, outside the raw snapshot admission contract."""
from copy import deepcopy
import json
from pathlib import Path
import sys

from kassiber.core.ownership_transfers import derive_recorded_fanout_transfers
from tests.custody_fanout_generator import fanout_snapshot

PATH = Path(__file__).parent / "fixtures/custody_fanout_cases.json"
CASES = (
    "handled_source", "handled_target", "handled_other_source", "zero_other_source", "negative_other_source",
    "source_string", "source_unicode", "target_string", "fee_string", "huge_integer", "sum_above_i64",
    "bad_source", "bad_target", "bad_fee", "bad_type", "bad_decimal", "snan_value", "nan_payload_overflow",
    "false_value", "empty_value", "empty_rate", "empty_exact_rate", "numeric_value", "float_value", "negative_zero_value",
    "nan_payload_limit", "positive_nan_value", "long_nan_zero_prefix", "unicode_wallet_ids", "nonstring_ids",
    "zero_target", "negative_target", "single_destination", "overflow_value",
)


def vectors():
    result = []
    for seed, case in enumerate(CASES):
        rows = deepcopy(fanout_snapshot(seed, case="bitcoin")["transactions"])
        source = next(row for row in rows if row["direction"] == "outbound")
        ins = [row for row in rows if row["direction"] == "inbound"]
        handled = set()
        if case == "handled_source": handled.add(source["id"])
        if case == "handled_target": handled.add(ins[0]["id"])
        if case in {"handled_other_source", "zero_other_source", "negative_other_source"}:
            other = {**source, "id": "other-source", "wallet_id": "wallet-d"}
            if case != "handled_other_source": other["amount"] = 0 if case.startswith("zero") else -1
            rows.append(other)
            handled.add(other["id"])
        if case == "source_string": source["amount"] = f"+{source['amount']:_}"
        if case == "source_unicode": source["amount"] = str(source["amount"]).translate(str.maketrans("0123456789", "٠١٢٣٤٥٦٧٨٩"))
        if case == "target_string": ins[0]["amount"] = str(ins[0]["amount"])
        if case == "fee_string": source["fee"] = " +٣_٠٠٠ "
        if case in {"huge_integer", "sum_above_i64"}:
            for row in ins: row["amount"] = 10**80 + 1 if case == "huge_integer" else (1 << 63) - 1
            source["amount"] = sum(row["amount"] for row in ins)
        if case in {"bad_source", "bad_type"}: source["amount"] = "not-an-int" if case == "bad_source" else [1]
        if case == "bad_target": ins[0]["amount"] = "not-an-int"
        if case == "bad_fee": source["fee"] = "not-an-int"
        if case == "overflow_value": source["fiat_value"] = "1e1000001"
        if case == "bad_decimal": source["fiat_value"] = "not-a-decimal"
        if case == "snan_value": source["fiat_value"] = "sNaN1"
        if case == "nan_payload_overflow": source["fiat_value"] = "NaN" + "1" * 33
        if case == "false_value": source["fiat_value"] = False
        if case == "empty_value": source["fiat_value"] = ""
        if case == "empty_rate": source.update(fiat_rate="", fiat_value="1.2345")
        if case == "empty_exact_rate": source.update(fiat_rate_exact="", fiat_value_exact="1.2345")
        if case == "numeric_value": source["fiat_value"] = 123
        if case == "float_value": source["fiat_value"] = 1.2345678901234567
        if case == "negative_zero_value": source["fiat_value"] = "-0.0000"
        if case == "nan_payload_limit": source["fiat_value"] = "NaN" + "1" * 32
        if case == "positive_nan_value": source["fiat_value"] = "+NaN0"
        if case == "long_nan_zero_prefix": source["fiat_value"] = "NaN" + "0" * 50 + "1"
        if case == "unicode_wallet_ids":
            for row, wid in zip(ins, ("\U0001e030", "\u0430")): row["wallet_id"] = wid
        if case == "nonstring_ids":
            source["id"] = None
            ins[0]["id"] = 17
        if case in {"zero_target", "negative_target"}:
            ins[0]["amount"] = 0 if case == "zero_target" else -1000
            source["amount"] = sum(row["amount"] for row in ins)
        if case == "single_destination":
            rows.remove(ins[1])
            source["amount"] = ins[0]["amount"]
        vector = {"case": case, "rows": rows, "handled_ids": sorted(handled)}
        try:
            expected = derive_recorded_fanout_transfers(rows, already_paired_ids=handled)
            vector["expected"] = {key: sorted(value) if isinstance(value, set) else value for key, value in vars(expected).items()}
        except (TypeError, ValueError, ArithmeticError) as exc:
            vector["error"] = type(exc).__name__
        result.append(vector)
    return result


if __name__ == "__main__":
    text = json.dumps(vectors(), indent=2, ensure_ascii=True, sort_keys=True) + "\n"
    if "--write" in sys.argv: PATH.write_text(text)
    else: assert PATH.read_text() == text
