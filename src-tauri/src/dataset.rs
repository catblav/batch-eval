use calamine::{open_workbook_from_rs, Data, Reader, Xlsx};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::io::Cursor;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dataset {
    pub columns: Vec<String>,
    pub rows: Vec<Map<String, Value>>,
    pub source: String,
    /// 解析时被跳过的坏行数（JSONL 格式错误、CSV 记录残缺），前端提示用
    #[serde(default)]
    pub skipped: usize,
}

fn decode(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.trim_start_matches('\u{feff}').to_string(),
        Err(_) => {
            let (cow, _, _) = encoding_rs::GB18030.decode(bytes);
            cow.trim_start_matches('\u{feff}').to_string()
        }
    }
}

fn sniff_delimiter(line: &str) -> char {
    let commas = line.matches(',').count();
    let tabs = line.matches('\t').count();
    if tabs > commas {
        '\t'
    } else {
        ','
    }
}

fn from_csv_text(text: &str, source: &str) -> Result<Dataset, String> {
    let first_line = text.lines().next().unwrap_or("");
    let delim = sniff_delimiter(first_line);
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delim as u8)
        .flexible(true)
        .from_reader(text.as_bytes());
    let headers = rdr
        .headers()
        .map_err(|e| format!("读取表头失败：{e}"))?
        .clone();
    let columns: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let t = h.trim();
            if t.is_empty() {
                format!("列{}", i + 1)
            } else {
                t.to_string()
            }
        })
        .collect();
    let mut rows = Vec::new();
    let mut skipped = 0usize;
    for rec in rdr.records() {
        let rec = match rec {
            Ok(r) => r,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let mut m = Map::new();
        let mut all_empty = true;
        for (i, col) in columns.iter().enumerate() {
            let val = rec.get(i).unwrap_or("").trim().to_string();
            if !val.is_empty() {
                all_empty = false;
            }
            m.insert(col.clone(), Value::String(val));
        }
        if !all_empty {
            rows.push(m);
        }
    }
    Ok(Dataset {
        columns,
        rows,
        source: source.to_string(),
        skipped,
    })
}

fn from_json_text(text: &str, source: &str) -> Result<Dataset, String> {
    let trimmed = text.trim();
    let mut skipped = 0usize;
    let items: Vec<Value> = if trimmed.starts_with('[') {
        serde_json::from_str(trimmed).map_err(|e| format!("JSON 数组解析失败：{e}"))?
    } else {
        // JSONL：坏行不再静默丢弃，计数后随 Dataset 返回
        let mut items = Vec::new();
        for line in trimmed.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str::<Value>(line) {
                Ok(v) if v.is_object() => items.push(v),
                _ => skipped += 1,
            }
        }
        items
    };
    let mut columns: Vec<String> = Vec::new();
    let mut rows = Vec::new();
    for item in items {
        if let Value::Object(obj) = item {
            for k in obj.keys() {
                if !columns.contains(k) {
                    columns.push(k.clone());
                }
            }
            rows.push(obj);
        } else {
            skipped += 1;
        }
    }
    if rows.is_empty() {
        return Err("没有解析到任何数据行（需为对象数组或每行一个对象）".into());
    }
    Ok(Dataset {
        columns,
        rows,
        source: source.to_string(),
        skipped,
    })
}

fn cell_to_string(d: &Data) -> String {
    match d {
        Data::Empty => String::new(),
        Data::String(s) => s.trim().to_string(),
        Data::Float(f) => {
            if f.fract() == 0.0 {
                format!("{}", *f as i64)
            } else {
                f.to_string()
            }
        }
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => b.to_string(),
        Data::DateTime(dt) => dt.to_string(),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("{:?}", e),
    }
}

