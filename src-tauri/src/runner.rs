use crate::api::{self, ApiProfile};
use crate::judge;
use crate::run_store::{self, PromptSnapshot, RunMeta, RunSummary};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

/// 取消后不再重试，run_one 看到这个错误直接丢弃该行（不算调用失败）
const CANCELLED: &str = "__cancelled__";

/// 同时最多运行几个任务，超出直接拒绝启动
pub const MAX_ACTIVE_RUNS: usize = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunConfig {
    pub run_id: String,
    /// 任务名称，由前端拼好传入（方案名 · 后缀 · 第N版）；可以为空
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub plan_id: String,
    #[serde(default)]
    pub plan_name: String,
    /// 方案三份提示词的快照
    pub prompts: PromptSnapshot,
    /// 从数据集库按 id 读取，IPC 不再传整份数据
    pub dataset_id: String,
    #[serde(default)]
    pub dataset_name: String,
    pub target: ApiProfile,
    pub judge: ApiProfile,
    pub concurrency: usize,
    pub max_retries: u32,
    /// newRun = 新任务；retryFailed = 只重跑失败行；resume = 从断点续跑
    pub mode: String,
    /// 需要重跑的行下标（retryFailed / resume 时由前端传入）
    pub retry_indices: Option<Vec<usize>>,
}

/// 分组标签的取值优先级，必须与前端 statusLabelOf 一致。
pub fn status_label_of(row: &Map<String, Value>) -> String {
    fn text(v: &Value) -> String {
        match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        }
    }
    if let Some(v) = row.get("数据组状态") {
        let s = text(v);
        return if s.is_empty() { "未分组".into() } else { s };
    }
    for k in ["状态", "分组", "类别", "status", "group", "case"] {
        if let Some(v) = row.get(k) {
            let s = text(v);
            if !s.trim().is_empty() {
                return s;
            }
        }
    }
    "未分组".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowResult {
    pub index: usize,
    /// 被测模型实际收到的 system 消息（渲染后）。旧结果文件没有这个字段
    #[serde(default)]
    pub system: String,
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
    /// 裁判这一侧的 tokens。旧结果文件没有这两个字段，必须给默认值，否则整行会被当成坏行
    #[serde(default)]
    pub judge_prompt_tokens: Option<u64>,
    #[serde(default)]
    pub judge_completion_tokens: Option<u64>,
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

/// 连接身份：同一条连接上的多个任务共享 429 退避，不互相打爆配额。
/// 用协议 + 地址 + Key 的哈希做 key，Key 不落到日志。
fn connection_key(p: &ApiProfile) -> String {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut h = OFFSET;
    for b in p
        .protocol
        .bytes()
        .chain(b"|".iter().copied())
        .chain(p.base_url.bytes())
        .chain(b"|".iter().copied())
        .chain(p.api_key.bytes())
    {
        h ^= b as u64;
        h = h.wrapping_mul(PRIME);
    }
    format!("conn-{h:016x}")
}

/// 应用级闸门表：按连接身份共享「等到某时刻再发请求」。
#[derive(Default)]
pub struct GateRegistry {
    gates: Mutex<HashMap<String, Arc<AtomicU64>>>,
}

impl GateRegistry {
    pub fn gate_for(&self, p: &ApiProfile) -> Arc<AtomicU64> {
        let key = connection_key(p);
        if let Ok(mut map) = self.gates.lock() {
            return map
                .entry(key)
                .or_insert_with(|| Arc::new(AtomicU64::new(0)))
                .clone();
        }
        Arc::new(AtomicU64::new(0))
    }
}

/// 运行中任务表：run_id -> 取消标记。
/// try_register 返回 RAII guard，任何提前返回都会释放占位。
#[derive(Default, Clone)]
pub struct RunRegistry {
    flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

pub struct RunLease {
    flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    id: String,
    flag: Arc<AtomicBool>,
}

impl RunLease {
    pub fn flag(&self) -> Arc<AtomicBool> {
        self.flag.clone()
    }
}

impl Drop for RunLease {
    fn drop(&mut self) {
        if let Ok(mut map) = self.flags.lock() {
            // 只移除自己那一条，避免误删同名的新任务
            if let Some(cur) = map.get(&self.id) {
                if Arc::ptr_eq(cur, &self.flag) {
                    map.remove(&self.id);
                }
            }
        }
    }
}

impl RunRegistry {
    pub fn try_register(&self, id: &str) -> Result<RunLease, String> {
        let mut map = self
            .flags
            .lock()
            .map_err(|_| "运行状态锁已损坏".to_string())?;
        if map.contains_key(id) {
            return Err("同名任务已经在运行".into());
        }
        if map.len() >= MAX_ACTIVE_RUNS {
            return Err(format!(
                "同时最多运行 {MAX_ACTIVE_RUNS} 个任务，请先等一个任务结束"
            ));
        }
        let flag = Arc::new(AtomicBool::new(false));
        map.insert(id.to_string(), flag.clone());
        Ok(RunLease {
            flags: self.flags.clone(),
            id: id.to_string(),
            flag,
        })
    }

    /// 返回是否真的找到了这个任务：找不到说明它已经结束了。
    pub fn cancel(&self, id: &str) -> bool {
        if let Ok(map) = self.flags.lock() {
            if let Some(flag) = map.get(id) {
                flag.store(true, Ordering::SeqCst);
                return true;
            }
        }
        false
    }

}

/// 从模板文本提取引用的变量名（{{列名}} / {{input}} / {{output}}），去重、去空白。
fn template_vars(tpl: &str) -> HashSet<String> {
    let bytes = tpl.as_bytes();
    let mut set = HashSet::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let mut j = i + 2;
            while j + 1 < bytes.len() && !(bytes[j] == b'}' && bytes[j + 1] == b'}') {
                j += 1;
            }
            if j + 1 < bytes.len() {
                let inner = tpl[i + 2..j].trim();
                if !inner.is_empty() {
                    set.insert(inner.to_string());
                }
                i = j + 2;
                continue;
            }
            break;
        }
        i += 1;
    }
    set
}

