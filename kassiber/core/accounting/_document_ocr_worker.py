"""Isolated local OCR worker: bounded stdin/stdout, no book/files/provider access.

Optional Tesseract uses its installed language data. PDF rasterization uses
Poppler's single-page stdout mode. No downloaded models or plaintext scratch
files are created. This is a resource boundary, not an OS network sandbox.
"""
from __future__ import annotations

import json
import re
import struct
import subprocess
import sys
import threading

MAX_SOURCE = 20 * 1024**2
MAX_IMAGE = 24 * 1024**2
MAX_TEXT = 2 * 1024**2
MAX_PIXELS = 12_000_000


def image_dimensions(content: bytes) -> tuple[int, int]:
    """Validate supported raster headers before a native decoder allocates pixels."""
    if content.startswith(b"\x89PNG\r\n\x1a\n") and len(content) >= 33 and content[12:16] == b"IHDR" and content[8:12] == b"\0\0\0\r":
        width, height = struct.unpack(">II", content[16:24])
    elif content.startswith(b"\xff\xd8"):
        offset = 2
        while offset < len(content):
            if content[offset] != 255:
                raise ValueError("image_header")
            while offset < len(content) and content[offset] == 255:
                offset += 1
            if offset >= len(content):
                raise ValueError("image_header")
            marker = content[offset]
            offset += 1
            if marker in {0xD8, 0xD9, 0xDA}:
                raise ValueError("image_header")
            if marker == 0x01 or 0xD0 <= marker <= 0xD7:
                continue
            if offset + 2 > len(content):
                raise ValueError("image_header")
            size = int.from_bytes(content[offset:offset + 2], "big")
            if size < 2 or offset + size > len(content):
                raise ValueError("image_header")
            if marker in {0xC0, 0xC1, 0xC2}:
                if size < 8:
                    raise ValueError("image_header")
                height, width = struct.unpack(">HH", content[offset + 3:offset + 7])
                break
            offset += size
        else:
            raise ValueError("image_header")
    else:
        raise ValueError("image_header")
    if not 1 <= width <= 10_000 or not 1 <= height <= 10_000 or width * height > MAX_PIXELS:
        raise ValueError("image_dimensions")
    return width, height


def bounded_run(args: list[str], content: bytes, limit: int, *, timeout: float = 20, merge_errors=False) -> bytes:
    process = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT if merge_errors else subprocess.DEVNULL)
    timer = threading.Timer(timeout, process.kill)
    timer.daemon = True
    timer.start()
    def feed():
        try:
            process.stdin.write(content)
            process.stdin.close()
        except (BrokenPipeError, OSError):
            pass
    writer = threading.Thread(target=feed, daemon=True)
    writer.start()
    try:
        output = process.stdout.read(limit + 1)
        if len(output) > limit:
            raise ValueError("output_budget")
        if process.wait(timeout=2):
            raise ValueError("local_process_failed")
        return output
    finally:
        timer.cancel()
        if process.poll() is None:
            process.kill()
        process.wait()
        process.stdout.close()
        writer.join(timeout=1)


def main():
    try:
        import resource
        for key, value in ((resource.RLIMIT_CPU, 20), (resource.RLIMIT_FSIZE, 0), (resource.RLIMIT_NOFILE, 32)):
            resource.setrlimit(key, (value, value))
        if sys.platform != "darwin":
            resource.setrlimit(resource.RLIMIT_AS, (768 * 1024**2, 768 * 1024**2))
    except (ImportError, OSError, ValueError):
        pass
    try:
        tesseract, renderer, media, language, selection = sys.argv[1:]
        pages = json.loads(selection)
        if not isinstance(pages, list) or not 1 <= len(pages) <= 8 or any(type(p) is not int or not 1 <= p <= 2000 for p in pages) or len(set(pages)) != len(pages):
            return 1
        if not re.fullmatch(r"[A-Za-z0-9_]{1,32}(?:\+[A-Za-z0-9_]{1,32}){0,2}", language):
            return 1
        content = sys.stdin.buffer.read(MAX_SOURCE + 1)
        if not content or len(content) > MAX_SOURCE:
            return 1
        if media == "application/pdf":
            if not content.startswith(b"%PDF-") or not renderer:
                return 1
        elif media in {"image/png", "image/jpeg"}:
            if pages != [1]:
                return 1
            image_dimensions(content)
        else:
            return 1
        version = bounded_run([tesseract, "--version"], b"", 16_384, timeout=3, merge_errors=True).decode("utf-8", "replace").splitlines()[0][:200]
        languages = bounded_run([tesseract, "--list-langs"], b"", 16_384, timeout=3, merge_errors=True).decode("utf-8", "replace").splitlines()
        if not set(language.split("+")) <= {line.strip() for line in languages}:
            sys.stdout.write(json.dumps({"error": "accounting_ocr_language_unavailable"}))
            return 0
        raster_version = ""
        if renderer:
            raster_version = bounded_run([renderer, "-v"], b"", 4096, timeout=3, merge_errors=True).decode("utf-8", "replace").splitlines()[0][:200]
        result = [""] * max(pages)
        remaining = MAX_TEXT
        for page in sorted(pages):
            image = bounded_run([renderer, "-f", str(page), "-l", str(page), "-singlefile", "-scale-to", "2400", "-png", "-"], content, MAX_IMAGE) if media == "application/pdf" else content
            image_dimensions(image)
            output = bounded_run([tesseract, "stdin", "stdout", "-l", language,
                "--psm", "3", "-c", "tessedit_write_images=0", "-c", "stream_filelist=0"], image, remaining, timeout=20)
            remaining -= len(output)
            result[page - 1] = output.decode("utf-8").rstrip("\f\n")
        sys.stdout.write(json.dumps({"pages": result,
            "version": f"{version}; {raster_version}; lang={language}; pages={','.join(map(str, sorted(pages)))}; scale=2400"}))
        return 0
    except (OSError, ValueError, IndexError, subprocess.SubprocessError):
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
