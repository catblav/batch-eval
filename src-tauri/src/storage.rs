//! 本地文件存储：设置、数据集库、文本文件读写。
//!
//! 数据集库（`app_config_dir` 下）：
//!
//! ```text
//! datasets/index.json   只存索引（id / 名称 / 行数 / 列名 / 来源 / 更新时间）
//! datasets/<id>.json    完整数据
//! dataset.json          当前方案的镜像，供 auto-iterate 脚本继续读取
//! ```
//!
//! 前端只加载索引和按页预览，完整数据由后端在启动任务时读取。

use crate::dataset::{self, Dataset};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn config_dir(app: &tauri::AppHandle) -> PathBuf {
    use tauri::Manager;
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// 旧 identifier 是 `com.batcheval.app`，macOS 上会和 `.app` 扩展名冲突，
/// 已改成 `com.batcheval.desktop`。改 identifier 会换配置目录，这里把旧目录的
/// 设置、数据集和运行记录复制到新目录；只补缺失文件，不覆盖、不删除旧目录。
pub fn migrate_legacy_config_dir(app: &tauri::AppHandle) {
    let new_dir = config_dir(app);
    let Some(parent) = new_dir.parent() else {
        return;
    };
    let old_dir = parent.join("com.batcheval.app");
    if old_dir == new_dir || !old_dir.is_dir() {
        return;
    }
    if new_dir.join("settings.json").exists() {
        return;
    }
    let _ = copy_tree_if_missing(&old_dir, &new_dir);
}

/// 递归复制，目标已存在的文件不覆盖。
fn copy_tree_if_missing(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree_if_missing(&from, &to)?;
        } else if !to.exists() {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// 同目录临时文件 + rename，避免写到一半崩溃把原文件写坏。
/// rename 失败（例如被杀毒软件占用）时回退为直接覆盖，保证保存最终能成功。
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
    }
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let tmp = path.with_file_name(format!("{name}.tmp"));
    std::fs::write(&tmp, bytes).map_err(|e| format!("写入临时文件失败：{e}"))?;
    match std::fs::rename(&tmp, path) {
        Ok(_) => Ok(()),
        Err(_) => {
            let _ = std::fs::remove_file(&tmp);
            std::fs::write(path, bytes).map_err(|e| format!("写入文件失败：{e}"))
        }
    }
}

/// 读取完整 settings.json（含历史遗留字段），仅供迁移使用。
pub fn read_raw_settings(app: &tauri::AppHandle) -> Option<Value> {
    let p = config_dir(app).join("settings.json");
    let text = std::fs::read_to_string(&p).ok()?;
    serde_json::from_str(&text).ok()
}

/// 读取设置文件（第一次运行时返回 null，前端用默认值）。
#[tauri::command]
pub async fn load_settings(app: tauri::AppHandle) -> Result<Option<Value>, String> {
    let p = config_dir(&app).join("settings.json");
    if !p.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&p).map_err(|e| format!("读取设置失败：{e}"))?;
    let mut v: Value = serde_json::from_str(&text).map_err(|e| format!("设置文件格式错误：{e}"))?;
    if let Some(obj) = v.as_object_mut() {
        // 旧版本把数据集和执行结果塞在设置里
        obj.remove("dataset");
        obj.remove("lastResults");
    }
    Ok(Some(v))
}

/// 写设置。首次写入 version 2 之前，把旧文件留一份 `settings.v1.bak`。
#[tauri::command]
pub async fn save_settings(app: tauri::AppHandle, settings: Value) -> Result<(), String> {
    let p = config_dir(&app).join("settings.json");
    let is_v2 = settings.get("version").and_then(|v| v.as_u64()) == Some(2);
    let backup = config_dir(&app).join("settings.v1.bak");
    if is_v2 && p.exists() && !backup.exists() {
        let _ = std::fs::copy(&p, &backup);
    }
    let text = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    atomic_write(&p, text.as_bytes())
}

// ---------- 敏感字段脱敏 ----------

fn sensitive_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    k.contains("authorization")
        || k.contains("x-api-key")
        || k.contains("api-key")
        || k.contains("api_key")
        || k.contains("apikey")
        || k.contains("token")
        || k.contains("secret")
        || k == "key"
}

fn redact_value(v: &mut Value) {
    match v {
        Value::Object(map) => {
            for (k, val) in map.iter_mut() {
                if sensitive_key(k) {
                    *val = Value::String("***".into());
                } else {
                    redact_value(val);
                }
            }
        }
        Value::Array(arr) => {
            for item in arr {
                redact_value(item);
            }
        }
        _ => {}
    }
}

