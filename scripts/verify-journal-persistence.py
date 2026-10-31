#!/usr/bin/env python3
"""Replay private R4-12 captures against the unregistered native probe."""
import argparse
from collections import Counter
import json
from pathlib import Path
import subprocess
import sys

ROOT=Path(__file__).resolve().parents[1]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directories',type=Path,nargs='+')
    parser.add_argument('--probe',type=Path,default=ROOT/'target/debug/examples/journal-persist')
    parser.add_argument('--summary',type=Path,required=True)
    args=parser.parse_args()
    totals={};failures=[]
    for directory in args.directories:
        counts=Counter()
        files=sorted(directory.glob('*.json'))
        if not files:
            raise SystemExit(f'no persistence captures in {directory}')
        with subprocess.Popen([str(args.probe)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True,cwd=ROOT) as child:
            for path in files:
                child.stdin.write(str(path.resolve())+'\n');child.stdin.flush()
                line=child.stdout.readline()
                if not line: raise RuntimeError(f'probe exited at {path}')
                result=json.loads(line)
                counts['compared']+=1
                if result['ok']:
                    counts['matched']+=1;counts['writes']+=result['writes']
                    counts.update({f'table:{t}':1 for t in set(result['tables'])})
                else:
                    counts['failed']+=1
                    if 'actual' in result:
                        expected=json.loads(path.read_text())['expected']
                        different=[table for table in expected if result['actual'].get(table)!=expected[table]]
                        result={k:v for k,v in result.items() if k!='actual'}|dict(different_tables=different)
                    failures.append(dict(path=str(path),result=result))
            child.stdin.close();child.wait(timeout=60)
            if child.returncode:raise RuntimeError(f'probe exited {child.returncode}')
        totals[str(directory)]=dict(counts)
        print(directory,counts['matched'],'/',counts['compared'],flush=True)
    summary=dict(totals=totals,failures=failures)
    args.summary.parent.mkdir(parents=True,exist_ok=True)
    args.summary.write_text(json.dumps(summary,indent=2)+'\n')
    if failures:print(json.dumps(failures[:10],indent=2),file=sys.stderr)
    return int(bool(failures))

if __name__=='__main__':raise SystemExit(main())
