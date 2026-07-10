"""Explicit LAN fast path: SPAKE2, pinned device keys, and rotating mDNS."""

from __future__ import annotations

import base64
from dataclasses import asdict, dataclass
import hashlib
import hmac
import json
import os
from pathlib import Path
import secrets
import socket
import sqlite3
import struct
import time
from typing import Any, Callable, Mapping
import uuid

from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.hkdf import HKDF
from cryptography.hazmat.primitives import hashes
from spake2 import SPAKE2_A, SPAKE2_B

from ...errors import AppError
from .bundle import MAX_BUNDLE_BYTES, build_bundle
from .crypto import canonical_json_bytes, decode_secret, hmac_identifier, sha256_hex
from .identity import connection_is_encrypted
from .merge import import_bundle


LAN_SCHEMA_VERSION = 1
LAN_SERVICE_TYPE = "_kassiber-sync._tcp.local."
LAN_OFFER_TTL_SECONDS = 10 * 60
MAX_HANDSHAKE_FRAME_BYTES = 256 * 1024
MAX_DATA_FRAME_BYTES = MAX_BUNDLE_BYTES * 2
_SHORT_CODE_ALPHABET = "23456789ABCDEFGHJKLMNPQRSTUVWXYZ"


@dataclass(frozen=True)
class LanPairingOffer:
    schema_version: int
    pairing_id: str
    code: str
    host: str
    port: int
    instance_name: str
    server_device_pin: str
    expires_at_unix: int

    def encode(self) -> str:
        return base64.urlsafe_b64encode(canonical_json_bytes(asdict(self))).decode("ascii")

    @classmethod
    def decode(cls, value: str) -> "LanPairingOffer":
        try:
            payload = json.loads(base64.urlsafe_b64decode(value.encode("ascii")))
            offer = cls(**payload)
        except Exception as exc:
            raise AppError("LAN pairing offer is invalid", code="sync_lan_offer_invalid") from exc
        if (
            offer.schema_version != LAN_SCHEMA_VERSION
            or not offer.pairing_id
            or not offer.code
            or not offer.host
            or not (0 < offer.port < 65536)
            or offer.expires_at_unix < int(time.time())
        ):
            raise AppError("LAN pairing offer is invalid or expired", code="sync_lan_offer_invalid")
        return offer


@dataclass(frozen=True)
class LanSyncResult:
    peer_device_id: str
    peer_device_label: str
    sent_bundle_hash: str | None
    received_bundle_hash: str | None
    sent_events: int
    applied_events: int
    duplicate_events: int
    conflicts_created: int


def _active_book(conn, profile_id: str):
    if not connection_is_encrypted(conn):
        raise AppError(
            "LAN sync requires an unlocked encrypted database",
            code="sync_requires_encrypted_database",
        )
    book = conn.execute(
        "SELECT * FROM sync_books WHERE profile_id = ? AND enabled = 1",
        (profile_id,),
    ).fetchone()
    if not book:
        raise AppError("sync is disabled", code="sync_disabled")
    return book


def _device_pin(recipient_public_key: str) -> str:
    return sha256_hex(f"kassiber-lan-device\x00{recipient_public_key}".encode("utf-8"))


def _local_device(conn, book):
    row = conn.execute(
        "SELECT * FROM sync_devices WHERE id = ? AND profile_id = ? AND revoked_at IS NULL",
        (book["local_device_id"], book["profile_id"]),
    ).fetchone()
    if not row:
        raise AppError("local device identity is unavailable", code="sync_identity_incomplete")
    return row


def _device_by_pin(conn, *, profile_id: str, pin: str):
    for row in conn.execute(
        "SELECT * FROM sync_devices WHERE profile_id = ? AND revoked_at IS NULL",
        (profile_id,),
    ).fetchall():
        if hmac.compare_digest(_device_pin(row["recipient_public_key"]), pin):
            member = conn.execute(
                "SELECT * FROM sync_members WHERE id = ? AND revoked_at IS NULL",
                (row["member_id"],),
            ).fetchone()
            if member:
                return row
    raise AppError("LAN peer device key is not an active pinned recipient", code="sync_lan_peer_untrusted")


def _short_code() -> str:
    raw = "".join(secrets.choice(_SHORT_CODE_ALPHABET) for _ in range(12))
    return f"{raw[:4]}-{raw[4:8]}-{raw[8:]}"