/// 把 JSON 对象字符串里的敏感键换成 `***`；不是合法 JSON 时原样返回。
/// 用于自定义请求头 / 请求体进入 meta、报告、导出前的兜底。
pub fn redact_sensitive_json(raw: &str) -> String {
    if raw.trim().is_empty() {
        return String::new();
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(mut v) => {
            redact_value(&mut v);
            serde_json::to_string(&v).unwrap_or_else(|_| raw.to_string())
        }
        Err(_) => raw.to_string(),
    }
}

// ---------- 数据集库 ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetMeta {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub rows: usize,
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(default)]
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetPreview {
    pub columns: Vec<String>,
    pub rows: Vec<serde_json::Map<String, Value>>,
    pub total: usize,
    pub skipped: usize,
    pub offset: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DatasetIndex {
    #[serde(default)]
    datasets: Vec<DatasetMeta>,
}

fn datasets_root(app: &tauri::AppHandle) -> PathBuf {
    config_dir(app).join("datasets")
}

fn index_path(app: &tauri::AppHandle) -> PathBuf {
    datasets_root(app).join("index.json")
}

fn dataset_file(app: &tauri::AppHandle, id: &str) -> Result<PathBuf, String> {
    if !valid_dataset_id(id) {
        return Err("数据集 ID 不合法".into());
    }
    Ok(datasets_root(app).join(format!("{id}.json")))
}

fn legacy_dataset_path(app: &tauri::AppHandle) -> PathBuf {
    config_dir(app).join("dataset.json")
}

fn valid_dataset_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn read_index(app: &tauri::AppHandle) -> DatasetIndex {
    let p = index_path(app);
    let Ok(text) = std::fs::read_to_string(&p) else {
        return DatasetIndex::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn write_index(app: &tauri::AppHandle, index: &DatasetIndex) -> Result<(), String> {
    let text = serde_json::to_string_pretty(index).map_err(|e| e.to_string())?;
    atomic_write(&index_path(app), text.as_bytes())
}

fn write_dataset_file(app: &tauri::AppHandle, id: &str, ds: &Dataset) -> Result<(), String> {
    let p = dataset_file(app, id)?;
    let text = serde_json::to_string(ds).map_err(|e| e.to_string())?;
    atomic_write(&p, text.as_bytes())
}

fn read_dataset_file(app: &tauri::AppHandle, id: &str) -> Result<Dataset, String> {
    let p = dataset_file(app, id)?;
    let text = std::fs::read_to_string(&p).map_err(|e| format!("读取数据集失败：{e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("数据集文件格式错误：{e}"))
}

fn meta_of(id: &str, name: &str, ds: &Dataset) -> DatasetMeta {
    DatasetMeta {
        id: id.to_string(),
        name: name.to_string(),
        source: ds.source.clone(),
        rows: ds.rows.len(),
        columns: ds.columns.clone(),
        updated_at: crate::run_store::now_ms(),
    }
}

/// 首次启动：把旧版 dataset.json 复制成 `ds-default`「默认数据集」。
/// 旧文件保留为 auto-iterate 镜像，不删。
fn ensure_default_dataset(app: &tauri::AppHandle) -> DatasetIndex {
    let mut index = read_index(app);
    if !index.datasets.is_empty() {
        return index;
    }
    let legacy = legacy_dataset_path(app);
    if !legacy.exists() {
        return index;
    }
    let Ok(text) = std::fs::read_to_string(&legacy) else {
        return index;
    };
    let Ok(ds) = serde_json::from_str::<Dataset>(&text) else {
        return index;
    };
    if ds.rows.is_empty() {
        return index;
    }
    if write_dataset_file(app, "ds-default", &ds).is_err() {
        return index;
    }
    let mut meta = meta_of("ds-default", "默认数据集", &ds);
    if meta.source.trim().is_empty() {
        meta.source = "dataset.json".into();
    }
    index.datasets.push(meta);
    let _ = write_index(app, &index);
    index
}

pub fn load_dataset_by_id(app: &tauri::AppHandle, id: &str) -> Result<Dataset, String> {
    read_dataset_file(app, id)
}

/// 稳定签名：行数 + 列名 + 每行内容的 FNV-1a 哈希。
/// 续跑前用它判断数据集是否被替换过。
pub fn dataset_signature(ds: &Dataset) -> String {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut h = OFFSET;
    let eat = |bytes: &[u8], h: &mut u64| {
        for b in bytes {
            *h ^= *b as u64;
            *h = h.wrapping_mul(PRIME);
        }
    };
    eat(&(ds.rows.len() as u64).to_le_bytes(), &mut h);
    for c in &ds.columns {
        eat(c.as_bytes(), &mut h);
        eat(b"\x1f", &mut h);
    }
    for row in &ds.rows {
        let s = serde_json::to_string(row).unwrap_or_default();
        eat(s.as_bytes(), &mut h);
        eat(b"\x1e", &mut h);
    }
    format!("{}-{:016x}", ds.rows.len(), h)
}

/// 把数据集库里的某份数据复制到旧路径 `dataset.json`，供 auto-iterate 脚本读取。
pub fn sync_dataset_mirror(app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    let ds = read_dataset_file(app, id)?;
    let text = serde_json::to_string(&ds).map_err(|e| e.to_string())?;
    atomic_write(&legacy_dataset_path(app), text.as_bytes())
}

fn new_dataset_id(app: &tauri::AppHandle) -> String {
    let base = format!("ds-{}", crate::run_store::now_ms());
    let index = read_index(app);
    if !index.datasets.iter().any(|d| d.id == base) {
        return base;
    }
    for i in 2..1000 {
        let id = format!("{base}-{i}");
        if !index.datasets.iter().any(|d| d.id == id) {
            return id;
        }
    }
    format!("{base}-{}", index.datasets.len() + 1)
}

fn add_dataset(
    app: &tauri::AppHandle,
    name: &str,
    ds: Dataset,
) -> Result<DatasetMeta, String> {
    let id = new_dataset_id(app);
    write_dataset_file(app, &id, &ds)?;
    let meta = meta_of(&id, name, &ds);
    let mut index = read_index(app);
    index.datasets.push(meta.clone());
    write_index(app, &index)?;
    Ok(meta)
}

fn file_stem(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "新数据集".into())
}

#[tauri::command]
pub async fn list_datasets(app: tauri::AppHandle) -> Result<Vec<DatasetMeta>, String> {
    Ok(ensure_default_dataset(&app).datasets)
}

#[tauri::command]
pub async fn import_dataset_file(
    app: tauri::AppHandle,
    path: String,
    name: Option<String>,
) -> Result<DatasetMeta, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("读取文件失败：{e}"))?;
    let file_name = std::path::Path::new(&path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "数据集".into());
    let ds = dataset::parse_bytes(&bytes, &file_name)?;
    let label = name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| file_stem(&path));
    add_dataset(&app, &label, ds)
}

#[tauri::command]
pub async fn import_dataset_text(
    app: tauri::AppHandle,
    text: String,
    format: Option<String>,
    name: Option<String>,
) -> Result<DatasetMeta, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("内容为空".into());
    }
    let ds = dataset::parse_text(trimmed, format.as_deref())?;
    let label = name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "粘贴数据".into());
    add_dataset(&app, &label, ds)
}

