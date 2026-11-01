"""Freeze the unchanged Python PSBT decoder and public-analysis contracts."""
import argparse
import base64
from hashlib import new, sha256
import json
from pathlib import Path
import random
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from embit.script import Script, Witness
from embit.transaction import Transaction, TransactionInput, TransactionOutput
from kassiber.core.chain_analysis.psbt import analyze_psbt, decode_psbt_structure
from kassiber.errors import AppError

DEST = ROOT / "crates/wallets/tests/data/psbt.json"
KEY = bytes.fromhex("0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")
SCRIPT = b"\x00\x14" + new("ripemd160", sha256(KEY).digest()).digest()
SIG = bytes.fromhex("300602010102010101")


def compact(n):
    if n < 253:
        return bytes([n])
    width, marker = (2, 253) if n <= 65535 else (4, 254) if n <= 0xffffffff else (8, 255)
    return bytes([marker]) + n.to_bytes(width, "little")


def mapping(rows):
    return b"".join(compact(len(k)) + k + compact(len(v)) + v for k, v in rows) + b"\0"


def encode(g, inputs, outputs):
    return b"psbt\xff" + mapping(g) + b"".join(mapping(r) for r in inputs + outputs)


def sample():
    prev = Transaction(vin=[TransactionInput(b"\x99" * 32, 0)], vout=[TransactionOutput(100000, Script(SCRIPT))])
    tx = Transaction(vin=[TransactionInput(prev.txid(), 0, sequence=0xfffffffd)], vout=[TransactionOutput(99000, Script(SCRIPT))], locktime=100)
    return [(b"\0", tx.serialize())], [[(b"\0", prev.serialize()), (b"\1", prev.vout[0].serialize())]], [[]]


def v2():
    return ([(b"\xfb", (2).to_bytes(4,"little")), (b"\2", (2).to_bytes(4,"little")), (b"\4", b"\1"), (b"\5", b"\1")],
            [[(b"\x0e", b"\x11" * 32), (b"\x0f", bytes(4))]], [[(b"\3", (1000).to_bytes(8,"little")), (b"\4", SCRIPT)]])


def evaluate(case):
    raw = bytes.fromhex(case["binary_hex"]) if "binary_hex" in case else case["text"]
    try:
        result = decode_psbt_structure(raw) if case["operation"] == "structure" else analyze_psbt(raw, network=case.get("network","main"), subject_id=case.get("subject","psbt:transaction"))
        return {"ok": result}
    except AppError as e:
        return {"error":dict(code=e.code, message=str(e), hint=e.hint, details=e.details, retryable=e.retryable)}
    except RuntimeError as e:
        return {"error":dict(code="internal_error",message=str(e),hint=None,details=None,retryable=False)}


