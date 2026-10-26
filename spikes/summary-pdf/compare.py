"""Compare semantic strings, table rows, chart labels, pagination, and repeat bytes."""
from collections import Counter
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import time

from kassiber import summary_pdf_report as p
from kassiber._pdf_common import require_reportlab, register_fonts

root = Path(sys.argv[1] if len(sys.argv)>1 else "spikes/artifacts/summary").resolve()
report = json.loads((root/"input.json").read_text())
python_render_ms=[]
for name in ("python.pdf", "python-repeat.pdf"):
    started=time.perf_counter()
    p.write_summary_pdf(root/name, report)
    python_render_ms.append(round((time.perf_counter()-started)*1000, 3))
binary = Path(__file__).resolve().parent/"target/release/r0-summary-pdf"
runs=[]
for name in ("rust.pdf", "rust-repeat.pdf"):
    runs.append(json.loads(subprocess.check_output([str(binary), str(root/"input.json"), str(root/name)])))

def normalize(value):
    return " ".join(str(value).split())

texts={}
infos={}
for side in ("python", "rust"):
    pdf=root/f"{side}.pdf"
    text=subprocess.check_output(["pdftotext", "-layout", str(pdf), "-"], text=True)
    (root/f"{side}.txt").write_text(text)
    texts[side]=normalize(text)
    info=subprocess.check_output(["pdfinfo",str(pdf)],text=True)
    (root/f"{side}.pdfinfo").write_text(info)
    infos[side]={"pages":int(re.search(r"Pages:\s+(\d+)",info)[1]), "bytes":pdf.stat().st_size, "sha256":hashlib.sha256(pdf.read_bytes()).hexdigest()}

c=report["fiat_currency"]
expected=[report["title"],f"{report['workspace']} / {report['profile']}", *p._perf_summary_lines(c,report["metrics"],report["benchmark"])]
for row in report["top_movements"]:
    expected.append(" ".join([row["occurred_at"][:10],row["wallet"],p._direction_label(row["direction"]),row["asset"],p._btc(row["quantity"]),p._money(c,row["fiat_value"]),row["counterparty"]]))
for row in report["top_disposals"]:
    expected.append(" ".join([row["occurred_at"][:10],row["wallet"],p._btc(-p.decimal_value(row["quantity"])),p._money(c,row["proceeds"]),p._money(c,row["cost_basis"]),p._signed_money(c,row["gain_loss"])]))
for row in report["wallet_appendix"]:
    expected.append(" ".join([row["wallet"],row["scope"],str(row["tx_count"]),p._asset_quantity_text(row),p._money(c,row["end_market_value"])]))
for row in report["snapshot"]["wallets"]:
    expected.append(" ".join([row["wallet"],", ".join(row["assets"]),p._asset_quantity_text(row),p._money(c,row["market_value"])]))
expected += ["Data Integrity","Priced transactions 4 / 4 (100.0%)","Journals Current","Quarantines 0","Quarantine reasons None in scope","Portfolio Movement","Portfolio Composition","Period Activity","Largest disposals","Largest activity","Wallet Appendix",p._holding_age_summary(report["holding_age"])]
rl=require_reportlab("spike");rl["summary_fonts"]=register_fonts(rl)
chart_labels=[]
for drawing in [p._line_chart(rl,"Total balance over time",report["balance_history"],c),p._donut_chart(rl,"Holdings by wallet at period end",report["wallet_holdings"],c),p._bar_chart(rl,"Realized PnL per period",report["realized_pnl_periods"],c),p._bar_chart(rl,"Inflows vs outflows volume",report["flow_periods"],c,paired=True)]:
    chart_labels += [shape.text for shape in drawing.contents if hasattr(shape,"text")]
expected += chart_labels
missing={side:[value for value in expected if normalize(value) not in text] for side,text in texts.items()}
assert not any(missing.values()), missing
assert (root/"rust.pdf").read_bytes()==(root/"rust-repeat.pdf").read_bytes()
assert (root/"python.pdf").read_bytes()==(root/"python-repeat.pdf").read_bytes()
# Count differences are reported separately: repeated table headers and page furniture differ.
counts={side:Counter(text.split()) for side,text in texts.items()}
diff={"python_only":dict(counts["python"]-counts["rust"]),"rust_only":dict(counts["rust"]-counts["python"])}
result={"versions":{"typst":"0.15.1","reportlab":"4.5.0"},"outputs":infos,"semantic_strings_checked":len(expected),"chart_labels_checked":len(chart_labels),"all_present":True,"repeat_bytes_identical":True,"render_runs":runs,"python_render_ms":python_render_ms,"stripped_release_binary_bytes":binary.stat().st_size,"word_count_differences":diff}
(root/"comparison.json").write_text(json.dumps(result,indent=2)+"\n")
print(json.dumps(result,indent=2))
