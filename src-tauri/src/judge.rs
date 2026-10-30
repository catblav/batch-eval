use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// 评测结果的归一化形态。无论裁判模型输出什么结构，都收敛到这一层。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    pub pass: bool,
    pub error_types: Vec<String>,
    pub reasons: Vec<String>,
    pub violations: Vec<Value>,
    pub details: String,
    pub parse_ok: bool,
    pub raw: String,
}

pub const PARSE_FAIL: &str = "评测输出解析失败";

fn strip_fences(s: &str) -> String {
    let t = s.trim();
    if !t.starts_with("```") {
        return t.to_string();
    }
    let after = t.trim_start_matches("```");
    let after = match after.find('\n') {
        Some(i) => &after[i + 1..],
        None => after,
    };
    let end = after.rfind("```").unwrap_or(after.len());
    after[..end].trim().to_string()
}

/// 从任意文本中找出第一个括号配平的 JSON 对象。
fn find_json_object(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let start = s.find('{')?;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escape = false;
    for i in start..bytes.len() {
        let c = bytes[i] as char;
        if in_str {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

fn get_ci<'a>(obj: &'a Map<String, Value>, names: &[&str]) -> Option<&'a Value> {
    for (k, v) in obj {
        let lk = k.to_lowercase().replace('_', "").replace('-', "");
        for n in names {
            if lk == n.to_lowercase().replace('_', "") {
                return Some(v);
            }
        }
    }
    None
}

fn as_string_list(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => {
            if s.trim().is_empty() {
                Vec::new()
            } else {
                vec![s.trim().to_string()]
            }
        }
        Value::Array(arr) => arr
            .iter()
            .filter_map(|x| match x {
                Value::String(s) => Some(s.trim().to_string()),
                Value::Object(o) => get_ci(o, &["errorType", "type", "name", "category"])
                    .and_then(|t| t.as_str())
                    .map(|s| s.trim().to_string()),
                _ => None,
            })
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn as_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.trim().to_lowercase().as_str() {
            "true" | "pass" | "passed" | "通过" | "合格" | "yes" => Some(true),
            "false" | "fail" | "failed" | "不通过" | "不合格" | "no" => Some(false),
            _ => None,
        },
        Value::Number(n) => Some(n.as_f64().unwrap_or(0.0) != 0.0),
        _ => None,
    }
}

/// 归一化分类名：去掉空白与常见修饰，保持枚举字面可比。
pub fn normalize_category(s: &str) -> String {
    s.trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`' || c == '。' || c == '、' || c == ',')
        .trim()
        .to_string()
}

pub fn parse_verdict(raw: &str) -> Verdict {
    let cleaned = strip_fences(raw);
    let mut v = Verdict {
        raw: raw.to_string(),
        ..Default::default()
    };

    let json_str = find_json_object(&cleaned);
    if let Some(js) = json_str {
        if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(js) {
            let mut violations = Vec::new();
            if let Some(vs) = get_ci(&obj, &["violations", "issues", "problems"]) {
                if let Value::Array(arr) = vs {
                    violations = arr.clone();
                }
            }
            let mut error_types: Vec<String> = Vec::new();
            if let Some(et) = get_ci(&obj, &["errorType", "errorTypes", "errors", "categories", "category"])
            {
                error_types = as_string_list(et).into_iter().map(|s| normalize_category(&s)).collect();
            }
            if error_types.is_empty() && !violations.is_empty() {
                for viol in &violations {
                    if let Value::Object(o) = viol {
                        if let Some(t) = get_ci(o, &["errorType", "type", "category", "name"]) {
                            error_types.extend(as_string_list(t));
                        }
                    }
                }
            }
            let mut dedup: Vec<String> = Vec::new();
            for t in error_types {
                let t = normalize_category(&t);
                if !t.is_empty() && !dedup.iter().any(|x| x == &t) {
                    dedup.push(t);
                }
            }

            let mut reasons: Vec<String> = Vec::new();
            for viol in &violations {
                if let Value::Object(o) = viol {
                    let mut s = String::new();
                    if let Some(ev) = get_ci(o, &["evidence", "quote", "snippet"]) {
                        if let Some(e) = ev.as_str() {
                            s.push_str(&format!("证据：{}", e.trim()));
                        }
                    }
                    if let Some(rs) = get_ci(o, &["reason", "why", "explanation"]) {
                        if let Some(r) = rs.as_str() {
                            if !s.is_empty() {
                                s.push_str("｜");
                            }
                            s.push_str(r.trim());
                        }
                    }
                    if !s.is_empty() {
                        reasons.push(s);
                    }
                }
            }
            let details = get_ci(&obj, &["details", "detail", "summary", "comment"])
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if let Some(d) = get_ci(&obj, &["errorReason", "reason"])
                .and_then(|d| d.as_str())
            {
                if !d.trim().is_empty() && !reasons.iter().any(|r| r.contains(d.trim())) {
                    reasons.push(d.trim().to_string());
                }
            }

            let pass = get_ci(&obj, &["pass", "passed", "ok", "result", "verdict"])
                .and_then(as_bool)
                .unwrap_or_else(|| dedup.is_empty());

            v.pass = pass;
            if pass {
                v.reasons.clear();
                v.error_types = Vec::new();
            } else {
                v.error_types = dedup;
            }
            v.reasons = reasons;
            v.violations = violations;
            v.details = details;
            v.parse_ok = true;
            if !v.pass && v.error_types.is_empty() {
                v.parse_ok = false;
            }
            return v;
        }
    }

    // 退化路径：没有可解析 JSON，用文本线索兜底
    let lower = cleaned.to_lowercase();
    let pass_hint = lower.contains("\"pass\": true")
        || lower.contains("\"pass\":true")
        || lower.contains("判为通过")
        || lower.contains("判定：通过");
    let fail_hint = lower.contains("判为不通过")
        || lower.contains("判定：不通过")
        || lower.contains("\"pass\": false")
        || lower.contains("\"pass\":false");
    v.pass = if fail_hint {
        false
    } else {
        pass_hint
    };
    v.details = cleaned.chars().take(400).collect();
    v.parse_ok = false;
    if !v.pass {
        v.error_types = vec![PARSE_FAIL.to_string()];
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clean_json() {
        let raw = r#"{"violations":[{"errorType":"字段缺失","evidence":"x","reason":"缺少必填字段"}],"pass":false,"errorType":["字段缺失"],"details":"缺字段"}"#;
        let v = parse_verdict(raw);
        assert!(!v.pass);
        assert_eq!(v.error_types, vec!["字段缺失"]);
        assert!(v.parse_ok);
    }

    #[test]
    fn parses_fenced_json() {
        let raw = "```json\n{\"violations\":[],\"pass\":true,\"errorType\":[],\"details\":\"\"}\n```";
        let v = parse_verdict(raw);
        assert!(v.pass);
        assert!(v.error_types.is_empty());
    }

    #[test]
    fn falls_back_on_garbage() {
        let v = parse_verdict("抱歉我无法判断");
        assert!(!v.parse_ok);
        assert_eq!(v.error_types, vec![PARSE_FAIL]);
    }
}
