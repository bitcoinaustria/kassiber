from contextlib import contextmanager
from pathlib import Path
import json
import socket
import socketserver
import ssl
import threading


ROOT = Path(__file__).resolve().parents[2]


@contextmanager
def transport(calls, *, tls=False, proxy=None, status=200, raw=None, wrong_name=False):
    hits, routes = [], []
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(ROOT / "crates/net/tests/fixtures/server.pem",
                           ROOT / "crates/net/tests/fixtures/server-key.pem")

    class Handler(socketserver.BaseRequestHandler):
        def handle(self):
            stream = self.request
            stream.settimeout(5)

            def exact(count):
                data = b""
                while len(data) < count:
                    part = stream.recv(count - len(data))
                    if not part:
                        raise EOFError
                    data += part
                return data

            def head():
                data = b""
                while not data.endswith(b"\r\n\r\n"):
                    if len(data) >= 65536:
                        raise ValueError("fixture header limit")
                    data += exact(1)
                return data.decode("ascii")

            try:
                if proxy == "socks5h":
                    version, count = exact(2)
                    assert version == 5 and 0 in exact(count)
                    stream.sendall(b"\x05\x00")
                    assert exact(3) == b"\x05\x01\x00"
                    kind = exact(1)[0]
                    target = exact(exact(1)[0]).decode() if kind == 3 else exact(4).hex()
                    port = int.from_bytes(exact(2), "big")
                    routes.append((target, port))
                    stream.sendall(b"\x05\x00\x00\x01\x7f\x00\x00\x01\x00\x00")
                elif proxy == "http" and tls:
                    request = head()
                    routes.append(request.splitlines()[0])
                    assert request.startswith("CONNECT ")
                    assert "macaroon" not in request.lower()
                    stream.sendall(b"HTTP/1.1 200 Connection established\r\n\r\n")
                if tls:
                    stream = context.wrap_socket(stream, server_side=True)
                request = head()
                headers = dict(line.split(":", 1) for line in request.split("\r\n")[1:] if ":" in line)
                length = next((int(v) for k, v in headers.items() if k.lower() == "content-length"), 0)
                body = exact(length) if length else b""
                index = len(hits)
                hits.append({"request": request, "body": json.loads(body) if body else None})
                response = raw if raw is not None else json.dumps(calls[min(index, len(calls) - 1)]["response"]).encode()
                stream.sendall(f"HTTP/1.1 {status} Fixture\r\nContent-Length: {len(response)}\r\nConnection: close\r\n\r\n".encode() + response)
            except (OSError, EOFError):
                pass
            finally:
                stream.close()

    class Server(socketserver.ThreadingTCPServer):
        block_on_close = True

    server = Server(("127.0.0.1", 0), Handler)
    target = socket.socket()
    target.bind(("127.0.0.1", 0))
    thread = threading.Thread(target=lambda: server.serve_forever(poll_interval=0.01))
    thread.start()
    host = "wrong-name.invalid" if wrong_name else "localhost"
    url = f"{'https' if tls else 'http'}://{host}:{server.server_address[1]}"
    backend = {"kind": "lnd", "name": "fixture", "url": url, "token": "aabb", "timeout": 2}
    if proxy:
        backend["url"] = f"{'https' if tls else 'http'}://{host if wrong_name else 'localhost'}:{target.getsockname()[1]}"
        backend["tor_proxy"] = f"{proxy}://127.0.0.1:{server.server_address[1]}"
    try:
        yield backend, hits, routes
    finally:
        server.shutdown()
        server.server_close()
        thread.join()
        target.close()
