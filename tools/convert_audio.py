"""Convert the user's own DOOM Eternal sounds into the WAV set doomslayer.dll plays.

Step 1 (done once): EternalAudioExtractor unpacks base/sound/soundbanks/pc/sfx*.snd into
C:/Users/<you>/doom-extract/sound (named .opus / .wem files).
Step 2 (this script): pick the sounds each Doom action needs, convert to 48 kHz stereo WAV with
ffmpeg, write dist/natives/doom_audio/<event>/<n>.wav. Nothing from the game is committed.

    python tools/convert_audio.py [extract_dir]
"""

import re
import subprocess
import sys
from pathlib import Path

import paths  # noqa: E402  (tools/paths.py: DOOMRING_* locations)

ROOT = Path(__file__).resolve().parent.parent
EXTRACT = Path(sys.argv[1]) if len(sys.argv) > 1 else paths.WORK / "sound"
OUT = paths.NATIVES / "doom_audio"

# event -> regexes over the extracted base names (without the _id#123 suffix).
EVENTS = {
    # --- weapon mods ---
    # Doom's Play_shotgun_pop_rocket_fire (soundmetadata.bin): the loud shot (fire_5/7/8/9, random)
    # layered with a body (named horde_end_score_11 by the extractor - shared wem) and a quiet tail
    # (fire_1/2/3/4/6). Picking one of all nine at random often played only a faint tail.
    # (horde_end_score_11, the old "body" layer, is a classic Doom sample - band-limited to 5.5 kHz,
    # user heard the old Doom sound in it: dropped)
    "sticky_fire": {"layers": [r"shotgun_pop_rocket_fire_(5|7|8|9)", r"shotgun_pop_rocket_fire_(1|2|3|4|6)"],
                    "delays_ms": [0, 0]},
    "sticky_ready": [r"pop_rocket_ready_\d+"],
    # one bomb recharged (Play_cs_sticky_reload_passive)
    "sticky_recharge": [r"cs_sticky_reload_passive_\d+"],
    "sticky_timer": [r"cs_sticky_bomb_timer_1_mono"],
    "sticky_explode": [r"pop_rocket_explosion_\d+"],
    "sticky_reload": [r"cs_sticky_reload_\d+"],
    "hc_zoom_in": [r"gauss_zoom_in"],
    "hc_zoom_out": [r"heavy_cannon_zoom_out_\d+"],
    "hc_bolt_fire": [r"heavy_cannon_bolt_fire_\d+"],
    "heat_level_1": [r"plasma_heat_blast_charge_level_1"],
    "heat_level_2": [r"plasma_heat_blast_charge_level_2"],
    "heat_level_3": [r"plasma_heat_blast_charge_level_3"],
    "heat_ready": [r"plasma_heat_blast_charged_alert"],
    "heat_blast": [r"plasma_heat_blast_fire_switch_\d+_\d+"],
    "rocket_detonate": {"layers": [r"player_rocket_remote_explo_\d+", r"player_rocket_remote_explo_shockwave_\d+"], "delays_ms": [0, 0]},
    "arb_into": [r"gauss_ballista_into_\d+"],
    "arb_charged": [r"gauss_charged_tone"],
    "arb_fire": [r"gauss_ballista_fire_\d+"],
    "arb_out": [r"gauss_ballista_out_\d+"],
    "arb_explode": [r"ballista_explode_small_\d+"],
    "turret_open": [r"chaingun_turret_open_\d+"],
    "turret_close": [r"chaingun_turret_close_\d+"],
    # Play_chaingunTurret_loop: the turret's thin cracks layered with the chaingun's heavy body
    # (metal_climb_hit_*, shared ids). loop_13 / hit_187 are long shell-fall tails.
    # Mobile Turret: each shot plays a chaingun_fire AND one of these 8 - the same two layers
    # (body + crack) paired differently ("shift"), so two random chaingun sounds per turret shot
    # (user). (chaingunturret_loop_* are the casings, not the gun.)
    "turret_fire": {"layers": [r"metal_climb_hit_(16|47|54|87)", r"wpn_sp_chaingun_fire_\d+"], "delays_ms": [0, 0], "shift": [2, 3]},
    "ssg_fire": [r"eol_shotgun_double_fire_\d+"],
    "ssg_reload": [r"wpn_sp_shotgun_double_shells_out_\d+"],
    # Combat shotgun, matched against a capture of the real game: the shot is shotgun_burst_fire_N
    # (corr 0.77) with the fire_end tail ~0.1 s later; wpn_sp_shotgun_fire_N never plays (corr 0.1).
    # Only the blast variants (burst_fire_4/6 are mostly mechanical clicks), and the action tail at
    # the pump (0.25 s) so it never masks the shot.
    "shotgun_fire": {"layers": [r"shotgun_burst_fire_(10|2|7|8)", r"wpn_sp_shotgun_fire_end_\d+"], "delays_ms": [0, 260]},
    "shotgun_pump": [r"wpn_sp_shotgun_pump_in_\d+"],
    "heavy_cannon_fire": [r"wpn_sp_heavy_cannon_fire_\d+"],
    "plasma_fire": [r"plasma_shot(_\d+)?"],
    # Play_wpn_sp_rocket_launcher_fire: the launch (fire_0/6/7/8; fire_3 is a lone fireball whoosh)
    # always with the metallic cling the extractor names lock_on_fire_upgraded_2/4/6/8 (shared ids).
    "rocket_fire": {"layers": [r"wpn_sp_rocket_launcher_fire_(0|6|7|8)", r"lock_on_fire_upgraded_(2|4|6|8)"], "delays_ms": [0, 0], "gains": [1.0, 2.0]},
    "rocket_explode": [r"player_rocket_explosion_\d+"],
    "ballista_fire": [r"gauss_base_fire_\d+"],
    # Chaingun: Doom's Play_wpn_sp_chaingun_fire (soundmetadata.bin) layers the thin crack
    # (wpn_sp_chaingun_fire_N, 0.1 s, no bass) with a heavy 0.5 s body the extractor happens to
    # name metal_climb_hit_16/47/54/87 (shared ids) - the crack alone sounded like a toy.
    "chaingun_fire": {"layers": [r"metal_climb_hit_(16|47|54|87)", r"wpn_sp_chaingun_fire_\d+"], "delays_ms": [0, 0]},
    "bfg_fire": [r"bfg_fire_\d+"],
    # bfg_charge_2 is a 1.7 s tail; the other four are the ~0.45 s wind-up before the shot.
    "bfg_charge": [r"bfg_charge_(0|1|3|4)"],
    "bfg_explode": [r"bfg_explosion_\d+"],
    # heavy_cannon_dry_fire_5 is a Doomguy grunt the extractor named after a shared id (user) - out.
    "dry_fire": [r"heavy_cannon_dry_fire_([0-4]|[6-9]|10)"],
    # player_dash2_3 is a 0.2 s low thump (sounded like a dodge on its own): the four whooshes only.
    "dash": [r"player_dash2_(0|1|2|4)"],
    "dash_recharge": [r"dash_recharge"],
    # Doom's Play_double_jump: a thrust (in_flight_mobility 0/4/16/19) over an air rush (the
    # quiet variants), 8 combinations so it never repeats the same way.
    "double_jump": {"layers": [r"in_flight_mobility_(0|4|16|19)", r"in_flight_mobility_(2|6|8|11|14|21|23|29)"], "delays_ms": [0, 0]},
    # Doom's Play_chainsaw_on (soundmetadata.bin): a cord pull (chainsaw_on_0/1/3) layered with an
    # engine rev burst the extractor names metal_climb_hit_32/94 (shared ids) - the pull alone
    # (what we had) sounded wrong (user).
    "chainsaw_rev": {"layers": [r"chainsaw_on_(0|1|3)", r"metal_climb_hit_(32|94)"], "delays_ms": [0, 200]},
    # Play_chainsaw_out_of_range: the "no target" sputter (out_of_range_1 + metal_climb_hit_61/120/156).
    "chainsaw_no_target": {"layers": [r"chainsaw_out_of_range_1", r"metal_climb_hit_(61|120|156)"], "delays_ms": [0, 0]},
    # glory_chainsaw_* are Doom's 2.4-5.5 s cinematic sequences (too long for our quick kill):
    # the saw biting flesh (Play_chainsaw_impact_flesh, 0.25-0.5 s) instead.
    "chainsaw_kill": [r"chainsaw_impact_flesh_\d+"],
    # Play_chainsaw_insufficient_fuel: insufficient_fuel_3 + the same sputter.
    "chainsaw_no_fuel": {"layers": [r"chainsaw_insufficient_fuel_\d+", r"metal_climb_hit_(61|120|156)"], "delays_ms": [0, 0]},
    # Chainsaw ready again (fuel pip refilled): Doom has no "chainsaw ready" event of its own; the
    # equipment-slot "recharged" blip of the frag grenade (Play_UI_Grenade_Frag_Cooldown, user's pick).
    "chainsaw_ready": [r"ui_grenade_frag_cooldown_1"],
    "glory_whoosh": [r"glory_punch_whoosh_\d+"],
    "glory_snap": [r"glory_gore_bone_snap_\d+"],
    "glory_gore": [r"glory_huge_guts_explode_\d+"],
    "melee": [r"punch_flesh_\d+", r"normal_punch_\d+"],
    # Doom's Play_bloodpunch_dlc_hit (soundmetadata.bin): one of the short hits layered with the
    # big impact dlc_hit_1 every time. Picking single files at random sometimes played only
    # charged_2 (a quiet charge-up layer, silent for 0.16 s) - "no punch sound" (user).
    # charged_3 rings at 12.6 kHz for ~2 s and dlc_hit_7 opens on a pure 2.9 kHz tone: the
    # "high pitch piercing sound" (user) - left out.
    # Only hits 0/3/8 carry the punch itself (a crack in the first 20 ms); 4/5/6 are swells peaking
    # 0.2-0.4 s in, 10 dB quieter - with them half the punches had "no punch sound" (user).
    # The big impact layer dlc_hit_1 (5.5 s, low hum) under every punch droned the same each time
    # (user): punch hits only now.
    # (kept although named "dlc": the game update gave every copy these hits - the base game's own
    # Play_bloodpunch_charged plays bloodpunch_dlc_hit_1 - so no DLC is needed; user, 2026-10-07)
    "blood_punch": [r"bloodpunch_dlc_hit_(0|3|8)"],
    # The Crucible (Doom's events: Play_crucible_open / _close / _movements_short / _impacts /
    # _no_energy / _ammo_pickup)
    "crucible_open": [r"crucible_open"],
    "crucible_close": [r"crucible_close"],
    "crucible_swing": [r"crucible_movements_short_\d+"],
    "crucible_hit": [r"glory_crucible_slice_(0|7|12|13|14|22|23|24)"],
    "crucible_no_energy": [r"crucible_no_energy"],
    "crucible_pickup": [r"crucible_ammo_pickup"],
    # meathook_fire_1 is the classic Doom super shotgun (dsdshtgn, identical audio) - left out (user).
    "meathook": [r"wpn_shotgun_double_meathook_fire_(0|2|3)"],
    "meathook_hit": [r"meathook_impact_flesh.*"],
    "flame_belch": [r"flame_belch_\d+"],
    "belch_ready": [r"flame_belch_ready"],
    "pickup_health": [r"pickup_health", r"pickup_health_small"],
    "pickup_armor": [r"pickup_armor_small", r"pickup_armor"],
    "pickup_ammo": [r"ammo_shells_rand_loud_\d+", r"pickup_ammo_generic"],
    "pickup_chainsaw_ammo": [r"pickup_ammo_chainsaw_\d+"],
    "armor_break": [r"armor_break"],
    "low_ammo": [r"low_ammo"],
    "weapon_switch": [r"ui_back"],
}

