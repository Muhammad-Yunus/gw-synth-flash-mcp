# gw-synth-flash-mcp

[![gw-synth-flash-mcp](https://img.shields.io/crates/v/gw-synth-flash-mcp.svg)](https://crates.io/crates/gw-synth-flash-mcp)
[![gw-synth-flash-mcp](https://docs.rs/gw-synth-flash-mcp/badge.svg)](https://docs.rs/gw-synth-flash-mcp)

`gw_sh`（Tcl実行）と `programmer_cli`（FPGA書き込み）を、**MCP(Model Context Protocol)** のツールとして提供する非公式MCPサーバーです。

- 実装: Rust + `rmcp`
- 目的: LLM/エディタから「任意Tcl実行」「ケーブル自動検出」「SRAM書き込み」を呼べるようにする
- 対象OS: Windows 11（現状）

## このリポの構成（想定）

公開リポとしては、以下のような構成（= リポ直下がcrate直下）を想定しています。

- `src/`: サーバー本体
- `examples/`: MCPクライアント接続用テンプレ
- `target/`: ビルド成果物（生成物、git管理外）

## 前提

- Gowin IDE が Windows 11 にインストールされていること
  - デフォルト: `C:\Gowin\Gowin_V1.9.11.03_Education_x64`
    - `IDE\bin\gw_sh.exe`
    - `Programmer\bin\programmer_cli.exe`
  - 変更したい場合: ツール引数の `gowin_ide_path` を指定

## インストール

crates.io からインストール:

```powershell
cargo install gw-synth-flash-mcp
```

## ソースからビルド（開発者向け）

```powershell
cargo build --release
```

ローカルソースからインストール:

```powershell
cargo install --path .
```

## 起動（stdio）

```powershell
.\target\release\gw-synth-flash-mcp.exe
```

または（インストール済みなら）:

```powershell
gw-synth-flash-mcp
```

このサーバーは各ツール呼び出しで `project_root` を基準にパス解決します。

- 推奨: 各ツール引数で `project_root` を明示する
- 便利: 起動時に環境変数 `GOWIN_MCP_PROJECT_ROOT` を設定する
- `project_root` 未指定の場合: `cwd` から自動検出（`run_gowin.tcl` / `*.gprj` を親方向に探索）し、見つからなければ `cwd` を使います

例:

```powershell
$env:GOWIN_MCP_PROJECT_ROOT = "C:\ABS\PATH\TO\your\project"
.\target\release\gw-synth-flash-mcp.exe
```

## クイックスタート

1) インストール

```sh
cargo install gw-synth-flash-mcp
```

1) MCPクライアント（VS Code/Copilotなど）の設定に、インストールしたバイナリを指定

- 例: `gw-synth-flash-mcp`（`$PATH` にインストールした場合）
- または: `${workspaceFolder}\target\release\gw-synth-flash-mcp.exe`（ソースからビルドした場合）

1) 操作したいGowinプロジェクトを `GOWIN_MCP_PROJECT_ROOT`（またはツール引数の `project_root`）で指定

## VS Code（GitHub Copilot）接続テンプレ

テンプレ: [examples/vscode.mcp.json](examples/vscode.mcp.json)

- `cargo install` でインストールした場合: `gw-synth-flash-mcp` コマンドを使用
- ソースからビルドした場合: `${workspaceFolder}\target\release\gw-synth-flash-mcp.exe` を使用
- `GOWIN_MCP_PROJECT_ROOT` を設定して、どこから起動しても同じ project を操作できるようにします

## Claude Code 接続テンプレ

テンプレ: [examples/claude-code.mcp.json](examples/claude-code.mcp.json)

- 変数展開が効かない場合に備えて **絶対パス**版
- `command` / `cwd` / `GOWIN_MCP_PROJECT_ROOT` の `C:\ABS\PATH\...` を置き換えてください

## ツール一覧

### `gowin.run_tcl`

- 任意の Tcl を `gw_sh` で実行します
- `tcl_path`（ファイル）か `tcl_inline`（文字列）のどちらかを指定
- `project_root` を指定すると、相対パスは `project_root` 基準で解決されます

### `gowin.list_cables`

- `programmer_cli` の列挙系オプションを複数パターン試行して、ケーブル名を抽出します

### `gowin.program_fs`

- `.fs` を SRAM へ書き込みます
- `cable` 未指定なら `list_cables` で検出したケーブルから自動選択します
- それでもダメなら `--cable` 省略で一度書き込みを試し、出力からケーブル名を推定して再試行します

## ログ

各ツール実行ごとに `<project_root>\.gowin-mcp\logs\` にログを保存します。

- `*.log`: stdout/stderr をまとめたテキスト
- `*.json`: 実行メタ情報（exit code, duration, 使用引数など）

## 注意

- 実機書き込みは FPGA に影響します（自己責任）。
- ビルド/書き込みをCI等で自動実行する場合、`project_root` と `gowin_ide_path` を明示して事故を避けてください。