fn from_xlsx_bytes(bytes: &[u8], source: &str) -> Result<Dataset, String> {
    let cursor = Cursor::new(bytes.to_vec());
    let mut wb: Xlsx<_> = open_workbook_from_rs(cursor).map_err(|e| format!("打开 Excel 失败：{e}"))?;
    let sheet = wb
        .sheet_names()
        .first()
        .cloned()
        .ok_or_else(|| "Excel 中没有工作表".to_string())?;
    let range = wb
        .worksheet_range(&sheet)
        .map_err(|e| format!("读取工作表失败：{e}"))?;
    let mut iter = range.rows();
    let header = iter.next().ok_or_else(|| "Excel 内容为空".to_string())?;
    let columns: Vec<String> = header
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let t = cell_to_string(c);
            if t.is_empty() {
                format!("列{}", i + 1)
            } else {
                t
            }
        })
        .collect();
    let mut rows = Vec::new();
    for row in iter {
        let mut m = Map::new();
        let mut empty = true;
        for (i, col) in columns.iter().enumerate() {
            let val = row.get(i).map(cell_to_string).unwrap_or_default();
            if !val.is_empty() {
                empty = false;
            }
            m.insert(col.clone(), Value::String(val));
        }
        if !empty {
            rows.push(m);
        }
    }
    Ok(Dataset {
        columns,
        rows,
        source: source.to_string(),
        skipped: 0,
    })
}

fn from_plain_text(text: &str, source: &str) -> Result<Dataset, String> {
    let blocks: Vec<String> = if text.contains("\n---") {
        text.split("\n---")
            .map(|s| s.trim().trim_start_matches('-').trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        text.split("\n\n")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    if blocks.is_empty() {
        return Err("没有解析到内容".into());
    }
    let columns = vec!["content".to_string()];
    let rows = blocks
        .into_iter()
        .map(|b| {
            let mut m = Map::new();
            m.insert("content".to_string(), Value::String(b));
            m
        })
        .collect();
    Ok(Dataset {
        columns,
        rows,
        source: source.to_string(),
        skipped: 0,
    })
}

fn detect_and_parse(bytes: &[u8], filename: &str) -> Result<Dataset, String> {
    let lower = filename.to_lowercase();
    if lower.ends_with(".xlsx") || lower.ends_with(".xlsm") || bytes.starts_with(b"PK\x03\x04") {
        return from_xlsx_bytes(bytes, filename);
    }
    let text = decode(bytes);
    if filename.to_lowercase().ends_with(".jsonl") || filename.to_lowercase().ends_with(".json") {
        return from_json_text(&text, filename);
    }
    if filename.to_lowercase().ends_with(".txt") || filename.to_lowercase().ends_with(".md") {
        return from_plain_text(&text, filename);
    }
    from_csv_text(&text, filename)
}

#[tauri::command]
pub async fn parse_dataset_file(path: String) -> Result<Dataset, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("读取文件失败：{e}"))?;
    let name = std::path::Path::new(&path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or(path.clone());
    let mut ds = detect_and_parse(&bytes, &name)?;
    ds.source = name;
    Ok(ds)
}

/// 直接粘贴的文本：优先按 JSON 解析，其次 CSV/TSV，最后按文本块切分。
#[tauri::command]
pub async fn parse_dataset_text(text: String, format: Option<String>) -> Result<Dataset, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("内容为空".into());
    }
    let fmt = format.unwrap_or_default();
    let mut ds = match fmt.as_str() {
        "json" => from_json_text(trimmed, "粘贴内容")?,
        "csv" => from_csv_text(trimmed, "粘贴内容")?,
        "text" => from_plain_text(trimmed, "粘贴内容")?,
        _ => {
            if trimmed.starts_with('[') || trimmed.starts_with('{') {
                from_json_text(trimmed, "粘贴内容")
                    .or_else(|_| from_csv_text(trimmed, "粘贴内容"))
                    .or_else(|_| from_plain_text(trimmed, "粘贴内容"))?
            } else if trimmed.lines().next().map(|l| l.contains('\t') || l.contains(',')) == Some(true)
            {
                from_csv_text(trimmed, "粘贴内容")?
            } else {
                from_plain_text(trimmed, "粘贴内容")?
            }
        }
    };
    ds.source = "粘贴内容".to_string();
    Ok(ds)
}
