//! 任务结果落盘与任务列表。
//!
//! 目录结构（`app_config_dir` 下）：
//!
//! ```text
//! runs/<runId>/meta.json      任务元信息
//! runs/<runId>/results.jsonl  每完成一条追加一行，同一 index 以最后一行为准
//! runs/<runId>/dataset.json   启动时的数据集快照
//! ```
//!
//! 每个任务一个目录，互不覆盖。多个任务可以同时运行。
//! 旧版本的 `runs/current` 与 `runs/history/<runId>` 会在启动时迁移成扁平的 `runs/<runId>`。

use crate::api::ApiProfile;
use crate::judge::PARSE_FAIL;
use crate::report::ProfileInfo;
use crate::runner::RowResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const STATUS_RUNNING: &str = "running";
pub const STATUS_FINISHED: &str = "finished";
pub const STATUS_CANCELLED: &str = "cancelled";
pub const STATUS_FAILED: &str = "failed";
pub const STATUS_INTERRUPTED: &str = "interrupted";

/// 自动清理时保留多少个已结束任务；运行中 / 中断任务不自动删
pub const MAX_FINISHED_RUNS: usize = 10;

/// 与前端 EXCLUDED_CATEGORIES 保持同一口径
pub const TARGET_FAIL: &str = "被测模型调用失败";
pub const JUDGE_FAIL: &str = "评测模型调用失败";

/// 启动任务时对方案三份提示词的快照，重跑失败项前用它判断方案是否改过。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PromptSnapshot {
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub user_template: String,
    #[serde(default)]
    pub eval_prompt: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub total: usize,
    pub valid: usize,
    pub passed: usize,
    pub failed: usize,
    pub excluded: usize,
}

impl RunSummary {
    /// 汇总口径必须与前端 store.js 的 computeStats 一致：
    /// 调用链路没跑通的（targetError / parseOk=false / 三类兜底分类）不计入通过率。
    pub fn of(results: &[RowResult]) -> Self {
        let mut s = RunSummary {
            total: results.len(),
            ..Default::default()
        };
        for r in results {
            if is_excluded(r) {
                s.excluded += 1;
            } else if r.pass {
                s.passed += 1;
            }
        }
        s.valid = s.total - s.excluded;
        s.failed = s.valid - s.passed;
        s
    }
}

fn is_excluded(r: &RowResult) -> bool {
    if r.target_error.is_some() || !r.parse_ok {
        return true;
    }
    r.error_types
        .iter()
        .any(|t| t == TARGET_FAIL || t == JUDGE_FAIL || t == PARSE_FAIL)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunMeta {
    pub run_id: String,
    /// 任务名称，默认由「方案名 · 日期」生成
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub plan_id: String,
    #[serde(default)]
    pub plan_name: String,
    #[serde(default)]
    pub dataset_id: String,
    #[serde(default)]
    pub dataset_name: String,
    /// 启动时的提示词快照
    #[serde(default)]
    pub prompts: PromptSnapshot,
    /// 启动时的数据集签名（行数 + 内容哈希）
    #[serde(default)]
    pub dataset_signature: String,
    pub total: usize,
    pub started_at: u64,
    #[serde(default)]
    pub finished_at: Option<u64>,
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<RunSummary>,
    /// 本次运行用的模型参数（不含 API Key 和接口地址），供报告还原
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_profile: Option<ProfileInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_profile: Option<ProfileInfo>,
}

/// 从 API 配置里裁出可以落盘、可以进报告的部分。
/// `extra_headers` / `extra_body` 里的敏感键先脱敏，避免 Key 进 meta 和报告。
pub fn profile_snapshot(p: &ApiProfile) -> ProfileInfo {
    ProfileInfo {
        protocol: p.protocol.clone(),
        model: p.model.clone(),
        context_window: p.context_window.unwrap_or(0) as f64,
        max_tokens: p.max_tokens.unwrap_or(0) as f64,
        temperature: p.temperature.unwrap_or(0.0),
        top_p: p.top_p.unwrap_or(0.0),
        timeout_secs: p.timeout_secs.unwrap_or(0) as f64,
        extra_body: crate::storage::redact_sensitive_json(
            p.extra_body.as_deref().unwrap_or_default(),
        ),
        extra_headers: crate::storage::redact_sensitive_json(
            p.extra_headers.as_deref().unwrap_or_default(),
        ),
    }
}

/// 任务列表项，只读 meta，不解析结果，避免一次拉几十 MB。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEntry {
    pub run_id: String,
    pub title: String,
    pub plan_id: String,
    pub plan_name: String,
    pub dataset_id: String,
    pub dataset_name: String,
    pub total: usize,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub status: String,
    pub error: Option<String>,
    pub summary: RunSummary,
    pub target_profile: Option<ProfileInfo>,
    pub judge_profile: Option<ProfileInfo>,
}

