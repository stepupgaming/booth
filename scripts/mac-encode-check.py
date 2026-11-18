#!/usr/bin/env python3
"""Exact-mode reference encode on a Mac, run by .github/workflows/mac-encode.yml.

A Mac (M2) user saw "reference audio encoding failed (maximum 60 seconds)" when making a take
in Exact mode. This runs the engine that shipped in the app, with nothing in between, and asks
it to make one line in Exact mode from a 10, 45 and 60 second reference clip, first on Metal
(--gpu), then on the processor (--cpu). After each run it prints the engine's log.

A pass on the processor says nothing about Metal. The summary says whether Metal was there at
all: GitHub's Macs are virtual machines and may not offer it.

With --mcp it then runs the booth-mcp built from this branch with Run on set to auto (Metal),
clones a voice from the 60 second clip and speaks with it in Exact mode, the way the app does.
If Metal fails the encode, Booth should restart the engine on the processor and the take should
still come out.

Standard library only. Usage:
  mac-encode-check.py --engine DIR --models DIR [--mcp PATH] [--metal yes|no|unknown]
"""

import argparse
import base64
import hashlib
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import wave
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LOCK = ROOT / "crates" / "booth-core" / "models.lock.json"
LINE = "This line is made in Exact mode from a long reference recording."
CLIP_SECONDS = (10, 45, 60)
PORT = 18765
EXE = ".exe" if os.name == "nt" else ""


def step(msg):
    print(f"\n== {msg}", flush=True)


def fetch(dest, packs):
    """Model packs from Booth's own mirror, checked against the lock's sha256."""
    lock = json.loads(LOCK.read_text(encoding="utf-8"))
    for pack in packs:
        for f in lock["packs"][pack]:
            out = dest / f["rel"]
            if out.is_file() and out.stat().st_size == f["size"]:
                continue
            out.parent.mkdir(parents=True, exist_ok=True)
            part = out.with_name(out.name + ".part")
            h = hashlib.sha256()
            with open(part, "wb") as w:
                for p in f["mirror"]:
                    with urllib.request.urlopen(f"{lock['mirror']}/{p['name']}", timeout=120) as r:
                        while chunk := r.read(1 << 20):
                            h.update(chunk)
                            w.write(chunk)
            if h.hexdigest() != f["sha256"]:
                sys.exit(f"checksum mismatch for {f['rel']}")
            part.replace(out)


def make_clips(models, out):
    """Speech clips of each length: a built-in voice's sample, repeated with short gaps."""
    for src in sorted((models / "voices" / "English").glob("*.wav")):
        try:
            with wave.open(str(src)) as w:
                if w.getsampwidth() != 2:
                    continue
                rate, ch, frames = w.getframerate(), w.getnchannels(), w.readframes(w.getnframes())
        except wave.Error:
            continue
        print(f"clips made from {src.name} ({rate} Hz, {ch} ch, {len(frames) / (2 * ch * rate):.1f} s)")
        gap = b"\0" * (2 * ch * int(rate * 0.4))
        clips = {}
        for secs in CLIP_SECONDS:
            need = 2 * ch * rate * secs
            data = b""
            while len(data) < need:
                data += frames + gap
            path = out / f"ref-{secs}s.wav"
            with wave.open(str(path), "wb") as w:
                w.setnchannels(ch)
                w.setsampwidth(2)
                w.setframerate(rate)
                w.writeframes(data[:need])
            clips[secs] = path
        return clips
    sys.exit("no 16-bit WAV among the built-in English voices to make clips from")


def server_args(engine, models, flag):
    m = lambda n: str(models / n)  # noqa: E731
    args = [str(engine / f"zonos3-server{EXE}"), m("backbone-ptq.gguf"), flag, "--host", "127.0.0.1", "--port", str(PORT)]
    args += ["--batch", "2", "--tts-default-voices-dir", m("voices"), "--spk-encoder", m("spk-encoder.gguf")]
    args += ["--steer", m("steer.gguf"), "--zac-encoder", m("zac-encoder.gguf"), "--ssl-teacher", m("ssl-teacher.gguf")]
    return args


def http(method, path, body=None, timeout=900):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}{path}", data=data, method=method)
    req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()


