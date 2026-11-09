"""Hand-built CoinJoin and look-alike transaction shapes with expected sources.

Each vector is a stored-transaction ``raw_json`` and the collaboration source
the public shape rules must give it (``None``: no shape). The expectations are
written by hand from docs/reference/local-chain-analysis.md ("CoinJoin shape
rules"), not computed by the code under test. ``old`` is what the rules before
the 13-19-output fix gave, so the changed cases stay visible.
"""
from __future__ import annotations

EQUAL = "equal_output_coinjoin"
LARGE = "large_equal_output_coinjoin"
DISTINCT = "distinct_value_coinjoin"
OP_RETURN = {"value": 0, "scriptpubkey": "6a04deadbeef", "scriptpubkey_type": "op_return"}


def script(side: str, index: int, number: int = 0) -> str:
    """A P2WPKH-shaped script unique to one leg (``side`` is one hex digit)."""
    return "0014" + side + f"{number:03x}{index:036x}"


def distinct(count: int, base: int) -> list[int]:
    """``count`` pairwise different, irregular amounts."""
    return [base + 7_919 * index * index + 13 * index for index in range(count)]


def tx(number: int, inputs: list[int | None], outputs: list[int], *, prevouts: bool = True,
       input_scripts: list[str] | None = None, output_scripts: list[str] | None = None,
       op_returns: int = 0) -> dict:
    input_scripts = input_scripts or [script("a", index, number) for index in range(len(inputs))]
    output_scripts = output_scripts or [script("b", index, number) for index in range(len(outputs))]
    vin = []
    for index, (value, locking) in enumerate(zip(inputs, input_scripts)):
        entry = {"txid": f"{number:032x}{index + 1:032x}", "vout": index % 3, "sequence": 0xFFFFFFFD}
        if prevouts:
            entry["prevout"] = {"scriptpubkey": locking, "scriptpubkey_type": "v0_p2wpkh"}
            if value is not None:
                entry["prevout"]["value"] = value
        vin.append(entry)
    vout = [{"n": index, "value": value, "scriptpubkey": locking, "scriptpubkey_type": "v0_p2wpkh"}
            for index, (value, locking) in enumerate(zip(outputs, output_scripts))]
    for _ in range(op_returns):
        vout.append({"n": len(vout), **OP_RETURN})
    return {"txid": f"{number:064x}", "version": 2, "locktime": 0, "vin": vin, "vout": vout}


