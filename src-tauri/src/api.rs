use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Instant;

/// 一个 API 连接配置。protocol 为 "openai"（OpenAI 兼容 /chat/completions）或 "anthropic"（/v1/messages）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiProfile {
    pub protocol: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_tokens: Option<u32>,
    /// 上下文长度（tokens）。设置后超出预算的输入会被截断，避免请求被上游拒绝。
    pub context_window: Option<u32>,
    pub timeout_secs: Option<u64>,
    /// 额外请求体字段，JSON 对象字符串，如 {"seed":1}
    pub extra_body: Option<String>,
    /// 额外请求头，JSON 对象字符串
    pub extra_headers: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatResult {
    pub text: String,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub latency_ms: u64,
    pub truncated: bool,
    pub finish_reason: Option<String>,
}

pub fn build_client(p: &ApiProfile) -> Result<reqwest::Client, String> {
    let secs = p.timeout_secs.unwrap_or(180).max(5);
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(secs))
        .connect_timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))
}

fn endpoint(base: &str, protocol: &str) -> String {
    let b = base.trim().trim_end_matches('/').to_string();
    if b.ends_with("/chat/completions") || b.ends_with("/messages") {
        return b;
    }
    let tail = if protocol == "anthropic" { "/messages" } else { "/chat/completions" };
    if b.ends_with("/v1") {
        format!("{b}{tail}")
    } else {
        format!("{b}/v1{tail}")
    }
}

fn parse_json_object(raw: &Option<String>) -> Option<serde_json::Map<String, Value>> {
    let s = raw.as_ref()?.trim();
    if s.is_empty() {
        return None;
    }
    match serde_json::from_str::<Value>(s) {
        Ok(Value::Object(m)) => Some(m),
        _ => None,
    }
}

/// 粗略估算 token 数：中日韩字符按 1 计，其它按 0.35 计。
fn estimate_tokens(s: &str) -> u64 {
    let mut cjk = 0u64;
    let mut other = 0u64;
    for ch in s.chars() {
        let c = ch as u32;
        if (0x3000..=0x9FFF).contains(&c) || (0xF900..=0xFAFF).contains(&c) || (0xFF00..=0xFFEF).contains(&c)
        {
            cjk += 1;
        } else {
            other += 1;
        }
    }
    cjk + (other * 35 / 100) + 1
}

/// 把一段文本按预算头尾保留截断。
fn truncate_middle(text: &str, budget_tokens: u64) -> (String, bool) {
    let est = estimate_tokens(text);
    if est <= budget_tokens {
        return (text.to_string(), false);
    }
    let chars: Vec<char> = text.chars().collect();
    let ratio = budget_tokens as f64 / est as f64;
    let keep = ((chars.len() as f64) * ratio * 0.92) as usize;
    if keep < 8 {
        // 预算太小也至少保留开头一小段，避免发出空请求
        let n = chars.len().min(8);
        return (chars[..n].iter().collect(), true);
    }
    // 头 60% 尾 40%
    let head = keep * 3 / 5;
    let tail = keep - head;
    let text = format!(
        "{}\n\n……（超出上下文长度，中间部分已省略）……\n\n{}",
        chars[..head].iter().collect::<String>(),
        chars[chars.len() - tail..].iter().collect::<String>()
    );
    (text, true)
}

/// 按上下文长度裁剪系统提示词与用户输入，保留开头与结尾。
/// 返回（系统提示词, 用户输入, 是否发生截断）。
pub fn fit_context(p: &ApiProfile, system: &str, user: &str) -> (String, String, bool) {
    let budget = match p.context_window {
        Some(w) if w > 512 => (w as u64).saturating_sub(p.max_tokens.unwrap_or(2048) as u64 + 256),
        _ => return (system.to_string(), user.to_string(), false),
    };
    if budget < 32 {
        return (system.to_string(), user.to_string(), false);
    }

    let mut sys = system.to_string();
    let sys_tokens = estimate_tokens(system);
    // 系统提示词已占满预算时，也要截断它，否则请求会直接被上游拒绝
    let remain = if sys_tokens >= budget {
        let sys_budget = (budget * 7 / 10).max(32);
        let (t, cut) = truncate_middle(system, sys_budget);
        sys = t;
        let u = budget.saturating_sub(sys_budget).max(16);
        let (user_fit, ucut) = truncate_middle(user, u);
        return (sys, user_fit, cut || ucut);
    } else {
        budget - sys_tokens
    };

    let (user_fit, cut) = truncate_middle(user, remain);
    (sys, user_fit, cut)
}

