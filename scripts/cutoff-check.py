#!/usr/bin/env python3
"""Cutoff check: takes that stop early must be redone or marked, never handed over as fine.

ZONOS3-Mini sometimes stops a take before the text is done: mid-word (the take ends loud), or
cleanly after the first sentence or with words left out (the take ends in silence but is far too
short). Booth redoes such takes, and marks one it couldn't fix with "may_be_cut_off". This drives
the booth-mcp program the way Claude Desktop does and speaks lines that caught the model out
(2026-11-26 report), at normal and fast pace, then measures every take Booth handed back:

  - a take that ends loud (last 10 ms above -50 dB) or is too short for its text (more than
    25 letters and digits per second of speech) and is not marked fails the check;
  - a marked take is reported, but passes: Booth told the caller.

Standard library only. Uses the MCP client and model download of smoke-release.py.
  cutoff-check.py --mcp PATH --models DIR [--home DIR] [--seeds 1,2,3] [--pace 0,2] [--laci on|off]
                  [--device cpu|auto]

--home reuses a Booth home folder (for example one with an NVIDIA engine in engines/); without
it a fresh one is made, and the engine that ships next to booth-mcp runs.
"""

import argparse
import importlib.util
import json
import math
import os
import struct
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("smoke", HERE / "smoke-release.py")
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)

LINES = [
    # Stopped cleanly after the first sentence (3.2 s of 11) at a fast speaking rate.
    "Then a leaked intelligence file hits the press. U.S. spies were watching his secret affair "
    "with Alice Huang, an executive at a Singapore datacenter firm.",
    # Ended mid-word on "Nvidia."
    "Officials suspect Beijing is using her to funnel banned AI chips into China. More workplace "
    "affairs spill out inside Nvidia.",
    # Ended mid-word, leaving out "firm."
    "U.S. spies were watching his secret affair with Alice Huang, an executive at a Singapore data center firm.",
]
CUT_DB = -50.0  # crates/booth-core/src/takes.rs
MAX_LETTERS_PER_SECOND = 25.0
SPEECH_DB = -45.0


def pcm(path):
    b = Path(path).read_bytes()
    i, fmt, data = 12, None, b""
    while i + 8 <= len(b):
        cid, n = b[i : i + 4], struct.unpack("<I", b[i + 4 : i + 8])[0]
        if cid == b"fmt ":
            fmt = struct.unpack("<HHIIHH", b[i + 8 : i + 24])
        elif cid == b"data":
            data = b[i + 8 : i + 8 + n]
        i += 8 + n + (n & 1)
    _, ch, rate, _, _, bits = fmt
    if bits != 16:
        smoke.fail(f"{path}: expected 16-bit PCM, got {bits}-bit")
    return rate, ch, struct.unpack(f"<{len(data) // 2}h", data[: len(data) // 2 * 2])


def db(chunk):
    ms = sum((s / 32768) ** 2 for s in chunk) / max(len(chunk), 1)
    return 10 * math.log10(max(ms, 1e-12))


def measure(path, text):
    rate, ch, s = pcm(path)
    win = rate // 100 * ch
    end_db = db(s[-win:])
    loud = [db(s[k : k + win]) > SPEECH_DB for k in range(0, len(s), win)]
    speech = (len(loud) - loud[::-1].index(True) - loud.index(True)) / 100 if any(loud) else 0.0
    letters = sum(c.isalnum() for c in text)
    rate_lps = letters / speech if speech else 0.0
    return {
        "seconds": round(len(s) / ch / rate, 2),
        "speech_seconds": round(speech, 2),
        "letters_per_second": round(rate_lps, 1),
        "end_db": round(end_db, 1),
        "cut": end_db > CUT_DB,
        "short": letters >= 20 and rate_lps > MAX_LETTERS_PER_SECOND,
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--mcp", required=True)
    ap.add_argument("--models", required=True)
    ap.add_argument("--home", help="Booth home folder to use (kept); default: a fresh temporary one")
    ap.add_argument("--seeds", default="1,2,3")
    ap.add_argument("--pace", default="0,2", help="pace doses to try (Booth's -2..2 slider)")
    ap.add_argument("--laci", choices=["on", "off"], help="set Guard against skips; default: Booth's own default")
    ap.add_argument("--device", choices=["cpu", "auto"], help="where the voice model runs; default: Booth's own")
    ap.add_argument("--voice", default="Wyatt")
    ap.add_argument("--report", help="write every take's measurements here as JSON")
    a = ap.parse_args()

    models = Path(a.models).resolve()
    smoke.step("Model files")
    smoke.fetch_models(models)

    home = Path(a.home).resolve() if a.home else Path(tempfile.mkdtemp(prefix="booth-cutoff-"))
    home.mkdir(parents=True, exist_ok=True)
    settings = {}
    if a.laci:
        settings["laci"] = a.laci == "on"
    if a.device:
        settings["device"] = a.device
    (home / "settings.json").write_text(json.dumps(settings))
    env = {k: v for k, v in os.environ.items() if not k.startswith("BOOTH_")}
    env.update(BOOTH_HOME=str(home), BOOTH_MODELS_DIR=str(models), RUST_BACKTRACE="1")

    s = smoke.Mcp(Path(a.mcp).resolve(), env)
    rows, bad = [], []
    try:
        s.call("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                              "clientInfo": {"name": "booth-cutoff", "version": "1"}}, 60)
        s._send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        s.tool("status", {}, 300)
        for text in LINES:
            for pace in [float(p) for p in a.pace.split(",")]:
                for seed in [int(x) for x in a.seeds.split(",")]:
                    args = {"text": text, "voice": a.voice, "seed": seed}
                    if pace:
                        args["controls"] = {"pace": pace}
                    t0 = time.monotonic()
                    err, out = s.tool("speak", args)
                    made_in = round(time.monotonic() - t0, 1)
                    if err:
                        smoke.fail(f"speak failed: {out}")
                    take = json.loads(out)
                    m = measure(take["file"], text)
                    marked = bool(take.get("may_be_cut_off"))
                    row = {"text": text[:40], "pace": pace, "seed": seed, "kept_seed": take.get("seed"),
                           "marked": marked, "made_in": made_in, "file": take["file"], **m}
                    rows.append(row)
                    verdict = "MARKED" if marked else ("BAD" if m["cut"] or m["short"] else "ok")
                    print(f"  {verdict:6} pace {pace:+.0f} seed {seed}: {m['seconds']} s, "
                          f"{m['letters_per_second']} letters/s, ends {m['end_db']} dB, made in {made_in} s")
                    if verdict == "BAD":
                        bad.append(row)
    finally:
        s.close()
    if a.report:
        Path(a.report).write_text(json.dumps(rows, indent=1))
    marked = sum(r["marked"] for r in rows)
    made = sum(r["made_in"] for r in rows)
    smoke.step(f"{len(rows)} takes in {made:.0f} s: {len(rows) - marked - len(bad)} clean, {marked} marked, "
               f"{len(bad)} handed over cut off")
    if bad:
        for r in bad:
            print(f"  {r}")
        smoke.fail("Booth handed over a take that stopped early without marking it")
    print("PASSED")


if __name__ == "__main__":
    sys.exit(main())