def _rotating_instance_name(book, pairing_id: str, *, now: int | None = None) -> str:
    bucket = int(now or time.time()) // (10 * 60)
    key = decode_secret(book["hmac_key_b64"])
    opaque = hmac_identifier(key, "lan-mdns", f"{bucket}:{pairing_id}")[:20]
    return f"ks-{opaque}"


def _lan_address() -> str:
    probe = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        probe.connect(("192.0.2.1", 9))
        address = probe.getsockname()[0]
        return address if address and address != "0.0.0.0" else "127.0.0.1"
    except OSError:
        return "127.0.0.1"
    finally:
        probe.close()


def _send_frame(sock: socket.socket, payload: bytes, *, maximum: int) -> None:
    if len(payload) > maximum:
        raise AppError("LAN sync frame is too large", code="sync_lan_frame_invalid")
    sock.sendall(struct.pack("!I", len(payload)) + payload)


def _recv_exact(sock: socket.socket, size: int) -> bytes:
    output = bytearray()
    while len(output) < size:
        chunk = sock.recv(size - len(output))
        if not chunk:
            raise AppError("LAN peer closed the connection", code="sync_lan_connection_closed")
        output.extend(chunk)
    return bytes(output)


def _recv_frame(sock: socket.socket, *, maximum: int) -> bytes:
    size = struct.unpack("!I", _recv_exact(sock, 4))[0]
    if size <= 0 or size > maximum:
        raise AppError("LAN sync frame length is invalid", code="sync_lan_frame_invalid")
    return _recv_exact(sock, size)


def _send_json(sock: socket.socket, payload: Mapping[str, Any]) -> None:
    _send_frame(sock, canonical_json_bytes(payload), maximum=MAX_HANDSHAKE_FRAME_BYTES)


def _recv_json(sock: socket.socket) -> dict[str, Any]:
    try:
        payload = json.loads(_recv_frame(sock, maximum=MAX_HANDSHAKE_FRAME_BYTES))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise AppError("LAN handshake is invalid", code="sync_lan_handshake_invalid") from exc
    if not isinstance(payload, dict):
        raise AppError("LAN handshake is invalid", code="sync_lan_handshake_invalid")
    return payload


def _session_key(shared: bytes, transcript: bytes) -> bytes:
    return HKDF(
        algorithm=hashes.SHA256(),
        length=32,
        salt=hashlib.sha256(transcript).digest(),
        info=b"kassiber-lan-sync-v1",
    ).derive(shared)


def _confirmation(key: bytes, side: bytes, transcript: bytes) -> str:
    return base64.b64encode(hmac.new(key, side + b"\x00" + transcript, hashlib.sha256).digest()).decode("ascii")


def _seal(key: bytes, payload: Mapping[str, Any], *, aad: bytes) -> bytes:
    nonce = os.urandom(12)
    return nonce + AESGCM(key).encrypt(nonce, canonical_json_bytes(payload), aad)


def _open(key: bytes, payload: bytes, *, aad: bytes) -> dict[str, Any]:
    if len(payload) < 29:
        raise AppError("LAN encrypted frame is invalid", code="sync_lan_handshake_invalid")
    try:
        opened = AESGCM(key).decrypt(payload[:12], payload[12:], aad)
        decoded = json.loads(opened)
    except Exception as exc:
        raise AppError("LAN encrypted frame authentication failed", code="sync_lan_handshake_invalid") from exc
    if not isinstance(decoded, dict):
        raise AppError("LAN encrypted frame is invalid", code="sync_lan_handshake_invalid")
    return decoded


def _bundle_payload(result) -> dict[str, Any]:
    if result is None:
        return {"bundle": None, "bundle_hash": None, "event_count": 0}
    return {
        "bundle": base64.b64encode(result.ciphertext).decode("ascii"),
        "bundle_hash": result.bundle_hash,
        "event_count": result.event_count,
    }


def _decode_bundle(payload: Mapping[str, Any]) -> bytes | None:
    value = payload.get("bundle")
    if value is None:
        return None
    if not isinstance(value, str):
        raise AppError("LAN bundle payload is invalid", code="sync_lan_frame_invalid")
    try:
        decoded = base64.b64decode(value, validate=True)
    except ValueError as exc:
        raise AppError("LAN bundle payload is invalid", code="sync_lan_frame_invalid") from exc
    if sha256_hex(decoded) != payload.get("bundle_hash"):
        raise AppError("LAN bundle hash is invalid", code="sync_lan_frame_invalid")
    return decoded


