# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Gowin IDE の CLI ワークフロー (`gw_sh`, `programmer_cli`) を MCP (Model Context Protocol) ツールとして公開する非公式サーバー。Rust 2024 Edition で書かれた単一バイナリ (`gw-synth-flash-mcp`)。Windows 11 専用。

## Build & Test Commands

```powershell
# ビルド
cargo build --release

# テスト（スモークテストのみ実行、ハードウェア不要）
cargo test

# ハードウェア接続が必要な統合テスト（実際のFPGAデバイスが必要）
cargo test -- --ignored

# インストール
cargo install --path .
```

## Architecture

全コードは `src/main.rs` の単一ファイル（約900行）に収まっている。

### MCP Server 構成

- **フレームワーク**: `rmcp` クレートの `#[tool_router]` マクロで宣言的にツールを定義
- **トランスポート**: stdio ベースの JSON-RPC
- **ランタイム**: Tokio 非同期ランタイム

### 3つのMCPツール

1. **`gowin.run_tcl`** — `gw_sh` 経由で Tcl スクリプトを実行（タイムアウト: 30分）
2. **`gowin.list_cables`** — プログラマケーブルの列挙。`programmer_cli` のバージョン差を吸収するため複数の引数パターン (`--list-cables`, `--list_cables`, `--cableList` 等) を順番に試行する
3. **`gowin.program_fs`** — `.fs` ビットストリームを SRAM に書き込み。ケーブル自動検出、リトライ、複数バリアント試行のフォールバック戦略を持つ

### 主要な内部関数

- `resolve_project_root()` — プロジェクトルート解決（引数 → 環境変数 `GOWIN_MCP_PROJECT_ROOT` → cwd から上方探索 → cwd フォールバック）
- `gw_sh_env()` — Windows 向けに `PATH`（先頭に `IDE\bin` と `Programmer\bin` を追加）、`TCL_LIBRARY`、`TCLLIBPATH` を構築
- `gowin_paths()` — Gowin IDE パス解決（デフォルト: `C:\Gowin\Gowin_V1.9.11.03_Education_x64`）
- `exec_with_timeout()` — タイムアウト付き非同期サブプロセス実行。全ツール共通
- `write_run_logs()` — `.gowin-mcp/logs/` にログ（`.log` + `.json`メタデータ）を書き出し
- `parse_cable_names()` — `programmer_cli` の出力からケーブル名を抽出するパーサー

### Windows 固有の前提

- 想定パス: `C:\Gowin\Gowin_V1.9.11.03_Education_x64\`
  - `IDE\bin\gw_sh.exe`
  - `IDE\lib\`（Tcl/itcl/tcl8.6 のスタブ）
  - `Programmer\bin\programmer_cli.exe`
- パス区切り文字は `\`、環境変数のリスト区切りは `;` を使用
- `PATH` を `cmd.exe` の慣例どおり `;` で連結
- 絶対パスは `r"C:\..."` の raw string として記述

### 設計方針

- **フォールバック重視**: ツールバージョン差を複数パターン試行で吸収
- **全実行ログ記録**: デバッグ用に `.gowin-mcp/logs/` へ自動保存
- **タイムアウト保護**: 全ツールにデフォルトタイムアウトあり
- **エラーハンドリング**: `anyhow::Result<T>` + MCP `ErrorData` への変換

### テスト構成

- `tests/mcp_smoke.rs` — MCPサーバーをchild processとして起動し `list_tools` RPC を検証
- `tests/mcp_hardware.rs` — 実ハードウェア接続テスト（`#[ignore]`、通常は実行しない）