def vectors() -> list[dict]:
    rows: list[dict] = []

    def add(name: str, raw: dict, expected: str | None, old: str | None, why: str) -> None:
        rows.append({"name": name, "raw": raw, "expected": expected, "old": old, "why": why})

    number = iter(range(1, 1_000))
    # Whirlpool: five 0.01 BTC outputs; two premix inputs carry the miner fee.
    add("whirlpool_5x5", tx(next(number), [1_000_250, 1_000_000, 1_000_000, 1_000_000, 1_000_310], [1_000_000] * 5),
        EQUAL, EQUAL, "Whirlpool 5x5: five equal outputs")
    add("whirlpool_5x5_without_prevouts", tx(next(number), [None] * 5, [1_000_000] * 5, prevouts=False),
        EQUAL, EQUAL, "the equal-output rule needs no input values")
    # JoinMarket: n equal outputs plus a change output for every maker.
    for parties, inputs in ((6, 9), (9, 12), (12, 15)):
        outputs = [2_500_000] * parties + distinct(parties - 1, 3_100_000)
        old = EQUAL if len(outputs) <= 12 else None
        add(f"joinmarket_{parties}_party", tx(next(number), distinct(inputs, 2_700_000), outputs),
            EQUAL, old, f"JoinMarket, {parties} equal outputs and {parties - 1} changes")
    add("joinmarket_4_party", tx(next(number), distinct(6, 2_700_000), [2_500_000] * 4 + distinct(3, 3_100_000)),
        None, None, "four equal outputs: below the shape threshold, a known miss")
    # The gap: five or more inputs and 13 to 19 outputs.
    for count in range(13, 20):
        add(f"gap_5in_{count}out", tx(next(number), distinct(5, 900_000), [500_000] * 5 + distinct(count - 5, 610_000)),
            EQUAL, None, "five equal outputs among 13-19")
    add("gap_19in_19out", tx(next(number), distinct(19, 900_000), [500_000] * 5 + distinct(14, 610_000)),
        EQUAL, None, "below the large rule's 20 legs on both sides")
    # Wasabi 1.x: a large 0.1 BTC class, some doubles and many changes.
    add("wasabi1_large", tx(next(number), distinct(60, 11_000_000), [10_000_000] * 50 + [20_000_000] * 8 + distinct(30, 400_000)),
        LARGE, LARGE, "Wasabi 1.x round")
    add("large_unbalanced_preserved", tx(next(number), distinct(20, 50_000_000), [1_000_000] * 5 + distinct(395, 70_000)),
        LARGE, LARGE, "the large rule keeps no balance limit")
    # WabiSabi (Wasabi 2.x): many standard denominations, each repeated.
    denominations = [5_000, 6_561, 8_192, 10_000, 19_683, 20_000, 50_000, 59_049, 65_536, 100_000, 131_072, 177_147]
    wabisabi = [value for value in denominations for _ in range(4)]
    add("wabisabi_four_per_denomination", tx(next(number), distinct(20, 30_000), wabisabi),
        DISTINCT, None, "no class of five, but few distinct values")
    add("wabisabi_six_per_denomination", tx(next(number), distinct(30, 30_000), [value for value in denominations for _ in range(6)]),
        LARGE, LARGE, "a class of six among 20+ legs per side")
    add("wabisabi_without_prevouts", tx(next(number), distinct(20, 30_000), wabisabi, prevouts=False),
        None, None, "unavailable input values are not repetition")
    add("wabisabi_without_input_values", tx(next(number), [None] * 20, wabisabi),
        None, None, "scripts without values: still unavailable")
    add("wabisabi_one_input_value_missing", tx(next(number), [None] + distinct(19, 30_000), wabisabi),
        None, None, "one unavailable value makes the ratio unavailable")
    add("wabisabi_negative_input_value", tx(next(number), [-1] + distinct(19, 30_000), wabisabi),
        None, None, "a negative amount is not an observed value")
    tag = next(number)
    unscripted = [script("a", index, tag) for index in range(20)]
    unscripted[6] = ""
    add("wabisabi_input_script_missing", tx(tag, distinct(20, 30_000), wabisabi, input_scripts=unscripted),
        None, None, "an unavailable script cannot be checked for reuse")
    tag = next(number)
    reused = [script("b", index, tag) for index in range(len(wabisabi))]
    reused[7] = script("a", 3, tag)
    add("wabisabi_output_reuses_input_script", tx(tag, distinct(20, 30_000), wabisabi, output_scripts=reused),
        None, None, "a reused script rules out the distinct-value shape")
    tag = next(number)
    twice = [script("b", index, tag) for index in range(len(wabisabi))]
    twice[9] = twice[2]
    add("wabisabi_output_script_repeated", tx(tag, distinct(20, 30_000), wabisabi, output_scripts=twice),
        None, None, "an output script paid twice rules it out")
    classes = [value for value in denominations[:7] for _ in range(4)]
    tag = next(number)
    spenders = [script("a", index, tag) for index in range(20)]
    spenders[4] = spenders[11]
    add("wabisabi_input_script_repeated", tx(tag, distinct(20, 30_000), wabisabi, input_scripts=spenders),
        None, None, "two inputs from one script rule it out")
    add("distinct_value_balance_edge", tx(next(number), distinct(6, 30_000), classes + [77_777]),
        DISTINCT, None, "29 outputs from 6 inputs, under five times")
    add("distinct_value_unbalanced", tx(next(number), distinct(6, 30_000), classes + [77_777, 88_888]),
        None, None, "30 outputs from 6 inputs: a batch payout shape")
    add("distinct_value_exactly_half", tx(next(number), distinct(6, 30_000), [value for value in denominations[:3] for _ in range(4)]),
        DISTINCT, None, "9 distinct values among 18 legs")
    add("distinct_value_over_half", tx(next(number), distinct(6, 30_000), [value for value in denominations[:3] for _ in range(4)][:-1] + [77_777]),
        None, None, "10 distinct values among 18 legs")
    pooled = [value for value in (40_000, 50_000, 60_000, 70_000, 80_000) for _ in range(5)]
    add("distinct_value_24in_5out", tx(next(number), pooled[:24], [97_000, 97_000, 98_000, 98_000, 99_000]),
        DISTINCT, None, "24 inputs into 5 outputs, under five times")
    add("distinct_value_25in_5out", tx(next(number), pooled, [97_000, 97_000, 98_000, 98_000, 99_000]),
        None, None, "25 inputs into 5 outputs: a consolidation shape")
    # Look-alikes that must stay unmarked.
    add("batch_payout_2in_40out", tx(next(number), distinct(2, 90_000_000), distinct(40, 120_000)),
        None, None, "ordinary batch payout")
    add("exchange_batch_6in_40out", tx(next(number), distinct(6, 90_000_000), [1_000_000] * 10 + distinct(30, 120_000)),
        None, None, "ten equal withdrawals among 40 outputs from 6 inputs")
    add("batch_5in_13out_distinct", tx(next(number), distinct(5, 9_000_000), distinct(13, 120_000)),
        None, None, "no repeated output value")
    add("payroll_5in_20out", tx(next(number), distinct(5, 9_000_000), [value for value in (300_000, 450_000, 520_000) for _ in range(4)] + distinct(8, 120_000)),
        None, None, "repeated salaries, no class of five, too many distinct values")
    add("consolidation_30in_1out", tx(next(number), [100_000] * 30, [2_999_000]),
        None, None, "consolidation of equal payouts")
    add("consolidation_25in_5out", tx(next(number), [100_000] * 25, distinct(5, 480_000)),
        None, None, "equal inputs and distinct outputs")
    add("equal_inputs_distinct_outputs", tx(next(number), [100_000] * 10, distinct(5, 190_000)),
        None, None, "repetition only on the input side hides no payment")
    add("four_inputs_five_equal", tx(next(number), distinct(4, 900_000), [500_000] * 5),
        None, None, "fewer than five inputs")
    add("op_return_not_an_output", tx(next(number), distinct(5, 900_000), [500_000] * 4, op_returns=1),
        None, None, "an OP_RETURN output is not a fifth equal output")
    add("op_return_ignored", tx(next(number), distinct(5, 900_000), [500_000] * 5, op_returns=1),
        EQUAL, EQUAL, "an OP_RETURN output does not hide the shape")
    add("equal_balance_edge_5in_24out", tx(next(number), distinct(5, 9_000_000), [500_000] * 5 + distinct(19, 120_000)),
        EQUAL, None, "24 outputs from 5 inputs, under five times")
    add("equal_balance_edge_5in_25out", tx(next(number), distinct(5, 9_000_000), [500_000] * 5 + distinct(20, 120_000)),
        None, None, "25 outputs from 5 inputs: a batch payout shape")
    # Accepted false positive: a many-input batch paying one amount repeatedly.
    add("repeated_amount_batch", tx(next(number), distinct(6, 9_000_000), [250_000] * 8 + distinct(16, 120_000)),
        EQUAL, None, "accepted false positive: suppresses guesses only")
    return rows