/// 槽位渲染：只替换「该槽位注入的列」{{列名}}；{{input}}/{{output}} 仅在 data_in 时替换。
/// 提示词或模板里引用但没注入本槽位的变量保持字面——数据只渲染在它被填入的槽位。
fn render_slot(
    text: &str,
    row: &Map<String, Value>,
    slot_cols: &HashSet<String>,
    data_in: bool,
    input: &str,
    output: &str,
) -> String {
    let mut out = text.to_string();
    for (k, v) in row {
        if slot_cols.contains(k) {
            let val = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            // 数据注入：每列带「===== 列名 =====" 标题、各自被一对括号包裹。
            // 提示词是提示词、数据是数据，模型才不会把数据里的文字
            // （看似指令、要求）当成提示词的一部分执行；括号让每一列数据
            // 的边界明确，多列数据挤在一起也能分清。
            out = out.replace(
                &format!("{{{{{}}}}}", k),
                &format!("(===== {k} =====\n\n{val})"),
            );
        }
    }
    if data_in {
        out = out.replace("{{input}}", input);
        out = out.replace("{{output}}", output);
        out = out.replace("{{ input }}", input);
        out = out.replace("{{ output }}", output);
    }
    out
}

/// 全列渲染 + input/output 渲染（测试用）
#[cfg(test)]
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
    out = out.replace("{{ input }}", input);
    out = out.replace("{{ output }}", output);
    out
}

/// 裁判看到的 `{{input}}`：被评估模型实际收到的消息。
///
/// 只给用户消息不够——很多缺陷是「提示词要求了某件事，但输出没做到」，
/// 裁判必须同时看到提示词才能判。
/// 空的那条消息不出现，也就不留空标题：数据全在被测 system 里时，
/// 不该凭空多出一段空的「本条输入」。
fn judge_input_text(system_prompt: &str, user_msg: &str) -> String {
    let sys = system_prompt.trim();
    let user = user_msg.trim();
    if sys.is_empty() {
        return user.to_string();
    }
    if user.is_empty() {
        return format!("【被评估模型的提示词】\n{sys}");
    }
    format!("【被评估模型的提示词】\n{sys}\n\n【本条输入】\n{user}")
}