fn build_body(p: &ApiProfile, system: &str, user: &str) -> Value {
    let mut body = if p.protocol == "anthropic" {
        json!({
            "model": p.model,
            "max_tokens": p.max_tokens.unwrap_or(2048),
            "messages": [{ "role": "user", "content": user }],
        })
    } else {
        let mut msgs = Vec::new();
        if !system.trim().is_empty() {
            msgs.push(json!({ "role": "system", "content": system }));
        }
        msgs.push(json!({ "role": "user", "content": user }));
        json!({
            "model": p.model,
            "messages": msgs,
            "stream": false,
        })
    };

    if let Value::Object(ref mut map) = body {
        if p.protocol == "anthropic" && !system.trim().is_empty() {
            map.insert("system".into(), json!(system));
        }
        if let Some(t) = p.temperature {
            map.insert("temperature".into(), json!(t));
        }
        if let Some(tp) = p.top_p {
            map.insert("top_p".into(), json!(tp));
        }
        if p.protocol != "anthropic" {
            if let Some(m) = p.max_tokens {
                map.insert("max_tokens".into(), json!(m));
            }
        }
        if let Some(extra) = parse_json_object(&p.extra_body) {
            for (k, v) in extra {
                map.insert(k, v);
            }
        }
    }
    body
}

fn extract_text(v: &Value) -> String {
    if let Some(choices) = v.get("choices") {
        if let Some(first) = choices.get(0) {
            if let Some(msg) = first.get("message") {
                if let Some(c) = msg.get("content") {
                    if let Some(s) = c.as_str() {
                        return s.to_string();
                    }
                    if let Some(arr) = c.as_array() {
                        let parts: Vec<String> = arr
                            .iter()
                            .filter_map(|p| {
                                p.get("text")
                                    .and_then(|t| t.as_str())
                                    .map(|s| s.to_string())
                            })
                            .collect();
                        if !parts.is_empty() {
                            return parts.join("");
                        }
                    }
                }
            }
            if let Some(t) = first.get("text").and_then(|t| t.as_str()) {
                return t.to_string();
            }
        }
    }
    if let Some(arr) = v.get("content").and_then(|c| c.as_array()) {
        let parts: Vec<String> = arr
            .iter()
            .filter_map(|p| {
                if p.get("type").and_then(|t| t.as_str()) == Some("text") {
                    p.get("text").and_then(|t| t.as_str()).map(|s| s.to_string())
                } else {
                    p.get("text").and_then(|t| t.as_str()).map(|s| s.to_string())
                }
            })
            .collect();
        if !parts.is_empty() {
            return parts.join("");
        }
    }
    String::new()
}

fn num(v: &Value, path: &[&str]) -> Option<u64> {
    let mut cur = v;
    for k in path {
        cur = cur.get(k)?;
    }
    cur.as_u64()
}

pub async fn chat(
    client: &reqwest::Client,
    p: &ApiProfile,
    system: &str,
    user: &str,
) -> Result<ChatResult, String> {
    let started = Instant::now();
    let (sys_fit, user_fit, truncated) = fit_context(p, system, user);
    let body = build_body(p, &sys_fit, &user_fit);
    let url = endpoint(&p.base_url, &p.protocol);
    let mut req = client.post(&url).json(&body);

    if p.protocol == "anthropic" {
        req = req
            .header("x-api-key", p.api_key.trim())
            .header("anthropic-version", "2023-06-01");
    } else if !p.api_key.trim().is_empty() {
        req = req.header("authorization", format!("Bearer {}", p.api_key.trim()));
    }
    if let Some(h) = parse_json_object(&p.extra_headers) {
        for (k, v) in h {
            if let Some(s) = v.as_str() {
                req = req.header(k, s);
            }
        }
    }

    let resp = req
        .send()
        .await
        .map_err(|e| classify_transport(&e))?;
    let status = resp.status();
    // 429/503 时上游常给 Retry-After，用它决定退避时间比固定指数更稳
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok());
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(http_error(status.as_u16(), &text, retry_after));
    }
    let v: Value = serde_json::from_str(&text).map_err(|e| {
        format!(
            "响应不是合法 JSON（{e}）：{}",
            text.chars().take(300).collect::<String>()
        )
    })?;

    if v.get("error").is_some() {
        let reason = provider_reason(&text).unwrap_or_else(|| {
            v.get("error")
                .map(|e| e.to_string())
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect()
        });
        return Err(format!("接口返回错误：{reason}"));
    }

    let out = extract_text(&v);
    if out.trim().is_empty() {
        return Err(format!(
            "模型返回空内容：{}",
            text.chars().take(300).collect::<String>()
        ));
    }

    let prompt_tokens = num(&v, &["usage", "prompt_tokens"])
        .or_else(|| num(&v, &["usage", "input_tokens"]));
    let completion_tokens = num(&v, &["usage", "completion_tokens"])
        .or_else(|| num(&v, &["usage", "output_tokens"]));
    let finish_reason = v
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("finish_reason"))
        .and_then(|s| s.as_str())
        .or_else(|| v.get("stop_reason").and_then(|s| s.as_str()))
        .map(|s| s.to_string());

    Ok(ChatResult {
        text: out,
        prompt_tokens,
        completion_tokens,
        latency_ms: started.elapsed().as_millis() as u64,
        truncated,
        finish_reason,
    })
}

