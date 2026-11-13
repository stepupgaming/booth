<p align="center"><img src="assets/booth-logo.png" width="96" alt="Booth"></p>

# Booth

Free, open-source text to speech for Windows. Type a line, pick a voice and a mood, and get takes to
choose from. Booth runs [ZONOS3-Mini](https://huggingface.co/Zyphra/ZONOS3-Mini) by Zyphra on your own
NVIDIA graphics card, so nothing you write leaves your PC. Claude can use it too.

**[Download Booth-Setup.exe](https://github.com/stepupgaming/booth/releases/latest/download/Booth-Setup.exe)**
· [Website](https://nofacetube.com/tools/booth/) · [Releases](https://github.com/stepupgaming/booth/releases)

## What it does

- Preset voices with mood, pace, energy and delivery controls
- Several takes per line, so you pick the best read
- Scripts: whole scripts rendered line by line, each with its own voice and direction
- Voice cloning from a short clip, only of voices you have permission to use
- Voice design from a preset: shift gender, age, texture, accent and room
- Claude support: a one-click Claude Desktop extension (`booth.mcpb`) and an MCP server (`booth-mcp.exe`)

## Requirements

- Windows 10 or 11, 64-bit
- An NVIDIA graphics card (RTX 20 series or newer) with a recent driver
- About 1.2 GB of disk for the voice model, up to 9 GB with every add-on

The installer holds the app and the voice engine. The model weights download from Hugging Face on
first launch. Your voices, takes and settings live in `%LOCALAPPDATA%\NoFaceTube\Booth`.

Windows SmartScreen may warn that the installer is from an unknown publisher, because it is not
code-signed yet. Choose **More info**, then **Run anyway**.

## Use it from Claude

In Booth, open **Settings** and click **Add to Claude Desktop**. Claude can then list voices, speak
lines and render scripts. The **Claude Code** tab next to it gives the `claude mcp add` command.

## Build from source

Needs Node 20+, pnpm, Rust stable, Bun, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

```bash
pnpm install
pnpm tauri dev                 # the app, against an engine in BOOTH_ENGINE_DIR
scripts/ui-preview.cmd         # the UI alone in a browser, on sample data, no GPU
pwsh scripts/build-mcpb.ps1    # booth-mcp.exe and dist/booth.mcpb
```

The voice engine is `zonos3-server.exe` from Zyphra's
[ZONOS3-Mini](https://github.com/Zyphra/ZONOS3-Mini) repo (`servers/ggml`), built with CUDA.
`.github/workflows/release.yml` shows the exact build. Pushing a `v*` tag builds everything and
attaches `Booth-Setup.exe` to that release.

| Folder              | What                                                     |
| ------------------- | -------------------------------------------------------- |
| `apps/native`       | The Tauri app                                            |
| `packages/core/src/booth` | The UI                                             |
| `crates/booth-core` | Engine control, voices, controls, model downloads        |
| `crates/booth-mcp`  | The MCP server Claude uses                               |
| `packaging/mcpb`    | The Claude Desktop extension manifest                    |

## Credits and license

Booth is made by [NoFaceTube](https://nofacetube.com) and released under the Apache License 2.0
(see `LICENSE`). The ZONOS3-Mini model and engine are by [Zyphra](https://www.zyphra.com) under the
Apache 2.0 (weights) and MIT (code) licenses; the installer includes the engine's license as
`engine/LICENSE-ZONOS3.txt`. It also bundles NVIDIA CUDA runtime libraries under NVIDIA's
redistribution terms.

Clone only voices you own or have permission to use, and label AI voiceover where your platform
asks you to.
