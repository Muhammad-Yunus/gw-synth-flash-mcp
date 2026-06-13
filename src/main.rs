use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow};
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::tool::ToolRouter,
    handler::server::wrapper::{Json, Parameters},
    model::*,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::{
    io::AsyncReadExt,
    process::Command,
    time::{Instant, timeout},
};

const DEFAULT_GOWIN_IDE_PATH: &str = r"C:\Gowin\Gowin_V1.9.11.03_Education_x64";
const DEFAULT_PROJECT_ROOT_ENV: &str = "GOWIN_MCP_PROJECT_ROOT";

fn log_warning(msg: &str) {
    // stderr に出してログファイル (.gowin-mcp/logs/*.log) にも拾われる。
    eprintln!("[gowin-mcp WARN] {msg}");
}
const KILL_WAIT_TIMEOUT_SEC: u64 = 10;
const MAX_OUTPUT_BYTES: u64 = 10 * 1024 * 1024; // 10 MB

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct ExpectedFileCheck {
    path: String,
    exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct ExecMeta {
    exit_code: i32,
    timed_out: bool,
    duration_ms: u128,
    stdout: String,
    stderr: String,
}

async fn ensure_dir(dir: &Path) -> Result<()> {
    tokio::fs::create_dir_all(dir)
        .await
        .with_context(|| format!("create_dir_all({})", dir.display()))
}

fn stamp() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis();
    ms.to_string()
}

fn safe_file_stem(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn detect_project_root(start_dir: &Path) -> Option<PathBuf> {
    // start_dir から親へ辿り、Gowinプロジェクトの「目印」を探す。
    // - run_gowin.tcl がある
    // - *.gprj がある
    // 見つからない場合は None。
    let mut current = Some(start_dir);
    while let Some(dir) = current {
        if dir.join("run_gowin.tcl").is_file() {
            return Some(dir.to_path_buf());
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_file()
                    && let Some(ext) = p.extension()
                    && ext == "gprj"
                {
                    return Some(dir.to_path_buf());
                }
            }
        }

        current = dir.parent();
    }
    None
}