def run_engine(engine, models, clips, accel, logs):
    """Start the engine on one device and ask for an Exact take from each clip."""
    flag = "--gpu" if accel == "metal" else "--cpu"
    log_path = logs / f"engine-{accel}.log"
    env = dict(os.environ, ZONOS3_FFMPEG=str(engine / f"booth-decode{EXE}"))
    results = {}
    with open(log_path, "wb") as log:
        p = subprocess.Popen(server_args(engine, models, flag), cwd=engine, env=env, stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 600
            while True:
                if p.poll() is not None:
                    results["start"] = f"engine exited with code {p.returncode} before it was ready"
                    break
                try:
                    if http("GET", "/v2/health", timeout=5)[0] == 200:
                        break
                except OSError:
                    pass
                if time.monotonic() > deadline:
                    results["start"] = "engine not ready after 600 s"
                    break
                time.sleep(2)
            if "start" not in results:
                for secs, clip in clips.items():
                    t0 = time.monotonic()
                    body = {
                        "text": LINE,
                        "speaker_mode": "reference",
                        "speaker_audio_base64": base64.b64encode(clip.read_bytes()).decode(),
                        "speaker_audio_name": clip.name,
                        "seed": 1,
                    }
                    try:
                        status, out = http("POST", "/v2/generate", body)
                    except OSError as e:
                        status, out = 0, str(e).encode()
                    took = time.monotonic() - t0
                    if status == 200 and out[:4] == b"RIFF":
                        results[secs] = f"ok in {took:.0f} s ({len(out) // 1024} KB of audio)"
                    else:
                        results[secs] = f"FAILED in {took:.0f} s: HTTP {status}: {out[:400].decode(errors='replace')}"
                    print(f"  {accel} {secs:>2} s clip: {results[secs]}", flush=True)
                    if p.poll() is not None:
                        results["crash"] = f"engine exited with code {p.returncode}"
                        break
        finally:
            if p.poll() is None:
                p.terminate()
                try:
                    p.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    p.kill()
    step(f"engine log, {accel}")
    print(log_path.read_text(errors="replace")[-12000:], flush=True)
    return results


def run_mcp(mcp_exe, engine, models, clip):
    """The branch's booth-mcp on auto (Metal): clone from the clip, speak in Exact mode."""
    spec = importlib.util.spec_from_file_location("smoke", ROOT / "scripts" / "smoke-release.py")
    smoke = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(smoke)
    home = Path(tempfile.mkdtemp(prefix="booth-mac-encode-"))
    (home / "settings.json").write_text(json.dumps({"mcp_allow_cloning": True, "device": "auto"}))
    env = {k: v for k, v in os.environ.items() if not k.startswith("BOOTH_") and k != "ZONOS3_FFMPEG"}
    env.update(BOOTH_HOME=str(home), BOOTH_MODELS_DIR=str(models), BOOTH_ENGINE_DIR=str(engine), RUST_BACKTRACE="1")
    s = smoke.Mcp(mcp_exe, env)
    ok = False
    try:
        info = {"name": "booth-mac-encode", "version": "1"}
        s.call("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": info}, 60)
        s._send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        err, _ = s.tool(
            "create_voice",
            {"name": "Long clip", "audio_path": str(clip), "speaker": "me", "consent_confirmed": True},
        )
        if not err:
            err, text = s.tool("speak", {"text": LINE, "voice": "Long clip", "mode": "exact"})
            ok = not err
            if err and "maximum 60 seconds" in text:
                print("the engine's own words reached the caller; they should have been replaced")
                ok = False
    finally:
        s.close()
    logs = home / "logs"
    for name in ("booth.log", "engine.prev.log", "engine.log"):
        if (logs / name).is_file():
            step(f"booth-mcp run: {name}")
            print((logs / name).read_text(errors="replace")[-8000:], flush=True)
    return ok


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--engine", required=True, help="the engine-metal folder from Booth.app")
    ap.add_argument("--models", required=True, help="model folder (kept as a cache)")
    ap.add_argument("--mcp", help="a booth-mcp built from this branch")
    ap.add_argument("--metal", default="unknown", help="whether this Mac reports a Metal device")
    ap.add_argument("--devices", default="metal,cpu", help="which to run, for a local try: metal,cpu")
    a = ap.parse_args()
    engine, models = Path(a.engine).resolve(), Path(a.models).resolve()

    step("Models: core and Exact packs")
    fetch(models, ["core", "exact"])
    work = Path(tempfile.mkdtemp(prefix="booth-clips-"))
    clips = make_clips(models, work)

    summary = {}
    for accel in a.devices.split(","):
        step(f"Engine on {accel}")
        summary[accel] = run_engine(engine, models, clips, accel, work)

    mcp_ok = None
    if a.mcp:
        step("Branch booth-mcp, Run on auto, Exact take from the 60 s clip")
        mcp_ok = run_mcp(Path(a.mcp).resolve(), engine, models, clips[60])

    step("Summary")
    print(f"Metal device on this machine: {a.metal}")
    for accel, res in summary.items():
        for k, v in res.items():
            print(f"  {accel:5} {k if isinstance(k, str) else f'{k} s clip':12} {v}")
    if mcp_ok is not None:
        print(f"  branch booth-mcp, Exact on auto: {'ok' if mcp_ok else 'FAILED'}")
    metal_ok = all(str(v).startswith("ok") for v in summary.get("metal", {}).values())
    cpu_ok = all(str(v).startswith("ok") for v in summary.get("cpu", {}).values())
    if a.metal != "yes":
        print("No Metal device was confirmed here, so the Metal rows don't stand for a real Mac.")
    elif metal_ok:
        print("Metal passed on this machine. A pass on a GitHub VM doesn't clear real Macs either.")
    else:
        print("Metal failed here: the bug reproduces.")
    if not cpu_ok or mcp_ok is False:
        sys.exit(1)


if __name__ == "__main__":
    main()
