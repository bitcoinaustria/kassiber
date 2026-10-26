"""Exercise the locked Python Windows backend against an in-memory WinCred stand-in.

This pins Python's target-name and encoding algorithm on Linux; it does NOT
claim to test Credential Manager, DPAPI, persistence, or Windows ACLs.
"""
import json
from types import SimpleNamespace
from unittest.mock import patch
from keyring.backends import Windows

class NotFound(Exception):
    winerror = 1168
    funcname = "CredRead"

vault = {}
def write(record, flags):
    record = dict(record)
    record["CredentialBlob"] = record["CredentialBlob"].encode("utf-16-le")
    vault[record["TargetName"]] = record

def read(*, Type, TargetName):
    if TargetName not in vault:
        raise NotFound()
    return vault[TargetName]

api = SimpleNamespace(CRED_TYPE_GENERIC=1, CRED_PERSIST_ENTERPRISE=3, CredRead=read, CredWrite=write)
with patch.object(Windows,"win32cred",api,create=True),patch.object(Windows,"pywintypes",SimpleNamespace(error=NotFound),create=True):
    backend=Windows.WinVaultKeyring()
    service="Kassiber R0 synthetic Windows"
    backend.set_password(service,"account-a","R0 '🔑")
    assert set(vault)=={service}
    backend.set_password(service,"account-b","R0 B")
    assert set(vault)=={service,"account-a@"+service}
    assert backend.get_password(service,"account-a")=="R0 '🔑"
    assert backend.get_password(service,"account-b")=="R0 B"
    assert all(r["Type"]==1 and r["Persist"]==3 for r in vault.values())
    assert vault["account-a@"+service]["CredentialBlob"]=="R0 '🔑".encode("utf-16-le")
    print(json.dumps({"python_keyring":"25.7.0","backend":"in-memory WinCred stand-in on Linux","targets":{k:{"username":v["UserName"],"type":v["Type"],"persistence":v["Persist"],"encoding":"UTF-16LE"} for k,v in vault.items()}},indent=2))
    print("PASS: bare-service first item, compound previous account, UTF-16LE, generic/enterprise defaults")