class MdnsAdvertisement:
    def __init__(self, *, instance_name: str, address: str, port: int, pairing_id: str) -> None:
        from zeroconf import ServiceInfo, Zeroconf

        self._zeroconf = Zeroconf()
        self.info = ServiceInfo(
            LAN_SERVICE_TYPE,
            f"{instance_name}.{LAN_SERVICE_TYPE}",
            addresses=[socket.inet_aton(address)],
            port=port,
            properties={"v": b"1", "pairing": pairing_id.encode("ascii")},
            server=f"{instance_name}.local.",
        )
        self._zeroconf.register_service(self.info, allow_name_change=True)

    def close(self) -> None:
        self._zeroconf.unregister_service(self.info)
        self._zeroconf.close()


def discover_lan_services(*, timeout_seconds: float = 1.5) -> list[dict[str, Any]]:
    from zeroconf import ServiceBrowser, ServiceListener, Zeroconf

    results: dict[str, dict[str, Any]] = {}

    class Listener(ServiceListener):
        def add_service(self, zc, service_type, name):
            info = zc.get_service_info(service_type, name, timeout=1000)
            if not info:
                return
            addresses = info.parsed_scoped_addresses()
            results[name] = {
                "instance_name": name.removesuffix("." + LAN_SERVICE_TYPE),
                "host": addresses[0] if addresses else None,
                "port": info.port,
                "pairing_id": (info.properties.get(b"pairing") or b"").decode("ascii", "ignore"),
            }

        def update_service(self, zc, service_type, name):
            self.add_service(zc, service_type, name)

        def remove_service(self, _zc, _service_type, name):
            results.pop(name, None)

    zc = Zeroconf()
    browser = ServiceBrowser(zc, LAN_SERVICE_TYPE, Listener())
    try:
        time.sleep(max(0.05, timeout_seconds))
        return sorted(results.values(), key=lambda item: item["instance_name"])
    finally:
        browser.cancel()
        zc.close()