/// 启动时一起存下的原始数据行，供历史记录还原详情里的「原始数据」。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetSnapshot {
    pub rows: Vec<serde_json::Map<String, Value>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedRun {
    pub meta: RunMeta,
    pub results: Vec<RowResult>,
    /// 崩溃或磁盘写入中断可能留下半行，解析失败的行数（不静默丢弃）
    pub bad_lines: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dataset: Option<DatasetSnapshot>,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn runs_root(app: &tauri::AppHandle) -> PathBuf {
    crate::storage::config_dir(app).join("runs")
}

/// 目录名会被拼进路径，只接受字母数字、下划线、连字符，杜绝 ../ 之类。
pub fn valid_run_id(id: &str) -> bool {
    const MAX: usize = 128;
    !id.is_empty()
        && id.len() <= MAX
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn check_run_id(id: &str) -> Result<(), String> {
    if valid_run_id(id) {
        Ok(())
    } else {
        Err("任务 ID 不合法".into())
    }
}

pub fn run_dir(app: &tauri::AppHandle, run_id: &str) -> Result<PathBuf, String> {
    check_run_id(run_id)?;
    Ok(runs_root(app).join(run_id))
}

pub fn meta_path(dir: &Path) -> PathBuf {
    dir.join("meta.json")
}

pub fn results_path(dir: &Path) -> PathBuf {
    dir.join("results.jsonl")
}

pub fn dataset_path(dir: &Path) -> PathBuf {
    dir.join("dataset.json")
}

pub fn write_dataset(dir: &Path, dataset: &DatasetSnapshot) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建任务目录失败：{e}"))?;
    let text = serde_json::to_string(dataset).map_err(|e| e.to_string())?;
    crate::storage::atomic_write(&dataset_path(dir), text.as_bytes())
}

pub fn read_dataset(dir: &Path) -> Option<DatasetSnapshot> {
    let text = std::fs::read_to_string(dataset_path(dir)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write_meta(dir: &Path, meta: &RunMeta) -> Result<(), String> {
    let text = serde_json::to_string(meta).map_err(|e| e.to_string())?;
    crate::storage::atomic_write(&meta_path(dir), text.as_bytes())
}

pub fn read_meta(dir: &Path) -> Result<Option<RunMeta>, String> {
    let p = meta_path(dir);
    if !p.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&p).map_err(|e| format!("读取任务信息失败：{e}"))?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| format!("任务信息格式错误：{e}"))
}

/// 新任务：创建目录、截断结果文件并写 meta。
pub fn reset(dir: &Path, meta: &RunMeta) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建任务目录失败：{e}"))?;
    std::fs::File::create(results_path(dir)).map_err(|e| format!("清空结果文件失败：{e}"))?;
    write_meta(dir, meta)
}

/// 追加模式打开结果文件（重跑 / 续跑不截断）。
pub fn open_append(dir: &Path) -> Result<BufWriter<std::fs::File>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建任务目录失败：{e}"))?;
    let f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(results_path(dir))
        .map_err(|e| format!("打开结果文件失败：{e}"))?;
    Ok(BufWriter::new(f))
}

pub fn write_row(w: &mut impl Write, r: &RowResult) -> std::io::Result<()> {
    serde_json::to_writer(&mut *w, r)?;
    w.write_all(b"\n")
}

/// 读取任务结果：按 index 去重（后写覆盖先写）并排序。
pub fn load_dir(dir: &Path) -> Result<Option<LoadedRun>, String> {
    let mp = meta_path(dir);
    let rp = results_path(dir);
    if !mp.exists() && !rp.exists() {
        return Ok(None);
    }

    let meta = match read_meta(dir)? {
        Some(m) => m,
        None => RunMeta {
            run_id: dir
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".into()),
            title: String::new(),
            plan_id: String::new(),
            plan_name: String::new(),
            dataset_id: String::new(),
            dataset_name: String::new(),
            prompts: PromptSnapshot::default(),
            dataset_signature: String::new(),
            total: 0,
            started_at: 0,
            finished_at: None,
            status: STATUS_FINISHED.into(),
            error: None,
            summary: None,
            target_profile: None,
            judge_profile: None,
        },
    };

    let mut by_index: BTreeMap<usize, RowResult> = BTreeMap::new();
    let mut bad_lines = 0usize;
    if rp.exists() {
        let f = std::fs::File::open(&rp).map_err(|e| format!("读取结果文件失败：{e}"))?;
        for line in BufReader::new(f).lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => {
                    bad_lines += 1;
                    continue;
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<RowResult>(&line) {
                Ok(r) => {
                    by_index.insert(r.index, r);
                }
                Err(_) => bad_lines += 1,
            }
        }
    }

    Ok(Some(LoadedRun {
        meta,
        results: by_index.into_values().collect(),
        bad_lines,
        dataset: read_dataset(dir),
    }))
}

