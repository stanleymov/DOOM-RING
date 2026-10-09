"""Fix the granule positions of a Vorbis Ogg file (what revorb does - its licence is unclear, so this is
our own, from the Vorbis I spec). ww2ogg's output decodes fine but ends 1024 frames short without it.

    python tools/ogg_regranule.py in.ogg [out.ogg]
"""
import struct
import sys
import zlib  # noqa: F401  (Ogg uses its own CRC below)

# ---------------------------------------------------------------- Ogg pages / packets

_CRC = []
for i in range(256):
    r = i << 24
    for _ in range(8):
        r = ((r << 1) ^ 0x04C11DB7) if r & 0x80000000 else (r << 1)
    _CRC.append(r & 0xFFFFFFFF)


def ogg_crc(data):
    c = 0
    for b in data:
        c = ((c << 8) & 0xFFFFFFFF) ^ _CRC[((c >> 24) & 0xFF) ^ b]
    return c


def read_packets(b):
    """All packets of the (single) logical stream, and its serial number."""
    packets, cur, o, serial = [], b"", 0, None
    while o < len(b):
        if b[o:o + 4] != b"OggS":
            raise ValueError(f"no Ogg page at {o}")
        serial = struct.unpack_from("<I", b, o + 14)[0]
        nseg = b[o + 26]
        lacing = b[o + 27:o + 27 + nseg]
        p = o + 27 + nseg
        for ln in lacing:
            cur += b[p:p + ln]
            p += ln
            if ln < 255:
                packets.append(cur)
                cur = b""
        o = p
    if cur:
        packets.append(cur)
    return packets, serial


def write_pages(packets, granules, serial):
    """One page per packet group: headers on their own pages, then each audio packet on a page."""
    out = bytearray()
    seq = 0

    def page(pkts, granule, flags):
        nonlocal seq
        lacing = bytearray()
        body = bytearray()
        for p in pkts:
            n = len(p)
            while n >= 255:
                lacing.append(255)
                n -= 255
            lacing.append(n)
            body += p
        hdr = struct.pack("<4sBBqIII", b"OggS", 0, flags, granule, serial, seq, 0) + bytes([len(lacing)]) + bytes(lacing)
        pg = bytearray(hdr + body)
        struct.pack_into("<I", pg, 22, ogg_crc(pg))
        seq += 1
        return pg

    out += page([packets[0]], 0, 0x02)
    out += page(packets[1:3], 0, 0)
    for k in range(3, len(packets)):
        flags = 0x04 if k == len(packets) - 1 else 0
        out += page([packets[k]], granules[k], flags)
    return bytes(out)


# ---------------------------------------------------------------- Vorbis setup header

class Bits:
    def __init__(self, data):
        self.d = data
        self.pos = 0

    def read(self, n):
        v = 0
        for i in range(n):
            byte = self.d[self.pos >> 3]
            v |= ((byte >> (self.pos & 7)) & 1) << i
            self.pos += 1
        return v


def ilog(v):
    n = 0
    while v > 0:
        n += 1
        v >>= 1
    return n


def lookup1_values(entries, dims):
    r = int(round(entries ** (1.0 / dims)))
    while (r + 1) ** dims <= entries:
        r += 1
    while r ** dims > entries:
        r -= 1
    return r


def mode_blockflags(setup, channels):
    """Each mode's block flag, read through the whole setup header (Vorbis I spec 4.2.4)."""
    b = Bits(setup)
    assert b.read(8) == 5
    b.read(48)  # "vorbis"
    for _ in range(b.read(8) + 1):  # codebooks
        assert b.read(24) == 0x564342, "codebook sync"
        dims = b.read(16)
        entries = b.read(24)
        if b.read(1):  # ordered
            cur = 0
            b.read(5)
            while cur < entries:
                cur += b.read(ilog(entries - cur))
        else:
            sparse = b.read(1)
            for _ in range(entries):
                if not sparse or b.read(1):
                    b.read(5)
        lookup = b.read(4)
        if lookup in (1, 2):
            b.read(32)
            b.read(32)
            bits = b.read(4) + 1
            b.read(1)
            n = lookup1_values(entries, dims) if lookup == 1 else entries * dims
            for _ in range(n):
                b.read(bits)
    for _ in range(b.read(6) + 1):  # time domain transforms
        b.read(16)
    for _ in range(b.read(6) + 1):  # floors
        ftype = b.read(16)
        if ftype == 0:
            b.read(8)
            b.read(16)
            b.read(16)
            b.read(6)
            b.read(8)
            for _ in range(b.read(4) + 1):
                b.read(8)
        else:
            parts = b.read(5)
            classes = [b.read(4) for _ in range(parts)]
            dims = []
            for _ in range(max(classes) + 1 if classes else 0):
                dims.append(b.read(3) + 1)
                sub = b.read(2)
                if sub:
                    b.read(8)
                for _ in range(1 << sub):
                    b.read(8)
            b.read(2)
            rangebits = b.read(4)
            for c in classes:
                for _ in range(dims[c]):
                    b.read(rangebits)
    for _ in range(b.read(6) + 1):  # residues
        b.read(16)
        b.read(24)
        b.read(24)
        b.read(24)
        ncls = b.read(6) + 1
        b.read(8)
        cascade = []
        for _ in range(ncls):
            low = b.read(3)
            high = b.read(5) if b.read(1) else 0
            cascade.append(high * 8 + low)
        for c in cascade:
            for j in range(8):
                if c & (1 << j):
                    b.read(8)
    for _ in range(b.read(6) + 1):  # mappings
        b.read(16)
        submaps = b.read(4) + 1 if b.read(1) else 1
        if b.read(1):
            for _ in range(b.read(8) + 1):
                b.read(ilog(channels - 1))
                b.read(ilog(channels - 1))
        b.read(2)
        if submaps > 1:
            for _ in range(channels):
                b.read(4)
        for _ in range(submaps):
            b.read(8)
            b.read(8)
            b.read(8)
    flags = []
    for _ in range(b.read(6) + 1):  # modes
        flags.append(b.read(1))
        b.read(16)
        b.read(16)
        b.read(8)
    return flags


def regranule(data):
    packets, serial = read_packets(data)
    ident = packets[0]
    channels = ident[11]
    bs0 = 1 << (ident[28] & 0x0F)
    bs1 = 1 << (ident[28] >> 4)
    flags = mode_blockflags(packets[2], channels)
    mode_bits = ilog(len(flags) - 1)
    granules = [0] * len(packets)
    total, prev = 0, None
    for k in range(3, len(packets)):
        p = packets[k]
        if not p:
            granules[k] = total
            continue
        bits = Bits(p)
        bits.read(1)  # packet type (0 = audio)
        mode = bits.read(mode_bits) if mode_bits else 0
        size = bs1 if flags[mode] else bs0
        if prev is not None:
            total += prev // 4 + size // 4
        prev = size
        granules[k] = total
    return write_pages(packets, granules, serial)


if __name__ == "__main__":
    src = sys.argv[1]
    dst = sys.argv[2] if len(sys.argv) > 2 else src
    data = open(src, "rb").read()
    open(dst, "wb").write(regranule(data))
