#!/usr/bin/env python3
"""Smoke test for an installed Booth, run by .github/workflows/smoke.yml after each release.

It drives the booth-mcp program that ships with the app, the way Claude Desktop does, on a
machine with no ffmpeg: speak a line with a built-in voice, clone a new voice from that take,
then speak with the clone. That runs the shipped engine, Booth's own search for its audio
decoder and the decoder itself. With --decoder-check it then hides booth-decode and checks that
cloning fails, so a pass can't come from some other decoder.

Standard library only. Usage:
  smoke-release.py --mcp PATH --models DIR [--device auto|cpu] [--resource-dir DIR] [--decoder-check]
"""

import argparse
import hashlib
import json
import math
import os
import queue
import shutil
import struct
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LOCK = ROOT / "crates" / "booth-core" / "models.lock.json"
EXE = ".exe" if os.name == "nt" else ""
FIRST_LINE = (
    "Hello, this is a short test of Booth. It checks that a voice can be cloned on a computer "
    "that has never had ffmpeg, using only the files that ship with the app."
)
CLONE_LINE = "And this line is spoken by the voice that was just cloned."


def step(msg):
    print(f"\n== {msg}", flush=True)


def fail(msg):
    print(f"\nFAILED: {msg}", flush=True)
    sys.exit(1)


def fetch_models(dest):
    """The core pack from Booth's own model mirror, checked against the lock's sha256."""
    lock = json.loads(LOCK.read_text(encoding="utf-8"))
    for f in lock["packs"]["core"]:
        out = dest / f["rel"]
        if out.is_file() and out.stat().st_size == f["size"]:
            continue  # cached by an earlier run; it was checked when it was written
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
            fail(f"checksum mismatch for {f['rel']}")
        part.replace(out)


def path_without_ffmpeg():
    """PATH minus every folder that holds an ffmpeg."""
    dirs = [d for d in os.environ.get("PATH", "").split(os.pathsep) if d]
    keep = [d for d in dirs if not shutil.which("ffmpeg", path=d)]
    for d in set(dirs) - set(keep):
        if Path(d).resolve() in (Path("/usr/bin"), Path("/bin")):
            fail(f"ffmpeg is in {d}; uninstall it before this test")
        print(f"left off PATH (has ffmpeg): {d}")
    path = os.pathsep.join(keep)
    if shutil.which("ffmpeg", path=path):
        fail("ffmpeg is still reachable")
    return path


