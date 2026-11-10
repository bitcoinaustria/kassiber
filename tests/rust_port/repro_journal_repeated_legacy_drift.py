"""Reproduce Python's lineage refusal on a second evidence-only pair drift.

Run with the locked environment from the repository root. The synthetic books
stay in memory. Python product code is unchanged; the native oracle retains
this refusal rather than selecting another revision parent.
"""
from tests.rust_port.journal_legacy_refresh_oracle import vectors


def main():
    case = next(case for case in vectors() if case['name'] == 'repeated-drift-refusal')
    counts = case['result']['consolidation']
    assert counts == dict(activated=0, unchanged=0, skipped=1), counts
    issue = case['expected']['custody_authored_migration_issues']
    column = issue['columns'].index('issue_code')
    codes = [row[column] for row in issue['rows']]
    assert codes == ['custody_component_lineage_exists'], codes
    print('Second evidence-only drift: skipped=1, custody_component_lineage_exists')


if __name__ == '__main__':
    main()
