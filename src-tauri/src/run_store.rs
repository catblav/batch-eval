//! 当前一次运行的结果落盘与恢复。
//!
//! 目录结构（`app_config_dir` 下）：
//!
//! ```text
//! runs/current/meta.json     运行元信息
//! runs/current/results.jsonl 每完成一条追加一行，同一 index 以最后一行为准
//! ```
//!
//! 只保留「当前一次运行」，新跑时截断；重跑 / 断点续跑在同一个文件上追加。

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunMeta {
    pub run_id: String,
    pub total: usize,
    pub started_at: u64,
    #[serde(default)]
    pub finished_at: Option<u64>,
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedRun {
    pub meta: RunMeta,
    pub results: Vec<RowResult>,
    /// 崩溃或磁盘写入中断可能留下半行，解析失败的行数（不静默丢弃）
    pub bad_lines: usize,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn current_dir(app: &tauri::AppHandle) -> PathBuf {
    crate::storage::config_dir(app).join("runs").join("current")
}

pub fn meta_path(dir: &Path) -> PathBuf {
    dir.join("meta.json")
}

pub fn results_path(dir: &Path) -> PathBuf {
    dir.join("results.jsonl")
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
    let text = std::fs::read_to_string(&p).map_err(|e| format!("读取运行信息失败：{e}"))?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| format!("运行信息格式错误：{e}"))
}

/// 新跑：截断结果文件并写 meta。
pub fn reset(dir: &Path, meta: &RunMeta) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建运行目录失败：{e}"))?;
    std::fs::File::create(results_path(dir)).map_err(|e| format!("清空结果文件失败：{e}"))?;
    write_meta(dir, meta)
}

/// 追加模式打开结果文件（重跑 / 续跑不截断）。
pub fn open_append(dir: &Path) -> Result<BufWriter<std::fs::File>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建运行目录失败：{e}"))?;
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

/// 读取当前运行结果：按 index 去重（后写覆盖先写）并排序。
pub fn load_dir(dir: &Path) -> Result<Option<LoadedRun>, String> {
    let mp = meta_path(dir);
    let rp = results_path(dir);
    if !mp.exists() && !rp.exists() {
        return Ok(None);
    }

    let meta = match read_meta(dir)? {
        Some(m) => m,
        None => RunMeta {
            run_id: "unknown".into(),
            total: 0,
            started_at: 0,
            finished_at: None,
            status: STATUS_FINISHED.into(),
            error: None,
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
    }))
}

/// 清空当前运行：删掉结果与元信息文件（不存在视为已清空）。
pub fn clear(app: &tauri::AppHandle) -> Result<(), String> {
    let dir = current_dir(app);
    for p in [meta_path(&dir), results_path(&dir)] {
        if p.exists() {
            std::fs::remove_file(&p).map_err(|e| format!("删除结果文件失败：{e}"))?;
        }
    }
    Ok(())
}

/// 用户主动放弃本次结果：清空磁盘与内存后回到未运行状态。
#[tauri::command]
pub async fn clear_run(app: tauri::AppHandle) -> Result<(), String> {
    clear(&app)
}

/// 读取上次运行结果供前端恢复。
/// 没有运行文件时，尝试从旧版 settings.json 的 `lastResults` 迁移。
#[tauri::command]
pub async fn load_last_run(app: tauri::AppHandle) -> Result<Option<LoadedRun>, String> {
    let dir = current_dir(&app);

    if let Some(mut run) = load_dir(&dir)? {
        // 进程在运行中被杀：meta 还停在 running，这里标记为中断，前端据此提示可续跑
        if run.meta.status == STATUS_RUNNING {
            run.meta.status = STATUS_INTERRUPTED.into();
            run.meta.finished_at = Some(now_ms());
            let _ = write_meta(&dir, &run.meta);
        }
        return Ok(Some(run));
    }

    // 旧版迁移：把 settings.json 里的 lastResults 写进新的结果文件
    if let Some(raw) = crate::storage::read_raw_settings(&app) {
        if let Some(Value::Array(arr)) = raw.get("lastResults") {
            let mut results: Vec<RowResult> = Vec::new();
            let mut bad = 0usize;
            for item in arr {
                match serde_json::from_value::<RowResult>(item.clone()) {
                    Ok(r) => results.push(r),
                    Err(_) => bad += 1,
                }
            }
            if !results.is_empty() {
                results.sort_by_key(|r| r.index);
                let meta = RunMeta {
                    run_id: "legacy".into(),
                    total: results.len(),
                    started_at: now_ms(),
                    finished_at: Some(now_ms()),
                    status: STATUS_FINISHED.into(),
                    error: None,
                };
                reset(&dir, &meta)?;
                let mut w = open_append(&dir)?;
                for r in &results {
                    write_row(&mut w, r).map_err(|e| format!("写入迁移结果失败：{e}"))?;
                }
                w.flush().map_err(|e| format!("写入迁移结果失败：{e}"))?;
                return Ok(Some(LoadedRun {
                    meta,
                    results,
                    bad_lines: bad,
                }));
            }
        }
    }

    Ok(None)
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
            truncated: false,
            status_label: "g".into(),
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("batch-eval-test-{name}-{}", now_ms()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn dedupes_by_index_last_wins() {
        let dir = temp_dir("dedupe");
        let meta = RunMeta {
            run_id: "r".into(),
            total: 3,
            started_at: 1,
            finished_at: None,
            status: STATUS_RUNNING.into(),
            error: None,
        };
        reset(&dir, &meta).unwrap();
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
        let meta = RunMeta {
            run_id: "r".into(),
            total: 2,
            started_at: 1,
            finished_at: None,
            status: STATUS_FINISHED.into(),
            error: None,
        };
        reset(&dir, &meta).unwrap();
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
}
