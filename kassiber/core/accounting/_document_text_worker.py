"""Trusted isolated stdin/stdout PDF text worker; no database access."""
import json
import subprocess
import sys


def main():
    try:
        import resource
        for key, value in ((resource.RLIMIT_CPU, 15), (resource.RLIMIT_FSIZE, 0),
                           (resource.RLIMIT_NOFILE, 32)):
            resource.setrlimit(key, (value, value))
        # macOS does not reliably support an address-space cap. CPU/time/output
        # limits still apply there; never claim OS-level memory isolation.
        if sys.platform != "darwin":
            resource.setrlimit(resource.RLIMIT_AS, (384 * 1024**2, 384 * 1024**2))
    except (ImportError, OSError, ValueError):
        pass
    content = sys.stdin.buffer.read(20 * 1024**2 + 1)
    if not content.startswith(b"%PDF-") or len(content) > 20 * 1024**2:
        return 1
    executable = sys.argv[1]
    process = None
    try:
        version = subprocess.run([executable, "-v"], stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=3, check=True).stdout[:256].decode("utf-8", "replace").splitlines()[0]
        # stdout is read incrementally, never accumulated beyond the text cap.
        import threading
        process = subprocess.Popen([executable, "-enc", "UTF-8", "-layout", "-", "-"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        def feed():
            try:
                process.stdin.write(content)
                process.stdin.close()
            except (BrokenPipeError, OSError):
                pass
        writer = threading.Thread(target=feed, daemon=True)
        writer.start()
        output = process.stdout.read(2 * 1024**2 + 1)
        if len(output) > 2 * 1024**2:
            process.kill()
            process.wait()
            return 1
        if process.wait(timeout=20):
            return 1
        pages = output.decode("utf-8").split("\f")
        if pages and not pages[-1].strip():
            pages.pop()
        if not 1 <= len(pages) <= 2000:
            return 1
        sys.stdout.write(json.dumps({"pages": pages, "version": version}))
        return 0
    except (OSError, ValueError, subprocess.SubprocessError):
        return 1
    finally:
        if process is not None:
            if process.poll() is None:
                process.kill()
            process.wait()
            process.stdout.close()


if __name__ == "__main__":
    raise SystemExit(main())
