"""Freeze SVG font metrics and finding hints from the locked Python oracle."""
import argparse
import json
from pathlib import Path

from kassiber._pdf_common import require_reportlab, register_fonts
from kassiber.core.source_funds_hints import _FINDING_HINTS

DESTINATION = Path(__file__).resolve().parents[1]/'src'


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--write',action='store_true')
    args=parser.parse_args()
    reportlab=require_reportlab('native source-funds SVG oracle')
    fonts=register_fonts(reportlab)
    assert fonts==dict(regular='Helvetica',bold='Helvetica-Bold',mono='Courier'),fonts
    metrics={}
    for key,font in fonts.items():
        def width(codepoint):
            return reportlab['pdfmetrics'].stringWidth(chr(codepoint),font,1000)
        default=width(0x10ffff)
        metrics[key]=dict(default=default,widths={str(i):width(i) for i in range(0x10000) if width(i)!=default})
    for filename,data in [('source_funds_font_metrics.json',metrics),('source_funds_hints.json',_FINDING_HINTS)]:
        generated=json.dumps(data,separators=(',',':'))+'\n'
        path=DESTINATION/filename
        if args.write:
            path.write_text(generated)
        else:
            assert path.read_text()==generated,filename
    print('Locked ReportLab metrics and Python finding hints verified')


if __name__=='__main__':
    main()
