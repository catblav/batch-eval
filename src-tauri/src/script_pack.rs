//! 自动迭代脚本随安装包分发。
//!
//! 脚本和两份说明在编译时嵌入二进制（`include_str!`），不再单独依赖安装目录里的资源文件，
//! 也不会因为打包器对中文路径的处理方式而产生差异。导出时写到用户选择的工作目录：
//!
//! ```text
//! <工作目录>/
//! ├─ 怎么用.txt
//! ├─ 脚本/脚本_提示词自动迭代.py
//! ├─ 说明文档/自动迭代_说明.md      给 agent 的操作手册
//! ├─ 说明文档/提示词自动迭代脚本.md  给用户的产品说明
//! ├─ 评测提示词/
//! ├─ 评测数据/
//! ├─ 评测输出/
//! └─ 提示词历史/
//! ```
//!
//! 只写这四个固定文件、只创建空目录，不动工作目录里的其他任何内容。

use std::fs;
use std::path::{Path, PathBuf};

const SCRIPT: &str = include_str!("../resources/auto-iterate/script_auto_iterate.py");
const GUIDE_AGENT: &str = include_str!("../resources/auto-iterate/guide_agent.md");
const GUIDE_USER: &str = include_str!("../resources/auto-iterate/guide_user.md");

/// 导出后脚本依赖这些目录的相对位置，不能改名。
const DIRS: &[&str] = &["脚本", "说明文档", "评测提示词", "评测数据", "评测输出", "提示词历史"];

fn write_if_changed(path: &Path, content: &str) -> Result<(), String> {
    if let Ok(old) = fs::read(path) {
        if old == content.as_bytes() {
            return Ok(());
        }
    }
    fs::write(path, content.as_bytes())
        .map_err(|e| format!("写入 {} 失败：{e}", path.display()))
}

#[tauri::command]
pub async fn export_script_pack(dest: String) -> Result<String, String> {
    let root = PathBuf::from(dest.trim());
    if root.as_os_str().is_empty() {
        return Err("没有选择导出目录".into());
    }
    fs::create_dir_all(&root).map_err(|e| format!("创建目录失败：{e}"))?;
    for dir in DIRS {
        fs::create_dir_all(root.join(dir)).map_err(|e| format!("创建 {} 失败：{e}", dir))?;
    }
    write_if_changed(&root.join("脚本").join("脚本_提示词自动迭代.py"), SCRIPT)?;
    write_if_changed(&root.join("说明文档").join("自动迭代_说明.md"), GUIDE_AGENT)?;
    write_if_changed(
        &root.join("说明文档").join("提示词自动迭代脚本.md"),
        GUIDE_USER,
    )?;

    let howto = format!(
        "把这个文件夹交给 agent，复制下面这句：\n\n请读取工作目录：{} 并阅读使用说明：说明文档/自动迭代_说明.md\n",
        root.display()
    );
    write_if_changed(&root.join("怎么用.txt"), &howto)?;

    Ok(root.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn export_writes_pack_and_keeps_existing_output() {
        let dir = std::env::temp_dir().join(format!("heimao-script-pack-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("评测输出")).unwrap();
        fs::write(dir.join("评测输出").join("result_old.csv"), "keep").unwrap();

        export_script_pack(dir.display().to_string()).await.unwrap();

        assert_eq!(
            fs::read_to_string(dir.join("脚本").join("脚本_提示词自动迭代.py")).unwrap(),
            SCRIPT
        );
        assert_eq!(
            fs::read_to_string(dir.join("说明文档").join("自动迭代_说明.md")).unwrap(),
            GUIDE_AGENT
        );
        assert_eq!(
            fs::read_to_string(dir.join("说明文档").join("提示词自动迭代脚本.md")).unwrap(),
            GUIDE_USER
        );
        assert_eq!(
            fs::read_to_string(dir.join("评测输出").join("result_old.csv")).unwrap(),
            "keep"
        );
        for dir_name in DIRS {
            assert!(dir.join(dir_name).is_dir(), "{dir_name} 未创建");
        }
        let howto = fs::read_to_string(dir.join("怎么用.txt")).unwrap();
        assert!(howto.contains("说明文档/自动迭代_说明.md"));

        let _ = fs::remove_dir_all(&dir);
    }
}
