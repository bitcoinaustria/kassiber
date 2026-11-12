"""Quantity oracle stops before finalized tax projection or journal fusion."""
from kassiber.core.custody_compute import compute_custody_decisions

def python_stages(snapshot):
    class Complete(Exception):
        pass
    stages={}
    def observe(name,value):
        stages[name]=value
        if name=='quantity':raise Complete
    try:
        compute_custody_decisions(snapshot,stage_observer=observe)
    except Complete:
        return stages
    raise AssertionError('Python did not emit quantity')

def python_stage(snapshot): return python_stages(snapshot)['quantity']


def wrappers():
    import json
    from pathlib import Path
    from tests.custody_quantity_composition_vectors import outcome
    cases=json.loads(Path('tests/fixtures/custody_interpretation_wrappers.json').read_text())
    return [dict(case=c['case'],input=c['input'],**outcome(lambda:python_stage(c['input']))) for c in cases]

if __name__=='__main__':
    import json
    from pathlib import Path
    Path('tests/fixtures/custody_quantity_wrappers.json').write_text(json.dumps(wrappers(),separators=(',',':'))+'\n')