fn entry_of(run_id: String, meta: RunMeta) -> RunEntry {
    RunEntry {
        run_id,
        title: meta.title,
        plan_id: meta.plan_id,
        plan_name: meta.plan_name,
        dataset_id: meta.dataset_id,
        dataset_name: meta.dataset_name,
        total: meta.total,
        started_at: meta.started_at,
        finished_at: meta.finished_at,
        status: meta.status,
        error: meta.error,
        summary: meta.summary.unwrap_or_default(),
        target_profile: meta.target_profile,
        judge_profile: meta.judge_profile,
    }
}

/// 扫描 `runs/*/meta.json`，按开始时间倒序返回。只读 meta。
pub fn list_run_entries(root: &Path) -> Vec<RunEntry> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(root) else {
        return out;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let Some(name) = p.file_name().map(|s| s.to_string_lossy().to_string()) else {
            continue;
        };
        if !valid_run_id(&name) {
            continue;
        }
        let Ok(Some(meta)) = read_meta(&p) else {
            continue;
        };
        out.push(entry_of(name, meta));
    }
    out.sort_by(|a, b| {
        b.started_at
            .cmp(&a.started_at)
            .then_with(|| b.run_id.cmp(&a.run_id))
    });
    out
}

/// 进程被杀时 meta 还停在 running，启动时统一改成 interrupted，保留已跑完的结果。
pub fn recover_running(root: &Path) -> usize {
    let mut changed = 0usize;
    let Ok(rd) = std::fs::read_dir(root) else {
        return changed;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let Ok(Some(mut meta)) = read_meta(&p) else {
            continue;
        };
        if meta.status == STATUS_RUNNING {
            meta.status = STATUS_INTERRUPTED.into();
            meta.finished_at = Some(now_ms());
            if write_meta(&p, &meta).is_ok() {
                changed += 1;
            }
        }
    }
    changed
}

fn move_dir(from: &Path, target: &Path) -> Result<(), String> {
    std::fs::create_dir_all(
        target
            .parent()
            .ok_or_else(|| "目标目录不合法".to_string())?,
    )
    .map_err(|e| format!("创建任务目录失败：{e}"))?;
    std::fs::rename(from, target).map_err(|e| format!("迁移旧任务目录失败：{e}"))
}

fn unique_target(root: &Path, base: &str) -> PathBuf {
    let first = root.join(base);
    if !first.exists() {
        return first;
    }
    for i in 2..1000 {
        let p = root.join(format!("{base}-{i}"));
        if !p.exists() {
            return p;
        }
    }
    root.join(format!("{base}-{}", now_ms()))
}

/// 旧目录迁移：`runs/current` 与 `runs/history/*` → `runs/<runId>`。
/// 幂等；迁移失败时保留旧目录，不删用户数据。
pub fn migrate_legacy_dirs(app: &tauri::AppHandle) -> usize {
    let root = runs_root(app);
    let mut moved = 0usize;

    let current = root.join("current");
    if current.is_dir() {
        let run_id = read_meta(&current)
            .ok()
            .flatten()
            .map(|m| m.run_id)
            .filter(|id| valid_run_id(id) && id != "current")
            .unwrap_or_else(|| format!("run-legacy-{}", now_ms()));
        let target = unique_target(&root, &run_id);
        if move_dir(&current, &target).is_ok() {
            moved += 1;
        }
    }

    let history = root.join("history");
    if history.is_dir() {
        if let Ok(rd) = std::fs::read_dir(&history) {
            for entry in rd.flatten() {
                let p = entry.path();
                if !p.is_dir() {
                    continue;
                }
                let Some(name) = p.file_name().map(|s| s.to_string_lossy().to_string()) else {
                    continue;
                };
                if !valid_run_id(&name) {
                    continue;
                }
                let target = unique_target(&root, &name);
                if move_dir(&p, &target).is_ok() {
                    moved += 1;
                }
            }
        }
        // 只剩空目录时顺手挪走，避免下次启动重复扫描
        let _ = std::fs::remove_dir(&history);
    }

    moved
}

