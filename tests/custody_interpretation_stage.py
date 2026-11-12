"""Full interpretation oracle; stops before unported gap/quantity stages."""
from kassiber.core.custody_compute import compute_custody_decisions


def python_stage(snapshot):
    class Complete(Exception):
        pass
    stages = {}
    def observe(name, value):
        stages[name] = value
        if name == "interpretation":
            raise Complete
    try:
        compute_custody_decisions(snapshot, stage_observer=observe)
    except Complete:
        return stages["interpretation"]
    raise AssertionError("Python did not emit interpretation")


def suites():
    from tests.custody_snapshot_generator import SCENARIOS, snapshot, COMPONENT_SCENARIOS, component_snapshot
    from tests.custody_interpretation_generator import SAMOURAI_SCENARIOS, samourai_snapshot, RECORDED_SCENARIOS, recorded_snapshot
    from tests.custody_fanout_generator import FANOUT_SCENARIOS, fanout_snapshot
    from tests.custody_graph_generator import GRAPH_SCENARIOS, graph_snapshot
    from tests.custody_pipeline_generator import PIPELINE_SCENARIOS, pipeline_snapshot
    from tests.custody_safety_generator import SAFETY_SCENARIOS, safety_snapshot
    from tests.custody_channel_generator import CHANNEL_SCENARIOS, channel_snapshot
    from tests.custody_native_generator import NATIVE_SCENARIOS, native_snapshot
    from tests.custody_reconciliation_generator import SCENARIOS as RS, snapshot as reconciliation_snapshot
    return (
        ("evidence", SCENARIOS, snapshot), ("components", COMPONENT_SCENARIOS, component_snapshot),
        ("samourai", SAMOURAI_SCENARIOS, samourai_snapshot), ("recorded", RECORDED_SCENARIOS, recorded_snapshot),
        ("fanout", FANOUT_SCENARIOS, fanout_snapshot), ("graph", GRAPH_SCENARIOS, graph_snapshot),
        ("pipeline", PIPELINE_SCENARIOS, pipeline_snapshot), ("safety", SAFETY_SCENARIOS, safety_snapshot),
        ("channels", CHANNEL_SCENARIOS, channel_snapshot), ("native", NATIVE_SCENARIOS, native_snapshot),
        ("reconciliation", RS, reconciliation_snapshot),
    )
