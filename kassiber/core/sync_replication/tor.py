"""Optional Tor onion leg for the authenticated direct-sync protocol."""

from __future__ import annotations

from pathlib import Path
from urllib.parse import urlsplit

from ...errors import AppError
from ...proxy import connect_via_socks5, is_onion_endpoint
from .lan import LanSyncResult, LanSyncServer, connect_lan


def _onion_host(value: str) -> str:
    raw = str(value or "").strip()
    parsed = urlsplit(raw if "://" in raw else f"//{raw}")
    host = (parsed.hostname or "").lower().rstrip(".")
    if not host or not host.endswith(".onion") or not is_onion_endpoint(host):
        raise AppError("Tor sync endpoint must be a v3 .onion host", code="sync_tor_endpoint_invalid")
    # v3 onion names are 56 base32 characters plus the suffix.
    label = host.removesuffix(".onion")
    if len(label) != 56 or any(char not in "abcdefghijklmnopqrstuvwxyz234567" for char in label):
        raise AppError("Tor sync endpoint must be a v3 .onion host", code="sync_tor_endpoint_invalid")
    return host


class TorOnionSyncServer(LanSyncServer):
    """Loopback listener intended for a user-controlled Tor HiddenServicePort."""

    def __init__(
        self,
        conn,
        *,
        profile_id: str,
        onion_host: str,
        onion_port: int,
        local_port: int,
    ) -> None:
        host = _onion_host(onion_host)
        if not (0 < onion_port < 65536 and 0 < local_port < 65536):
            raise AppError("Tor sync ports are invalid", code="validation")
        super().__init__(
            conn,
            profile_id=profile_id,
            bind_host="127.0.0.1",
            bind_port=local_port,
            advertise_host=host,
            advertise_port=onion_port,
            advertise_mdns=False,
        )


def connect_onion(
    conn,
    *,
    profile_id: str,
    offer_code: str,
    proxy_url: str,
    attachments_root: Path | None = None,
    timeout_seconds: float = 60.0,
) -> LanSyncResult:
    from .lan import LanPairingOffer

    offer = LanPairingOffer.decode(offer_code)
    _onion_host(offer.host)
    if not str(proxy_url or "").strip():
        raise AppError(
            "Tor sync requires a SOCKS5 proxy",
            code="network_proxy_required",
            hint="Start Tor and pass its socks5h://127.0.0.1:9050 proxy.",
        )
    return connect_lan(
        conn,
        profile_id=profile_id,
        offer_code=offer_code,
        attachments_root=attachments_root,
        timeout_seconds=timeout_seconds,
        connector=lambda host, port, timeout: connect_via_socks5(
            proxy_url, host, port, timeout
        ),
    )
