"""Only one disposable item; never read Kassiber's actual credential namespace."""
import os
from pathlib import Path
import subprocess
import uuid

import secretstorage
from keyring.backends.SecretService import Keyring

service = "Kassiber R0 synthetic " + uuid.uuid4().hex
account = "synthetic-account"
binary = Path(__file__).resolve().parent / "target/debug/r0-keyring"
print("session_bus_configured:", bool(os.environ.get("DBUS_SESSION_BUS_ADDRESS")))
try:
    bus = secretstorage.dbus_init()
    available = secretstorage.check_service_availability(bus)
    print("secret_service_available:", available)
    if not available:
        raise SystemExit("SKIP: no running or activatable Secret Service; no item written")
    collection = secretstorage.get_default_collection(bus)
    if collection.is_locked():
        raise SystemExit("SKIP: default collection locked; no prompt or item written")
except secretstorage.exceptions.SecretStorageException as e:
    raise SystemExit(f"SKIP: {type(e).__name__}; no item written") from None

backend = Keyring()
try:
    backend.set_password(service, account, "Python R0 '🔑")
    items = list(collection.search_items({"service": service, "username": account}))
    assert len(items) == 1
    before_path = items[0].item_path
    before_label = items[0].get_label()
    attrs = items[0].get_attributes()
    print("synthetic item attributes:", {k: v if k not in ("service", "username") else "<synthetic>" for k, v in attrs.items()})
    assert attrs.items() >= {"service": service, "username": account, "application": "Python keyring library"}.items()
    subprocess.run([str(binary), service, account], check=True)
    assert backend.get_password(service, account) == "Rust R0 '🔐"
    after = list(collection.search_items({"service": service, "username": account}))
    assert len(after) == 1 and after[0].item_path == before_path
    assert after[0].get_attributes() == attrs
    assert after[0].get_label() == before_label
    print("PASS: Python reads Rust update; item path, label and all attributes unchanged")
finally:
    for item in collection.search_items({"service": service, "username": account}):
        item.delete()
    bus.close()
