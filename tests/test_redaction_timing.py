"""Stress deadlines measure redaction CPU, even with slow interpreter startup."""
import subprocess

import pytest

from tests import test_redaction_shared_corpus as shared


@pytest.mark.parametrize('shape', ['identifier', 'adversarial'])
def test_redaction_deadline_excludes_interpreter_startup(monkeypatch, shape):
    run = subprocess.run

    def slow_startup(command, **kwargs):
        command = list(command)
        script_index = command.index('-c') + 1
        command[script_index] = 'import time; time.sleep(2.1)\n' + command[script_index]
        return run(command, **kwargs)

    monkeypatch.setattr(subprocess, 'run', slow_startup)
    if shape == 'identifier':
        shared.test_long_identifier_redaction_is_linear('-')
    else:
        shared.test_adversarial_secret_shapes_keep_a_bounded_deadline()