#[tauri::command]
pub async fn preview_dataset(
    app: tauri::AppHandle,
    id: String,
    offset: usize,
    limit: usize,
) -> Result<DatasetPreview, String> {
    let ds = read_dataset_file(&app, &id)?;
    let offset = offset.min(ds.rows.len());
    let limit = limit.clamp(1, 500);
    let end = (offset + limit).min(ds.rows.len());
    Ok(DatasetPreview {
        columns: ds.columns,
        rows: ds.rows[offset..end].to_vec(),
        total: ds.rows.len(),
        skipped: ds.skipped,
        offset,
    })
}

#[tauri::command]
pub async fn get_dataset_row(
    app: tauri::AppHandle,
    id: String,
    index: usize,
) -> Result<Option<serde_json::Map<String, Value>>, String> {
    let ds = read_dataset_file(&app, &id)?;
    Ok(ds.rows.get(index).cloned())
}

#[tauri::command]
pub async fn replace_dataset(
    app: tauri::AppHandle,
    id: String,
    path: String,
) -> Result<DatasetMeta, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("读取文件失败：{e}"))?;
    let file_name = std::path::Path::new(&path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "数据集".into());
    let ds = dataset::parse_bytes(&bytes, &file_name)?;
    let mut index = read_index(&app);
    let pos = index
        .datasets
        .iter()
        .position(|d| d.id == id)
        .ok_or_else(|| "数据集不存在".to_string())?;
    let name = index.datasets[pos].name.clone();
    write_dataset_file(&app, &id, &ds)?;
    let meta = meta_of(&id, &name, &ds);
    index.datasets[pos] = meta.clone();
    write_index(&app, &index)?;
    Ok(meta)
}

#[tauri::command]
pub async fn rename_dataset(
    app: tauri::AppHandle,
    id: String,
    name: String,
) -> Result<DatasetMeta, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("名称不能为空".into());
    }
    let mut index = read_index(&app);
    let pos = index
        .datasets
        .iter()
        .position(|d| d.id == id)
        .ok_or_else(|| "数据集不存在".to_string())?;
    index.datasets[pos].name = name;
    let meta = index.datasets[pos].clone();
    write_index(&app, &index)?;
    Ok(meta)
}

