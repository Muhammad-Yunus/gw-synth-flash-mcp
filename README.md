# gw-synth-flash-mcp

[![gw-synth-flash-mcp](https://img.shields.io/crates/v/gw-synth-flash-mcp.svg)](https://crates.io/crates/gw-synth-flash-mcp)

An unofficial MCP (Model Context Protocol) server that exposes Gowin EDA CLI tools (`gw_sh`, `programmer_cli`) as MCP tools. Designed for **Windows 11** with Gowin IDE V1.9.11 Education.

- Backend: Rust + [`rmcp`](https://github.com/sigoden/rmcp)
- Transport: stdio (JSON-RPC)
- Target OS: Windows 11

Japanese README: [README_JA.md](README_JA.md)

## Table of Contents

- [Prerequisites](#prerequisites)
- [Install](#install)
- [Build from source](#build-from-source)
- [Configure MCP client](#configure-mcp-client)
  - [Claude Code](#claude-code)
  - [VS Code (GitHub Copilot)](#vs-code-github-copilot)
- [MCP Tools](#mcp-tools)
  - [`gowin.list_cables`](#gwinlist_cables)
  - [`gowin.program_fs`](#gwinprogram_fs)
  - [`gowin.run_tcl`](#gwinrun_tcl)
- [Tang Nano 9K quick reference](#tang-nano-9k-quick-reference)
- [Logs](#logs)
- [Safety / Disclaimer](#safety--disclaimer)
- [Tested setup](#tested-setup)

## Prerequisites

- **OS:** Windows 11
- **Gowin IDE** installed, default location:
  ```
  C:\Gowin\Gowin_V1.9.11.03_Education_x64\
    IDE\bin\gw_sh.exe
    Programmer\bin\programmer_cli.exe
  ```
  If installed elsewhere, pass `gowin_ide_path` in tool parameters.

- **Rust toolchain** (for building from source):
  ```powershell
  cargo --version  # 1.84+ recommended
  ```

## Install

```powershell
cargo install gw-synth-flash-mcp
```

Installed binary: `~/.cargo/bin/gw-synth-flash-mcp.exe`

## Build from source

```powershell
cargo build --release
cargo install --path .
```

## Configure MCP client

### Claude Code

In your project root, create or edit `.mcp.json`:

```json
{
  "mcpServers": {
    "gowin": {
      "command": "gw-synth-flash-mcp",
      "args": [],
      "cwd": "${workspaceFolder}",
      "env": {
        "GOWIN_MCP_PROJECT_ROOT": "C:\\D\\MY\\DEV\\TangNano\\TangNano-9K-example"
      }
    }
  }
}
```

Or use the template: [examples/claude-code.mcp.json](examples/claude-code.mcp.json)

### VS Code (GitHub Copilot)

Use the template: [examples/vscode.mcp.json](examples/vscode.mcp.json)

Set `GOWIN_MCP_PROJECT_ROOT` to your Gowin project directory.

## MCP Tools

### `gowin.list_cables`

Enumerates available programming cables connected via USB/JTAG.

**Request parameters:**

| Parameter | Type | Description |
|---|---|---|
| `project_root` | string (optional) | Base directory for log output |
| `gowin_ide_path` | string (optional) | Override default IDE path |
| `timeout_sec` | uint64 (optional) | Timeout in seconds, default 20 |

**Response fields:**

| Field | Type | Description |
|---|---|---|
| `cables` | array | List of `CableInfo` objects |
| `attempts` | array | Each attempt's arguments and exit code |
| `log_file` | string | Path to `.log` output |
| `meta_file` | string | Path to `.json` metadata |

**CableInfo object:**

| Field | Type | Description |
|---|---|---|
| `name` | string | Cable name (e.g. `"USB Debugger A"`) |
| `index` | string (optional) | 1-based cable index from scan output |
| `location` | string (optional) | USB location identifier |

### `gowin.program_fs`

Programs a `.fs` bitstream into FPGA SRAM via `programmer_cli`. Supports cable auto-selection, retry, and fallback strategies.

**Request parameters:**

| Parameter | Type | Description |
|---|---|---|
| `project_root` | string (optional) | Base directory for log output |
| `fs_file_path` | string | Path to `.fs` bitstream file (relative or absolute) |
| `device` | string | FPGA device name (e.g. `GW1NR-9C`, `GW5A-25A`) |
| `frequency` | string (optional) | JTAG clock frequency, default `"15MHz"` |
| `retries` | uint32 (optional) | Number of retries, default `2` |
| `timeout_sec` | uint64 (optional) | Timeout in seconds, default `120` |
| `cable` | string (optional) | Cable name (e.g. `"USB Debugger A"`). If omitted, `"USB Debugger A"` is used by default |
| `cable_index` | string (optional) | Cable type index for `--cable-index`: `0`=GWU2X, `1`=FT2CH, `2`=LPT, `3`=Digilent, `4`=USB Debugger A |
| `location` | string (optional) | USB channel/location number (e.g. `"273"`) |
| `operation_index` | string (optional) | `2`=SRAM Program, `5`=embFlash Erase+Program. Default `2` |
| `gowin_ide_path` | string (optional) | Override default IDE path |

**Operation index values:**

| Value | Description |
|---|---|
| `2` | SRAM Program (volatile, ~5–10s) |
| `5` | embFlash Erase+Program (permanent flash, ~30–60s) |

**Cable index values (`--cable-index`):**

| Value | Cable Type |
|---|---|
| `0` | Gowin USB Cable (GWU2X) |
| `1` | Gowin USB Cable (FT2CH) |
| `2` | Parallel Port (LPT) |
| `3` | Digilent USB Device |
| `4` | USB Debugger A (Sipeed) |

### `gowin.run_tcl`

Executes Tcl scripts via `gw_sh` (the Gowin Tcl interpreter).

**Request parameters:**

| Parameter | Type | Description |
|---|---|---|
| `project_root` | string (optional) | Base directory |
| `tcl_path` | string (optional) | Path to Tcl file |
| `tcl_inline` | string (optional) | Inline Tcl code |
| `timeout_sec` | uint64 (optional) | Timeout in seconds, default `1800` |
| `expected_files` | array (optional) | File paths to check existence after execution |
| `env` | map (optional) | Additional environment variables |
| `gowin_ide_path` | string (optional) | Override default IDE path |

**Response fields:**

| Field | Type | Description |
|---|---|---|
| `exit_code` | int | Exit code from `gw_sh` |
| `stdout` | string | Script output |
| `stderr` | string | Script errors |
| `expected_checks` | array | Existence checks for `expected_files` |
| `log_file` | string | Path to `.log` output |
| `meta_file` | string | Path to `.json` metadata |

## Tang Nano 9K quick reference

For **Sipeed Tang Nano 9K** (Gowin GW1NR-9C), use these values:

```json
{
  "fs_file_path": "your_project.fs",
  "device": "GW1NR-9C",
  "cable_index": "4",
  "location": "273",
  "frequency": "15MHz"
}
```

- `cable_index: "4"` → `--cable-index 4` (USB Debugger A)
- `location: "273"` → `--channel 273` (USB channel from scan output)
- `device: "GW1NR-9C"` → correct Gowin device name for GW1NR-9 on Tang Nano 9K
- The `location` value is the decimal number from the `Cable found:` line (e.g. `USB Debugger A/0/273/null` → `273`)

## Logs

Each tool call writes logs under `<project_root>\.gowin-mcp\logs\`:

- `*.log`: combined stdout/stderr text
- `*.json`: execution metadata (exit code, duration, arguments, variant info)

## Safety / Disclaimer

- This is **unofficial** software and is **not affiliated with Gowin Semiconductor**.
- Programming hardware can affect your FPGA/board configuration. **Use at your own risk.**
- SRAM programming (`operation_index: 2`) is volatile — content is lost on power cycle.
- Flash programming (`operation_index: 5`) writes permanent configuration to the board's SPI flash.

## Tested setup

Validated end-to-end against:

- **OS:** Windows 11 Home Single Language
- **Gowin IDE:** V1.9.11.03 Education
  ```
  C:\Gowin\Gowin_V1.9.11.03_Education_x64\
    IDE\bin\gw_sh.exe
    Programmer\bin\programmer_cli.exe
  ```
- **FPGA board:** Sipeed **Tang Nano 9K** (Gowin GW1NR-9C / GW1NR-LV9QN88PC6/I5)
- **Cable:** USB Debugger A (Sipeed, FT2232HL-based)
  - `Cable found: USB Debugger A/0/273/null (USB location:273)`
- All three tools exercised: `run_tcl`, `list_cables`, `program_fs`

Other OSes (macOS, Linux) are **not** supported. Different cable types or IDE versions may require overriding `gowin_ide_path` per tool call.
