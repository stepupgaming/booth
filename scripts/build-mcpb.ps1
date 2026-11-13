# Builds booth-mcp in release mode and packs it into dist/booth.mcpb for Claude Desktop.
# Runs on Windows and macOS (pwsh). The bundle holds the binary for the platform it was built on.
# Usage: pwsh scripts/build-mcpb.ps1
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

cargo build --release -p booth-mcp
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

$exe = if ($IsWindows) { 'booth-mcp.exe' } else { 'booth-mcp' }
$platform = if ($IsWindows) { 'win32' } elseif ($IsMacOS) { 'darwin' } else { 'linux' }

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$stage = Join-Path $root 'target/mcpb'
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force (Join-Path $stage 'server') | Out-Null

# Keep the bundle's version in step with the workspace version, and point it at this platform's binary.
$manifest = Get-Content packaging/mcpb/manifest.json -Raw | ConvertFrom-Json
$manifest.version = $version
$manifest.server.entry_point = "server/$exe"
$manifest.server.mcp_config.command = '${__dirname}/server/' + $exe
$manifest.compatibility.platforms = @($platform)
$manifest | ConvertTo-Json -Depth 10 | Set-Content (Join-Path $stage 'manifest.json') -Encoding utf8NoBOM
Copy-Item packaging/mcpb/icon.png $stage
Copy-Item (Join-Path 'target/release' $exe) (Join-Path $stage 'server')

New-Item -ItemType Directory -Force dist | Out-Null
$out = Join-Path $root 'dist/booth.mcpb'
bunx --bun @anthropic-ai/mcpb validate (Join-Path $stage 'manifest.json')
if ($LASTEXITCODE -ne 0) { throw 'manifest failed validation' }
bunx --bun @anthropic-ai/mcpb pack $stage $out
if ($LASTEXITCODE -ne 0) { throw 'mcpb pack failed' }
Write-Host "Built $out"