class LanSyncServer:
    """Single-use explicit listener. Construction is the opt-in bind point."""

    def __init__(
        self,
        conn,
        *,
        profile_id: str,
        bind_host: str = "0.0.0.0",
        bind_port: int = 0,
        advertise_host: str | None = None,
        advertise_port: int | None = None,
        advertise_mdns: bool = True,
    ) -> None:
        book = _active_book(conn, profile_id)
        device = _local_device(conn, book)
        self.profile_id = profile_id
        self.book_id = book["book_id"]
        self._socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._socket.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self._socket.bind((bind_host, bind_port))
        self._socket.listen(1)
        self._closed = False
        pairing_id = str(uuid.uuid4())
        host = advertise_host or (bind_host if bind_host not in {"0.0.0.0", "::"} else _lan_address())
        self.offer = LanPairingOffer(
            schema_version=LAN_SCHEMA_VERSION,
            pairing_id=pairing_id,
            code=_short_code(),
            host=host,
            port=int(advertise_port or self._socket.getsockname()[1]),
            instance_name=_rotating_instance_name(book, pairing_id),
            server_device_pin=_device_pin(device["recipient_public_key"]),
            expires_at_unix=int(time.time()) + LAN_OFFER_TTL_SECONDS,
        )
        self._mdns = (
            MdnsAdvertisement(
                instance_name=self.offer.instance_name,
                address=host,
                port=self.offer.port,
                pairing_id=pairing_id,
            )
            if advertise_mdns
            else None
        )

    @property
    def listening(self) -> bool:
        return not self._closed and self._socket.fileno() >= 0

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        if self._mdns is not None:
            self._mdns.close()
        self._socket.close()

    def serve_once(
        self,
        conn,
        *,
        attachments_root: Path | None = None,
        timeout_seconds: float = 60.0,
    ) -> LanSyncResult:
        book = _active_book(conn, self.profile_id)
        if book["book_id"] != self.book_id or int(time.time()) > self.offer.expires_at_unix:
            self.close()
            raise AppError("LAN pairing offer expired or changed books", code="sync_lan_offer_invalid")
        deadline = time.monotonic() + timeout_seconds
        self._socket.settimeout(min(1.0, timeout_seconds))
        try:
            while True:
                try:
                    peer_socket, _address = self._socket.accept()
                    break
                except socket.timeout as exc:
                    # Re-check the DB between accepts. Disabling sync from
                    # another process closes this explicit listener promptly.
                    _active_book(conn, self.profile_id)
                    if time.monotonic() >= deadline:
                        raise AppError(
                            "LAN pairing offer timed out",
                            code="sync_lan_timeout",
                            retryable=True,
                        ) from exc
            conn.execute("SAVEPOINT sync_lan_server")
            with peer_socket:
                peer_socket.settimeout(max(0.1, deadline - time.monotonic()))
                result = self._handle_peer(conn, peer_socket, book, attachments_root)
            conn.execute("RELEASE SAVEPOINT sync_lan_server")
            return result
        except Exception:
            if conn.in_transaction:
                try:
                    conn.execute("ROLLBACK TO SAVEPOINT sync_lan_server")
                    conn.execute("RELEASE SAVEPOINT sync_lan_server")
                except sqlite3.DatabaseError:
                    pass
            raise
        finally:
            self.close()

    def _handle_peer(self, conn, peer_socket, book, attachments_root) -> LanSyncResult:
        hello = _recv_json(peer_socket)
        if hello.get("schema_version") != LAN_SCHEMA_VERSION or hello.get("pairing_id") != self.offer.pairing_id:
            raise AppError("LAN pairing id is invalid", code="sync_lan_handshake_invalid")
        try:
            message_a = base64.b64decode(str(hello["spake_message"]), validate=True)
        except Exception as exc:
            raise AppError("LAN SPAKE2 message is invalid", code="sync_lan_handshake_invalid") from exc
        spake = SPAKE2_B(
            self.offer.code.encode("ascii"),
            idA=b"kassiber-lan-initiator-v1",
            idB=b"kassiber-lan-responder-v1",
        )
        message_b = spake.start()
        shared = spake.finish(message_a)
        transcript = canonical_json_bytes(
            {
                "pairing_id": self.offer.pairing_id,
                "message_a": base64.b64encode(message_a).decode("ascii"),
                "message_b": base64.b64encode(message_b).decode("ascii"),
            }
        )
        key = _session_key(shared, transcript)
        _send_json(
            peer_socket,
            {
                "schema_version": LAN_SCHEMA_VERSION,
                "spake_message": base64.b64encode(message_b).decode("ascii"),
                "confirmation": _confirmation(key, b"server", transcript),
            },
        )
        client_frame = _open(
            key,
            _recv_frame(peer_socket, maximum=MAX_DATA_FRAME_BYTES),
            aad=b"client\x00" + transcript,
        )
        if client_frame.get("confirmation") != _confirmation(key, b"client", transcript):
            raise AppError("LAN client key confirmation failed", code="sync_lan_handshake_invalid")
        peer = _device_by_pin(
            conn,
            profile_id=self.profile_id,
            pin=str(client_frame.get("device_pin") or ""),
        )
        incoming = _decode_bundle(client_frame)
        imported = (
            import_bundle(
                conn,
                profile_id=self.profile_id,
                ciphertext=incoming,
                attachments_root=attachments_root,
            )
            if incoming is not None
            else None
        )
        outgoing = build_bundle(
            conn,
            profile_id=self.profile_id,
            attachments_root=attachments_root,
        )
        local_device = _local_device(conn, book)
        response = {
            "confirmation": _confirmation(key, b"server-data", transcript),
            "device_id": local_device["id"],
            "device_label": local_device["label"],
            "device_pin": _device_pin(local_device["recipient_public_key"]),
            **_bundle_payload(outgoing),
        }
        _send_frame(
            peer_socket,
            _seal(key, response, aad=b"server\x00" + transcript),
            maximum=MAX_DATA_FRAME_BYTES,
        )
        return LanSyncResult(
            peer_device_id=peer["id"],
            peer_device_label=peer["label"],
            sent_bundle_hash=outgoing.bundle_hash if outgoing else None,
            received_bundle_hash=sha256_hex(incoming) if incoming else None,
            sent_events=outgoing.event_count if outgoing else 0,
            applied_events=imported.applied_events if imported else 0,
            duplicate_events=imported.duplicate_events if imported else 0,
            conflicts_created=imported.conflicts_created if imported else 0,
        )


