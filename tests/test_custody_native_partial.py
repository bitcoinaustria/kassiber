"""Retained independent HTLC helper vectors; generated parity uses full stages."""
import json


def test_native_frozen_helpers():
    from tests.custody_native_vectors import PATH,vectors
    assert json.loads(PATH.read_text())==vectors()
