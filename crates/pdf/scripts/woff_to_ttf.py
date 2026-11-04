"""WOFF 1.0 -> sfnt: decompress each table (zlib) and rebuild the table
directory exactly as the WOFF spec defines. Lossless; stdlib only."""
import struct, sys, zlib

def convert(src, dst):
    data = open(src, 'rb').read()
    sig, flavor, length, num, _, total_sfnt, *_ = struct.unpack('>4sIIHHI', data[:20]) + (None,)
    assert sig == b'wOFF', src
    tables = []
    for i in range(num):
        tag, off, comp, orig, csum = struct.unpack('>4sIIII', data[44 + 20 * i:64 + 20 * i])
        raw = data[off:off + comp]
        if comp != orig:
            raw = zlib.decompress(raw)
        assert len(raw) == orig
        tables.append((tag, csum, raw))
    search = 1
    sel = 0
    while search * 2 <= num:
        search *= 2
        sel += 1
    out = bytearray(struct.pack('>IHHHH', flavor, num, search * 16, sel, num * 16 - search * 16))
    offset = 12 + 16 * num
    body = bytearray()
    for tag, csum, raw in tables:
        out += struct.pack('>4sIII', tag, csum, offset + len(body), len(raw))
        body += raw + b'\0' * ((4 - len(raw) % 4) % 4)
    out += body
    assert len(out) == total_sfnt, (len(out), total_sfnt)
    open(dst, 'wb').write(bytes(out))

def features(path):
    d = open(path, 'rb').read()
    num = struct.unpack('>H', d[4:6])[0]
    for i in range(num):
        tag, _, off, _ = struct.unpack('>4sIII', d[12 + 16 * i:28 + 16 * i])
        if tag == b'GSUB':
            flist = off + struct.unpack('>H', d[off + 6:off + 8])[0]
            n = struct.unpack('>H', d[flist:flist + 2])[0]
            return sorted({d[flist + 2 + 6 * j:flist + 6 + 6 * j].decode() for j in range(n)})
    return []

if __name__ == '__main__':
    convert(sys.argv[1], sys.argv[2])
    print(sys.argv[2], features(sys.argv[2]))