async fn resolve_project_root(explicit: Option<&str>) -> PathBuf {
    // 優先順位:
    // 1) リクエストの project_root
    // 2) 環境変数 GOWIN_MCP_PROJECT_ROOT
    // 3) cwd から自動検出
    // 4) cwd
    if let Some(p) = explicit {
        return PathBuf::from(p);
    }

    if let Ok(p) = std::env::var(DEFAULT_PROJECT_ROOT_ENV)
        && !p.trim().is_empty()
    {
        return PathBuf::from(p);
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    tokio::task::spawn_blocking(move || detect_project_root(&cwd).unwrap_or(cwd))
        .await
        .unwrap_or_else(|_| PathBuf::from("."))
}

async fn write_run_logs(
    project_root: &Path,
    tool_name: &str,
    meta: &serde_json::Value,
    log_text: &str,
) -> Result<(PathBuf, PathBuf)> {
    let log_dir = project_root.join(".gowin-mcp").join("logs");
    ensure_dir(&log_dir).await?;

    let base = format!("{}_{}", stamp(), safe_file_stem(tool_name));
    let log_file = log_dir.join(format!("{base}.log"));
    let meta_file = log_dir.join(format!("{base}.json"));

    tokio::fs::write(&log_file, log_text)
        .await
        .with_context(|| format!("write({})", log_file.display()))?;
    tokio::fs::write(&meta_file, serde_json::to_vec_pretty(meta)?)
        .await
        .with_context(|| format!("write({})", meta_file.display()))?;

    Ok((log_file, meta_file))
}

fn resolve_under(project_root: &Path, p: &str) -> PathBuf {
    let path = PathBuf::from(p);
    if path.is_absolute() {
        path
    } else {
        project_root.join(path)
    }
}

fn list_cables_arg_candidates() -> Vec<Vec<String>> {
    // programmer_cli v1.9.8.07 (Education) の公式フラグは `--scan-cables`。
    // 旧バージョンや別ビルドとの互換性のため他候補も残すが、
    // `--scan-cables` を先頭に置く。
    vec![
        vec!["--scan-cables".into()],
        vec!["--scan".into()],
        vec!["--list-cables".into()],
        vec!["--list_cables".into()],
        vec!["--cableList".into()],
        vec!["--listCable".into()],
        vec!["--cables".into()],
        vec!["--scan_cables".into()],
        vec!["-l".into()],
        vec!["--list".into()],
        vec!["--enumerate".into()],
    ]
}

fn gowin_paths(gowin_ide_path: &str) -> (PathBuf, PathBuf, PathBuf) {
    // Windows 11 レイアウト:
    //   <ide_root>\IDE\bin\gw_sh.exe
    //   <ide_root>\Programmer\bin\programmer_cli.exe
    //   <ide_root>\IDE\lib                  (DLL ディレクトリ)
    //   <ide_root>\Programmer\bin           (programmer_cli の DLL)
    let ide_root = PathBuf::from(gowin_ide_path);
    let ide_base = ide_root.join("IDE");
    let programmer_base = ide_root.join("Programmer");
    let gw_sh = ide_base.join("bin").join("gw_sh.exe");
    let programmer_cli = programmer_base.join("bin").join("programmer_cli.exe");
    (ide_base, gw_sh, programmer_cli)
}

fn gw_sh_env(ide_base: &Path, programmer_base: &Path) -> HashMap<String, String> {
    // Windows 11: PATH に IDE\bin と Programmer\bin を先頭に追加（gw_sh.exe と
    // programmer_cli.exe 가 의존하는 DLL을 찾을 수 있도록）。
    // 환경변수의 PATH 보존하여 다른 도구도 계속 동작하게 한다.
    let mut env = HashMap::new();
    let ide_bin = ide_base.join("bin");
    let programmer_bin = programmer_base.join("bin");
    let ide_lib = ide_base.join("lib");

    let path_sep = ";";
    let extra_path = format!(
        "{}{}{}",
        ide_bin.display(),
        path_sep,
        programmer_bin.display()
    );

    let path_value = match std::env::var("PATH") {
        Ok(v) if !v.is_empty() => format!("{extra_path}{path_sep}{v}"),
        _ => extra_path,
    };
    env.insert("PATH".into(), path_value);

    env.insert(
        "TCL_LIBRARY".into(),
        ide_lib.join("tcl8.6").display().to_string(),
    );
    env.insert(
        "TCLLIBPATH".into(),
        format!(
            "{}{}{}{}{}",
            ide_lib.display(),
            path_sep,
            ide_lib.join("itcl4.0.3").display(),
            path_sep,
            ide_lib.join("tcl8.6").display(),
        ),
    );

    env
}

async fn exec_with_timeout(
    command: &Path,
    args: &[String],
    cwd: Option<&Path>,
    env: Option<&HashMap<String, String>>,
    timeout_sec: u64,
) -> Result<ExecMeta> {
    let start = Instant::now();

    let mut cmd = Command::new(command);
    cmd.args(args);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    if let Some(env) = env {
        for (k, v) in env {
            cmd.env(k, v);
        }
    }
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawn {}", command.display()))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("stdout pipe missing"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("stderr pipe missing"))?;

    let stdout_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stdout.take(MAX_OUTPUT_BYTES).read_to_end(&mut buf).await;
        buf
    });
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stderr.take(MAX_OUTPUT_BYTES).read_to_end(&mut buf).await;
        buf
    });

    let mut timed_out = false;
    let status = match timeout(Duration::from_secs(timeout_sec), child.wait()).await {
        Ok(r) => r?,
        Err(_) => {
            timed_out = true;
            let _ = child.kill().await;
            match timeout(Duration::from_secs(KILL_WAIT_TIMEOUT_SEC), child.wait()).await {
                Ok(r) => r?,
                Err(_) => {
                    stdout_task.abort();
                    stderr_task.abort();
                    return Ok(ExecMeta {
                        exit_code: 124,
                        timed_out: true,
                        duration_ms: start.elapsed().as_millis(),
                        stdout: String::new(),
                        stderr: format!(
                            "kill 後 {} 秒以内にプロセスが終了しませんでした",
                            KILL_WAIT_TIMEOUT_SEC
                        ),
                    });
                }
            }
        }
    };

    let stdout_bytes = stdout_task.await.unwrap_or_default();
    let stderr_bytes = stderr_task.await.unwrap_or_default();

    let stdout = String::from_utf8_lossy(&stdout_bytes).to_string();
    let stderr = String::from_utf8_lossy(&stderr_bytes).to_string();

    Ok(ExecMeta {
        exit_code: status.code().unwrap_or(if timed_out { 124 } else { 1 }),
        timed_out,
        duration_ms: start.elapsed().as_millis(),
        stdout,
        stderr,
    })
}

