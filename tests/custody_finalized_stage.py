"""Full custody oracle, including finalized-projection refusal admission."""
from kassiber.core.custody_compute import compute_custody_projection


def python_stages(snapshot):
    stages = {}
    compute_custody_projection(snapshot, stage_observer=lambda name, value: stages.setdefault(name, value))
    return stages
