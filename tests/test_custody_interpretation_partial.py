"""Retain isolated vectors as independent witnesses after full-stage cutover."""
import json


def test_frozen_partial_vectors_and_python_text_table():
    from tests.custody_pipeline_vectors import PATH as PIPELINE_PATH, vectors as pipeline_vectors, helper_vectors as pipeline_helpers
    assert pipeline_vectors() == json.loads(PIPELINE_PATH.read_text())
    assert pipeline_helpers() == json.loads(PIPELINE_PATH.with_name("custody_pipeline_helpers.json").read_text())
    from tests.custody_consolidation_vectors import PATH as CONSOLIDATION_PATH, vectors as consolidation_vectors, helper_vectors as consolidation_helpers
    assert consolidation_vectors() == json.loads(CONSOLIDATION_PATH.read_text())
    assert consolidation_helpers() == json.loads(CONSOLIDATION_PATH.with_name("custody_consolidation_helpers.json").read_text())
    from tests.custody_ownership_vectors import PATH as OWNERSHIP_PATH, vectors as ownership_vectors, helper_vectors
    assert ownership_vectors() == json.loads(OWNERSHIP_PATH.read_text())
    assert helper_vectors() == json.loads(OWNERSHIP_PATH.with_name("custody_ownership_helpers.json").read_text())
    from tests.custody_liquid_vectors import PATH as LIQUID_PATH, vectors as liquid_vectors
    assert liquid_vectors() == json.loads(LIQUID_PATH.read_text())
    from tests.custody_graph_vectors import PATH as GRAPH_PATH, vectors as graph_vectors
    from tests.custody_snapshot import to_jsonable
    assert to_jsonable(graph_vectors(), redact_config=False) == json.loads(GRAPH_PATH.read_text())
    from tests.custody_interpretation_vectors import PATH, vectors
    from tests.custody_interpretation_text import PATH as TEXT_PATH, source
    assert vectors() == json.loads(PATH.read_text())
    assert TEXT_PATH.read_text() == source()
    from tests.custody_pair_vectors import PATH as PAIR_PATH, vectors as pair_vectors
    assert pair_vectors() == json.loads(PAIR_PATH.read_text())
    from tests.custody_transfer_vectors import PATH as TRANSFER_PATH, vectors as transfer_vectors
    assert transfer_vectors() == json.loads(TRANSFER_PATH.read_text())
    from tests.custody_fanout_vectors import PATH as FANOUT_PATH, vectors as fanout_vectors
    assert fanout_vectors() == json.loads(FANOUT_PATH.read_text())
