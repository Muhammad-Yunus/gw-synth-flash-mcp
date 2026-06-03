# gw-synth-flash-mcp

[![gw-synth-flash-mcp](https://img.shields.io/crates/v/gw-synth-flash-mcp.svg)](https://crates.io/crates/gw-synth-flash-mcp)
[![gw-synth-flash-mcp](https://docs.rs/gw-synth-flash-mcp/badge.svg)](https://docs.rs/gw-synth-flash-mcp)

An unofficial MCP (Model Context Protocol) server that exposes a few Gowin IDE CLI workflows as tools.

- Backend: Rust + `rmcp`
- Provides MCP tools (tool names are kept as `gowin.*` for compatibility)
- Current target OS: Windows 11

Japanese README: [README_JA.md](README_JA.md)

## Repo layout (assumed)

This repository is intended to be a standalone Rust crate:

- `src/`: MCP server implementation
- `examples/`: MCP client configuration templates
- `target/`: build artifacts (generated; not tracked)

## Prerequisites

- Gowin IDE installed on Windows 11
  - Default location: `C:\Gowin\Gowin_V1.9.11.03_Education_x64`
    - `IDE\bin\gw_sh.exe`
    - `Programmer\bin\programmer_cli.exe`
  - If different: pass `gowin_ide_path` in tool parameters

## Install

Install from crates.io:

```powershell
cargo install gw-synth-flash-mcp
```

## Build from source (for development)

```powershell
cargo build --release
```

Optional: install from local source:

```powershell
cargo install --path .
```

## Run (stdio)

```powershell
.\target\release\gw-synth-flash-mcp.exe
```

Or, if installed:

```powershell
gw-synth-flash-mcp
```

This server resolves relative paths based on a `project_root`.

Priority order:

1. Per-tool parameter: `project_root`
2. Environment variable: `GOWIN_MCP_PROJECT_ROOT`
3. Auto-detect from `cwd` by searching upward for `run_gowin.tcl` or `*.gprj`
4. Fallback: `cwd`

Example:

```powershell
$env:GOWIN_MCP_PROJECT_ROOT = "C:\ABS\PATH\TO\your\gowin\project"
.\target\release\gw-synth-flash-mcp.exe
```

## Quick start

1) Install

```powershell
cargo install gw-synth-flash-mcp
```

2) Point your MCP client (e.g., VS Code/Copilot) at the installed binary

- Example: `gw-synth-flash-mcp` (if installed in `$PATH`)
- Or: `${workspaceFolder}\target\release\gw-synth-flash-mcp.exe` (if built from source)

3) Set `GOWIN_MCP_PROJECT_ROOT` (or pass `project_root` per tool call)

## VS Code (GitHub Copilot) template

Template: [examples/vscode.mcp.json](examples/vscode.mcp.json)

- If installed via `cargo install`: use `gw-synth-flash-mcp` command
- If built from source: use `${workspaceFolder}\target\release\gw-synth-flash-mcp.exe`
- Set `GOWIN_MCP_PROJECT_ROOT` to your Gowin project directory

## Claude Code template

Template: [examples/claude-code.mcp.json](examples/claude-code.mcp.json)

- Absolute-path template (replace `C:\ABS\PATH\...` values)

## Tools

### `gowin.run_tcl`

- Runs Tcl via `gw_sh`
- Provide either `tcl_path` (file) or `tcl_inline` (string)
- If `project_root` is set, relative paths resolve under it
- `gowin_ide_path` overrides the default `C:\Gowin\Gowin_V1.9.11.03_Education_x64`

### `gowin.list_cables`

- Enumerates available programmer cables via `programmer_cli` (tries multiple listing patterns)

### `gowin.program_fs`

- Programs a `.fs` bitstream into SRAM via `programmer_cli`
- If `cable` is omitted, it auto-selects from `list_cables`
- If needed, it retries with different cable inference strategies

## Logs

Each tool call writes logs under `<project_root>\.gowin-mcp\logs\`:

- `*.log`: combined stdout/stderr
- `*.json`: execution metadata (exit code, duration, args, etc.)

## Safety / Disclaimer

- This is unofficial software and is not affiliated with Gowin.
- Programming hardware can affect your FPGA/board. Use at your own risk.

## Tested

This server is **Windows 11 only** and has been validated end-to-end against the following setup:

- **OS:** Windows 11 (single-language edition)
- **Gowin IDE:** `C:\Gowin\Gowin_V1.9.11.03_Education_x64`
  - `IDE\bin\gw_sh.exe`
  - `Programmer\bin\programmer_cli.exe`
- **FPGA board:** Sipeed **Tang Nano 9K** (Gowin GW1NR-9C)
  - `gowin.run_tcl` and `gowin.program_fs` exercised against this board.
  - `gowin.list_cables` validated with the Tang Nano 9K's FT2232HL-based programmer attached.

If you use a different board, cable, or IDE version you may need to override `gowin_ide_path` per tool call. Other operating systems (macOS, Linux) are **not** supported by this build.
