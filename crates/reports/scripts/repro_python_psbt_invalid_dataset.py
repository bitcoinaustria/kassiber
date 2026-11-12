"""Reproduce the unchanged graph's uncaught corrupt-manifest KeyError locally.

This seeds malformed stored state, not a manifest accepted by dataset import.
The native inventory checkpoint refuses the dataset source before reading it.
"""
from contextlib import closing

from gen_psbt_inventory_vectors import OVERLAY
from kassiber.cli.handlers import _report_hooks
from kassiber.core import reports


def main():
    with closing(OVERLAY.connection(OVERLAY.inventory_sql())) as conn:
        conn.execute("""INSERT INTO chain_analysis_datasets
            (id,profile_id,dataset_key,version,chain,network,visibility,status,manifest_json,created_at,updated_at)
            VALUES('pack','p','key','1','bitcoin','main','public','active','{}','fixed','fixed')""")
        try:
            reports.report_psbt_privacy(conn, None, None, _report_hooks(), psbt_text=OVERLAY.proposal())
        except KeyError as exc:
            assert exc.args == ("valid_from",)
            print("Reproduced KeyError('valid_from') from a corrupt stored dataset manifest")
        else:
            raise AssertionError("The Python reference no longer reproduces the error")


if __name__ == "__main__":
    main()
