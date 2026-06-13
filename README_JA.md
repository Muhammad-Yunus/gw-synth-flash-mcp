# gw-synth-flash-mcp

[![gw-synth-flash-mcp](https://img.shields.io/crates/v/gw-synth-flash-mcp.svg)](https://crates.io/crates/gw-synth-flash-mcp)

Gowin EDA の CLI ツール（`gw_sh`、`programmer_cli`）を **MCP (Model Context Protocol)** のツールとして公開する非公式 MCP サーバーです。**Windows 11** + Gowin IDE V1.9.11 Education 向け。

- 実装: Rust + [`rmcp`](https://github.com/sigoden/rmcp)
- トランスポート: stdio (JSON-RPC)
- 対象OS: Windows 11

English README: [README.md](README.md)

## 目次

- [前提](#前提)
- [インストール](#インストール)
- [ソースからビルド](#ソースからビルド)
- [MCPクライアントの設定](#mcpクライアントの設定)
  - [Claude Code](#claude-code)
  - [VS Code (GitHub Copilot)](#vs-code-github-copilot)
- [MCPツール](#mcpツール)
  - [`gowin.list_cables`](#gowinlist_cables)
  - [`gowin.program_fs`](#gwinprogram_fs)
  - [`gowin.run_tcl`](#gwinrun_tcl)
- [Tang Nano 9K クイックリファレンス](#tang-nano-9k-クイックリファレンス)
- [ログ](#ログ)
- [注意](#注意)
- [検証済み環境](#検証済み環境)

## 前提

- **OS:** Windows 11
- **Gowin IDE** がインストールされていること、デフォルトパス:
  ```
  C:\Gowin\Gowin_V1.9.11.03_Education_x64\
    IDE\bin\gw_sh.exe
    Programmer\bin\programmer_cli.exe
  ```
  別パスにインストールした場合はツール引数の `gowin_ide_path` を指定してください。

- **Rust toolchain**（ソースからビルドする場合）:
  ```powershell
  cargo --version  # 1.84+ 推奨
  ```

## インストール

```powershell
cargo install gw-synth-flash-mcp
```

インストール先: `~/.cargo/bin/gw-synth-flash-mcp.exe`

## ソースからビルド

```powershell
cargo build --release
cargo install --path .
```

## MCPクライアントの設定

### Claude Code

プロジェクトルートに `.mcp.json` を作成または編集:

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

テンプレ: [examples/claude-code.mcp.json](examples/claude-code.mcp.json)

### VS Code (GitHub Copilot)

テンプレ: [examples/vscode.mcp.json](examples/vscode.mcp.json)

`GOWIN_MCP_PROJECT_ROOT` を Gowin プロジェクトのパスに設定してください。

## MCPツール

### `gowin.list_cables`

USB/JTAG 経由で接続されたプログラマケーブルを列挙します。

**リクエストパラメータ:**

| パラメータ | 型 | 説明 |
|---|---|---|
| `project_root` | string (optional) | ログ出力の基準ディレクトリ |
| `gowin_ide_path` | string (optional) | IDE パス上書き |
| `timeout_sec` | uint64 (optional) | タイムアウト秒数、デフォルト 20 |

**レスポンスフィールド:**

| フィールド | 型 | 説明 |
|---|---|---|
| `cables` | array | `CableInfo` オブジェクトのリスト |
| `attempts` | array | 各試行の引数と exit code |
| `log_file` | string | `.log` ログファイルのパス |
| `meta_file` | string | `.json` メタデータファイルのパス |

**CableInfo オブジェクト:**

| フィールド | 型 | 説明 |
|---|---|---|
| `name` | string | ケーブル名（例: `"USB Debugger A"`） |
| `index` | string (optional) | scan 出力からの 1-based ケーブルインデックス |
| `location` | string (optional) | USB ロケーション識別子 |

### `gowin.program_fs`

`.fs` ビットストリームを `programmer_cli` 経由で FPGA の SRAM に書き込みます。ケーブル自動選択・リトライ・フォールバック戦略を備えます。

**リクエストパラメータ:**

| パラメータ | 型 | 説明 |
|---|---|---|
| `project_root` | string (optional) | ログ出力の基準ディレクトリ |
| `fs_file_path` | string | `.fs` ビットストリームファイルのパス（相対 or 絶対） |
| `device` | string | FPGA デバイス名（例: `GW1NR-9C`, `GW5A-25A`） |
| `frequency` | string (optional) | JTAG クロック周波数、デフォルト `"15MHz"` |
| `retries` | uint32 (optional) | リトライ回数、デフォルト `2` |
| `timeout_sec` | uint64 (optional) | タイムアウト秒数、デフォルト `120` |
| `cable` | string (optional) | ケーブル名（例: `"USB Debugger A"`）。省略時は `"USB Debugger A"` をデフォルト使用 |
| `cable_index` | string (optional) | `--cable-index` 用ケーブル種別インデックス: `0`=GWU2X, `1`=FT2CH, `2`=LPT, `3`=Digilent, `4`=USB Debugger A |
| `location` | string (optional) | USB channel/location 番号（例: `"273"`） |
| `operation_index` | string (optional) | `2`=SRAM Program, `5`=embFlash Erase+Program。デフォルト `2` |
| `gowin_ide_path` | string (optional) | IDE パス上書き |

**operation_index 値:**

| 値 | 説明 |
|---|---|
| `2` | SRAM Program（揮発、~5–10秒） |
| `5` | embFlash Erase+Program（恒久フラッシュ、~30–60秒） |

**cable_index 値 (`--cable-index`):**

| 値 | ケーブル種別 |
|---|---|
| `0` | Gowin USB Cable (GWU2X) |
| `1` | Gowin USB Cable (FT2CH) |
| `2` | Parallel Port (LPT) |
| `3` | Digilent USB Device |
| `4` | USB Debugger A (Sipeed) |

### `gowin.run_tcl`

Tcl スクリプトを `gw_sh`（Gowin Tcl インタプリタ）で実行します。

**リクエストパラメータ:**

| パラメータ | 型 | 説明 |
|---|---|---|
| `project_root` | string (optional) | 基準ディレクトリ |
| `tcl_path` | string (optional) | Tcl ファイルパス |
| `tcl_inline` | string (optional) | インライン Tcl コード |
| `timeout_sec` | uint64 (optional) | タイムアウト秒数、デフォルト `1800` |
| `expected_files` | array (optional) | 実行後に存在チェックするファイルパスのリスト |
| `env` | map (optional) | 追加の環境変数 |
| `gowin_ide_path` | string (optional) | IDE パス上書き |

**レスポンスフィールド:**

| フィールド | 型 | 説明 |
|---|---|---|
| `exit_code` | int | `gw_sh` の exit code |
| `stdout` | string | スクリプトの標準出力 |
| `stderr` | string | スクリプトの標準エラー |
| `expected_checks` | array | `expected_files` の存在チェック結果 |
| `log_file` | string | `.log` ログファイルのパス |
| `meta_file` | string | `.json` メタデータファイルのパス |

## Tang Nano 9K クイックリファレンス

**Sipeed Tang Nano 9K** (Gowin GW1NR-9C) で使う場合:

```json
{
  "fs_file_path": "your_project.fs",
  "device": "GW1NR-9C",
  "cable_index": "4",
  "location": "273",
  "frequency": "15MHz"
}
```

- `cable_index: "4"` → `--cable-index 4`（USB Debugger A）
- `location: "273"` → `--channel 273`（scan 出力の USB channel）
- `device: "GW1NR-9C"` → Tang Nano 9K の正しい Gowin デバイス名
- `location` の値は `Cable found:` 行の 10 進数値（例: `USB Debugger A/0/273/null` → `273`）

## ログ

各ツール実行ごとに `<project_root>\.gowin-mcp\logs\` にログを保存します。

- `*.log`: stdout/stderr をまとめたテキスト
- `*.json`: 実行メタ情報（exit code, duration, 使用引数, 試行バリアントなど）

## 注意

- これは **非公式** ソフトウェアであり、**Gowin Semiconductor とは関係ありません**。
- 実機書き込みは FPGA に影響します。**自己責任** で行ってください。
- SRAM 書き込み（`operation_index: 2`）は揮発性で、電源オフで消えます。
- フラッシュ書き込み（`operation_index: 5`）は基板の SPI フラッシュに恒久的に書き込みます。

## 検証済み環境

以下で end-to-end 検証済み:

- **OS:** Windows 11 Home Single Language
- **Gowin IDE:** V1.9.11.03 Education
  ```
  C:\Gowin\Gowin_V1.9.11.03_Education_x64\
    IDE\bin\gw_sh.exe
    Programmer\bin\programmer_cli.exe
  ```
- **FPGA ボード:** Sipeed **Tang Nano 9K** (Gowin GW1NR-9C / GW1NR-LV9QN88PC6/I5)
- **ケーブル:** USB Debugger A（Sipeed、FT2232HL ベース）
  - `Cable found: USB Debugger A/0/273/null (USB location:273)`
- 3 ツールすべて検証済み: `run_tcl`, `list_cables`, `program_fs`

macOS / Linux は **サポート対象外**。異なるケーブル種別・IDE バージョンを使う場合は、ツール呼び出し時に `gowin_ide_path` を上書きしてください。