/// 只保留最近 N 个已结束任务；运行中、中断任务不自动删。
pub fn cleanup_finished(root: &Path) {
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    let mut finished: Vec<(u64, String, PathBuf)> = Vec::new();
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let Some(name) = p.file_name().map(|s| s.to_string_lossy().to_string()) else {
            continue;
        };
        if !valid_run_id(&name) {
            continue;
        }
        let Ok(Some(meta)) = read_meta(&p) else {
            continue;
        };
        match meta.status.as_str() {
            STATUS_RUNNING | STATUS_INTERRUPTED => continue,
            _ => {}
        }
        let at = meta.finished_at.unwrap_or(meta.started_at);
        finished.push((at, name, p));
    }
    finished.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    for (_, _, p) in finished.into_iter().skip(MAX_FINISHED_RUNS) {
        let _ = std::fs::remove_dir_all(&p);
    }
}

// ---------- Tauri 命令 ----------

#[tauri::command]
pub async fn list_runs(app: tauri::AppHandle) -> Result<Vec<RunEntry>, String> {
    Ok(list_run_entries(&runs_root(&app)))
}

#[tauri::command]
pub async fn load_run(
    app: tauri::AppHandle,
    run_id: String,
) -> Result<Option<LoadedRun>, String> {
    let dir = run_dir(&app, &run_id)?;
    load_dir(&dir)
}

