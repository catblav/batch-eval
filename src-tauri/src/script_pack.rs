//! 自动迭代脚本的安装位置。
//!
//! 脚本和说明作为资源随应用安装（见 `tauri.conf.json` 的 `bundle.resources`），
//! 不往用户目录复制。用户自己建工作目录放数据，界面只负责把脚本、说明的路径
//! 和一句可复制的引导语给出来。

use serde::Serialize;
use std::path::PathBuf;
use tauri::Manager;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoIteratePaths {
    pub dir: String,
    pub script: String,
    pub guide: String,
    pub user_guide: String,
}

/// 打包后从资源目录取；开发时 `tauri dev` 可能还没复制资源，回退到仓库里的源文件。
fn resource_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(root) = app.path().resource_dir() {
        let dir = root.join("auto-iterate");
        if dir.join("script_auto_iterate.py").is_file() {
            return dir;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join("auto-iterate")
}

#[tauri::command]
pub fn auto_iterate_paths(app: tauri::AppHandle) -> Result<AutoIteratePaths, String> {
    let dir = resource_dir(&app);
    let script = dir.join("script_auto_iterate.py");
    let guide = dir.join("guide_agent.md");
    let user_guide = dir.join("guide_user.md");
    if !script.is_file() || !guide.is_file() {
        return Err(format!(
            "安装目录里缺少自动迭代脚本：{}。请重新安装应用。",
            dir.display()
        ));
    }
    Ok(AutoIteratePaths {
        dir: dir.display().to_string(),
        script: script.display().to_string(),
        guide: guide.display().to_string(),
        user_guide: user_guide.display().to_string(),
    })
}
