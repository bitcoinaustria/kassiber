"""Run the real CLI on a disposable, offline book and capture its PDF input."""
from contextlib import redirect_stdout
import io
import json
import os
from pathlib import Path
import socket
import sys
import tempfile
from unittest.mock import patch

out = Path(sys.argv[1] if len(sys.argv) > 1 else "spikes/artifacts/summary").resolve()
out.mkdir(parents=True, exist_ok=True)

with tempfile.TemporaryDirectory(prefix="kassiber-r0-summary-") as tmp:
    os.environ.update({
        "KASSIBER_NO_EGRESS": "1", "KASSIBER_DISABLE_UPDATE_CHECK": "1",
        "KASSIBER_STATE_ROOT": tmp + "/state", "XDG_CONFIG_HOME": tmp + "/config",
        "XDG_DATA_HOME": tmp + "/data", "XDG_CACHE_HOME": tmp + "/cache",
    })
    from kassiber.cli.main import main
    from kassiber import summary_pdf_report as pdf
    from kassiber._pdf_common import require_reportlab, register_fonts

    root = tmp + "/book"
    commands = []

    def deny(*args, **kwargs):
        raise AssertionError("network access attempted during offline summary spike")

    def cli(*args):
        stdout = io.StringIO()
        with redirect_stdout(stdout), patch.object(socket.socket, "connect", deny), patch.object(socket, "getaddrinfo", deny):
            code = main(["--data-root", root, "--machine", *args])
        payload = json.loads(stdout.getvalue())
        assert code == 0, (args, payload)
        commands.append({"args": list(args), "kind": payload["kind"], "exit": code})
        return payload

    cli("init")
    cli("workspaces", "create", "R0")
    cli("profiles", "create", "--workspace", "R0", "--fiat-currency", "EUR", "--tax-country", "generic", "Synthetic")
    scope = ["--workspace", "R0", "--profile", "Synthetic"]
    cli("wallets", "create", *scope, "--label", "Synthetic wallet", "--kind", "custom")
    csv = Path(tmp) / "synthetic.csv"
    csv.write_text(
        "date,txid,direction,asset,amount,fee,fiat_rate,description,kind\n"
        "2026-01-10T10:00:00Z,r0-buy-1,inbound,BTC,0.10000000,0,40000,First buy,buy\n"
        "2026-02-10T10:00:00Z,r0-buy-2,inbound,BTC,0.05000000,0,50000,Second buy,buy\n"
        "2026-03-10T10:00:00Z,r0-sell,outbound,BTC,0.02000000,0.00001000,60000,Sale,sell\n"
        "2026-04-10T10:00:00Z,r0-buy-3,inbound,BTC,0.03000000,0,55000,Third buy,buy\n"
    )
    cli("wallets", "import-csv", *scope, "--wallet", "Synthetic wallet", "--file", str(csv))
    cli("journals", "process", *scope)
    original = pdf.write_summary_pdf

    def capture(path, report):
        report = dict(report)
        report["generated_at"] = "2026-05-01T12:00:00Z"
        if report.get("snapshot"):
            report["snapshot"]["as_of"] = report["generated_at"]
        (out / "input.json").write_text(json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2, default=str) + "\n")
        return original(path, report)

    with patch.object(pdf, "write_summary_pdf", capture):
        cli("reports", "export-summary-pdf", *scope, "--start", "2026-01-01T00:00:00Z", "--end", "2026-04-30T23:59:59Z", "--include-snapshot", "--file", str(out / "python.pdf"))
    report = json.loads((out / "input.json").read_text())
    original(out / "python-repeat.pdf", report)
    assert (out / "python.pdf").read_bytes() == (out / "python-repeat.pdf").read_bytes()
    # Store only synthetic evidence, with disposable paths redacted for repeatable review.
    (out / "cli.json").write_text(json.dumps(commands, indent=2).replace(tmp, "<temporary-root>") + "\n")
    print(json.dumps({"commands": len(commands), "reportlab_fonts": register_fonts(require_reportlab("spike")), "python_identical_bytes": True, "input": str(out / "input.json")}))