#[tauri::command]
pub async fn delete_dataset(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let mut index = read_index(&app);
    let before = index.datasets.len();
    index.datasets.retain(|d| d.id != id);
    if index.datasets.len() == before {
        return Err("数据集不存在".into());
    }
    write_index(&app, &index)?;
    if let Ok(p) = dataset_file(&app, &id) {
        let _ = std::fs::remove_file(p);
    }
    Ok(())
}

#[tauri::command]
pub async fn sync_dataset_mirror_cmd(
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    sync_dataset_mirror(&app, &id)
}

// ---------- 旧版单数据集命令（保持兼容） ----------

fn dataset_path(app: &tauri::AppHandle) -> PathBuf {
    legacy_dataset_path(app)
}

#[tauri::command]
pub async fn load_dataset(app: tauri::AppHandle) -> Result<Option<Value>, String> {
    let p = dataset_path(&app);
    if p.exists() {
        let text = std::fs::read_to_string(&p).map_err(|e| format!("读取数据集失败：{e}"))?;
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("数据集文件格式错误：{e}"))?;
        return Ok(Some(v));
    }
    if let Some(raw) = read_raw_settings(&app) {
        if let Some(ds) = raw.get("dataset") {
            if ds.get("rows").and_then(|r| r.as_array()).is_some() {
                let text = serde_json::to_string(ds).map_err(|e| e.to_string())?;
                atomic_write(&p, text.as_bytes())?;
                return Ok(Some(ds.clone()));
            }
        }
    }
    Ok(None)
}

#[tauri::command]
pub async fn save_dataset(app: tauri::AppHandle, dataset: Value) -> Result<(), String> {
    let p = dataset_path(&app);
    let text = serde_json::to_string(&dataset).map_err(|e| e.to_string())?;
    atomic_write(&p, text.as_bytes())
}

#[tauri::command]
pub async fn read_text_file(path: String) -> Result<String, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("读取文件失败：{e}"))?;
    Ok(match String::from_utf8(bytes.clone()) {
        Ok(s) => s.trim_start_matches('\u{feff}').to_string(),
        Err(_) => {
            let (cow, _, _) = encoding_rs::GB18030.decode(&bytes);
            cow.to_string()
        }
    })
}

#[tauri::command]
pub async fn save_text_file(path: String, content: String) -> Result<(), String> {
    if let Some(parent) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&path, content).map_err(|e| format!("写入文件失败：{e}"))
}

/// 追加写入，供大结果集分块导出，避免前端一次拼出整份大字符串。
#[tauri::command]
pub async fn append_text_file(path: String, content: String) -> Result<(), String> {
    if let Some(parent) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("打开文件失败：{e}"))?;
    f.write_all(content.as_bytes())
        .map_err(|e| format!("写入文件失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_sensitive_keys_recursively() {
        let raw = r#"{"Authorization":"Bearer abc","nested":{"x-api-key":"k","ok":1},"list":[{"token":"t"}],"X-Org-Id":"org"}"#;
        let out = redact_sensitive_json(raw);
        assert!(out.contains(r#""ok":1"#));
        assert!(out.contains("org"));
        assert!(!out.contains("Bearer abc"));
        assert!(!out.contains(r#""k""#));
        assert!(!out.contains(r#""t""#));
        assert_eq!(out.matches("***").count(), 3);
    }

    #[test]
    fn redaction_leaves_non_json_untouched() {
        assert_eq!(redact_sensitive_json("not json"), "not json");
        assert_eq!(redact_sensitive_json(""), "");
    }

    #[test]
    fn copy_tree_fills_missing_without_overwriting() {
        let root = std::env::temp_dir().join(format!("batch-eval-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let src = root.join("old");
        let dst = root.join("new");
        std::fs::create_dir_all(src.join("datasets")).unwrap();
        std::fs::write(src.join("settings.json"), "old").unwrap();
        std::fs::write(src.join("datasets/index.json"), "idx").unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(dst.join("settings.json"), "new").unwrap();

        copy_tree_if_missing(&src, &dst).unwrap();

        assert_eq!(
            std::fs::read_to_string(dst.join("settings.json")).unwrap(),
            "new"
        );
        assert_eq!(
            std::fs::read_to_string(dst.join("datasets/index.json")).unwrap(),
            "idx"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn signature_changes_when_data_changes() {
        let a = Dataset {
            columns: vec!["q".into()],
            rows: vec![serde_json::json!({"q":"1"}).as_object().unwrap().clone()],
            source: "t".into(),
            skipped: 0,
        };
        let mut b = a.clone();
        assert_eq!(dataset_signature(&a), dataset_signature(&b));
        b.rows[0].insert("q".into(), Value::String("2".into()));
        assert_ne!(dataset_signature(&a), dataset_signature(&b));
    }
}