def connect_lan(
    conn,
    *,
    profile_id: str,
    offer_code: str,
    attachments_root: Path | None = None,
    timeout_seconds: float = 30.0,
    connector: Callable[[str, int, float], socket.socket] | None = None,
) -> LanSyncResult:
    book = _active_book(conn, profile_id)
    offer = LanPairingOffer.decode(offer_code)
    local_device = _local_device(conn, book)
    conn.execute("SAVEPOINT sync_lan_client")
    try:
        outgoing = build_bundle(
            conn,
            profile_id=profile_id,
            attachments_root=attachments_root,
        )
        connection = (
            connector(offer.host, offer.port, timeout_seconds)
            if connector is not None
            else socket.create_connection((offer.host, offer.port), timeout=timeout_seconds)
        )
        with connection as peer_socket:
            peer_socket.settimeout(timeout_seconds)
            spake = SPAKE2_A(
                offer.code.encode("ascii"),
                idA=b"kassiber-lan-initiator-v1",
                idB=b"kassiber-lan-responder-v1",
            )
            message_a = spake.start()
            _send_json(
                peer_socket,
                {
                    "schema_version": LAN_SCHEMA_VERSION,
                    "pairing_id": offer.pairing_id,
                    "spake_message": base64.b64encode(message_a).decode("ascii"),
                },
            )
            server_hello = _recv_json(peer_socket)
            try:
                message_b = base64.b64decode(str(server_hello["spake_message"]), validate=True)
            except Exception as exc:
                raise AppError("LAN SPAKE2 response is invalid", code="sync_lan_handshake_invalid") from exc
            shared = spake.finish(message_b)
            transcript = canonical_json_bytes(
                {
                    "pairing_id": offer.pairing_id,
                    "message_a": base64.b64encode(message_a).decode("ascii"),
                    "message_b": base64.b64encode(message_b).decode("ascii"),
                }
            )
            key = _session_key(shared, transcript)
            if server_hello.get("confirmation") != _confirmation(key, b"server", transcript):
                raise AppError("LAN server key confirmation failed", code="sync_lan_handshake_invalid")
            request = {
                "confirmation": _confirmation(key, b"client", transcript),
                "device_id": local_device["id"],
                "device_label": local_device["label"],
                "device_pin": _device_pin(local_device["recipient_public_key"]),
                **_bundle_payload(outgoing),
            }
            _send_frame(
                peer_socket,
                _seal(key, request, aad=b"client\x00" + transcript),
                maximum=MAX_DATA_FRAME_BYTES,
            )
            server_frame = _open(
                key,
                _recv_frame(peer_socket, maximum=MAX_DATA_FRAME_BYTES),
                aad=b"server\x00" + transcript,
            )
        if server_frame.get("confirmation") != _confirmation(key, b"server-data", transcript):
            raise AppError("LAN server data confirmation failed", code="sync_lan_handshake_invalid")
        if not hmac.compare_digest(str(server_frame.get("device_pin") or ""), offer.server_device_pin):
            raise AppError("LAN server device pin changed", code="sync_lan_peer_untrusted")
        peer = _device_by_pin(
            conn,
            profile_id=profile_id,
            pin=offer.server_device_pin,
        )
        incoming = _decode_bundle(server_frame)
        imported = (
            import_bundle(
                conn,
                profile_id=profile_id,
                ciphertext=incoming,
                attachments_root=attachments_root,
            )
            if incoming is not None
            else None
        )
        conn.execute("RELEASE SAVEPOINT sync_lan_client")
        return LanSyncResult(
            peer_device_id=peer["id"],
            peer_device_label=peer["label"],
            sent_bundle_hash=outgoing.bundle_hash if outgoing else None,
            received_bundle_hash=sha256_hex(incoming) if incoming else None,
            sent_events=outgoing.event_count if outgoing else 0,
            applied_events=imported.applied_events if imported else 0,
            duplicate_events=imported.duplicate_events if imported else 0,
            conflicts_created=imported.conflicts_created if imported else 0,
        )
    except Exception:
        conn.execute("ROLLBACK TO SAVEPOINT sync_lan_client")
        conn.execute("RELEASE SAVEPOINT sync_lan_client")
        raise
