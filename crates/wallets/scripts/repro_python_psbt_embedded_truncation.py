"""Reproduce uncaught embedded-decoder errors without changing Python."""
from generate_psbt_vectors import encode, sample
from kassiber.core.chain_analysis.psbt import analyze_psbt, decode_psbt_structure

for operation in (analyze_psbt, decode_psbt_structure):
    g, i, o = sample()
    i[0] = [(b"\0", b"")]
    try:
        operation(encode(g, i, o), **({"network": "main"} if operation is analyze_psbt else {}))
    except RuntimeError as error:
        assert str(error) == "Can't read one byte from the stream"
        print(operation.__name__, type(error).__name__, str(error))
    else:
        raise AssertionError("Reference behavior changed")