NAME = re.compile(r"^(.*)_id#(\d+)\.(opus|wem|ogg)$")

# Doom's sounds peak ABOVE full scale (up to +8.4 dBFS: Doom mixes in floating point). Written
# straight to 16-bit WAV every peak over 0 dBFS was cut flat - audible distortion on the plasma,
# ballista, Flame Belch, Blood Punch, glory kills (user, 2026-10-08; 171 of 286 sources over).
# Everything is converted this much quieter; audio.rs plays it back louder (HEADROOM_GAIN).
HEADROOM_DB = -9.0


def main():
    files = {}
    for f in EXTRACT.rglob("*"):
        m = NAME.match(f.name)
        if m:
            # Same base name can exist with different ids (variants): keep them all.
            files[f"{m.group(1)}#{m.group(2)}"] = f
    print(f"{len(files)} extracted sounds indexed from {EXTRACT}")
    OUT.mkdir(parents=True, exist_ok=True)
    total = 0
    only = set(filter(None, __import__("os").environ.get("DOOM_AUDIO_ONLY", "").split(",")))
    for event, patterns in EVENTS.items():
        if only and event not in only:
            continue
        layers = patterns.get("layers") if isinstance(patterns, dict) else None
        if layers:
            groups = [sorted(n for n in files if re.match(f"^{p}$", n.split("#")[0])) for p in layers]
            d = OUT / event
            d.mkdir(exist_ok=True)
            for old in d.glob("*.wav"):
                old.unlink()
            count = max(len(g) for g in groups) if all(groups) else 0
            for i in range(min(count, 8)):
                shift = patterns.get("shift", [0] * len(groups))
                srcs = [files[g[(i + sh) % len(g)]] for g, sh in zip(groups, shift)]
                args = [paths.FFMPEG, "-v", "error", "-y"]
                for src in srcs:
                    args += ["-i", str(src)]
                delays = patterns.get("delays_ms", [0] * len(srcs))
                gains = patterns.get("gains", [1.0] * len(srcs))
                hr = 10 ** (HEADROOM_DB / 20)
                pre = "".join(f"[{k}:a]adelay={d}|{d},volume={g * hr:.4f}[l{k}];" for k, (d, g) in enumerate(zip(delays, gains)))
                ins = "".join(f"[l{k}]" for k in range(len(srcs)))
                args += ["-filter_complex", f"{pre}{ins}amix=inputs={len(srcs)}:duration=longest:normalize=0,alimiter=limit=0.89:level=false",
                         "-ar", "48000", "-ac", "2", "-c:a", "pcm_s16le", str(d / f"{i}.wav")]
                subprocess.run(args, capture_output=True)
                total += 1
            print(f"{event}: {count} layered from {[len(g) for g in groups]}")
            continue
        # {"files": [...], "trim": [start, end]}: cut a part out of each file (fade out at the end)
        trim = patterns.get("trim") if isinstance(patterns, dict) else None
        if isinstance(patterns, dict):
            patterns = patterns["files"]
        regs = [re.compile(f"^{p}$") for p in patterns]
        picks = sorted(n for n in files if any(r.match(n.split("#")[0]) for r in regs))
        if not picks:
            print(f"!! {event}: no match for {patterns}")
            continue
        d = OUT / event
        d.mkdir(exist_ok=True)
        for old in d.glob("*.wav"):
            old.unlink()
        for i, name in enumerate(picks[:12]):
            dst = d / f"{i}.wav"
            chain = []
            if trim:
                a, b = trim
                chain.append(f"atrim={a}:{b},asetpts=PTS-STARTPTS,afade=t=out:st={max(b - a - 0.04, 0)}:d=0.04")
            chain.append(f"volume={HEADROOM_DB}dB")
            af = ["-af", ",".join(chain)]
            r = subprocess.run(
                [paths.FFMPEG, "-v", "error", "-y", "-i", str(files[name]), *af,
                 "-ar", "48000", "-ac", "2", "-c:a", "pcm_s16le", str(dst)],
                capture_output=True, text=True)
            if r.returncode != 0:
                print(f"!! {name}: {r.stderr.strip()[:200]}")
                continue
            total += 1
        print(f"{event}: {len(picks[:12])} <- {', '.join(p.split('#')[0] for p in picks[:3])}{' ...' if len(picks) > 3 else ''}")
    print(f"wrote {total} wav files to {OUT}")


if __name__ == "__main__":
    main()