/// 槽位注入的列集合：仅当槽位启用且文本非空时有效。
/// 提示词（主体/评测）里引用列只在此集合内才渲染，其余保持字面——
/// 数据只渲染在它被填入的槽位。
fn slot_cols(slot: &run_store::InjectSlot) -> HashSet<String> {
    if slot.enabled && !slot.text.trim().is_empty() {
        template_vars(&slot.text)
    } else {
        HashSet::new()
    }
}

/// 组装一个槽位的最终文本。
///
/// 导入了提示词本体的槽位：消息 = 提示词本体（按本槽位声明的列渲染）。
/// 数据模板在这里只负责声明「注入哪几列」，本身不作为文本发出去——
/// 否则数据会在提示词里渲染一遍、模板里再贴一遍，同一份数据出现两次。
///
/// 没导入提示词本体的槽位：消息 = 数据模板渲染后的内容（比如被测 user 单独收数据）。
/// 被测侧传空 input/output（那两个变量只对评测侧有意义）。
fn build_slot(
    prompt: &str,
    slot: &run_store::InjectSlot,
    row: &Map<String, Value>,
    input: &str,
    output: &str,
) -> String {
    let cols = slot_cols(slot);
    if slot.prompt_in {
        return render_slot(prompt, row, &cols, slot.data_in, input, output);
    }
    if slot.enabled && !slot.text.trim().is_empty() {
        let rendered = render_slot(&slot.text, row, &cols, slot.data_in, input, output);
        if rendered.trim().is_empty() {
            return String::new();
        }
        // 纯数据槽位：列名标题与括号已由 render_slot 逐列生成，这里不再整体包裹
        return rendered;
    }
    String::new()
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

/// 等待取消信号：每 80ms 检查一次标志，配合 `tokio::select!` 使用。
///
/// 取消时 select 会丢弃另一分支的 future，reqwest 的请求随之被中止，
/// 不必等它自然返回或超时——这才叫「取消」而不是「不再发起新的」。
async fn cancel_signal(flag: &AtomicBool) {
    while !flag.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(80)).await;
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
        // 取消优先：已取消就不再发请求；请求在途时取消会立刻丢弃 future，断开连接
        let sent = tokio::select! {
            biased;
            _ = cancel_signal(flag) => return Err(CANCELLED.into()),
            r = api::chat(client, profile, system, user) => r,
        };
        match sent {
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
    target_gate: &AtomicU64,
    judge_gate: &AtomicU64,
) -> Option<RowResult> {
    if flag.load(Ordering::SeqCst) {
        return None;
    }
    let inj = &cfg.prompts.injections;
    // 被测两侧槽位：主体提示词本体（prompt_in 决定发到哪个槽位）+ 数据模板。
    // input/output 只对评测侧有意义，这里传空。
    let target_system = build_slot(&cfg.prompts.system_prompt, &inj.target_system, row, "", "");
    let target_user = build_slot(&cfg.prompts.system_prompt, &inj.target_user, row, "", "");
    let mut res = RowResult {
        index: idx,
        system: target_system.clone(),
        input: target_user.clone(),
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
        judge_prompt_tokens: None,
        judge_completion_tokens: None,
        truncated: false,
        status_label: status_label.to_string(),
    };

    let target = match call_with_retry(
        target_client,
        &cfg.target,
        &target_system,
        &target_user,
        cfg.max_retries,
        flag,
        target_gate,
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

    let input_for_judge = judge_input_text(&target_system, &target_user);
    // 评测两侧槽位：评测提示词本体（prompt_in）+ 数据模板；
    // 「被测数据导入」（data_in）决定 {{input}}/{{output}} 在该槽位是否渲染成真实数据，
    // 不勾选则保留字面花括号。
    let judge_system = build_slot(
        &cfg.prompts.eval_prompt,
        &inj.judge_system,
        row,
        &input_for_judge,
        &res.output,
    );
    let judge_user = build_slot(
        &cfg.prompts.eval_prompt,
        &inj.judge_user,
        row,
        &input_for_judge,
        &res.output,
    );
    match call_with_retry(
        judge_client,
        &cfg.judge,
        &judge_system,
        &judge_user,
        cfg.max_retries,
        flag,
        judge_gate,
    )
    .await
    {
        Ok(r) => {
            res.judge_latency_ms = r.latency_ms;
            res.judge_prompt_tokens = r.prompt_tokens;
            res.judge_completion_tokens = r.completion_tokens;
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
    run_id: String,
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
                json!({
                    "runId": self.run_id,
                    "done": self.done,
                    "total": self.total,
                    "results": results
                }),
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
    let target_gate = AtomicU64::new(0);
    let judge_gate = AtomicU64::new(0);
    run_one(
        cfg,
        0,
        row,
        &target_client,
        &judge_client,
        "测试",
        &flag,
        &target_gate,
        &judge_gate,
    )
    .await
}

#[tauri::command]
pub async fn start_run(app: AppHandle, cfg: RunConfig) -> Result<RunOutcome, String> {
    if cfg.target.base_url.trim().is_empty() || cfg.target.model.trim().is_empty() {
        return Err("被测连接的 Base URL 与模型名不能为空".into());
    }
    if cfg.judge.base_url.trim().is_empty() || cfg.judge.model.trim().is_empty() {
        return Err("评测连接的 Base URL 与模型名不能为空".into());
    }
    if cfg.prompts.eval_prompt.trim().is_empty() {
        return Err("评测提示词不能为空".into());
    }
    if cfg.dataset_id.trim().is_empty() {
        return Err("没有选择数据集".into());
    }

    // 数据集由后端按 id 读取，IPC 不再传整份数据
    let ds = crate::storage::load_dataset_by_id(&app, &cfg.dataset_id)?;
    if ds.rows.is_empty() {
        return Err("数据集为空".into());
    }
    let signature = crate::storage::dataset_signature(&ds);
    let rows = ds.rows;
    let labels: Vec<String> = rows.iter().map(status_label_of).collect();

    // try_register 的 guard 在任何提前返回时自动释放占位
    let lease = app.state::<RunRegistry>().try_register(&cfg.run_id)?;
    let flag = lease.flag();
    let target_client = api::build_client(&cfg.target)?;
    let judge_client = api::build_client(&cfg.judge)?;
    let concurrency = cfg.concurrency.clamp(1, 32);

    let dir = run_store::run_dir(&app, &cfg.run_id)?;
    let mut meta = if cfg.mode == "new" {
        if dir.exists() {
            return Err("同名任务目录已存在，请重新创建任务".into());
        }
        RunMeta {
            run_id: cfg.run_id.clone(),
            title: cfg.title.clone(),
            plan_id: String::new(),
            plan_name: String::new(),
            dataset_id: String::new(),
            dataset_name: String::new(),
            prompts: PromptSnapshot::default(),
            dataset_signature: String::new(),
            total: rows.len(),
            started_at: run_store::now_ms(),
            finished_at: None,
            status: run_store::STATUS_RUNNING.into(),
            error: None,
            summary: None,
            target_profile: None,
            judge_profile: None,
        }
    } else {
        let m = run_store::read_meta(&dir)?
            .ok_or_else(|| "任务不存在，无法续跑或重跑".to_string())?;
        if m.status == run_store::STATUS_RUNNING {
            return Err("任务正在运行，不能重复启动".into());
        }
        m
    };

    if cfg.mode != "new" {
        // 数据集被替换后继续跑，会把新旧数据混在同一份结果里
        if !meta.dataset_signature.is_empty() && meta.dataset_signature != signature {
            return Err("数据集已被替换，续跑或重跑会把新旧数据混在一起。请新建任务。".into());
        }
        // 方案提示词改过再重跑失败项，旧结果和新判定口径不一致
        if cfg.mode == "retry" && meta.prompts != cfg.prompts {
            return Err("方案提示词已修改，重跑失败项会和旧结果混在一起。请新建任务。".into());
        }
    }

    meta.run_id = cfg.run_id.clone();
    meta.title = cfg.title.clone();
    meta.plan_id = cfg.plan_id.clone();
    meta.plan_name = cfg.plan_name.clone();
    meta.dataset_id = cfg.dataset_id.clone();
    meta.dataset_name = cfg.dataset_name.clone();
    meta.prompts = cfg.prompts.clone();
    meta.dataset_signature = signature;
    meta.total = rows.len();
    meta.status = run_store::STATUS_RUNNING.into();
    meta.finished_at = None;
    meta.error = None;
    // 只留能进报告的部分，API Key 和接口地址不落盘
    meta.target_profile = Some(run_store::profile_snapshot(&cfg.target));
    meta.judge_profile = Some(run_store::profile_snapshot(&cfg.judge));

    if cfg.mode == "new" {
        meta.started_at = run_store::now_ms();
        meta.summary = None;
        run_store::reset(&dir, &meta)?;
    } else {
        run_store::write_meta(&dir, &meta)?;
    }
    // 原始数据行跟着任务落盘，之后替换数据集库也不影响这次任务
    if let Err(e) = run_store::write_dataset(&dir, &run_store::DatasetSnapshot { rows: rows.clone() }) {
        eprintln!("保存原始数据快照失败：{e}");
    }
    let writer = run_store::open_append(&dir)?;

    let indices: Vec<usize> = match &cfg.retry_indices {
        Some(list) if !list.is_empty() => list.iter().copied().filter(|i| *i < rows.len()).collect(),
        _ => (0..rows.len()).collect(),
    };
    let total = indices.len();
    if total == 0 {
        return Err("没有需要处理的数据行".into());
    }

    // 429 闸门按连接共享：同一连接上的多个任务一起退避
    let (target_gate, judge_gate) = {
        let gates = app.state::<GateRegistry>();
        (gates.gate_for(&cfg.target), gates.gate_for(&cfg.judge))
    };

    let sink = Arc::new(Mutex::new(Sink {
        app: app.clone(),
        run_id: cfg.run_id.clone(),
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
        flag: flag.clone(),
    }));
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
        let target_gate = target_gate.clone();
        let judge_gate = judge_gate.clone();
        let sink = sink.clone();
        let tc = target_client.clone();
        let jc = judge_client.clone();
        async move {
            if flag.load(Ordering::SeqCst) {
                return;
            }
            let row = rows[idx].clone();
            let label = labels.get(idx).cloned().unwrap_or_default();
            if let Some(res) = run_one(
                &cfg,
                idx,
                &row,
                &tc,
                &jc,
                &label,
                &flag,
                &target_gate,
                &judge_gate,
            )
            .await
            {
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

    // 汇总结果写进 meta，任务列表不用解析 results.jsonl
    meta.summary = run_store::load_dir(&dir)
        .ok()
        .flatten()
        .map(|r| RunSummary::of(&r.results));
    meta.status = status.into();
    meta.finished_at = Some(run_store::now_ms());
    meta.error = error.clone();
    let _ = run_store::write_meta(&dir, &meta);

    run_store::cleanup_expired(&app);
    drop(lease);

    Ok(RunOutcome {
        run_id: cfg.run_id.clone(),
        done,
        total,
        status: status.into(),
        error,
    })
}

#[tauri::command]
pub async fn cancel_run(app: AppHandle, run_id: String) -> Result<bool, String> {
    Ok(app.state::<RunRegistry>().cancel(&run_id))
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

    /// 导出的 system 列必须是渲染后的全文：槽位声明了 resume 列，
    /// 表格里就要能看到 resume 的实际内容，而不是 {{resume}} 字面。
    #[test]
    fn exported_system_column_contains_rendered_data() {
        let r = row(&[(
            "resume",
            "张三\n教育经历：某大学\n工作经历：某公司｜运营｜3年",
        )]);
        let slot = run_store::InjectSlot {
            enabled: true,
            text: "{{resume}}".into(),
            prompt_in: true,
            data_in: false,
        };
        let sys = build_slot("你是招聘助手，请评估以下简历：\n{{resume}}", &slot, &r, "", "");
        assert!(sys.contains("张三"));
        assert!(sys.contains("工作经历：某公司｜运营｜3年"));
        assert!(!sys.contains("{{resume}}"));
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

    /// 核心规则：数据只渲染在它被填入的槽位。
    /// 列没填进某个槽位时，该槽位文本里提到的同名变量保持字面花括号。
    #[test]
    fn columns_render_only_in_the_slot_they_are_filled_into() {
        let r = row(&[("content", "原始经历"), ("mode", "strict")]);
        // 列填进 targetUser：被测 system 里提到 {{content}} 也不渲染
        let target_sys_cols = template_vars("{{mode}}");
        let sys = render_slot("整理 {{content}}（模式 {{mode}}）", &r, &target_sys_cols, false, "", "");
        assert!(sys.contains("strict"), "填进本槽位的列要渲染：{sys}");
        assert!(sys.contains("{{content}}"), "没填进本槽位的列要保持字面：{sys}");
    }

    /// {{input}}/{{output}} 只在 data_in 时渲染；不勾选时保留花括号字面。
    #[test]
    fn io_placeholders_render_only_when_data_in() {
        let r = row(&[]);
        let on = render_slot("{{input}}|{{output}}", &r, &HashSet::new(), true, "IN", "OUT");
        assert_eq!(on, "IN|OUT");
        let off = render_slot("{{input}}|{{output}}", &r, &HashSet::new(), false, "IN", "OUT");
        assert_eq!(off, "{{input}}|{{output}}");
    }

    /// 槽位未启用 / 模板为空时注入为空串——不发送空段。
    #[test]
    fn disabled_or_empty_slot_injects_nothing() {
        let r = row(&[("a", "1")]);
        let off = run_store::InjectSlot {
            enabled: false,
            text: "{{a}}".into(),
            ..Default::default()
        };
        assert_eq!(build_slot("", &off, &r, "", ""), "");
        let blank = run_store::InjectSlot {
            enabled: true,
            text: "   ".into(),
            ..Default::default()
        };
        assert_eq!(build_slot("", &blank, &r, "", ""), "");
        assert!(slot_cols(&off).is_empty(), "未启用的槽位不提供列集合");
    }

    /// 提示词本体导入：勾了才发到该槽位，没勾则该槽位只有数据模板。
    /// 主体提示词与评测提示词走同一套规则，各自独立决定去向。
    #[test]
    fn prompt_body_goes_only_to_slots_that_import_it() {
        let r = row(&[("a", "1")]);
        let data = run_store::InjectSlot {
            enabled: true,
            text: "数据：{{a}}".into(),
            prompt_in: false,
            ..Default::default()
        };
        // 纯数据槽位：数据带列名标题、每列各括号包裹
        assert_eq!(build_slot("提示词本体", &data, &r, "", ""), "数据：(===== a =====\n\n1)");

        let empty = run_store::InjectSlot {
            prompt_in: true,
            ..Default::default()
        };
        assert_eq!(build_slot("提示词本体", &empty, &r, "", ""), "提示词本体");
    }

    /// 导入了提示词本体的槽位，数据模板只声明注入哪些列，不作为文本发出。
    /// 数据靠提示词本体里的 {{列}} 渲染进去——同一份数据不会出现两次。
    #[test]
    fn data_template_only_declares_columns_when_prompt_body_is_imported() {
        let r = row(&[("title", "周末记事"), ("body", "今天天气很好")]);
        let slot = run_store::InjectSlot {
            enabled: true,
            text: "{{title}}、{{body}}".into(),
            prompt_in: true,
            ..Default::default()
        };
        let out = build_slot("把 {{body}} 改写成标题 {{title}} 的短文", &slot, &r, "", "");
        assert!(
            out.contains("(===== body =====\n\n今天天气很好)"),
            "数据带列名标题与括号：{out}"
        );
        assert!(out.contains("(===== title =====\n\n周末记事)"), "数据带列名标题与括号：{out}");
        assert!(!out.contains("周末记事、今天天气很好"), "模板不该再贴一遍：{out}");
    }

    /// 没导入提示词本体的槽位，数据模板就是消息内容。
    #[test]
    fn data_template_is_the_message_without_prompt_body() {
        let r = row(&[("title", "周末记事")]);
        let slot = run_store::InjectSlot {
            enabled: true,
            text: "{{title}}".into(),
            prompt_in: false,
            ..Default::default()
        };
        assert_eq!(
            build_slot("提示词本体", &slot, &r, "", ""),
            "(===== title =====\n\n周末记事)"
        );
    }

    /// 提示词本体在、但本槽位没声明任何列 → 提示词里的变量保持字面，数据不注入。
    #[test]
    fn prompt_body_keeps_literals_when_no_column_is_declared() {
        let r = row(&[("body", "今天天气很好")]);
        let slot = run_store::InjectSlot {
            prompt_in: true,
            ..Default::default()
        };
        assert_eq!(build_slot("把 {{body}} 改写成短文", &slot, &r, "", ""), "把 {{body}} 改写成短文");
    }

    /// {{input}} 如实反映被测模型收到的消息，空的那条不留空标题。
    #[test]
    fn judge_input_mirrors_actual_messages() {
        // 两条消息都在 → 两段都带标题
        let both = judge_input_text("你是客服", "用户问：怎么退货");
        assert!(both.starts_with("【被评估模型的提示词】"));
        assert!(both.contains("【本条输入】"));

        // 只有 system（数据全在 system 里）→ 不留空的「本条输入」
        let sys_only = judge_input_text("你是客服", "");
        assert_eq!(sys_only, "【被评估模型的提示词】\n你是客服");
        assert!(!sys_only.contains("本条输入"));

        // 只有 user → 不留空的「被评估模型的提示词」
        assert_eq!(judge_input_text("   ", "只有输入"), "只有输入");
        assert_eq!(judge_input_text("", ""), "");
    }

    #[test]
    fn hard_error_detects_target_and_judge_failures() {
        let mut r = RowResult {
            index: 0,
            system: String::new(),
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
            judge_prompt_tokens: None,
            judge_completion_tokens: None,
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

    /// 回归用：取消必须打断在途请求。
    /// 曾经的实现只在「发起下一个请求前」检查标志，一个卡住的请求能把取消拖到超时（默认 180s），
    /// 用户点完取消毫无反应。
    #[tokio::test]
    async fn cancel_aborts_inflight_request() {
        use std::io::Read;
        use std::net::TcpListener;

        // 只收请求、永不应答，模拟一个卡死的接口
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf);
                std::thread::sleep(Duration::from_secs(30));
            }
        });

        let profile = ApiProfile {
            protocol: "openai".into(),
            base_url: format!("http://127.0.0.1:{port}/v1"),
            api_key: "test".into(),
            model: "mock".into(),
            temperature: None,
            top_p: None,
            max_tokens: Some(64),
            context_window: None,
            timeout_secs: Some(30),
            extra_body: None,
            extra_headers: None,
        };
        let client = crate::api::build_client(&profile).unwrap();
        let flag = Arc::new(AtomicBool::new(false));
        let gate = AtomicU64::new(0);

        let f = flag.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            f.store(true, Ordering::SeqCst);
        });

        let started = Instant::now();
        let res = call_with_retry(&client, &profile, "", "ping", 0, &flag, &gate).await;
        let elapsed = started.elapsed();
        assert!(matches!(&res, Err(e) if e == CANCELLED), "实际返回：{res:?}");
        assert!(
            elapsed < Duration::from_secs(3),
            "取消应立刻生效，实际等了 {elapsed:?}"
        );
    }
}

/// 一次测速调用的结果
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeCall {
    pub latency_ms: u64,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub finish_reason: Option<String>,
    pub reply: String,
}

/// 测速：连续 3 次极短请求，用来把「慢」拆成两段看。
///
/// 输入只有几个 token，排除了提示词长度的影响：
/// - 3 次都在 1~3 秒 → 接口和网络正常，慢在模型生成（裁判提示词太长 / 模型在思考）
/// - 单次就几十秒 → 模型本身慢或上游排队，跟本工具无关
/// - 忽快忽慢、偶发 429 → 限流，降并发或换 Key
#[tauri::command]
pub async fn probe_api(profile: ApiProfile) -> Result<Vec<ProbeCall>, String> {
    let client = api::build_client(&profile)?;
    let mut out = Vec::with_capacity(3);
    for _ in 0..3 {
        let r = api::chat(&client, &profile, "", "只回复两个字：收到").await?;
        out.push(ProbeCall {
            latency_ms: r.latency_ms,
            prompt_tokens: r.prompt_tokens,
            completion_tokens: r.completion_tokens,
            finish_reason: r.finish_reason,
            reply: r.text.chars().take(40).collect(),
        });
    }
    Ok(out)
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