#[tauri::command]
pub async fn delete_run(app: tauri::AppHandle, run_id: String) -> Result<(), String> {
    let dir = run_dir(&app, &run_id)?;
    if let Ok(Some(meta)) = read_meta(&dir) {
        if meta.status == STATUS_RUNNING {
            return Err("任务正在运行，先取消再删除".into());
        }
    }
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("删除任务失败：{e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(index: usize, output: &str) -> RowResult {
        RowResult {
            index,
            input: format!("输入{index}"),
            output: output.into(),
            target_error: None,
            pass: true,
            error_types: vec![],
            reasons: vec![],
            details: String::new(),
            judge_raw: String::new(),
            parse_ok: true,
            target_latency_ms: 1,
            judge_latency_ms: 1,
            prompt_tokens: None,
            completion_tokens: None,
            judge_prompt_tokens: None,
            judge_completion_tokens: None,
            truncated: false,
            status_label: "g".into(),
        }
    }

    fn meta(run_id: &str, total: usize, status: &str, started_at: u64) -> RunMeta {
        RunMeta {
            run_id: run_id.into(),
            title: String::new(),
            plan_id: String::new(),
            plan_name: String::new(),
            dataset_id: String::new(),
            dataset_name: String::new(),
            prompts: PromptSnapshot::default(),
            dataset_signature: String::new(),
            total,
            started_at,
            finished_at: None,
            status: status.into(),
            error: None,
            summary: None,
            target_profile: None,
            judge_profile: None,
        }
    }

    /// 旧版结果文件没有裁判 tokens 两个字段，加了新字段后必须还能读。
    #[test]
    fn old_rows_without_judge_tokens_still_parse() {
        let raw = r#"{"index":0,"input":"a","output":"b","targetError":null,"pass":true,"errorTypes":[],"reasons":[],"details":"","judgeRaw":"","parseOk":true,"targetLatencyMs":10,"judgeLatencyMs":20,"promptTokens":null,"completionTokens":null,"truncated":false,"statusLabel":"g"}"#;
        let r: RowResult = serde_json::from_str(raw).unwrap();
        assert!(r.judge_prompt_tokens.is_none());
        assert!(r.judge_completion_tokens.is_none());
    }

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("batch-eval-test-{name}-{}", now_ms()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn dedupes_by_index_last_wins() {
        let dir = temp_dir("dedupe");
        reset(&dir, &meta("r", 3, STATUS_RUNNING, 1)).unwrap();
        let mut w = open_append(&dir).unwrap();
        write_row(&mut w, &row(0, "first")).unwrap();
        write_row(&mut w, &row(2, "third")).unwrap();
        write_row(&mut w, &row(0, "second")).unwrap();
        w.flush().unwrap();

        let loaded = load_dir(&dir).unwrap().unwrap();
        assert_eq!(loaded.results.len(), 2);
        assert_eq!(loaded.results[0].output, "second");
        assert_eq!(loaded.results[1].index, 2);
        assert_eq!(loaded.bad_lines, 0);
    }

    #[test]
    fn skips_broken_lines() {
        let dir = temp_dir("broken");
        reset(&dir, &meta("r", 2, STATUS_FINISHED, 1)).unwrap();
        let mut w = open_append(&dir).unwrap();
        write_row(&mut w, &row(0, "ok")).unwrap();
        w.write_all(b"{\"index\":1,\"input\":\"half").unwrap();
        w.flush().unwrap();

        let loaded = load_dir(&dir).unwrap().unwrap();
        assert_eq!(loaded.results.len(), 1);
        assert_eq!(loaded.bad_lines, 1);
    }

    #[test]
    fn atomic_write_replaces_file() {
        let dir = temp_dir("atomic");
        let p = dir.join("settings.json");
        crate::storage::atomic_write(&p, b"v1").unwrap();
        crate::storage::atomic_write(&p, b"v2").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "v2");
        assert!(!dir.join("settings.json.tmp").exists());
    }

    #[test]
    fn summary_matches_frontend_buckets() {
        let mut target_err = row(0, "x");
        target_err.target_error = Some("timeout".into());
        let mut parse_fail = row(1, "x");
        parse_fail.parse_ok = false;
        let mut judge_fail = row(2, "x");
        judge_fail.error_types = vec![JUDGE_FAIL.into()];
        let mut failed = row(3, "x");
        failed.pass = false;
        let ok = row(4, "x");

        let s = RunSummary::of(&[target_err, parse_fail, judge_fail, failed, ok]);
        assert_eq!(s.total, 5);
        assert_eq!(s.excluded, 3);
        assert_eq!(s.valid, 2);
        assert_eq!(s.passed, 1);
        assert_eq!(s.failed, 1);
    }

    fn make_run(root: &Path, run_id: &str, started_at: u64) {
        let dir = root.join(run_id);
        let r = row(0, "ok");
        let mut m = meta(run_id, 1, STATUS_FINISHED, started_at);
        m.summary = Some(RunSummary::of(std::slice::from_ref(&r)));
        m.finished_at = Some(started_at + 1);
        reset(&dir, &m).unwrap();
        let mut w = open_append(&dir).unwrap();
        write_row(&mut w, &r).unwrap();
        w.flush().unwrap();
    }

    #[test]
    fn lists_flat_runs_newest_first() {
        let root = temp_dir("list");
        make_run(&root, "run-1", 100);
        make_run(&root, "run-2", 200);
        let list = list_run_entries(&root);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].run_id, "run-2");
        assert_eq!(list[1].run_id, "run-1");
        assert_eq!(list[0].summary.total, 1);
    }

    #[test]
    fn recovers_running_to_interrupted() {
        let root = temp_dir("recover");
        let dir = root.join("run-x");
        reset(&dir, &meta("run-x", 2, STATUS_RUNNING, 1)).unwrap();
        assert_eq!(recover_running(&root), 1);
        let m = read_meta(&dir).unwrap().unwrap();
        assert_eq!(m.status, STATUS_INTERRUPTED);
        assert!(m.finished_at.is_some());
    }

    #[test]
    fn cleanup_keeps_finished_and_spares_interrupted() {
        let root = temp_dir("cleanup");
        for i in 0..12u64 {
            make_run(&root, &format!("run-{i}"), 100 + i);
        }
        let interrupted = root.join("run-broken");
        reset(&interrupted, &meta("run-broken", 1, STATUS_INTERRUPTED, 1)).unwrap();
        cleanup_finished(&root);
        let list = list_run_entries(&root);
        assert_eq!(list.len(), MAX_FINISHED_RUNS + 1);
        assert!(list.iter().any(|e| e.run_id == "run-broken"));
        assert!(!list.iter().any(|e| e.run_id == "run-0"));
        assert!(!list.iter().any(|e| e.run_id == "run-1"));
    }

    #[test]
    fn legacy_meta_without_new_fields_still_parses() {
        let dir = temp_dir("legacy-meta");
        std::fs::write(
            meta_path(&dir),
            r#"{"runId":"r","total":1,"startedAt":1,"status":"finished"}"#,
        )
        .unwrap();
        let m = read_meta(&dir).unwrap().unwrap();
        assert!(m.prompts.system_prompt.is_empty());
        assert!(m.dataset_signature.is_empty());
        assert!(m.summary.is_none());
    }

    #[test]
    fn rejects_unsafe_run_ids() {
        assert!(valid_run_id("run-123_abc"));
        assert!(!valid_run_id(""));
        assert!(!valid_run_id("../x"));
        assert!(!valid_run_id("a/b"));
        assert!(!valid_run_id("a.b"));
    }
}
