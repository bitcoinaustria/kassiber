"""Pin the terminal brand boundary and the shared desktop letter shapes."""

import io
from contextlib import redirect_stdout
import json
import os
import re
from pathlib import Path
from unittest.mock import Mock, patch

import pytest

from kassiber.cli import banner
import kassiber.cli.main as cli_main


class Terminal(io.StringIO):
    encoding = "utf-8"

    def isatty(self):
        return True

    def run(self, args):
        with redirect_stdout(self), patch.object(banner.sys, "stdin", self.input):
            return cli_main.main(args)


@pytest.fixture
def terminal(monkeypatch, tmp_path):
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    monkeypatch.setenv("TERM", "xterm-256color")
    output = Terminal()
    output.input = Terminal()
    monkeypatch.setattr(banner.shutil, "get_terminal_size", lambda: os.terminal_size((100, 30)))
    # Bare help must return before storage, migration, or update handling.
    for name in ("bootstrap_runtime", "_maybe_migrate_default_state_root", "show_cached_update"):
        monkeypatch.setattr(cli_main, name, Mock(side_effect=AssertionError(name)))
    return output


def test_banner_round_trips_to_desktop_grid():
    source = (Path(__file__).resolve().parents[1] / "ui-tauri/src/components/kb/kassiberWordmarkGrid.ts").read_text()
    grid = re.findall(r'^  "([.#]+)",$', source, re.M)
    assert tuple(grid) == banner.WORDMARK_GRID
    blocks = {" ": (".", "."), "▀": ("#", "."), "▄": (".", "#"), "█": ("#", "#")}
    decoded = []
    for line in banner.render_banner().splitlines():
        cells = [blocks[cell] for cell in line.ljust(len(grid[0]))]
        decoded.extend(("".join(c[0] for c in cells), "".join(c[1] for c in cells)))
    assert decoded == grid


def test_bare_interactive_cli_shows_banner_and_help(terminal):
    assert terminal.run([]) == 0
    assert terminal.getvalue().startswith(banner.render_banner() + "\n\n")
    assert "usage: kassiber" in terminal.getvalue()


@pytest.mark.parametrize("condition", ("stdin", "stdout", "dumb", "narrow", "encoding"))
def test_incapable_terminal_keeps_missing_command_error(terminal, monkeypatch, condition):
    if condition in {"stdin", "stdout"}:
        monkeypatch.setattr(terminal.input if condition == "stdin" else terminal, "isatty", lambda: False)
    elif condition == "dumb":
        monkeypatch.setenv("TERM", "dumb")
    elif condition == "narrow":
        monkeypatch.setattr(banner.shutil, "get_terminal_size", lambda: os.terminal_size((79, 30)))
    else:
        monkeypatch.setattr(Terminal, "encoding", "ascii")
    with pytest.raises(SystemExit) as exc:
        terminal.run([])
    assert exc.value.code == 2
    assert not terminal.getvalue()


def test_explicit_help_has_no_banner(terminal):
    with pytest.raises(SystemExit) as exc:
        terminal.run(["--help"])
    assert exc.value.code == 0
    assert terminal.getvalue().startswith("usage:")


def test_machine_missing_command_remains_json_even_on_tty(terminal):
    assert terminal.run(["--machine"]) == 2
    assert json.loads(terminal.getvalue())["error"]["code"] == "usage_error"
