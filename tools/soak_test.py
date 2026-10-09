"""Long-run combat soak test through the doomslayer bridge (game running, lab + god on).

Cycles weapons and enemies, fires continuously, keeps enemies topped up and records whether
shots still do damage, the ER bullet pool level and the cost of the enemy scan over time.
Usage: python tools/soak_test.py [minutes] > soak.log
"""
import json
import re
import sys
import time
from pathlib import Path

NAT = Path(__file__).resolve().parents[1] / "dist" / "natives"
CMD, OUT, STATE = NAT / "doomslayer_cmd.txt", NAT / "doomslayer_cmd.out", NAT / "doomslayer_state.json"


def cmd(c, wait=0.35):
    CMD.write_text(c)
    time.sleep(wait)
    try:
        return OUT.read_text(encoding="utf-8", errors="replace").strip().splitlines()[-1]
    except Exception:
        return ""


def state():
    try:
        return json.loads(STATE.read_text(encoding="utf-8"))
    except Exception:
        return {}


minutes = float(sys.argv[1]) if len(sys.argv) > 1 else 25
t0 = time.time()
cmd("lab 1")
shots = dmg_ok = dmg_zero = 0
last_scan = (0, 0)
k = 0
target = 0
while time.time() - t0 < minutes * 60:
    k += 1
    if k % 25 == 1:
        # Next human-sized hostile (max hp >= 150), teleport 6 m from it and keep shooting it.
        st = state()
        big = [e for e in st.get("enemies", []) if e.get("hostile") and e.get("max_hp", 0) >= 150]
        if big:
            target = big[(k // 25) % len(big)]["npc_param"]
        tp = cmd(f"tp_enemy 6 {target}", 1.0)
        print(f"{time.time()-t0:7.1f}s tp: {tp}", flush=True)
    if k % 9 == 1:
        cmd(f"select {(k // 9) % 8}", 0.9)
        cmd("ammo")
    cmd(f"wound 1.0 {target}")
    r = cmd(f"fire_at {target}", 0.45)
    shots += 1
    s = state()
    # fire_at reports the target's HP right before and right after the shot (same character).
    m = re.search(r"hp_before (\d+) hp_after (\d+)", r)
    if m:
        if int(m.group(2)) < int(m.group(1)):
            dmg_ok += 1
        else:
            dmg_zero += 1
            if dmg_zero % 10 == 1:
                print(f"   no damage: {r}", flush=True)
    if k % 20 == 0:
        # Do rays still hit characters at all? (the player's own capsule answers ring rays)
        print(f"         rays: {cmd('rayhits 2000058', 0.6)}", flush=True)
        scan = s.get("scan", [0, 0])
        dn, dc = scan[0] - last_scan[0], scan[1] - last_scan[1]
        last_scan = (scan[0], scan[1])
        avg = dn / dc / 1000 if dc else 0
        print(
            f"{time.time()-t0:7.1f}s shots {shots} dmg_ok {dmg_ok} no_dmg {dmg_zero} pool {s.get('pool')} "
            f"scan avg {avg:.1f} us x{dc} weapon {s.get('weapon')} hp {s.get('player', {}).get('hp')} | {r[:70]}",
            flush=True,
        )
cmd("lab 0")
print(f"done: shots {shots} dmg_ok {dmg_ok} no_dmg {dmg_zero}", flush=True)
