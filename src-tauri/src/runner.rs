use crate::api::{self, ApiProfile};
use crate::judge;
use crate::run_store::{self, RunMeta};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

/// 取消后不再重试，run_one 看到这个错误直接丢弃该行（不算调用失败）
const CANCELLED: &str = "__cancelled__";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunConfig {
    /// 主体提示词（被测模型的 system prompt）
    pub system_prompt: String,
    /// 用户消息模板，可含 {{列名}} 变量
    pub user_template: String,
    /// 评测提示词，可含 {{input}} {{output}} 与列名变量
    pub eval_prompt: String,
    pub target: ApiProfile,
    pub judge: ApiProfile,
    pub concurrency: usize,
    pub max_retries: u32,
    /// newRun = 全部重跑；retryFailed = 只重跑上一次失败的行；resume = 从断点续跑
    pub mode: String,
    /// 需要重跑的行下标（retryFailed / resume 时由前端传入）
    pub retry_indices: Option<Vec<usize>>,
    pub run_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowResult {
    pub index: usize,
    pub input: String,
    pub output: String,
    pub target_error: Option<String>,
    pub pass: bool,
    pub error_types: Vec<String>,
    pub reasons: Vec<String>,
    pub details: String,
    pub judge_raw: String,
    pub parse_ok: bool,
    pub target_latency_ms: u64,
    pub judge_latency_ms: u64,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub truncated: bool,
    pub status_label: String,
}

/// start_run 的返回值：结果本身走进度事件，这里只回执本次会话的进度与状态。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunOutcome {
    pub run_id: String,
    pub done: usize,
    pub total: usize,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct RunRegistry {
    /// run_id -> 取消标记
    flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

impl RunRegistry {
    fn register(&self, id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut map) = self.flags.lock() {
            map.insert(id.to_string(), flag.clone());
        }
        flag
    }

    fn remove(&self, id: &str) {
        if let Ok(mut map) = self.flags.lock() {
            map.remove(id);
        }
    }

    fn cancel(&self, id: &str) {
        if let Ok(map) = self.flags.lock() {
            if let Some(flag) = map.get(id) {
                flag.store(true, Ordering::SeqCst);
            }
        }
    }
}

fn render(template: &str, row: &Map<String, Value>, input: &str, output: &str) -> String {
    let mut out = template.to_string();
    for (k, v) in row {
        let val = match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        out = out.replace(&format!("{{{{{}}}}}", k), &val);
    }
    out = out.replace("{{input}}", input);
    out = out.replace("{{output}}", output);
    // 兼容 {{input}} 的空格写法
    out = out.replace("{{ input }}", input);
    out = out.replace("{{ output }}", output);
    out
}

/// 裁判看到的 `{{input}}`：被评估模型的提示词 + 本条已渲染的用户消息。
///
/// 只给用户消息不够——很多缺陷是「提示词要求了某件事，但输出没做到」，
/// 裁判必须同时看到提示词才能判。
fn judge_input_text(system_prompt: &str, user_msg: &str) -> String {
    let sys = system_prompt.trim();
    if sys.is_empty() {
        user_msg.to_string()
    } else {
        format!("【被评估模型的提示词】\n{sys}\n\n【本条输入】\n{user_msg}")
    }
}

fn jitter_ms(span: u64) -> u64 {
    if span == 0 {
        return 0;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    nanos % span
}

/// 可被取消打断的等待：取消后返回 false。
async fn sleep_cancellable(ms: u64, flag: &AtomicBool) -> bool {
    let deadline = Instant::now() + Duration::from_millis(ms);
    loop {
        if flag.load(Ordering::SeqCst) {
            return false;
        }
        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        let step = (deadline - now).min(Duration::from_millis(100));
        tokio::time::sleep(step).await;
    }
}

/// 全局限速闸门：任一请求收到 429/503 的 Retry-After 后，所有任务一起等到该时间点。
async fn wait_for_gate(gate: &AtomicU64, flag: &AtomicBool) -> bool {
    loop {
        if flag.load(Ordering::SeqCst) {
            return false;
        }
        let until = gate.load(Ordering::SeqCst);
        let now = run_store::now_ms();
        if now >= until {
            return true;
        }
        let step = (until - now).min(200);
        tokio::time::sleep(Duration::from_millis(step)).await;
    }
}

async fn call_with_retry(
    client: &reqwest::Client,
    profile: &ApiProfile,
    system: &str,
    user: &str,
    max_retries: u32,
    flag: &AtomicBool,
    gate: &AtomicU64,
) -> Result<api::ChatResult, String> {
    let max_retries = max_retries.min(10);
    let mut attempt = 0u32;
    loop {
        if !wait_for_gate(gate, flag).await {
            return Err(CANCELLED.into());
        }
        match api::chat(client, profile, system, user).await {
            Ok(r) => return Ok(r),
            Err(e) => {
                if flag.load(Ordering::SeqCst) {
                    return Err(CANCELLED.into());
                }
                if attempt >= max_retries || !api::is_retryable(&e) {
                    return Err(e);
                }

                let wait = if let Some(secs) = api::retry_after_secs(&e) {
                    let secs = secs.clamp(1, 300);
                    // 所有并发任务共享这个暂停点，避免继续打满配额
                    let until = run_store::now_ms() + secs * 1000;
                    gate.fetch_max(until, Ordering::SeqCst);
                    secs * 1000
                } else {
                    if e.starts_with("HTTP 429") {
                        let until = run_store::now_ms() + 2000;
                        gate.fetch_max(until, Ordering::SeqCst);
                    }
                    let base = 800u64 * 2u64.pow(attempt.min(4));
                    base + jitter_ms(base / 3 + 1)
                };

                if !sleep_cancellable(wait, flag).await {
                    return Err(CANCELLED.into());
                }
                attempt += 1;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_one(
    cfg: &RunConfig,
    idx: usize,
    row: &Map<String, Value>,
    target_client: &reqwest::Client,
    judge_client: &reqwest::Client,
    status_label: &str,
    flag: &AtomicBool,
    gate: &AtomicU64,
) -> Option<RowResult> {
    if flag.load(Ordering::SeqCst) {
        return None;
    }
    let user_msg = render(&cfg.user_template, row, "", "");
    // 系统提示词与用户消息同样按行渲染：两者都可能引用数据集列，
    // 不渲染会把 {{列名}} 字面量发给模型。
    let system_prompt = render(&cfg.system_prompt, row, "", "");
    let mut res = RowResult {
        index: idx,
        input: user_msg.clone(),
        output: String::new(),
        target_error: None,
        pass: false,
        error_types: Vec::new(),
        reasons: Vec::new(),
        details: String::new(),
        judge_raw: String::new(),
        parse_ok: true,
        target_latency_ms: 0,
        judge_latency_ms: 0,
        prompt_tokens: None,
        completion_tokens: None,
        truncated: false,
        status_label: status_label.to_string(),
    };

    let target = match call_with_retry(
        target_client,
        &cfg.target,
        &system_prompt,
        &user_msg,
        cfg.max_retries,
        flag,
        gate,
    )
    .await
    {
        Ok(r) => r,
        Err(e) if e == CANCELLED => return None,
        Err(e) => {
            res.target_error = Some(e);
            res.error_types = vec!["被测模型调用失败".to_string()];
            res.reasons = vec![res.target_error.clone().unwrap_or_default()];
            res.parse_ok = false;
            return Some(res);
        }
    };

    res.output = target.text;
    res.target_latency_ms = target.latency_ms;
    res.prompt_tokens = target.prompt_tokens;
    res.completion_tokens = target.completion_tokens;
    res.truncated = target.truncated;

    // 被测调用完成后再检查一次：已取消就不必再花评测模型的调用
    if flag.load(Ordering::SeqCst) {
        return None;
    }

    let input_for_judge = judge_input_text(&system_prompt, &user_msg);
    let judge_input = render(&cfg.eval_prompt, row, &input_for_judge, &res.output);
    match call_with_retry(
        judge_client,
        &cfg.judge,
        "",
        &judge_input,
        cfg.max_retries,
        flag,
        gate,
    )
    .await
    {
        Ok(r) => {
            res.judge_latency_ms = r.latency_ms;
            let v = judge::parse_verdict(&r.text);
            res.pass = v.pass;
            res.error_types = v.error_types;
            res.reasons = v.reasons;
            res.details = v.details;
            res.parse_ok = v.parse_ok;
            res.judge_raw = r.text;
        }
        Err(e) if e == CANCELLED => return None,
        Err(e) => {
            res.pass = false;
            res.parse_ok = false;
            res.error_types = vec!["评测模型调用失败".to_string()];
            res.reasons = vec![e];
            res.judge_raw = String::new();
        }
    }
    Some(res)
}

/// 一条结果是不是「硬错误」：被测模型或评测模型根本没被调通。
/// 返回具体原因，用于运行级报错文案。
fn hard_error_of(r: &RowResult) -> Option<String> {
    if let Some(e) = &r.target_error {
        return Some(e.clone());
    }
    if r.error_types.iter().any(|t| t == "评测模型调用失败") {
        return r.reasons.first().cloned();
    }
    None
}

/// 进度缓冲：按 10 条或 250ms 合并发事件，同时把每条结果追加进 results.jsonl。
struct Sink {
    app: AppHandle,
    writer: std::io::BufWriter<std::fs::File>,
    buf: Vec<RowResult>,
    last_emit: Instant,
    done: usize,
    total: usize,
    write_err: Option<String>,
    /// 硬错误连续出现的次数：配置错（Key / 地址 / 模型名）时不该把几千条数据全打一遍
    hard_streak: usize,
    /// 至少有一条真正调通了
    ok_rows: usize,
    /// 第一条硬错误的原因，用于「全部失败」时的运行级提示
    first_error: Option<String>,
    /// 因连续硬错误主动叫停的原因
    run_error: Option<String>,
    /// 落盘失败时置位，停止再发起新请求：结果存不下来，继续跑只是白花额度
    flag: Arc<AtomicBool>,
}

/// 连续多少条硬错误就主动叫停
const ABORT_AFTER_HARD_ERRORS: usize = 5;

impl Sink {
    fn fail_write(&mut self, e: std::io::Error) {
        if self.write_err.is_none() {
            self.write_err = Some(format!("写入结果失败：{e}，已停止发起新请求"));
        }
        self.flag.store(true, Ordering::SeqCst);
    }

    fn take_batch(&mut self) -> Option<Vec<RowResult>> {
        if self.buf.is_empty() {
            return None;
        }
        let results = std::mem::take(&mut self.buf);
        if let Err(e) = self.writer.flush() {
            self.fail_write(e);
        }
        self.last_emit = Instant::now();
        Some(results)
    }

    fn emit_batch(&mut self) {
        if let Some(results) = self.take_batch() {
            let _ = self.app.emit(
                "run://progress",
                json!({ "done": self.done, "total": self.total, "results": results }),
            );
        }
    }

    fn push(&mut self, r: RowResult) {
        self.done += 1;
        if self.write_err.is_none() {
            if let Err(e) = run_store::write_row(&mut self.writer, &r) {
                self.fail_write(e);
            }
        }
        // 连不上模型 / Key 无效 / 地址写错这类问题，重试也救不回来：
        // 连续出现就说明配置有误，先停下来告诉用户具体原因，不要白跑剩下的数据
        match hard_error_of(&r) {
            Some(e) => {
                self.hard_streak += 1;
                if self.first_error.is_none() {
                    self.first_error = Some(e.clone());
                }
                if self.run_error.is_none() && self.hard_streak >= ABORT_AFTER_HARD_ERRORS {
                    self.run_error = Some(format!(
                        "连续 {} 条都没调通，已自动停止本次运行。最后一处错误：{e}",
                        self.hard_streak
                    ));
                    self.flag.store(true, Ordering::SeqCst);
                }
            }
            None => {
                self.hard_streak = 0;
                self.ok_rows += 1;
            }
        }
        self.buf.push(r);
        let due = self.buf.len() >= 10 || self.last_emit.elapsed() >= Duration::from_millis(250);
        if due {
            self.emit_batch();
        }
    }
}

/// 测试入口：跑单行，不经过 Tauri 事件与并发调度。
/// 让集成测试能验证「数据列 → 渲染 → 被测模型 / 裁判」这条链路。
#[cfg(test)]
pub(crate) async fn run_one_for_test(
    cfg: &RunConfig,
    row: &Map<String, Value>,
) -> Option<RowResult> {
    let target_client = crate::api::build_client(&cfg.target).ok()?;
    let judge_client = crate::api::build_client(&cfg.judge).ok()?;
    let flag = AtomicBool::new(false);
    let gate = AtomicU64::new(0);
    run_one(
        cfg,
        0,
        row,
        &target_client,
        &judge_client,
        "测试",
        &flag,
        &gate,
    )
    .await
}

#[tauri::command]
pub async fn start_run(
    app: AppHandle,
    cfg: RunConfig,
    rows: Vec<Map<String, Value>>,
    labels: Vec<String>,
) -> Result<RunOutcome, String> {
    if cfg.target.base_url.trim().is_empty() || cfg.target.model.trim().is_empty() {
        return Err("被测模型的 Base URL 与模型名不能为空".into());
    }
    if cfg.judge.base_url.trim().is_empty() || cfg.judge.model.trim().is_empty() {
        return Err("评测模型的 Base URL 与模型名不能为空".into());
    }
    if cfg.eval_prompt.trim().is_empty() {
        return Err("评测提示词不能为空".into());
    }
    if rows.is_empty() {
        return Err("数据集为空".into());
    }

    let flag = app.state::<RunRegistry>().register(&cfg.run_id);
    let target_client = api::build_client(&cfg.target)?;
    let judge_client = api::build_client(&cfg.judge)?;
    let concurrency = cfg.concurrency.clamp(1, 32);

    let indices: Vec<usize> = match &cfg.retry_indices {
        Some(list) if !list.is_empty() => list.iter().copied().filter(|i| *i < rows.len()).collect(),
        _ => (0..rows.len()).collect(),
    };
    let total = indices.len();

    // 落盘：new 截断重来；重跑 / 续跑在同一个文件上追加
    let dir = run_store::current_dir(&app);
    let mut meta = run_store::read_meta(&dir)?.unwrap_or(RunMeta {
        run_id: cfg.run_id.clone(),
        total: rows.len(),
        started_at: run_store::now_ms(),
        finished_at: None,
        status: run_store::STATUS_RUNNING.into(),
        error: None,
    });
    meta.run_id = cfg.run_id.clone();
    meta.total = meta.total.max(rows.len());
    meta.status = run_store::STATUS_RUNNING.into();
    meta.finished_at = None;
    meta.error = None;
    if cfg.mode == "new" {
        run_store::reset(&dir, &meta)?;
    } else {
        run_store::write_meta(&dir, &meta)?;
    }
    let writer = run_store::open_append(&dir)?;

    let flag_for_sink = flag.clone();
    let sink = Arc::new(Mutex::new(Sink {
        app: app.clone(),
        writer,
        buf: Vec::new(),
        last_emit: Instant::now(),
        done: 0,
        total,
        write_err: None,
        hard_streak: 0,
        ok_rows: 0,
        first_error: None,
        run_error: None,
        flag: flag_for_sink,
    }));
    let gate = Arc::new(AtomicU64::new(0));
    let finished = Arc::new(AtomicBool::new(false));

    let ticker = {
        let sink = sink.clone();
        let finished = finished.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(250)).await;
                if finished.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(mut s) = sink.lock() {
                    s.emit_batch();
                }
            }
        })
    };

    let cfg = Arc::new(cfg);
    let rows = Arc::new(rows);
    let labels = Arc::new(labels);

    stream::iter(indices.into_iter().map(|idx| {
        let cfg = cfg.clone();
        let rows = rows.clone();
        let labels = labels.clone();
        let flag = flag.clone();
        let gate = gate.clone();
        let sink = sink.clone();
        let tc = target_client.clone();
        let jc = judge_client.clone();
        async move {
            if flag.load(Ordering::SeqCst) {
                return;
            }
            let row = rows[idx].clone();
            let label = labels.get(idx).cloned().unwrap_or_default();
            if let Some(res) = run_one(&cfg, idx, &row, &tc, &jc, &label, &flag, &gate).await {
                if let Ok(mut s) = sink.lock() {
                    s.push(res);
                }
            }
        }
    }))
    .buffer_unordered(concurrency)
    .for_each(|_| async {})
    .await;

    finished.store(true, Ordering::SeqCst);
    let _ = ticker.await;

    let (done, ok_rows, write_err, run_error, first_error) = {
        let mut s = match sink.lock() {
            Ok(s) => s,
            Err(_) => return Err("结果写入锁已损坏".into()),
        };
        s.emit_batch();
        let _ = s.writer.flush();
        (
            s.done,
            s.ok_rows,
            s.write_err.clone(),
            s.run_error.clone(),
            s.first_error.clone(),
        )
    };

    app.state::<RunRegistry>().remove(&cfg.run_id);

    let cancelled = flag.load(Ordering::SeqCst);
    // 运行级错误：落盘失败、连续调不通、或者跑完了但没有一条真正成功
    let mut error = write_err.or(run_error);
    if error.is_none() && done > 0 && ok_rows == 0 {
        error = Some(match &first_error {
            Some(e) => format!("本次 {done} 条全部调用失败，第一条原因：{e}"),
            None => format!("本次 {done} 条全部没跑通"),
        });
    }
    let status = if error.is_some() {
        run_store::STATUS_FAILED
    } else if cancelled {
        run_store::STATUS_CANCELLED
    } else {
        run_store::STATUS_FINISHED
    };
    meta.status = status.into();
    meta.finished_at = Some(run_store::now_ms());
    meta.error = error.clone();
    let _ = run_store::write_meta(&dir, &meta);

    Ok(RunOutcome {
        run_id: cfg.run_id.clone(),
        done,
        total,
        status: status.into(),
        error,
    })
}

#[tauri::command]
pub async fn cancel_run(app: AppHandle, run_id: String) -> Result<(), String> {
    app.state::<RunRegistry>().cancel(&run_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pairs: &[(&str, &str)]) -> Map<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Value::String(v.to_string())))
            .collect()
    }

    #[test]
    fn renders_columns_and_io_placeholders() {
        let r = row(&[("question", "Q1"), ("note", "N1")]);
        let out = render("{{question}} / {{note}}", &r, "", "");
        assert_eq!(out, "Q1 / N1");
        let out = render("输入：{{input}}；输出：{{output}}", &r, "IN", "OUT");
        assert_eq!(out, "输入：IN；输出：OUT");
        // 带空格的写法也要认
        assert_eq!(render("{{ input }}", &r, "IN", ""), "IN");
    }

    #[test]
    fn unknown_placeholder_stays_untouched() {
        let r = row(&[("a", "1")]);
        assert_eq!(render("{{missing}}", &r, "", ""), "{{missing}}");
    }

    /// 系统提示词里的 {{列名}} 必须按行渲染。
    /// 回归用：曾经只渲染 user_template，系统提示词被原样下发，
    /// 导致提示词里引用数据集列的写法全部失效。
    #[test]
    fn system_prompt_renders_row_columns() {
        let r = row(&[("mode", "strict"), ("rules", "- 规则甲\n- 规则乙")]);
        let out = render("模式：{{mode}}\n<<<RULES>>>\n{{rules}}", &r, "", "");
        assert!(out.contains("strict"));
        assert!(out.contains("- 规则甲"));
        assert!(!out.contains("{{mode}}"));
        assert!(!out.contains("{{rules}}"));
    }

    #[test]
    fn non_string_cells_are_stringified() {
        let mut r = Map::new();
        r.insert("n".into(), json!(3));
        r.insert("b".into(), json!(true));
        assert_eq!(render("{{n}}/{{b}}", &r, "", ""), "3/true");
    }

    #[test]
    fn judge_input_includes_system_prompt() {
        let text = judge_input_text("你是客服", "用户问：怎么退货");
        assert!(text.contains("你是客服"));
        assert!(text.contains("用户问：怎么退货"));
        assert!(text.starts_with("【被评估模型的提示词】"));
        // 没有主体提示词时不要留空标题
        assert_eq!(judge_input_text("   ", "只有输入"), "只有输入");
    }

    #[test]
    fn eval_prompt_gets_judge_input_and_output() {
        let r = row(&[("question", "Q1")]);
        let input = judge_input_text("系统提示", "用户消息");
        let out = render("<<<{{input}}>>>|<<<{{output}}>>>", &r, &input, "模型输出");
        assert!(out.contains("系统提示") && out.contains("用户消息"));
        assert!(out.ends_with(">>>|<<<模型输出>>>"));
    }

    #[test]
    fn hard_error_detects_target_and_judge_failures() {
        let mut r = RowResult {
            index: 0,
            input: "in".into(),
            output: String::new(),
            target_error: None,
            pass: true,
            error_types: vec![],
            reasons: vec![],
            details: String::new(),
            judge_raw: String::new(),
            parse_ok: true,
            target_latency_ms: 0,
            judge_latency_ms: 0,
            prompt_tokens: None,
            completion_tokens: None,
            truncated: false,
            status_label: "g".into(),
        };
        assert_eq!(hard_error_of(&r), None);

        r.target_error = Some("HTTP 404：接口地址或模型不存在".into());
        assert!(hard_error_of(&r).unwrap().contains("404"));

        // 被测模型没问题，评测模型挂了也算硬错误
        r.target_error = None;
        r.pass = false;
        r.error_types = vec!["评测模型调用失败".into()];
        r.reasons = vec!["HTTP 401：API Key 无效".into()];
        assert!(hard_error_of(&r).unwrap().contains("401"));

        // 普通的判定不通过，不是调用失败
        r.error_types = vec!["事实编造".into()];
        assert_eq!(hard_error_of(&r), None);
    }

    #[test]
    fn jitter_stays_in_range() {
        for _ in 0..50 {
            assert!(jitter_ms(30) < 30);
        }
        assert_eq!(jitter_ms(0), 0);
    }
}

#[tauri::command]
pub async fn test_connection(profile: ApiProfile) -> Result<String, String> {
    let client = api::build_client(&profile)?;
    let r = api::chat(&client, &profile, "你是一个连通性测试，只回复 OK 两个字母。", "ping")
        .await?;
    Ok(format!(
        "连接成功，{}ms，返回：{}",
        r.latency_ms,
        r.text.chars().take(60).collect::<String>()
    ))
}