/// 从错误响应体里提取服务商给出的原因，兼容 OpenAI 兼容与 Anthropic 两种结构。
fn provider_reason(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body.trim()).ok()?;
    let err = v.get("error");
    let message = err
        .and_then(|e| {
            e.get("message")
                .and_then(|m| m.as_str())
                .or_else(|| e.as_str())
        })
        .or_else(|| v.get("message").and_then(|m| m.as_str()))
        .or_else(|| v.get("msg").and_then(|m| m.as_str()))
        .or_else(|| v.get("detail").and_then(|m| m.as_str()))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let kind = err
        .and_then(|e| e.get("type").or_else(|| e.get("code")))
        .and_then(|c| {
            c.as_str()
                .map(|s| s.trim().to_string())
                .or_else(|| c.as_u64().map(|n| n.to_string()))
        })
        .filter(|s| !s.is_empty());
    match (message, kind) {
        (Some(m), Some(k)) if !m.contains(&k) => Some(format!("{m}（{k}）")),
        (Some(m), _) => Some(m),
        (None, Some(k)) => Some(format!("类型：{k}")),
        (None, None) => None,
    }
}

/// HTTP 状态码 → 人话解释 + 下一步建议。
fn explain_status(code: u16) -> (&'static str, &'static str) {
    match code {
        400 => (
            "请求被拒绝（参数或模型名不被服务端接受）",
            "检查模型名拼写、temperature / max_tokens 是否超出范围，以及 Base URL 是否指向了正确的接口",
        ),
        401 => (
            "身份验证失败（API Key 无效、已过期或没有填写）",
            "重新复制 API Key；用 Anthropic 协议时要确认 Key 与服务商匹配，不要和 OpenAI 的混用",
        ),
        402 => (
            "账户额度不足或未开通付费",
            "到服务商后台查看余额与套餐",
        ),
        403 => (
            "没有访问权限",
            "确认该 Key 已开通这个模型；部分服务商有地区或实名限制",
        ),
        404 => (
            "接口地址或模型不存在",
            "检查 Base URL 是否需要带 /v1，以及模型名是否与服务商文档一致",
        ),
        405 => (
            "请求方法不被允许",
            "Base URL 很可能填成了网页地址，应填 API 入口（例如 /v1 或 /v1/chat/completions）",
        ),
        408 => ("服务端等待超时", "稍后重试，或在高级参数里调大超时时间"),
        413 => (
            "请求内容太大，超出服务端限制",
            "调小上下文长度或减少单条输入的长度",
        ),
        422 => ("参数校验失败", "检查高级参数里的自定义请求体、请求头是否符合接口要求"),
        429 => (
            "请求太频繁或额度已用尽（限流）",
            "降低并发数、稍后重试；也可能是账户需要充值",
        ),
        500 => (
            "服务端内部错误（服务商那边的问题）",
            "稍后重试；持续出现就换模型或联系服务商",
        ),
        502 => (
            "网关错误：上游服务没有正常响应",
            "稍后重试；持续出现说明服务商故障，不是本地配置问题",
        ),
        503 => (
            "服务暂时不可用（上游过载或维护中）",
            "这不是你的配置问题，等一会儿重试即可；持续出现可换模型或联系服务商",
        ),
        504 => (
            "网关超时：上游响应太慢",
            "稍后重试，或缩短单次请求的输入长度",
        ),
        c if (500..600).contains(&c) => (
            "服务端错误",
            "稍后重试；持续出现请联系服务商",
        ),
        c if (300..400).contains(&c) => (
            "接口返回了重定向",
            "Base URL 可能填错，换成服务商文档里的完整 API 地址",
        ),
        _ => (
            "请求未被接受",
            "检查 Base URL、API Key、模型名是否与服务商文档一致",
        ),
    }
}

