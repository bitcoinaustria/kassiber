import json
import os
from pathlib import Path
import select
import subprocess
import time


class Session:
    def __init__(self, root, env):
        self.process = subprocess.Popen(
            [str(Path(os.environ["KASSIBER_RUST_BIN"]).resolve()), "--data-root", str(root), "daemon"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env,
        )
        self.buffer = b""
        self.rows = []

    def __enter__(self):
        assert self.read()["kind"] == "daemon.ready"
        return self

    def send(self, kind, request_id, args=None):
        row = {"kind": kind, "request_id": request_id}
        if args is not None:
            row["args"] = args
        self.process.stdin.write((json.dumps(row) + "\n").encode())
        self.process.stdin.flush()

    def read(self, timeout=30):
        deadline = time.monotonic() + timeout
        while b"\n" not in self.buffer:
            remaining = deadline - time.monotonic()
            assert remaining > 0 and select.select([self.process.stdout], [], [], remaining)[0], self.rows
            data = os.read(self.process.stdout.fileno(), 65536)
            assert data, (self.rows, self.process.poll())
            self.buffer += data
        line, self.buffer = self.buffer.split(b"\n", 1)
        row = json.loads(line)
        self.rows.append(row)
        return row

    def until(self, request_id, kind=None):
        while True:
            row = self.read()
            if row.get("request_id") == request_id and (
                row["kind"] == "error" or kind is None or row["kind"] == kind
            ):
                return row

    def finish(self):
        self.send("daemon.shutdown", "stop")
        result = self.until("stop")
        assert result["kind"] == "daemon.shutdown", result
        self.process.stdin.close()
        self.process.wait(timeout=30)
        assert self.process.returncode == 0
        assert self.process.stderr.read() == b""

    def __exit__(self, *_):
        if self.process.poll() is None:
            self.process.kill()
            self.process.wait(timeout=30)
        for stream in (self.process.stdin, self.process.stdout, self.process.stderr):
            stream.close()
