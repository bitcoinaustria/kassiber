from pathlib import Path
import json
import sys
sys.path.insert(0,str(Path(__file__).resolve().parents[3]))
from kassiber.core.reports import privacy_mirror_table_rows
root=Path(__file__).resolve().parents[3]
corpus=json.loads((root/'crates/wallets/tests/data/analysis_mirror.json').read_text())
cases=[]
for case in corpus['cases'][:12:2]:
    expected=case['expected']
    expected['scope']={'workspace':'active','profile':'active','workspace_selected':True,'profile_selected':True}
    cases.append({'name':case['name'],'sql':corpus['books'][case['book']],'expected':expected,'table':privacy_mirror_table_rows(expected)})
(root/'crates/reports/tests/data/privacy_mirror.json').write_text(json.dumps(cases,indent=2)+'\n')
