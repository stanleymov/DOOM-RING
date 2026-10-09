"""Read sounds out of DOOM Eternal's .snd archives (base/sound/soundbanks/pc), our own reader.

.snd layout (little endian): u32 version, u32 info size, u32 header size, <header size> bytes, then
(info size - header size) / 32 records of 32 bytes: 8 bytes, u32 source id, u32 encoded size,
u32 absolute data offset, u32 decoded size, u16 format (2 = Ogg Opus, else a Wwise .wem), 6 bytes.
"""
import struct
from pathlib import Path


def records(snd):
    """{source id: (offset, size, format)} of one .snd file."""
    out = {}
    with open(snd, "rb") as f:
        _ver, info, hdr = struct.unpack("<3I", f.read(12))
        f.seek(12 + hdr)
        n = (info - hdr) // 32
        raw = f.read(n * 32)
    for k in range(n):
        sid, size, off, _dec, fmt = struct.unpack_from("<IIIIH", raw, k * 32 + 8)
        if size:
            out[sid] = (off, size, fmt)
    return out


def read(snd, rec):
    off, size, _fmt = rec
    with open(snd, "rb") as f:
        f.seek(off)
        return f.read(size)


def extension(fmt):
    return ".opus" if fmt == 2 else ".wem"


def soundbanks(doom_base):
    return Path(doom_base) / "sound" / "soundbanks" / "pc"