def wav_stats(path):
    """(seconds, rms) of a PCM16 or float32 WAV."""
    b = Path(path).read_bytes()
    if b[:4] != b"RIFF" or b[8:12] != b"WAVE":
        fail(f"{path} is not a WAV file")
    i, fmt, data = 12, None, b""
    while i + 8 <= len(b):
        cid, n = b[i : i + 4], struct.unpack("<I", b[i + 4 : i + 8])[0]
        body = b[i + 8 : i + 8 + n]
        if cid == b"fmt ":
            fmt = struct.unpack("<HHIIHH", body[:16])
        elif cid == b"data":
            data = body
        i += 8 + n + (n & 1)
    if not fmt:
        fail(f"{path} has no fmt chunk")
    tag, channels, rate, _, _, bits = fmt
    if tag == 0xFFFE:  # WAVE_FORMAT_EXTENSIBLE: the real format is in the sub-format GUID
        tag = 3 if bits == 32 else 1
    if tag == 3 and bits == 32:
        samples = struct.unpack(f"<{len(data) // 4}f", data[: len(data) // 4 * 4])
    elif tag == 1 and bits == 16:
        samples = [s / 32768 for s in struct.unpack(f"<{len(data) // 2}h", data[: len(data) // 2 * 2])]
    else:
        fail(f"{path}: unexpected WAV format {tag}/{bits}-bit")
    if not samples:
        return 0.0, 0.0
    rms = math.sqrt(sum(s * s for s in samples) / len(samples))
    return len(samples) / channels / rate, rms


def check_take(path, what):
    secs, rms = wav_stats(path)
    print(f"{what}: {path} ({secs:.1f} s, rms {rms:.3f})")
    if secs < 1.0 or rms < 0.01:
        fail(f"{what} is silent or too short")


class Mcp:
    """A minimal MCP client over stdio (newline-delimited JSON-RPC)."""

    def __init__(self, exe, env):
        self.p = subprocess.Popen([str(exe)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=env)
        self.lines = queue.Queue()
        self.n = 0
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.p.stdout:
            self.lines.put(line)
        self.lines.put(None)

    def _send(self, msg):
        self.p.stdin.write((json.dumps(msg) + "\n").encode())
        self.p.stdin.flush()

    def call(self, method, params, timeout):
        self.n += 1
        self._send({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params})
        deadline = time.monotonic() + timeout
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                fail(f"{method} took longer than {timeout} s")
            try:
                line = self.lines.get(timeout=left)
            except queue.Empty:
                continue
            if line is None:
                fail(f"booth-mcp exited during {method} (code {self.p.wait()})")
            try:
                msg = json.loads(line)
            except ValueError:
                continue
            if msg.get("id") == self.n and "method" not in msg:
                if "error" in msg:
                    fail(f"{method}: {msg['error']}")
                return msg["result"]

    def tool(self, name, args, timeout=900):
        t0 = time.monotonic()
        res = self.call("tools/call", {"name": name, "arguments": args}, timeout)
        text = "".join(c.get("text", "") for c in res.get("content", []))
        print(f"{name} ({time.monotonic() - t0:.0f} s){' ERROR' if res.get('isError') else ''}: {text[:600]}")
        return bool(res.get("isError")), text

    def close(self):
        self.p.stdin.close()
        try:
            self.p.wait(timeout=120)
        except subprocess.TimeoutExpired:
            print("booth-mcp did not exit after its input closed; ending it")
            self.p.kill()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--mcp", required=True, help="booth-mcp program to test")
    ap.add_argument("--models", required=True, help="folder for the core model pack (kept as a cache)")
    ap.add_argument("--device", default="auto", choices=["auto", "cpu"])
    ap.add_argument("--resource-dir", help="write install.json pointing here, as the app does on first run")
    ap.add_argument("--decoder-check", action="store_true", help="also check cloning fails without booth-decode")
    a = ap.parse_args()

    mcp = Path(a.mcp).resolve()
    if not mcp.is_file():
        fail(f"no booth-mcp at {mcp}")

    step("Model files")
    models = Path(a.models).resolve()
    fetch_models(models)
    print(f"core pack ready in {models}")

    step("No ffmpeg")
    path = path_without_ffmpeg()

    home = Path(tempfile.mkdtemp(prefix="booth-smoke-"))
    (home / "settings.json").write_text(json.dumps({"mcp_allow_cloning": True, "device": a.device}))
    if a.resource_dir:
        (home / "install.json").write_text(json.dumps({"resource_dir": str(Path(a.resource_dir).resolve())}))
    env = {k: v for k, v in os.environ.items() if not k.startswith("BOOTH_") and k != "ZONOS3_FFMPEG"}
    env.update(PATH=path, BOOTH_HOME=str(home), BOOTH_MODELS_DIR=str(models), RUST_BACKTRACE="1")

    step(f"Engine folder shipped next to {mcp.name}")
    base = Path(a.resource_dir).resolve() if a.resource_dir else mcp.parent
    engines = [base / n for n in ("engine-cpu", "engine-metal", "engine") if (base / n / f"zonos3-server{EXE}").is_file()]
    if not engines:
        fail(f"no engine folder in {base}")
    engine = engines[0]
    for f in sorted(engine.iterdir()):
        print(f"  {f.name}")
    decoder = engine / f"booth-decode{EXE}"
    if not decoder.is_file():
        fail(f"{decoder} is missing from the installed app")
    if os.name != "nt" and not os.access(decoder, os.X_OK):
        fail(f"{decoder} is not executable")

    s = Mcp(mcp, env)
    try:
        s.call(
            "initialize",
            {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "booth-smoke", "version": "1"}},
            60,
        )
        s._send({"jsonrpc": "2.0", "method": "notifications/initialized"})

        step("Status")
        s.tool("status", {}, 120)

        step("Speak with a built-in voice")
        err, text = s.tool("speak", {"text": FIRST_LINE})
        if err:
            fail("speak failed")
        first = json.loads(text)["file"]
        check_take(first, "first take")

        step("Clone a voice from that take (the engine decodes it with booth-decode)")
        err, text = s.tool(
            "create_voice",
            {"name": "Smoke clone", "audio_path": first, "speaker": "me", "consent_confirmed": True, "transcript": FIRST_LINE},
        )
        if err:
            fail("cloning failed")

        step("Speak with the clone")
        err, text = s.tool("speak", {"text": CLONE_LINE, "voice": "Smoke clone", "mode": "shapeable"})
        if err:
            fail("speaking with the clone failed")
        check_take(json.loads(text)["file"], "clone take")

        if a.decoder_check:
            step("Without booth-decode, cloning must fail")
            hidden = decoder.with_name(decoder.name + ".off")
            try:
                decoder.rename(hidden)
            except OSError as e:
                print(f"skipped: can't move {decoder} ({e})")
            else:
                try:
                    err, text = s.tool(
                        "create_voice",
                        {"name": "Smoke clone 2", "audio_path": first, "speaker": "me", "consent_confirmed": True},
                    )
                finally:
                    hidden.rename(decoder)
                if not err:
                    fail("cloning worked without booth-decode, so the test isn't using it")
                print("failed as expected, so the clone above went through booth-decode")
    finally:
        s.close()
    print("\nPASSED")


if __name__ == "__main__":
    main()
