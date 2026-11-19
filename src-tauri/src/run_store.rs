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

/// 与前端 EXCLUDED_CATEGORIES 保持同一口径
pub const TARGET_FAIL: &str = "被测模型调用失败";
pub const JUDGE_FAIL: &str = "评测模型调用失败";

/// 一个数据注入槽位：勾选启用 + 数据模板文本（可写引导语 + {{列}}）。
/// 评测侧还有 prompt_in（评测提示词本体是否发到该槽位）与 data_in
/// （被测数据 {{input}}/{{output}} 是否在该槽位渲染；不勾则保留字面花括号）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InjectSlot {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub text: String,
    /// 评测提示词本体是否发到该槽位
    #[serde(default)]
    pub prompt_in: bool,
    /// 被测数据（{{input}}/{{output}}）是否在该槽位渲染
    #[serde(default)]
    pub data_in: bool,
}

/// 四个消息槽位的数据注入配置：被测 system / 被测 user / 评测 system / 评测 user
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InjectConfig {
    #[serde(default)]
    pub target_system: InjectSlot,
    #[serde(default)]
    pub target_user: InjectSlot,
    #[serde(default)]
    pub judge_system: InjectSlot,
    #[serde(default)]
    pub judge_user: InjectSlot,
}

/// 启动任务时对方案三份提示词的快照，重跑失败项前用它判断方案是否改过。
/// injections 是四个消息槽位的数据注入配置：评测提示词本体与被测数据的去向都由它决定。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PromptSnapshot {
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub eval_prompt: String,
    /// 四个消息槽位的数据注入配置
    #[serde(default)]
    pub injections: InjectConfig,
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
    /// 任务名称，由前端按「方案名 · 后缀 · 第N版」拼好传入
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
    crate::storage::data_dir(app).join("runs")
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

/// 超出保存范围的任务目录路径。`days == 0` 表示永久保存，一律不清理。
/// 正在跑的任务永不删；中断 / 失败 / 取消同样是记录，按时间一起清。
fn expired_run_dirs(root: &Path, days: u64) -> Vec<PathBuf> {
    if days == 0 {
        return Vec::new();
    }
    let cutoff = now_ms().saturating_sub(days.saturating_mul(86_400_000));
    let Ok(rd) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
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
        if meta.status == STATUS_RUNNING {
            continue;
        }
        if meta.finished_at.unwrap_or(meta.started_at) < cutoff {
            out.push(p);
        }
    }
    out
}

/// 按保存范围清理，返回清理条数。days 为 0 时不做任何事。
pub fn cleanup_expired_in(root: &Path, days: u64) -> usize {
    let dirs = expired_run_dirs(root, days);
    let mut n = 0;
    for p in dirs {
        if std::fs::remove_dir_all(&p).is_ok() {
            n += 1;
        }
    }
    n
}

/// 保存范围（天），0 = 永久。读不到或非法一律按永久处理，宁可留数据不乱删。
/// 用同步的 read_raw_settings：启动时的清理跑在 setup 里，没法 await 命令。
pub fn retention_days(app: &tauri::AppHandle) -> u64 {
    crate::storage::read_raw_settings(app)
        .and_then(|v| v.get("retentionDays").and_then(|d| d.as_u64()))
        .unwrap_or(0)
}

/// 启动时与每个任务结束后调用：按当前保存范围清理过期记录
pub fn cleanup_expired(app: &tauri::AppHandle) -> usize {
    cleanup_expired_in(&runs_root(app), retention_days(app))
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

/// 改成某个保存范围后会清掉几条，只数不删。给前端的确认弹窗用，
/// 和真正清理共用同一套判定，避免两边口径走偏。
#[tauri::command]
pub async fn preview_retention(app: tauri::AppHandle, days: u64) -> Result<usize, String> {
    Ok(expired_run_dirs(&runs_root(&app), days).len())
}

/// 立刻按给定保存范围清理一次，返回清理条数。
#[tauri::command]
pub async fn apply_retention(app: tauri::AppHandle, days: u64) -> Result<usize, String> {
    Ok(cleanup_expired_in(&runs_root(&app), days))
}

/// 清空某个任务的结果：截断结果文件，任务退回「已中断」。
///
/// 数据集快照和提示词快照都保留，所以清空后可以直接续跑，等于按当前方案全量重跑。
#[tauri::command]
pub async fn clear_run(app: tauri::AppHandle, run_id: String) -> Result<(), String> {
    let dir = run_dir(&app, &run_id)?;
    let mut meta = read_meta(&dir)?.ok_or_else(|| "任务不存在".to_string())?;
    if meta.status == STATUS_RUNNING {
        return Err("任务正在运行，先取消再清空".into());
    }
    std::fs::File::create(results_path(&dir)).map_err(|e| format!("清空结果文件失败：{e}"))?;
    meta.status = STATUS_INTERRUPTED.to_string();
    meta.summary = None;
    meta.error = None;
    meta.finished_at = None;
    write_meta(&dir, &meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(index: usize, output: &str) -> RowResult {
        RowResult {
            index,
            system: String::new(),
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
    fn cleanup_drops_only_expired_records() {
        let root = temp_dir("cleanup");
        let day = 86_400_000u64;
        let now = now_ms();
        // 三天内的新记录
        make_run(&root, "run-fresh", now - day);
        // 十天前的旧记录：过期
        make_run(&root, "run-old", now - 10 * day);
        // 过期但正在跑：留着
        let running = root.join("run-running");
        let mut m = meta("run-running", 1, STATUS_RUNNING, now - 10 * day);
        m.finished_at = None;
        reset(&running, &m).unwrap();
        // 过期且中断：算记录，一起清
        let broken = root.join("run-broken");
        reset(&broken, &meta("run-broken", 1, STATUS_INTERRUPTED, now - 10 * day)).unwrap();

        assert_eq!(cleanup_expired_in(&root, 3), 2);
        let list = list_run_entries(&root);
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|e| e.run_id == "run-fresh"));
        assert!(list.iter().any(|e| e.run_id == "run-running"));
    }

    #[test]
    fn cleanup_forever_keeps_everything() {
        let root = temp_dir("cleanup-forever");
        let now = now_ms();
        make_run(&root, "run-old", now - 365 * 86_400_000);
        assert_eq!(cleanup_expired_in(&root, 0), 0);
        assert_eq!(list_run_entries(&root).len(), 1);
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