/// 把非 2xx 响应整理成可读的中文说明：状态码含义 + 建议 + 服务商原文。
fn http_error(code: u16, body: &str, retry_after: Option<u64>) -> String {
    let (reason, next) = explain_status(code);
    let body = body.trim();
    let lower = body.to_ascii_lowercase();
    let is_html = body.starts_with('<') || lower.contains("<html") || lower.contains("<!doctype");

    let mut msg = format!("HTTP {code}：{reason}\n建议：{next}");
    if let Some(secs) = retry_after {
        msg.push_str(&format!("\nRetry-After：{secs}s"));
    }
    if let Some(r) = provider_reason(body) {
        msg.push_str(&format!("\n服务商返回：{r}"));
    } else if is_html {
        msg.push_str("\n服务商返回的是一个网页而不是接口数据，多半是网关或防护页拦下了请求");
    } else if !body.is_empty() {
        let snippet: String = body.chars().take(200).collect();
        msg.push_str(&format!("\n原始响应：{snippet}"));
    }
    msg
}

fn classify_transport(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "请求超时".to_string()
    } else if e.is_connect() {
        format!("连接失败（检查 Base URL / 网络 / 代理）：{e}")
    } else {
        format!("请求失败：{e}")
    }
}

/// 是否值得重试：网络错误、超时、429、5xx。
pub fn is_retryable(msg: &str) -> bool {
    if msg.starts_with("HTTP ") {
        let code: u32 = msg
            .trim_start_matches("HTTP ")
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .unwrap_or(0);
        return code == 429 || code == 408 || (500..600).contains(&code);
    }
    msg.starts_with("请求超时") || msg.starts_with("连接失败")
}

/// 从错误文案里取出服务商的 Retry-After 秒数。
pub fn retry_after_secs(msg: &str) -> Option<u64> {
    msg.lines().find_map(|line| {
        line.strip_prefix("Retry-After：")
            .and_then(|s| s.trim().trim_end_matches('s').trim().parse::<u64>().ok())
    })
}

#[cfg(test)]
mod error_text_tests {
    use super::*;

    #[test]
    fn explains_503_with_provider_message() {
        let body = r#"{"error":{"message":"Overloaded","type":"overloaded_error","code":503}}"#;
        let msg = http_error(503, body, None);
        assert!(msg.starts_with("HTTP 503："), "实际：{msg}");
        assert!(msg.contains("过载"));
        assert!(msg.contains("Overloaded"));
        assert!(msg.contains("overloaded_error"));
        assert!(is_retryable(&msg));
    }

    #[test]
    fn hides_html_gateway_page() {
        let msg = http_error(503, "<!DOCTYPE html><html><body>Cloudflare</body></html>", None);
        assert!(msg.contains("网页而不是接口数据"), "实际：{msg}");
        assert!(!msg.contains("<!DOCTYPE"));
    }

    #[test]
    fn extracts_anthropic_error() {
        let body =
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"max_tokens too large"}}"#;
        assert_eq!(
            provider_reason(body).as_deref(),
            Some("max_tokens too large（invalid_request_error）")
        );
    }

    #[test]
    fn handles_non_json_body() {
        let msg = http_error(401, "unauthorized", None);
        assert!(msg.contains("身份验证失败"), "实际：{msg}");
        assert!(msg.contains("原始响应：unauthorized"), "实际：{msg}");
    }

    #[test]
    fn parses_retry_after_from_error_text() {
        let msg = http_error(429, r#"{"error":{"message":"slow down"}}"#, Some(7));
        assert!(msg.contains("Retry-After：7s"), "实际：{msg}");
        assert_eq!(retry_after_secs(&msg), Some(7));
        assert_eq!(retry_after_secs("HTTP 429：限流"), None);
    }

    #[test]
    fn truncates_overlong_system_prompt_too() {
        let p = ApiProfile {
            protocol: "openai".into(),
            base_url: String::new(),
            api_key: String::new(),
            model: String::new(),
            temperature: None,
            top_p: None,
            max_tokens: Some(1000),
            context_window: Some(2000),
            timeout_secs: None,
            extra_body: None,
            extra_headers: None,
        };
        let system = "系".repeat(5000);
        let user = "问".repeat(5000);
        let (sys, usr, cut) = fit_context(&p, &system, &user);
        assert!(cut);
        assert!(sys.chars().count() < 2000, "系统提示词未截断：{}", sys.chars().count());
        assert!(usr.chars().count() < 3000, "用户输入未截断：{}", usr.chars().count());
        assert!(sys.contains("中间部分已省略"));
    }
}
