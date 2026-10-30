use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn config_dir(app: &tauri::AppHandle) -> PathBuf {
    use tauri::Manager;
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
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

/// 读取完整 settings.json（含历史遗留的 dataset / lastResults），仅供迁移使用。
pub fn read_raw_settings(app: &tauri::AppHandle) -> Option<Value> {
    let p = config_dir(app).join("settings.json");
    let text = std::fs::read_to_string(&p).ok()?;
    serde_json::from_str(&text).ok()
}

/// 读取设置文件（第一次运行时返回 null，前端用默认值）。
/// 旧版本把 dataset / lastResults 塞在设置里，这里剔除后再返回，避免巨型 JSON 走一遍 IPC。
#[tauri::command]
pub async fn load_settings(app: tauri::AppHandle) -> Result<Option<Value>, String> {
    let p = config_dir(&app).join("settings.json");
    if !p.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&p).map_err(|e| format!("读取设置失败：{e}"))?;
    let mut v: Value = serde_json::from_str(&text).map_err(|e| format!("设置文件格式错误：{e}"))?;
    if let Some(obj) = v.as_object_mut() {
        obj.remove("dataset");
        obj.remove("lastResults");
    }
    Ok(Some(v))
}

#[tauri::command]
pub async fn save_settings(app: tauri::AppHandle, settings: Value) -> Result<(), String> {
    let p = config_dir(&app).join("settings.json");
    let text = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    atomic_write(&p, text.as_bytes())
}

fn dataset_path(app: &tauri::AppHandle) -> PathBuf {
    config_dir(app).join("dataset.json")
}

/// 读取数据集；旧版本存在 settings.json 里的会自动迁移到独立文件。
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
    // 数据集可能几十 MB，用紧凑 JSON，不 pretty
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
