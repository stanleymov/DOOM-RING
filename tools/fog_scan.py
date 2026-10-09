"""Find the collision filter that sees fog walls: stand a few metres in front of one, facing it, then
    python tools/fog_scan.py
It sphere-casts straight ahead (flat) through the bridge with every layer 0..127, both with and
without the 0x2000000 bit, and prints the filters whose hit is nearer than the default filter's.
"""
import json
import time
from pathlib import Path

NAT = Path(__file__).resolve().parents[1] / "dist" / "natives"
CMD, OUT, STATE = NAT / "doomslayer_cmd.txt", NAT / "doomslayer_cmd.out", NAT / "doomslayer_state.json"


def bridge(line, timeout=3.0):
    before = OUT.stat().st_mtime if OUT.exists() else 0
    CMD.write_text(line + "\n")
    t0 = time.time()
    while time.time() - t0 < timeout:
        time.sleep(0.03)
        if OUT.exists() and OUT.stat().st_mtime != before:
            time.sleep(0.02)
            return OUT.read_text().strip().splitlines()[-1]
    return "timeout"


cam = json.loads(STATE.read_text())["camera"]["fwd"]
fx, fz = cam[0], cam[2]
n = (fx * fx + fz * fz) ** 0.5
d = (fx / n * 12.0, 0.0, fz / n * 12.0)


def cast(filt):
    r = bridge(f"cast {d[0]:.3f} {d[1]:.3f} {d[2]:.3f} {filt:x} 0.3")
    if "seg" in r:
        return float(r.split("seg ")[1].split()[0]) * 12.0, r
    return None, r


base, braw = cast(0x2000058)
print("default 0x2000058:", braw)
found = []
for hi in (0x2000000, 0):
    for layer in range(128):
        f = hi | layer
        dist, raw = cast(f)
        if dist is not None and (base is None or dist < base - 0.3):
            found.append((dist, f, raw))
            print(f"{f:#x}: {dist:.2f} m  {raw}")
print("nearer than default:", [(round(x[0], 2), hex(x[1])) for x in sorted(found)][:20])