def vectors():
    cases = []
    def add(group, name, raw, operation="analyze", **kw):
        c = dict(group=group, name=name, operation=operation, **kw)
        c["binary_hex" if isinstance(raw,bytes) else "text"] = raw.hex() if isinstance(raw,bytes) else raw
        c["expected"] = evaluate(c)
        cases.append(c)
    base = encode(*sample())
    for op in ("analyze","structure"):
        for n,vector in enumerate(json.loads((ROOT / "tests/fixtures/chain_analysis/psbt_bip_vectors.json").read_text())):
            add("bip", f"bip-{n}-{vector['bip']}-{vector['case']}-{op}", vector["psbt"],op)
        for name,raw in json.loads((ROOT / "tests/fixtures/chain_analysis/payjoin_bip78_vectors.json").read_text()).items():
            if isinstance(raw,str) and raw.startswith("cHNid"):
                add("bip",f"bip78-{name}-{op}",raw,op)
        add("analysis",f"sample-{op}",base,op)
        for i in range(len(base)):
            add("framing",f"truncated-{i}-{op}",base[:i],op)
        for suffix in (b"\0",b"x",b"\0\0",b"\xfd\0\0"):
            add("framing",f"trailing-{suffix.hex()}-{op}",base+suffix,op)
        for i in (0,5,6,7,9,10,len(base)-3):
            for byte in (0,1,252,253,254,255):
                doc=bytearray(base); doc[i]=byte
                add("framing",f"byte-{i}-{byte}-{op}",bytes(doc),op)
    for text in ("", "=", "====", "AA", "AA=", "AA==", "AA===", "AAAA=", "AAAA====", "AAAA==x", "AAAA=AAAA", "AAAA==AAAA", "AAAA!", "70736274ff0", "70736274ffxx", "70736274FF", "cHNidP8=", "cHNidP8===", "cHNidP9=", "cHNidP8", "\u00e4"):
        add("framing",f"encoding-{text!r}",text)
    for length in (b"\0",b"\xfd\0\0",b"\xfe"+bytes(4),b"\xff"+bytes(8)):
        for tail in (b"",b"\xff",b"\xff\xff",b"\xff"*3,b"\xff"*4,b"\xff"*4+b"\0\0"+bytes(4)):
            tx=(2).to_bytes(4,"little")+b"\1"+b"\x11"*32+bytes(4)+length+tail
            add("framing",f"unsigned-row-roundtrip-{length.hex()}-{tail.hex()}",encode([(b"\0",tx)],[[]],[]))
    for network in ("main","test","signet","regtest","auto",""):
        for op in ("analyze","structure"):
            add("analysis",f"network-{network}-{op}",base,op,network=network,subject="opaque:proposal")
    for space in (" ","\r\n","\t","\u00a0","\u001c","\u2003","\u0085"):
        add("framing",f"base64-whitespace-{space!r}",space.join(base64.b64encode(base).decode()))
        add("framing",f"hex-whitespace-{space!r}",space.join(base.hex()))
    for padding in ("", "=", "==", "===", "====", "=====", "==x"):
        for n in range(3):
            raw = base + bytes(n)
            add("framing",f"padding-{n}-{padding}",base64.b64encode(raw).decode()+padding)
    for version in (0,1,2,3,0xffffffff):
        for width in (0,1,3,4,5,8):
            g,i,o=sample();g.append((b"\xfb",version.to_bytes(4,"little")[:width] + bytes(max(0,width-4))))
            add("framing",f"psbt-version-{version}-{width}",encode(g,i,o))
    for scope in ("global","input","output"):
        for key in ({"global":(2,3,4,5,6),"input":(14,15,16,17,18),"output":(3,4)}[scope]):
            g,i,o=sample();target=g if scope=="global" else i[0] if scope=="input" else o[0]
            target.append((bytes([key]),bytes(4)))
            add("framing",f"v0-forbidden-{scope}-{key}",encode(g,i,o))
        for key in (b"\0x",b"\1x",b"\2x",b"\3x",b"\4x",b"\5x",b"\6x",b"\7x",b"\x08x",b"\x0ex",b"\xfbx",b"\xfd\x01\x00",b"\xfd\xfd\x00",b"\xfcprivate"):
            g,i,o=sample(); target=g if scope=="global" else i[0] if scope=="input" else o[0]
            target.append((key,b"private-metadata-canary"))
            add("framing",f"typed-{scope}-{key.hex()}",encode(g,i,o))
        for duplicate in (True,False):
            g,i,o=sample();target=g if scope=="global" else i[0] if scope=="input" else o[0]
            target.extend([(b"\xfctest",b"a"),(b"\xfctest" if duplicate else b"\xfcother",b"b")])
            add("framing",f"duplicate-{scope}-{duplicate}",encode(g,i,o))
    for raw in (b"",b"\0",b"\xfd\x01\0",b"\xfd\x01\x10",b"\xfe"+bytes(4),b"\1\0"):
        for key in (b"\4",b"\5"):
            g,i,o=v2();g=[(k,raw if k==key else v) for k,v in g]
            add("v2",f"count-{key.hex()}-{raw.hex()}",encode(g,i,o))
    for sequence in (0,1,0x400000,0x80000000,0xfffffffd,0xfffffffe,0xffffffff):
        for version in (0,1,2,3,0x80000000,0xffffffff):
            for fallback in (0,55,500000000):
                g,i,o=v2();g=[(k,version.to_bytes(4,"little") if k==b"\2" else v) for k,v in g]
                g.append((b"\3",fallback.to_bytes(4,"little")));i[0].append((b"\x10",sequence.to_bytes(4,"little")))
                add("v2",f"sequence-{sequence}-{version}-{fallback}",encode(g,i,o))
    for locks in ((None,None),(None,0),(None,1),(None,499999999),(None,500000000),(499999999,None),(500000000,None),(500000010,None),(0xffffffff,None),(500000001,100),(500000000,200)):
        for second in ((None,None),(500000005,None),(None,300),(500000006,400)):
            g,i,o=v2();g=[(k,b"\2" if k==b"\4" else v) for k,v in g];i.append([(b"\x0e",b"\x22"*32),(b"\x0f",bytes(4))])
            for row,pair in zip(i,(locks,second)):
                for key,n in zip((b"\x11",b"\x12"),pair):
                    if n is not None:row.append((key,n.to_bytes(4,"little")))
            add("v2",f"lock-{locks}-{second}",encode(g,i,o))
    g,i,o=v2();g=[(k,b"\2" if k==b"\5" else v) for k,v in g];o.append([(b"\3",(2100000000000000).to_bytes(8,"little")),(b"\4",SCRIPT)])
    add("v2","aggregate-output-range",encode(g,i,o))
    g,i,o=v2();g=[(k,b"\2" if k==b"\4" else v) for k,v in g];i.append(i[0])
    add("v2","duplicate-outpoint",encode(g,i,o))
    for scope in ("global","input","output"):
        for key in (b"\0",b"\2",b"\3",b"\4",b"\5",b"\6",b"\x0e",b"\x0f",b"\x10",b"\x11",b"\x12"):
            for size in (0,1,3,4,7,8,31,32,33):
                g,i,o=v2();target=g if scope=="global" else i[0] if scope=="input" else o[0]
                target[:]=[(k,v) for k,v in target if k!=key];target.append((key,bytes(size)))
                add("v2",f"field-{scope}-{key.hex()}-{size}",encode(g,i,o))
    for key in (b"\0",b"\1"):
        original = dict(sample()[1][0])[key]
        for n in range(len(original)+1):
            g,i,o=sample();i[0]=[(k,original[:n] if k==key else v) for k,v in i[0]]
            add("utxo",f"utxo-truncated-{key.hex()}-{n}",encode(g,i,o))
        for offset in (0,4,len(original)-1):
            g,i,o=sample();changed=bytearray(original);changed[offset]^=1;i[0]=[(k,bytes(changed) if k==key else v) for k,v in i[0]]
            add("utxo",f"utxo-mutated-{key.hex()}-{offset}",encode(g,i,o))
    for amount in (2100000000000000,2100000000000001):
        prev=Transaction(vin=[TransactionInput(b"\x99"*32,0)],vout=[TransactionOutput(amount,Script(SCRIPT))])
        tx=Transaction(vin=[TransactionInput(prev.txid(),0)],vout=[TransactionOutput(1,Script(SCRIPT))])
        add("utxo",f"previous-range-{amount}",encode([(b"\0",tx.serialize())],[[(b"\0",prev.serialize())]],[[]]))
        tx.vin[0].vout=1
        add("utxo",f"previous-index-{amount}",encode([(b"\0",tx.serialize())],[[(b"\0",prev.serialize())]],[[]]))
    for known in (False,True):
        g,i,o=v2();g=[(k,b"\3" if k==b"\4" else v) for k,v in g]
        i.extend([[(b"\x0e",bytes([n])*32),(b"\x0f",bytes(4))] for n in (0x22,0x33)])
        for row in i[:3 if known else 2]:row.append((b"\1",(2100000000000000).to_bytes(8,"little")+compact(len(SCRIPT))+SCRIPT))
        add("analysis",f"aggregate-input-range-{known}",encode(g,i,o))
    for script in (b"",b"\x51",b"\x6a",b"\x76\xa9\x14"+b"\x11"*20+b"\x88\xac",SCRIPT,b"\0\x20"+b"\x11"*32,b"\x51\x20"+b"\x11"*32,b"\x52\2ab"):
        for amount in (0,1,99000,100000,2100000000000000,2100000000000001,0xffffffffffffffff):
            g,i,o=sample();i[0]=[(b"\1",amount.to_bytes(8,"little")+compact(len(script))+script)]
            add("utxo",f"witness-utxo-{script.hex()}-{amount}",encode(g,i,o))
    for n,(outer,redeem,wscript) in enumerate(((b"\xa9\x14"+new("ripemd160",sha256(SCRIPT).digest()).digest()+b"\x87",SCRIPT,None),(b"\0\x20"+sha256(b"\x51").digest(),None,b"\x51"),(SCRIPT,b"\x51",None),(SCRIPT,None,b"\x51"))):
        for wrong in (False,True):
            g,i,o=sample();i[0]=[(b"\1",(100000).to_bytes(8,"little")+compact(len(outer))+outer)]
            for key,value in ((b"\4",redeem),(b"\5",wscript)):
                if value is not None:i[0].append((key,value+(b"x" if wrong else b"")))
            add("utxo",f"script-binding-{n}-{outer.hex()}-{wrong}",encode(g,i,o))
    for key in (b"\2"+KEY,b"\x13",b"\x14"+b"\x11"*64,b"\x14"+b"\x11"*63):
        for sig in (b"",SIG,SIG[:-1]+b"\x83",b"\x11"*64,b"\x11"*64+b"\1",b"\x11"*64+b"\0",b"\x11"*66):
            for sighash in (None,1,3,0x83):
                g,i,o=sample();i[0].append((key,sig))
                if sighash is not None:i[0].append((b"\3",sighash.to_bytes(4,"little")))
                add("analysis",f"partial-{key.hex()}-{sig.hex()}-{sighash}",encode(g,i,o))
    for key in (b"\6"+KEY,b"\6"+KEY[:-1],b"\6"+b"\x06"+KEY[1:],b"\6"+b"\2"+bytes(32)):
        for size in (0,1,3,4,5,8,12):
            g,i,o=sample();i[0].append((key,bytes(size)))
            add("analysis",f"derivation-{key.hex()}-{size}",encode(g,i,o))
    for scriptsig in (None,b"",b"\x51",b"x"*252,b"x"*253):
        for stacks in (None,[],[SIG,KEY],[b"\x11"*64],[b"x"*253,b""]):
            g,i,o=sample()
            if scriptsig is not None:i[0].append((b"\7",scriptsig))
            if stacks is not None:i[0].append((b"\x08",Witness(stacks).serialize()))
            add("analysis",f"final-{len(scriptsig) if scriptsig is not None else None}-{stacks!r}",encode(g,i,o))
    for final in (False,True):
        g,i,o=sample();i[0].append((b"\2"+KEY,SIG))
        if final:i[0].append((b"\x08",Witness([SIG,KEY]).serialize()))
        add("analysis",f"redundant-partial-{final}",encode(g,i,o))
    for raw in (b"",b"\1",b"\1\2a",b"\0x",b"\xfd\0\0"):
        g,i,o=sample();i[0].append((b"\x08",raw));add("analysis",f"invalid-final-{raw.hex()}",encode(g,i,o))
    for ni,no in ((0,0),(0,1),(1,0)):
        tx=Transaction(vin=[TransactionInput(b"\x11"*32,0)]*ni,vout=[TransactionOutput(1,Script(SCRIPT))]*no)
        add("analysis",f"construction-{ni}-{no}",encode([(b"\0",tx.serialize())],[[] for _ in range(ni)],[[] for _ in range(no)]))
    for material in (b"private-metadata-canary",b"private-metadata-changed"):
        g,i,o=sample();g.append((b"\1xpub",material));i[0].append((b"\xfcprivate",material));o[0].append((b"\7derivation",material))
        add("analysis",f"discard-{material!r}",encode(g,i,o))
    for op in ("analyze","structure"):
        g,i,o=v2();o[0][1]=(b"\4",b"\x6a"+b"x"*100000)
        add("analysis",f"byte-script-category-beyond-text-feature-cap-{op}",encode(g,i,o),op)
    rng=random.Random(21)
    for n in range(100):
        g,i,o=v2();g.append((b"\3",rng.choice((0,1,500000000)).to_bytes(4,"little")));i[0].append((b"\x10",rng.getrandbits(32).to_bytes(4,"little")))
        o[0][0]=(b"\3",rng.randrange(100000).to_bytes(8,"little"))
        add("v2",f"random-{n}",encode(g,i,o))
    assert len({c['name'] for c in cases}) == len(cases)
    return cases


if __name__ == "__main__":
    parser=argparse.ArgumentParser();parser.add_argument("--check",action="store_true");args=parser.parse_args()
    data=json.dumps(vectors(),sort_keys=True,separators=(",",":"),ensure_ascii=True)+"\n"
    if args.check:
        assert DEST.read_text() == data, "PSBT vector drift"
    else:
        DEST.write_text(data)
    print(f"{len(json.loads(data))} PSBT vectors")