#[derive(Debug, Clone)]
struct GowinMcp {
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl GowinMcp {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "gowin.run_tcl",
        description = "gw_sh で Tcl を実行（任意Tcl可）。ログ保存・タイムアウト対応"
    )]
    async fn run_tcl(
        &self,
        params: Parameters<RunTclRequest>,
    ) -> Result<Json<RunTclResponse>, McpError> {
        let req = params.0;
        let project_root = resolve_project_root(req.project_root.as_deref()).await;

        let gowin_ide_path = req
            .gowin_ide_path
            .as_deref()
            .unwrap_or(DEFAULT_GOWIN_IDE_PATH);

        let (ide_base, gw_sh, _programmer_cli) = gowin_paths(gowin_ide_path);
        let ide_bin_dir = ide_base.join("bin");

        let timeout_sec = req.timeout_sec.unwrap_or(1800);
        if timeout_sec == 0 {
            return Err(McpError::new(
                ErrorCode::INVALID_PARAMS,
                "timeout_sec は 1 以上を指定してください（0 は即タイムアウトになります）",
                None,
            ));
        }

        let tcl_file_path = if let Some(tcl_path) = req.tcl_path.as_deref() {
            resolve_under(&project_root, tcl_path)
        } else {
            let inline = req.tcl_inline.clone().ok_or_else(|| {
                McpError::new(
                    ErrorCode::INVALID_PARAMS,
                    "tcl_path と tcl_inline のどちらも未指定です。Tcl ファイルパス (tcl_path) またはインライン Tcl コード (tcl_inline) のいずれかを指定してください",
                    None,
                )
            })?;
            let tmp_dir = project_root.join(".gowin-mcp").join("tmp");
            ensure_dir(&tmp_dir)
                .await
                .map_err(|e| McpError::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
            let p = tmp_dir.join(format!("{}_inline.tcl", stamp()));
            tokio::fs::write(&p, inline)
                .await
                .map_err(|e| McpError::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
            p
        };

        let programmer_base = PathBuf::from(gowin_ide_path).join("Programmer");
        let mut env = gw_sh_env(&ide_base, &programmer_base);
        if let Some(extra) = req.env {
            for (k, v) in extra {
                env.insert(k, v);
            }
        }

        let exec = exec_with_timeout(
            &gw_sh,
            &[tcl_file_path.display().to_string()],
            Some(&ide_bin_dir),
            Some(&env),
            timeout_sec,
        )
        .await
        .map_err(|e| McpError::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;

        let expected_checks = if let Some(expected) = req.expected_files {
            let mut checks = Vec::new();
            for p in expected {
                let abs = resolve_under(&project_root, &p);
                let exists = tokio::fs::metadata(&abs).await.is_ok();
                checks.push(ExpectedFileCheck {
                    path: abs.display().to_string(),
                    exists,
                });
            }
            checks
        } else {
            Vec::new()
        };

        let meta_json = serde_json::json!({
            "tool": "gowin.run_tcl",
            "project_root": project_root.display().to_string(),
            "gowin_ide_path": gowin_ide_path,
            "gw_sh": gw_sh.display().to_string(),
            "cwd": ide_bin_dir.display().to_string(),
            "tcl_file": tcl_file_path.display().to_string(),
            "exit_code": exec.exit_code,
            "timed_out": exec.timed_out,
            "duration_ms": exec.duration_ms,
            "expected_checks": expected_checks,
        });

        let log_text = format!(
            "command: {} {:?}\n\nexit_code: {}\ntimed_out: {}\nduration_ms: {}\n\n--- stdout ---\n{}\n\n--- stderr ---\n{}\n",
            gw_sh.display(),
            vec![tcl_file_path.display().to_string()],
            exec.exit_code,
            exec.timed_out,
            exec.duration_ms,
            exec.stdout,
            exec.stderr,
        );

        let (log_file, meta_file) =
            write_run_logs(&project_root, "gowin.run_tcl", &meta_json, &log_text)
                .await
                .map_err(|e| McpError::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;

        Ok(Json(RunTclResponse {
            project_root: project_root.display().to_string(),
            tcl_file_path: tcl_file_path.display().to_string(),
            gowin_ide_path: gowin_ide_path.to_string(),
            exit_code: exec.exit_code,
            timed_out: exec.timed_out,
            duration_ms: exec.duration_ms,
            stdout: exec.stdout,
            stderr: exec.stderr,
            expected_checks,
            log_file: log_file.display().to_string(),
            meta_file: meta_file.display().to_string(),
        }))
    }

    #[tool(
        name = "gowin.list_cables",
        description = "programmer_cli で接続ケーブルを列挙（複数パターン試行）。ログ保存・タイムアウト対応"
    )]
    async fn list_cables(
        &self,
        params: Parameters<ListCablesRequest>,
    ) -> Result<Json<ListCablesResponse>, McpError> {
        let req = params.0;

        let project_root = resolve_project_root(req.project_root.as_deref()).await;

        let gowin_ide_path = req
            .gowin_ide_path
            .as_deref()
            .unwrap_or(DEFAULT_GOWIN_IDE_PATH);

        let timeout_sec = req.timeout_sec.unwrap_or(20);
        if timeout_sec == 0 {
            return Err(McpError::new(
                ErrorCode::INVALID_PARAMS,
                "timeout_sec は 1 以上を指定してください（0 は即タイムアウトになります）",
                None,
            ));
        }

        let (_ide_base, _gw_sh, programmer_cli) = gowin_paths(gowin_ide_path);

        if tokio::fs::metadata(&programmer_cli).await.is_err() {
            return Err(McpError::new(
                ErrorCode::INVALID_PARAMS,
                format!(
                    "programmer_cli が見つかりません: {}。gowin_ide_path を確認してください（現在: {}）",
                    programmer_cli.display(),
                    gowin_ide_path
                ),
                None,
            ));
        }

        let candidates: Vec<Vec<String>> = list_cables_arg_candidates();

        let mut attempts = Vec::new();
        let mut cables: Vec<CableInfo> = Vec::new();

        for argv in candidates {
            let exec = exec_with_timeout(&programmer_cli, &argv, None, None, timeout_sec)
                .await
                .unwrap_or(ExecMeta {
                    exit_code: 1,
                    timed_out: false,
                    duration_ms: 0,
                    stdout: "".into(),
                    stderr: "".into(),
                });

            let text = format!("{}\n{}", exec.stdout, exec.stderr);
            let parsed = parse_cable_entries(&text);
            attempts.push(Attempt {
                args: argv,
                exit_code: exec.exit_code,
            });

            if exec.exit_code == 0 && !parsed.is_empty() {
                cables = parsed;
                break;
            }
        }

        if cables.is_empty() {
            let argv = vec!["--help".into()];
            let exec = exec_with_timeout(&programmer_cli, &argv, None, None, timeout_sec)
                .await
                .unwrap_or(ExecMeta {
                    exit_code: 1,
                    timed_out: false,
                    duration_ms: 0,
                    stdout: "".into(),
                    stderr: "".into(),
                });
            let text = format!("{}\n{}", exec.stdout, exec.stderr);
            cables = parse_cable_entries(&text);
            attempts.push(Attempt {
                args: vec!["--help".into()],
                exit_code: exec.exit_code,
            });
        }

        let meta_json = serde_json::json!({
            "tool": "gowin.list_cables",
            "project_root": project_root.display().to_string(),
            "gowin_ide_path": gowin_ide_path,
            "programmer_cli": programmer_cli.display().to_string(),
            "attempts": attempts,
            "cables": cables,
        });

        let log_text = format!(
            "programmer_cli: {}\n\n--- attempts ---\n{}\n\n--- cables ---\n{}\n",
            programmer_cli.display(),
            attempts
                .iter()
                .map(|a| format!("{:?} => {}", a.args, a.exit_code))
                .collect::<Vec<_>>()
                .join("\n"),
            cables
                .iter()
                .map(|c| {
                    format!(
                        "{}{}{}",
                        c.name,
                        c.index
                            .as_deref()
                            .map(|i| format!(" @ {}", i))
                            .unwrap_or_default(),
                        c.location
                            .as_deref()
                            .map(|l| format!(" [{}]", l))
                            .unwrap_or_default(),
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
        );

        let (log_file, meta_file) =
            write_run_logs(&project_root, "gowin.list_cables", &meta_json, &log_text)
                .await
                .map_err(|e| McpError::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;

        Ok(Json(ListCablesResponse {
            project_root: project_root.display().to_string(),
            gowin_ide_path: gowin_ide_path.to_string(),
            cables,
            attempts,
            log_file: log_file.display().to_string(),
            meta_file: meta_file.display().to_string(),
        }))
    }

    #[tool(
        name = "gowin.program_fs",
        description = "programmer_cli で .fs を SRAM 書き込み（ケーブル完全自動検出）。ログ保存・タイムアウト対応"
    )]
    async fn program_fs(
        &self,
        params: Parameters<ProgramFsRequest>,
    ) -> Result<Json<ProgramFsResponse>, McpError> {
        let req = params.0;

        let project_root = resolve_project_root(req.project_root.as_deref()).await;

        let gowin_ide_path = req
            .gowin_ide_path
            .as_deref()
            .unwrap_or(DEFAULT_GOWIN_IDE_PATH);

        let (_ide_base, _gw_sh, programmer_cli) = gowin_paths(gowin_ide_path);

        let fs_file_path = req.fs_file_path.as_deref().ok_or_else(|| {
            McpError::new(
                ErrorCode::INVALID_PARAMS,
                "fs_file_path を指定してください（例: fpgaOscillator/impl/pnr/fpgaOscillator.fs）",
                None,
            )
        })?;
        let fs_abs = resolve_under(&project_root, fs_file_path);

        let device = req.device.ok_or_else(|| {
            McpError::new(
                ErrorCode::INVALID_PARAMS,
                "device を指定してください（例: GW1N-9C, GW2A-LV18PG256C8/I7, GW5A-25A）。board の chip 名に合わせてください",
                None,
            )
        })?;
        let frequency = req.frequency.unwrap_or_else(|| "15MHz".into());
        let retries = req.retries.unwrap_or(2);
        let timeout_sec = req.timeout_sec.unwrap_or(120);
        if timeout_sec == 0 {
            return Err(McpError::new(
                ErrorCode::INVALID_PARAMS,
                "timeout_sec は 1 以上を指定してください（0 は即タイムアウトになります）",
                None,
            ));
        }

        if tokio::fs::metadata(&fs_abs).await.is_err() {
            return Err(McpError::new(
                ErrorCode::INVALID_PARAMS,
                format!(
                    ".fs ファイルが見つかりません: {}。fs_file_path を確認してください",
                    fs_abs.display()
                ),
                None,
            ));
        }

        // Sipeed がカスタムした Gowin Programmer (Tang Nano 9K の onboard
        // programmer) は `programmer_cli --list_cables` で
        //   `USB Debugger A/1/321/null`
        // として報告される。list_cables を毎回走らせると 5–10s 追加でかかる上、
        // ボードを 1 個しかつなぐ想定なので固定で良い。`--cable` にそのまま渡せる
        // 名前 "USB Debugger A" をハードコードする。
        const DEFAULT_CABLE: &str = "USB Debugger A";
        let user_cable = req
            .cable
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let mut selected_cable: Option<String> =
            Some(user_cable.clone().unwrap_or_else(|| DEFAULT_CABLE.to_string()));
        let list_cables_attempts: Option<Vec<Attempt>> = None;
        let all_cables: Vec<CableInfo> = Vec::new();

        let base_args: Vec<String> = vec![
            "-r".into(),
            retries.to_string(),
            "--device".into(),
            device.clone(),
            "--fsFile".into(),
            fs_abs.display().to_string(),
            "--frequency".into(),
            frequency.clone(),
        ];

        let mut variants: Vec<(String, Vec<String>)> = Vec::new();

        // 0) Tang Nano 9K 等: ユーザー指定の cable_index + location を最優先で試す
        //
        // programmer_cli v1.9.8.07 (Education) の公式フラグは:
        //   --cable-index <int>   0..4 (4 = "USB Debugger A")
        //   --channel <int>       USB location (FT2CH 系) または device channel
        //   --location <int>      "Will ignore --channel option" (FT2CH のみ)
        //
        // 注意: `--location 627` を渡すと "USB Debugger A/0/0/null" を引いて
        //       cable open failed になる。`--channel 273` が正しい。
        if let (Some(idx), Some(channel)) = (req.cable_index.as_deref(), req.location.as_deref()) {
            // a) 公式フラグ: --cable-index <idx> --channel <channel>
            {
                let mut argv = Vec::new();
                argv.extend(base_args.iter().take(4).cloned());
                argv.push("--cable-index".into());
                argv.push(idx.to_string());
                argv.push("--channel".into());
                argv.push(channel.to_string());
                argv.extend(base_args.iter().skip(4).cloned());
                variants.push((
                    "with_cable_index:official".into(),
                    argv,
                ));
            }
            // b) 別表記フォールバック (古い CLI 互換)
            {
                let mut argv = Vec::new();
                argv.extend(base_args.iter().take(4).cloned());
                argv.push("--cable-index".into());
                argv.push(idx.to_string());
                argv.push("--location".into());
                argv.push(channel.to_string());
                argv.extend(base_args.iter().skip(4).cloned());
                variants.push((
                    "with_cable_index:--location".into(),
                    argv,
                ));
            }
            // c) cable-index のみ (channel 省略)
            {
                let mut argv = Vec::new();
                argv.extend(base_args.iter().take(4).cloned());
                argv.push("--cable-index".into());
                argv.push(idx.to_string());
                argv.extend(base_args.iter().skip(4).cloned());
                variants.push((
                    "with_cable_index:no_channel".into(),
                    argv,
                ));
            }
        }

        // 1) ユーザー指定の cable を最優先で試す (既に trim 済み)
        //
        // `--cable` は programmer_cli 側で「ケーブル種類の文字列」を期待する。
        // 例: "Gowin USB Cable(FT2CH)" / "USB Debugger A"。
        // cable 名がそのまま使える形式であればこの経路でも OK。
        if let Some(cable) = selected_cable.clone() {
            let mut argv = Vec::new();
            argv.extend(base_args.iter().take(4).cloned());
            argv.push("--cable".into());
            argv.push(cable);
            argv.extend(base_args.iter().skip(4).cloned());
            variants.push(("with_cable".into(), argv));
        }

        // 2) list_cables が返した全候補を順番に試す (1 以外)
        for (i, info) in all_cables.iter().enumerate().skip(1) {
            if Some(info.name.as_str()) == selected_cable.as_deref() {
                continue;
            }
            let mut argv = Vec::new();
            argv.extend(base_args.iter().take(4).cloned());
            argv.push("--cable".into());
            argv.push(info.name.clone());
            argv.extend(base_args.iter().skip(4).cloned());
            variants.push((format!("with_cable[{}]", i), argv));
        }

        // 3) 最後のフォールバック: --cable 無しで 1 回だけ試す
        variants.push(("without_cable".into(), base_args.clone()));

        let mut tried: Vec<VariantTried> = Vec::new();
        let mut last_exec: Option<ExecMeta> = None;
        let mut last_label: Option<String> = None;
        let mut cable_from_output: Option<String> = None;

        for (label, argv) in variants {
            let exec = exec_with_timeout(&programmer_cli, &argv, None, None, timeout_sec)
                .await
                .map_err(|e| McpError::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;

            tried.push(VariantTried {
                label: label.clone(),
                exit_code: exec.exit_code,
            });

            if label == "without_cable" && exec.exit_code != 0 {
                let parsed = parse_cable_names(&format!("{}\n{}", exec.stdout, exec.stderr));
                if !parsed.is_empty() {
                    cable_from_output = Some(parsed[0].clone());
                }
            }

            last_label = Some(label.clone());
            last_exec = Some(exec);

            if last_exec.as_ref().map(|e| e.exit_code).unwrap_or(1) == 0 {
                break;
            }
        }

        if last_exec.as_ref().map(|e| e.exit_code).unwrap_or(1) != 0
            && let Some(cable) = cable_from_output.clone()
        {
            let mut argv = Vec::new();
            argv.extend(base_args.iter().take(4).cloned());
            argv.push("--cable".into());
            argv.push(cable.clone());
            argv.extend(base_args.iter().skip(4).cloned());

            let exec = exec_with_timeout(&programmer_cli, &argv, None, None, timeout_sec)
                .await
                .map_err(|e| McpError::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;

            tried.push(VariantTried {
                label: "retry_cable_from_output".into(),
                exit_code: exec.exit_code,
            });

            selected_cable = Some(cable);
            last_label = Some("retry_cable_from_output".into());
            last_exec = Some(exec);
        }

        let exec = last_exec.ok_or_else(|| {
            McpError::new(
                ErrorCode::INTERNAL_ERROR,
                "programmer_cli の実行結果が得られませんでした",
                None,
            )
        })?;

        let meta_json = serde_json::json!({
            "tool": "gowin.program_fs",
            "project_root": project_root.display().to_string(),
            "gowin_ide_path": gowin_ide_path,
            "programmer_cli": programmer_cli.display().to_string(),
            "fs_file": fs_abs.display().to_string(),
            "device": device,
            "frequency": frequency,
            "retries": retries,
            "cable_index": req.cable_index,
            "location": req.location,
            "selected_cable": selected_cable,
            "list_cables_attempts": list_cables_attempts,
            "variants_tried": tried,
            "final_variant": last_label,
            "exit_code": exec.exit_code,
            "timed_out": exec.timed_out,
            "duration_ms": exec.duration_ms,
        });

        let log_text = format!(
            "programmer_cli: {}\nfs: {}\ndevice: {}\nfrequency: {}\nretries: {}\nselected_cable: {:?}\n\nvariants_tried: {:?}\n\nexit_code: {}\ntimed_out: {}\nduration_ms: {}\n\n--- stdout ---\n{}\n\n--- stderr ---\n{}\n",
            programmer_cli.display(),
            fs_abs.display(),
            device,
            frequency,
            retries,
            selected_cable,
            tried,
            exec.exit_code,
            exec.timed_out,
            exec.duration_ms,
            exec.stdout,
            exec.stderr,
        );

        let (log_file, meta_file) =
            write_run_logs(&project_root, "gowin.program_fs", &meta_json, &log_text)
                .await
                .map_err(|e| McpError::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;

        Ok(Json(ProgramFsResponse {
            project_root: project_root.display().to_string(),
            gowin_ide_path: gowin_ide_path.to_string(),
            fs_file: fs_abs.display().to_string(),
            selected_cable,
            list_cables_attempts,
            variants_tried: tried,
            exit_code: exec.exit_code,
            timed_out: exec.timed_out,
            duration_ms: exec.duration_ms,
            stdout: exec.stdout,
            stderr: exec.stderr,
            log_file: log_file.display().to_string(),
            meta_file: meta_file.display().to_string(),
        }))
    }
}

#[tool_handler]
impl ServerHandler for GowinMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "gw-synth-flash-mcp: gw_sh / programmer_cli をLLMから操作するためのWindows 11向けMCPサーバー（個人利用向け）".into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct RunTclRequest {
    project_root: Option<String>,
    gowin_ide_path: Option<String>,
    tcl_path: Option<String>,
    tcl_inline: Option<String>,
    timeout_sec: Option<u64>,
    env: Option<HashMap<String, String>>,
    expected_files: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct RunTclResponse {
    project_root: String,
    gowin_ide_path: String,
    tcl_file_path: String,
    exit_code: i32,
    timed_out: bool,
    duration_ms: u128,
    stdout: String,
    stderr: String,
    expected_checks: Vec<ExpectedFileCheck>,
    log_file: String,
    meta_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct Attempt {
    args: Vec<String>,
    exit_code: i32,
}

/// Parsed cable entry: name, 1-based index, and USB location id.
///
/// `Cable found: USB Debugger A/1/321/null (USB location:321)`
/// → `CableInfo { name: "USB Debugger A", index: Some("1"), location: Some("321") }`
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
struct CableInfo {
    name: String,
    index: Option<String>,
    location: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct ListCablesRequest {
    project_root: Option<String>,
    gowin_ide_path: Option<String>,
    timeout_sec: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct ListCablesResponse {
    project_root: String,
    gowin_ide_path: String,
    cables: Vec<CableInfo>,
    attempts: Vec<Attempt>,
    log_file: String,
    meta_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct ProgramFsRequest {
    project_root: Option<String>,
    gowin_ide_path: Option<String>,
    fs_file_path: Option<String>,
    device: Option<String>,
    frequency: Option<String>,
    retries: Option<u32>,
    timeout_sec: Option<u64>,
    /// Cable name (e.g. "USB Debugger A").
    /// If omitted, the first cable returned by `list_cables` is used.
    /// Combine with `cable_index` and `location` when known.
    cable: Option<String>,
    /// 1-based cable index from the `Cable found:` line (the segment after the
    /// first "/"). Omit to use programmer_cli's default behaviour.
    cable_index: Option<String>,
    /// USB location ID from the `Cable found:` line (the numeric segment
    /// before "/null", e.g. "321"). Omit to use programmer_cli's default.
    location: Option<String>,
    /// Operation index. 2 = SRAM Program (volatile, ~7-10s), 5 = embFlash
    /// Erase+Program (permanent, ~30-60s). Default: 2.
    operation_index: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct VariantTried {
    label: String,
    exit_code: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct ProgramFsResponse {
    project_root: String,
    gowin_ide_path: String,
    fs_file: String,
    selected_cable: Option<String>,
    list_cables_attempts: Option<Vec<Attempt>>,
    variants_tried: Vec<VariantTried>,
    exit_code: i32,
    timed_out: bool,
    duration_ms: u128,
    stdout: String,
    stderr: String,
    log_file: String,
    meta_file: String,
}

fn parse_cable_names(text: &str) -> Vec<String> {
    parse_cable_entries(text)
        .into_iter()
        .map(|c| c.name)
        .collect()
}

/// Windows 11 / Gowin programmer_cli 実機の出力から、ケーブル情報 (名前 +
/// index + location) を抽出する。
///
/// 主な抽出対象:
///   "Target Cable: Gowin USB Cable(FT2CH)"
///   "Cable found: Gowin USB Cable(FT2CH)/1/321/null"
///   "Cable Name: Gowin USB Cable(FT2CH)"
///   [1] "Gowin USB Cable(FT2CH)"
///   番号付きリスト "1. Gowin USB Cable(FT2CH)"
///
/// `Cable found:` 行で `/idx/loc/null` サフィックスを見つけた場合、
/// `CableInfo.index` と `CableInfo.location` に分離して格納する。
/// 同じ name を 2 度拾った場合は先に格納した entry を優先する。
fn parse_cable_entries(text: &str) -> Vec<CableInfo> {
    let mut found: Vec<CableInfo> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        // 0) 「Target Cable: ...」「Cable found: ...」形式
        for label in ["Target Cable:", "Cable found:", "Cable:", "Cable Name:"] {
            if let Some(pos) = line.to_lowercase().find(&label.to_lowercase()) {
                let value_start = pos + label.len();
                let raw = line[value_start..].trim().trim_matches('"').trim();
                push_cable_entry(&mut found, &mut seen, raw);
            }
        }

        // 0b) IDCode 単独行でもサフィックス除去 (冪等)
        if line.to_lowercase().contains("idcode") {
            if let Some(last) = found.last_mut() {
                let cleaned_name = strip_cable_suffix(&last.name);
                if !cleaned_name.is_empty() && cleaned_name != last.name {
                    if seen.remove(&last.name) {
                        seen.insert(cleaned_name.clone());
                        last.name = cleaned_name;
                    }
                }
            }
        }

        // 1) 引用符で囲まれた名前を抽出
        let mut rest = line;
        while let Some(start) = rest.find('"') {
            let after = &rest[start + 1..];
            if let Some(end) = after.find('"') {
                let v = after[..end].trim();
                push_cable_entry(&mut found, &mut seen, v);
                rest = &after[end + 1..];
            } else {
                break;
            }
        }

        // 2) 番号付きリスト (例: "1. Gowin USB Cable(FT2CH)" / "[1] Gowin USB Cable(FT2CH)")
        let stripped = line
            .trim_start_matches(|c: char| {
                c.is_ascii_digit() || c == '[' || c == ']' || c == '.' || c == ')' || c == '-'
            })
            .trim();
        if !stripped.is_empty() && stripped.len() <= 80 {
            let low = stripped.to_lowercase();
            if (low.starts_with("gowin") && low.contains("cable"))
                || low.contains("usb cable")
                || low.contains("ft2ch")
                || low.contains("ft2232")
            {
                push_cable_entry(&mut found, &mut seen, stripped);
            }
        }
    }

    found
}

fn push_cable_entry(
    found: &mut Vec<CableInfo>,
    seen: &mut std::collections::HashSet<String>,
    candidate: &str,
) {
    let trimmed = candidate.trim().trim_matches('"').trim();
    if trimmed.is_empty() {
        return;
    }

    // "Name/idx/loc/null" 形式を分解する。strip_cable_suffix と同じく、
    // "/" の個数が 2 以上のときだけサフィックス付きとみなす。
    let (name, idx, loc) = if trimmed.matches('/').count() >= 2 {
        if let Some(slash_pos) = trimmed.find('/') {
            let head = trimmed[..slash_pos].trim();
            let tail = &trimmed[slash_pos + 1..];
            let parts: Vec<&str> = tail.split('/').map(|p| p.trim()).collect();
            let idx = parts.first().copied().filter(|s| !s.is_empty()).map(String::from);
            // location は最後から 2 番目 (例: "Gowin USB Cable(FT2CH)/0/0/null" → loc="0")
            let loc = parts
                .iter()
                .rev()
                .nth(1)
                .copied()
                .filter(|s| !s.is_empty())
                .map(String::from);
            (head.to_string(), idx, loc)
        } else {
            (trimmed.to_string(), None, None)
        }
    } else {
        (trimmed.to_string(), None, None)
    };

    let name = strip_cable_suffix(&name);
    if name.is_empty() || name.len() < 3 {
        return;
    }
    let low = name.to_lowercase();
    let cable_like = low.contains("cable")
        || low.contains("gowin")
        || low.contains("ft2ch")
        || low.contains("ft2232")
        || low.contains("ft232")
        || low.contains("ft60x")
        || (low.contains("usb") && (low.contains("debugger") || low.contains("debug")))
        || low.contains("jtag");
    if !cable_like {
        return;
    }
    if seen.insert(name.clone()) {
        found.push(CableInfo {
            name,
            index: idx,
            location: loc,
        });
    }
}

/// Gowin programmer_cli が吐くケーブル名から、デバイス ID 系のサフィックスを落とす。
///
/// 例 (Windows 11 / programmer_cli 実機):
///   `Gowin USB Cable(FT2CH)/0/0/null` → `Gowin USB Cable(FT2CH)`
///   `Gowin USB Cable (FT2CH) / 1 / 2` → `Gowin USB Cable (FT2CH)`
///   `Gowin USB Cable(FT2CH)`           → `Gowin USB Cable(FT2CH)` (不変)
///
/// `--cable` 引数に `/idx/...` が含まれると programmer_cli は
/// `argument --cable: invalid choice` を返すので、ここで必ず正規化する。
fn strip_cable_suffix(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // "/0/0/null" "/1/2/null" "/0/0" のように "/<segment>" が 2 個以上連続する
    // 場合はデバイス ID 系のサフィックスなので取り除く。
    //
    // 例:
    //   "FTUSB-1B (JTAG-USB Cable)/0/0/null" → 2 個以上のサフィックス →  strip
    //   "GW2A-LV18PG256C8/I7"               → 1 個のサフィックス →       保持
    //
    // 注意: "GW2A-LV18PG256C8/I7" のようにデバイス識別子付きの名前を 1 個の
    // サフィックスで strip すると、本来のデバイス名 "GW2A-LV18PG256C8" と衝突
    // してしまうため、安全側に倒して 1 個の場合は保持する。
    let slash_count = trimmed.matches('/').count();
    if slash_count >= 2 {
        if let Some(pos) = trimmed.find('/') {
            return trimmed[..pos].trim().to_string();
        }
    }
    trimmed.to_string()
}

#[cfg(test)]
mod parser_tests {
    use super::*;

    #[test]
    fn strip_cable_suffix_removes_double_slash_suffix() {
        assert_eq!(strip_cable_suffix("FTUSB-1B (JTAG-USB Cable)/0/0/null"), "FTUSB-1B (JTAG-USB Cable)");
        assert_eq!(strip_cable_suffix("FTUSB-1B (JTAG-USB Cable)/1/2/null"), "FTUSB-1B (JTAG-USB Cable)");
    }

    #[test]
    fn strip_cable_suffix_keeps_clean_name() {
        assert_eq!(strip_cable_suffix("FTUSB-1B (JTAG-USB Cable)"), "FTUSB-1B (JTAG-USB Cable)");
    }

    #[test]
    fn strip_cable_suffix_keeps_single_segment_suffix() {
        // 1 個のサフィックス ("/I7" など) はデバイス ID の一部として保持する。
        // strip すると素のデバイス名 "GW2A-LV18PG256C8" と衝突する。
        assert_eq!(strip_cable_suffix("GW2A-LV18PG256C8/I7"), "GW2A-LV18PG256C8/I7");
    }

    #[test]
    fn strip_cable_suffix_handles_empty_and_whitespace() {
        assert_eq!(strip_cable_suffix(""), "");
        assert_eq!(strip_cable_suffix("   "), "");
        assert_eq!(strip_cable_suffix("  name/0/0/null  "), "name");
    }
}

#[cfg(test)]
mod list_cables_arg_tests {
    use super::*;

    #[test]
    fn list_cables_arg_candidates_includes_scan_cables() {
        // `--scan-cables` は programmer_cli の公式オプション (JTAGLoading モード)
        // 必ず候補に含めること（scan_cable_attempts に依存させない）。
        let candidates = list_cables_arg_candidates();
        let joined: Vec<String> = candidates
            .iter()
            .map(|argv| argv.join(" "))
            .collect();
        assert!(
            joined.iter().any(|c| c.contains("scan-cables")),
            "--scan-cables must be in candidate list: {:?}",
            joined
        );
    }

    #[test]
    fn list_cables_arg_candidates_non_empty() {
        let candidates = list_cables_arg_candidates();
        assert!(!candidates.is_empty(), "candidate list must not be empty");
        for c in &candidates {
            assert!(!c.is_empty(), "empty candidate in {:?}", candidates);
            for arg in c {
                assert!(!arg.trim().is_empty(), "empty arg in {:?}", c);
            }
        }
    }
}

#[cfg(test)]
mod gowin_paths_tests {
    use super::*;

    #[test]
    fn gowin_paths_windows_layout() {
        let (ide_base, gw_sh, programmer_cli) = gowin_paths(r"C:\Gowin\Gowin_V1.9.11.03_Education_x64");
        assert_eq!(ide_base, PathBuf::from(r"C:\Gowin\Gowin_V1.9.11.03_Education_x64\IDE"));
        assert_eq!(
            gw_sh,
            PathBuf::from(r"C:\Gowin\Gowin_V1.9.11.03_Education_x64\IDE\bin\gw_sh.exe")
        );
        assert_eq!(
            programmer_cli,
            PathBuf::from(r"C:\Gowin\Gowin_V1.9.11.03_Education_x64\Programmer\bin\programmer_cli.exe")
        );
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // コマンドライン引数の処理
    let args: Vec<String> = std::env::args().collect();

    // --help または -h
    if args.len() > 1 && (args[1] == "--help" || args[1] == "-h") {
        print_help();
        return Ok(());
    }

    // --version または -v
    if args.len() > 1 && (args[1] == "--version" || args[1] == "-v") {
        print_version();
        return Ok(());
    }

    let service = GowinMcp::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

fn print_help() {
    println!("gw-synth-flash-mcp {}", env!("CARGO_PKG_VERSION"));
    println!("{}", env!("CARGO_PKG_DESCRIPTION"));
    println!();
    println!("USAGE:");
    println!("    gw-synth-flash-mcp [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    -h, --help       Print help information");
    println!("    -v, --version    Print version information");
    println!();
    println!("ENVIRONMENT VARIABLES:");
    println!("    GOWIN_MCP_PROJECT_ROOT    Set the default project root directory");
    println!();
    println!("DESCRIPTION:");
    println!("    An unofficial MCP server that provides Gowin IDE CLI tools:");
    println!("    - gowin.run_tcl: Execute arbitrary Tcl scripts via gw_sh");
    println!("    - gowin.list_cables: Enumerate available programming cables");
    println!("    - gowin.program_fs: Program .fs files to SRAM");
    println!();
    println!("    This server communicates via stdio using the Model Context Protocol (MCP).");
    println!("    Configure your MCP client (VS Code, Claude Code, etc.) to start this server.");
    println!();
    println!("REPOSITORY:");
    println!("    {}", env!("CARGO_PKG_REPOSITORY"));
}

fn print_version() {
    println!("gw-synth-flash-mcp {}", env!("CARGO_PKG_VERSION"));
}
