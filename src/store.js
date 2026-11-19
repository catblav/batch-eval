import { reactive, computed, watch, markRaw } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  DEFAULT_EVAL_PROMPT,
  DEFAULT_SYSTEM_PROMPT,
  autoColsBlock,
  defaultInjectedColumns,
  defaultUserTemplate,
} from "./defaultPrompts";

/// 旧版常量保留，供分析页统计口径使用
export const PARSE_FAIL = "评测输出解析失败";
export const TARGET_FAIL = "被测模型调用失败";
export const JUDGE_FAIL = "评测模型调用失败";
export const UNCLASSIFIED = "未分类（裁判未给 errorType）";
export const BUILTIN_CATEGORIES = [PARSE_FAIL, TARGET_FAIL, JUDGE_FAIL, UNCLASSIFIED];
export const EXCLUDED_CATEGORIES = [TARGET_FAIL, JUDGE_FAIL, PARSE_FAIL];
export const TIMEOUT_LABEL = "模型超时";

const TIMEOUT_RE = /超时|timeout|timed\s*out|deadline\s*exceeded/i;
const MAX_ACTIVE_JOBS = 4;

export const PAGES = [
  { key: "run", name: "运行", icon: "play" },
  { key: "connections", name: "API 配置", icon: "plug" },
  { key: "plans", name: "提示词配置", icon: "sliders" },
  { key: "datasets", name: "数据集", icon: "database" },
  { key: "analysis", name: "分析", icon: "chart" },
];

function uid(prefix) {
  return `${prefix}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
}

function plain(v) {
  return JSON.parse(JSON.stringify(v));
}

export function blankProfile() {
  return {
    protocol: "openai",
    baseUrl: "",
    apiKey: "",
    model: "",
    temperature: 0.7,
    topP: 1,
    maxTokens: 4096,
    contextWindow: 128000,
    timeoutSecs: 180,
    extraBody: "",
    extraHeaders: "",
  };
}

export function blankConnection(name = "新连接") {
  return { id: uid("conn"), name, ...blankProfile() };
}

/// 空数据槽位：enabled = 该槽位是否注入数据，text = 数据模板（引导语 + {{列}}）
/// promptIn = 评测提示词本体是否发到该槽位，dataIn = 被测数据（{{input}}/{{output}}）是否渲染
function blankSlot() {
  return { enabled: false, text: "", promptIn: false, dataIn: false };
}

export function blankPlan(name, datasetId = "", columns = []) {
  const cols = defaultInjectedColumns(columns);
  return {
    id: uid("plan"),
    name,
    /// 下一版要用的版号，启动任务时 +1；0 表示还没打过标记
    version: 1,
    /// 主体提示词末尾自动接上「# 待处理内容 + {{列}}」，数据才渲染得进来
    systemPrompt: DEFAULT_SYSTEM_PROMPT + autoColsBlock(cols),
    evalPrompt: DEFAULT_EVAL_PROMPT,
    /// 上面那段自动列块用的列，换数据集时按它定位替换（见 syncAutoCols）
    autoCols: cols,
    datasetId,
    targetId: "",
    judgeId: "",
    /// 四槽位数据注入：默认两侧都把提示词本体和数据放 system。
    /// 被测 system 拿主体提示词，列声明即自动列块那几列；
    /// 评测 system 拿评测提示词，被测数据（{{input}}/{{output}}）在这里渲染。
    /// 两边的 user 槽位默认全不勾，需要时再开。
    injections: {
      targetSystem: { enabled: true, text: defaultUserTemplate(columns), promptIn: true, dataIn: false },
      targetUser: blankSlot(),
      judgeSystem: { enabled: false, text: "", promptIn: true, dataIn: true },
      judgeUser: blankSlot(),
    },
  };
}

/// 换数据集：把主体提示词末尾那段自动列块换掉。
/// 只在能定位到上次自动写入的那段时才改；用户改过提示词就原样保留，不去覆盖。
function syncAutoCols(plan, columns) {
  const prev = Array.isArray(plan.autoCols) ? plan.autoCols : [];
  const next = defaultInjectedColumns(columns);
  if (prev.join("\u0000") === next.join("\u0000")) return;
  const sp = String(plan.systemPrompt || "");
  const prevBlock = autoColsBlock(prev);
  const untouched = prevBlock ? sp.endsWith(prevBlock) : sp === DEFAULT_SYSTEM_PROMPT;
  if (untouched) {
    const base = prevBlock ? sp.slice(0, sp.length - prevBlock.length) : sp;
    plan.systemPrompt = base + autoColsBlock(next);
  }
  plan.autoCols = next;
}

/// 方案换数据集：数据集本身换掉，主体提示词里的自动列块跟着换
export function setPlanDataset(plan, id) {
  if (!plan) return;
  plan.datasetId = id;
  syncAutoCols(plan, datasetColumnsOf(id));
}

export const state = reactive({
  version: 2,
  ready: false,
  loadError: "",
  page: "run",
  theme: "light",
  sidebarCollapsed: false,
  /// 运行页左右两栏的占比：右侧实时明细占多少，左边任务列表拿剩下的
  liveWidthPct: 0.6,
  /// API 配置 / 提示词配置 / 数据集这三页左栏的宽度（px）与收起状态。
  /// 三页共用一份：左栏在它们那儿是同一种东西，切页时宽度跳来跳去反而奇怪
  listWidthPx: 280,
  listCollapsed: false,
  toast: { text: "", kind: "info", at: 0 },
  save: { at: 0, saving: false, error: "" },

  connections: [],
  activeConnectionId: "",

  datasets: [],
  activeDatasetId: "",
  datasetPreview: {
    id: "",
    columns: [],
    rows: [],
    total: 0,
    offset: 0,
    skipped: 0,
    loading: false,
  },

  plans: [],
  activePlanId: "",

  jobs: {
    list: [],
    progress: {},
    live: null,
  },
  runForm: {
    selectedPlanIds: [],
    titleSuffix: "",
    /// 勾选后任务名自动带上「第 N 版」，N 按方案各自的版号。
    /// 记住上次的勾选状态，重启后仍是勾上的，直到手动取消
    versionEnabled: false,
    concurrency: 4,
    maxRetries: 2,
    launchOpen: false,
  },
  /// 标黄的重点任务，存 runId，重启后还在
  starredRunIds: [],
  /// 任务记录保存范围（天），0 = 永久保存。超出范围的在启动时和任务结束后自动清理
  retentionDays: 0,
  /// 数据保存位置。真相在后端的 data-dir.txt 指针文件里，这里只是给界面显示
  dataDir: {
    current: "",
    defaultDir: "",
    isDefault: true,
    exists: true,
    runs: 0,
    datasets: 0,
  },
  /// 报告导出位置。真相在后端的 export-dir.txt；set 为 false 时导出仍弹保存框
  exportDir: {
    current: "",
    set: false,
    exists: false,
  },

  analysis: {
    runId: "",
    meta: null,
    results: [],
    badLines: 0,
    dataset: null,
    loading: false,
    /// 分析页是否跟最新一条：跑完新任务后自动跟，用户手动选过就停下不再跟
    follow: true,
    /// 实时模式：选中了运行中的任务时为 true，viewResults 改看 liveResults
    live: false,
    /// 实时模式下的结果缓存，run://progress 事件推送合并到这里（不进持久化）
    liveResults: [],
    liveDone: 0,
    liveTotal: 0,
  },

  view: {
    filterCategory: "",
    filterStatus: "all",
    filterGroup: "",
    query: "",
    queryApplied: "",
    page: 1,
    pageSize: 10,
    selectedIndex: -1,
  },
});

// ---------- 通用 ----------
let toastTimer = null;
export function toast(text, kind = "info", ms = 3500) {
  clearTimeout(toastTimer);
  state.toast = { text, kind, at: Date.now() };
  toastTimer = setTimeout(() => {
    state.toast = { text: "", kind: "info", at: 0 };
  }, ms);
}

/// 切页：先把当前编辑落盘，再进入目标页
export function usePage(key) {
  if (!PAGES.some((p) => p.key === key)) return;
  state.page = key;
  if (key === "run" || key === "analysis") loadJobs();
  flushSave();
}

/// 从方案页去运行页：勾选该方案并展开启动面板，不直接开跑
export function usePlanForRun(id) {
  state.runForm.selectedPlanIds = [id];
  state.runForm.launchOpen = true;
  usePage("run");
}

export function applyTheme(theme) {
  const dark = theme === "dark";
  document.documentElement.classList.toggle("dark", dark);
  state.theme = dark ? "dark" : "light";
}

export function toggleTheme() {
  applyTheme(state.theme === "dark" ? "light" : "dark");
  scheduleSave();
}

/// 左栏收起 / 展开，只留一排图标；选择随设置持久化
export function toggleSidebar() {
  state.sidebarCollapsed = !state.sidebarCollapsed;
  scheduleSave();
}

/// 三个两栏页（API 配置 / 提示词配置 / 数据集）左栏的收起 / 展开
export function toggleList() {
  state.listCollapsed = !state.listCollapsed;
  scheduleSave();
}

// ---------- 连接库 ----------
export function connectionById(id) {
  return state.connections.find((c) => c.id === id) || null;
}

/// 只挑 ApiProfile 需要的字段，id / name 不发给后端
export function profileOf(conn) {
  if (!conn) return blankProfile();
  return {
    protocol: conn.protocol || "openai",
    baseUrl: conn.baseUrl || "",
    apiKey: conn.apiKey || "",
    model: conn.model || "",
    temperature: conn.temperature ?? 0.7,
    topP: conn.topP ?? 1,
    maxTokens: conn.maxTokens ?? 4096,
    contextWindow: conn.contextWindow ?? 128000,
    timeoutSecs: conn.timeoutSecs ?? 180,
    extraBody: conn.extraBody || "",
    extraHeaders: conn.extraHeaders || "",
  };
}

export function createConnection() {
  const c = blankConnection(`连接 ${state.connections.length + 1}`);
  state.connections.push(c);
  state.activeConnectionId = c.id;
  return c;
}

export function duplicateConnection(id) {
  const src = connectionById(id);
  if (!src) return null;
  const copy = plain(src);
  copy.id = uid("conn");
  copy.name = uniqueName(plain(state.connections).map((c) => c.name), `${src.name} 副本`);
  state.connections.push(copy);
  state.activeConnectionId = copy.id;
  return copy;
}

/// 删除连接时清掉方案里的引用，返回受影响的方案名
export function deleteConnection(id) {
  const conn = connectionById(id);
  if (!conn) return [];
  const affected = state.plans
    .filter((p) => p.targetId === id || p.judgeId === id)
    .map((p) => p.name);
  for (const p of state.plans) {
    if (p.targetId === id) p.targetId = "";
    if (p.judgeId === id) p.judgeId = "";
  }
  state.connections = state.connections.filter((c) => c.id !== id);
  if (state.activeConnectionId === id) {
    state.activeConnectionId = state.connections[0]?.id || "";
  }
  return affected;
}

// ---------- 数据集库 ----------
export function datasetMetaById(id) {
  return state.datasets.find((d) => d.id === id) || null;
}

export function datasetColumnsOf(id) {
  return datasetMetaById(id)?.columns || [];
}

export async function loadDatasets() {
  const list = await invoke("list_datasets");
  state.datasets = Array.isArray(list) ? list : [];
  if (!datasetMetaById(state.activeDatasetId)) {
    state.activeDatasetId = state.datasets[0]?.id || "";
  }
}

export async function loadDatasetPreview(id, offset = 0, limit = 50) {
  if (!id) {
    state.datasetPreview = {
      id: "",
      columns: [],
      rows: [],
      total: 0,
      offset: 0,
      skipped: 0,
      loading: false,
    };
    return;
  }
  state.datasetPreview = {
    ...state.datasetPreview,
    id,
    loading: true,
  };
  try {
    const p = await invoke("preview_dataset", { id, offset, limit });
    if (state.datasetPreview.id !== id) return;
    state.datasetPreview = {
      id,
      columns: p.columns || [],
      rows: p.rows || [],
      total: p.total || 0,
      offset: p.offset || 0,
      skipped: p.skipped || 0,
      loading: false,
    };
  } catch (e) {
    state.datasetPreview = { ...state.datasetPreview, loading: false };
    toast("读取数据集失败：" + e, "err");
  }
}

export async function selectDataset(id) {
  state.activeDatasetId = id;
  await loadDatasetPreview(id, 0, 50);
}

export async function importDatasetFile(path) {
  const meta = await invoke("import_dataset_file", { path, name: null });
  await loadDatasets();
  await selectDataset(meta.id);
  toast(`已导入「${meta.name}」：${meta.rows} 行 · ${meta.columns.length} 列`, "ok");
  return meta;
}

export async function importDatasetText(text, format, name) {
  const meta = await invoke("import_dataset_text", {
    text,
    format: format || null,
    name: name || null,
  });
  await loadDatasets();
  await selectDataset(meta.id);
  toast(`已导入「${meta.name}」：${meta.rows} 行 · ${meta.columns.length} 列`, "ok");
  return meta;
}

export async function replaceDataset(id, path) {
  const meta = await invoke("replace_dataset", { id, path });
  await loadDatasets();
  if (state.activeDatasetId === id) await loadDatasetPreview(id, 0, 50);
  await syncDatasetMirrorIfCurrent(id);
  toast(`已替换「${meta.name}」：${meta.rows} 行 · ${meta.columns.length} 列`, "ok");
  return meta;
}

export async function renameDataset(id, name) {
  const meta = await invoke("rename_dataset", { id, name });
  await loadDatasets();
  return meta;
}

export async function removeDataset(id) {
  await invoke("delete_dataset", { id });
  for (const p of state.plans) {
    if (p.datasetId === id) p.datasetId = "";
  }
  await loadDatasets();
  if (state.activeDatasetId === id) {
    state.activeDatasetId = state.datasets[0]?.id || "";
    await loadDatasetPreview(state.activeDatasetId, 0, 50);
  }
}

async function syncDatasetMirrorIfCurrent(id) {
  const plan = activePlan();
  if (plan && plan.datasetId === id) {
    try {
      await invoke("sync_dataset_mirror_cmd", { id });
    } catch (e) {
      console.warn("同步 auto-iterate 数据集镜像失败", e);
    }
  }
}

export async function syncDatasetMirror(id) {
  if (!id) return;
  try {
    await invoke("sync_dataset_mirror_cmd", { id });
  } catch (e) {
    console.warn("同步 auto-iterate 数据集镜像失败", e);
  }
}

// ---------- 方案库 ----------
export function planById(id) {
  return state.plans.find((p) => p.id === id) || null;
}

export function activePlan() {
  return planById(state.activePlanId);
}

export function jsonObjectError(text) {
  const t = String(text || "").trim();
  if (!t) return "";
  try {
    const v = JSON.parse(t);
    if (!v || Array.isArray(v) || typeof v !== "object") return "需为 JSON 对象";
    return "";
  } catch {
    return "JSON 格式错误";
  }
}

function connectionProblems(conn, label) {
  const out = [];
  if (!conn) {
    out.push(`未选择${label}`);
    return out;
  }
  if (!String(conn.baseUrl || "").trim()) out.push(`${label}缺 Base URL`);
  if (!String(conn.model || "").trim()) out.push(`${label}缺模型名`);
  const bodyErr = jsonObjectError(conn.extraBody);
  if (bodyErr) out.push(`${label}自定义请求体${bodyErr}`);
  const headerErr = jsonObjectError(conn.extraHeaders);
  if (headerErr) out.push(`${label}自定义请求头${headerErr}`);
  return out;
}

export function planProblems(plan) {
  if (!plan) return ["方案不存在"];
  const out = [];
  if (!plan.datasetId) out.push("未选择数据集");
  else if (!datasetMetaById(plan.datasetId)) out.push("数据集不存在");
  out.push(...connectionProblems(connectionById(plan.targetId), "被测连接"));
  out.push(...connectionProblems(connectionById(plan.judgeId), "评测连接"));
  if (!String(plan.systemPrompt || "").trim()) out.push("主体提示词为空");
  if (!String(plan.evalPrompt || "").trim()) out.push("评测提示词为空");
  return out;
}

export function planBindingText(plan) {
  const ds = datasetMetaById(plan?.datasetId);
  const target = connectionById(plan?.targetId);
  const judge = connectionById(plan?.judgeId);
  const left = target?.model || target?.name || "未选被测";
  const right = judge?.model || judge?.name || "未选评测";
  return `${ds?.name || "未选数据集"} · ${left} → ${right}`;
}

function uniqueName(existing, base) {
  const set = new Set(existing);
  if (!set.has(base)) return base;
  for (let i = 2; i < 1000; i++) {
    const name = `${base} ${i}`;
    if (!set.has(name)) return name;
  }
  return `${base} ${Date.now()}`;
}

export function createPlan() {
  const first = state.datasets[0];
  const plan = blankPlan(
    uniqueName(plain(state.plans).map((p) => p.name), `方案 ${state.plans.length + 1}`),
    first?.id || "",
    first?.columns || []
  );
  state.plans.push(plan);
  state.activePlanId = plan.id;
  return plan;
}

export function duplicatePlan(id) {
  const src = planById(id);
  if (!src) return null;
  const copy = plain(src);
  copy.id = uid("plan");
  copy.name = uniqueName(plain(state.plans).map((p) => p.name), `${src.name} 副本`);
  // 副本没有自己的运行历史，版号从第 1 版重新算
  copy.version = 1;
  state.plans.push(copy);
  state.activePlanId = copy.id;
  return copy;
}

export function deletePlan(id) {
  state.plans = state.plans.filter((p) => p.id !== id);
  if (state.activePlanId === id) state.activePlanId = state.plans[0]?.id || "";
}

// ---------- 变量检查 ----------
const RESERVED_VARS = new Set(["input", "output"]);

export function referencedVars(text) {
  const out = new Set();
  for (const m of String(text || "").matchAll(/\{\{\s*([^{}]+?)\s*\}\}/g)) {
    out.add(m[1]);
  }
  return out;
}

export const templateVarCheck = computed(() => {
  const plan = activePlan();
  const cols = datasetColumnsOf(plan?.datasetId);
  const injTexts = Object.values(plan?.injections || {}).map((s) => s?.text || "");
  const used = new Set([
    ...referencedVars(plan?.systemPrompt),
    ...injTexts.flatMap((t) => [...referencedVars(t)]),
  ]);
  const colSet = new Set(cols);
  const missing = cols.filter((c) => !used.has(c));
  const unknown = [...used].filter((v) => !RESERVED_VARS.has(v) && !colSet.has(v));
  return { missing, unknown, ok: !missing.length && !unknown.length };
});

export function buildUserTemplate(columns) {
  const cols = defaultInjectedColumns(columns);
  if (!cols.length) return "";
  return cols.map((c) => `{{${c}}}`).join("\n\n");
}

// ---------- 任务 ----------
export const activeJobs = computed(() =>
  state.jobs.list.filter((j) => j.status === "running" || j.status === "cancelling")
);
export const pastJobs = computed(() =>
  state.jobs.list.filter((j) => j.status !== "running")
);

export function jobProgress(job) {
  const p = state.jobs.progress[job.runId];
  if (p) return p;
  const done = job.summary?.total || 0;
  return { done, total: job.total || done };
}

/// 任务列表用的统计：打开明细的任务走 live 实时结果，运行中的走 per-run 实时缓存，
/// 已结束的走后端 summary。口径与 computeStats 一致。
export function runStats(job) {
  if (job.runId === state.jobs.live?.runId) {
    const live = liveStats.value;
    // 还没结果时 passRate 是 0，界面应显示「—」而不是 0%
    if (live.total) return live;
    return emptyRunStats();
  }
  if (["running", "cancelling"].includes(job.status)) {
    const c = perRun.get(job.runId);
    if (c && c.results.length) return computeStats(c.results);
  }
  const r = emptyRunStats();
  const s = job.summary;
  r.total = s?.total ?? 0;
  r.valid = s?.valid ?? 0;
  r.passed = s?.passed ?? 0;
  r.failed = s?.failed ?? 0;
  r.excluded = s?.excluded ?? 0;
  r.passRate = r.valid ? r.passed / r.valid : null;
  return r;
}

function emptyRunStats() {
  return {
    total: 0,
    valid: 0,
    passed: 0,
    failed: 0,
    excluded: 0,
    passRate: null,
    byType: [],
    excludedTypes: [],
  };
}

export function jobPassRate(job) {
  return runStats(job).passRate;
}

/// 旧版本给任务标题自动拼了「· 11月12日 15:52」，列表已有时间列，显示时去掉这段后缀
const DATE_SUFFIX_RE = / · \d{1,2}月\d{1,2}日 \d{1,2}:\d{2}$/;
function stripDateSuffix(title) {
  return String(title || "").replace(DATE_SUFFIX_RE, "");
}

export async function loadJobs() {
  try {
    const list = await invoke("list_runs");
    const arr = Array.isArray(list) ? list : [];
    // 保留正在启动、后端还没写入目录的占位任务
    const runningPlaceholders = state.jobs.list.filter(
      (j) => j.pending && !arr.some((x) => x.runId === j.runId)
    );
    // 出现了之前没有的任务 = 刚跑完/刚启动一个新任务，分析页重新跟到最新一条。
    // 用户手动选过某条老任务后 follow 为 false，这里不再打扰他。
    // 注意：before 只算非占位任务。launchJobs 启动时会先 unshift 一个 pending 占位任务，
    // 若把占位 runId 也算进 before，list_runs 返回真身时会被误判为「已有任务」，follow 永远不置回。
    const before = new Set(
      state.jobs.list.filter((j) => !j.pending).map((j) => j.runId)
    );
    if (arr.some((j) => !before.has(j.runId))) state.analysis.follow = true;
    state.jobs.list = [
      ...runningPlaceholders,
      ...arr.map((j) => ({ ...j, title: stripDateSuffix(j.title) })),
    ];
    // 清理已结束任务的 per-run 实时缓存，避免残留；运行中/取消中的保留
    for (const runId of perRun.keys()) {
      const j = state.jobs.list.find((x) => x.runId === runId);
      if (!j || !["running", "cancelling"].includes(j.status)) perRun.delete(runId);
    }
  } catch (e) {
    console.warn("读取任务列表失败", e);
  }
}

/// 老方案第一次用版号：按已跑过的任务数补出下一版，之后改由 plan.version 自己累加，
/// 这样任务记录被清理也不会让版号回退、重号。
function initPlanVersions() {
  let changed = false;
  for (const p of state.plans) {
    if (p.version != null) continue;
    const runs = state.jobs.list.filter((j) => j.planId === p.id).length;
    p.version = runs + 1;
    changed = true;
  }
  if (changed) scheduleSave();
}

// ---------- 任务记录保存范围 ----------

export const RETENTION_OPTIONS = [
  { days: 0, label: "永久保存", short: "永久" },
  { days: 30, label: "保留近一个月", short: "近一个月" },
  { days: 7, label: "保留近一周", short: "近一周" },
  { days: 3, label: "保留近三天", short: "近三天" },
];

export function retentionLabelOf(days) {
  return RETENTION_OPTIONS.find((o) => o.days === Number(days))?.label || "永久保存";
}

export function retentionShortOf(days) {
  return RETENTION_OPTIONS.find((o) => o.days === Number(days))?.short || "永久";
}

/// 只数不删，给确认弹窗用
export async function previewRetention(days) {
  try {
    return Number(await invoke("preview_retention", { days: Number(days) || 0 })) || 0;
  } catch (e) {
    console.warn("统计过期任务失败", e);
    return 0;
  }
}

/// 应用保存范围：先落盘设置（后端下次启动读盘），再清理，最后同步两侧的选中项
export async function applyRetention(days) {
  state.retentionDays = Number(days) || 0;
  await flushSave();
  let removed = 0;
  try {
    removed = Number(await invoke("apply_retention", { days: state.retentionDays })) || 0;
  } catch (e) {
    toast("清理任务记录失败：" + e, "err");
    return 0;
  }
  await loadJobs();
  const alive = new Set(state.jobs.list.map((j) => j.runId));
  if (state.jobs.live && !alive.has(state.jobs.live.runId)) state.jobs.live = null;
  if (state.analysis.runId && !alive.has(state.analysis.runId)) {
    await selectAnalysisRun("");
  }
  return removed;
}

// ---------- 数据保存位置 ----------

export async function loadDataDir() {
  try {
    const info = await invoke("data_dir_info");
    if (info) state.dataDir = info;
  } catch (e) {
    console.warn("读取数据位置失败", e);
  }
}

/// 切换数据保存位置。moveExisting 为真时把现有任务记录和数据集复制过去。
/// 切完之后任务列表、数据集、分析页选中项全部按新位置重来。
export async function applyDataDir(dir, moveExisting) {
  const report = await invoke("set_data_dir", { dir, moveExisting });
  await loadDataDir();

  state.jobs.live = null;
  await loadJobs();
  await selectAnalysisRun("");

  await loadDatasets();
  if (state.activeDatasetId) await loadDatasetPreview(state.activeDatasetId, 0, 50);
  const plan = activePlan();
  if (plan?.datasetId) await syncDatasetMirror(plan.datasetId);

  return report || { runs: 0, datasets: 0 };
}

export async function openDataDir() {
  try {
    await invoke("open_data_dir");
  } catch (e) {
    toast("打开文件夹失败：" + e, "err");
  }
}

// ---------- 报告导出位置 ----------

export async function loadExportDir() {
  try {
    const info = await invoke("export_dir_info");
    if (info) state.exportDir = info;
  } catch (e) {
    console.warn("读取导出位置失败", e);
  }
}

/// 设置固定导出目录；传空字符串等于恢复「每次导出弹保存框」
export async function applyExportDir(dir) {
  const info = await invoke("set_export_dir", { dir });
  state.exportDir = info;
  return info;
}

export async function openExportDir() {
  try {
    await invoke("open_export_dir");
  } catch (e) {
    toast("打开文件夹失败：" + e, "err");
  }
}

/// 勾选多个方案并发启动，每个方案一个独立任务。
/// 不等待任务跑完，立即返回启动数量。
export function launchJobs() {
  const ids = state.runForm.selectedPlanIds;
  if (!ids.length) {
    toast("先勾选要运行的方案", "info");
    return 0;
  }
  if (activeJobs.value.length + ids.length > MAX_ACTIVE_JOBS) {
    toast(
      `同时最多运行 ${MAX_ACTIVE_JOBS} 个任务，当前已有 ${activeJobs.value.length} 个。先等一个任务结束。`,
      "err",
      6000
    );
    return 0;
  }

  const problems = [];
  const plans = [];
  for (const id of ids) {
    const plan = planById(id);
    const ps = planProblems(plan);
    if (ps.length) problems.push(`${plan?.name || id}：${ps.join("、")}`);
    else plans.push(plan);
  }
  if (problems.length) {
    toast("有方案还不能运行：" + problems.join("；"), "err", 8000);
    return 0;
  }

  const concurrency = Number(state.runForm.concurrency) || 1;
  const maxRetries = Number(state.runForm.maxRetries) || 0;
  const suffix = state.runForm.titleSuffix.trim();
  const withVersion = !!state.runForm.versionEnabled;
  const now = Date.now();

  let started = 0;
  for (const [i, plan] of plans.entries()) {
    const runId = `run-${now}-${i}-${Math.random().toString(36).slice(2, 6)}`;
    // 版号按方案各自的累计次数走：方案 A 第 3 版、方案 B 第 1 版互不影响
    const version = Math.max(1, Number(plan.version) || 1);
    let title = suffix ? `${plan.name} · ${suffix}` : plan.name;
    if (withVersion) title += ` · 第${version}版`;
    // 启动即占号，避免连点两次启动时两个任务用同一个版号
    if (withVersion) plan.version = version + 1;
    const ds = datasetMetaById(plan.datasetId);
    const job = {
      runId,
      title,
      planId: plan.id,
      planName: plan.name,
      datasetId: plan.datasetId,
      datasetName: ds?.name || "",
      total: ds?.rows || 0,
      startedAt: now,
      finishedAt: null,
      status: "running",
      error: null,
      summary: null,
      concurrency,
      pending: true,
    };
    state.jobs.list.unshift(job);
    state.jobs.progress[runId] = { done: 0, total: job.total };
    started++;

    const cfg = {
      runId,
      title,
      planId: plan.id,
      planName: plan.name,
      prompts: {
        systemPrompt: plan.systemPrompt,
        evalPrompt: plan.evalPrompt,
        injections: plan.injections,
      },
      datasetId: plan.datasetId,
      datasetName: ds?.name || "",
      target: profileOf(connectionById(plan.targetId)),
      judge: profileOf(connectionById(plan.judgeId)),
      concurrency,
      maxRetries,
      mode: "new",
      retryIndices: null,
    };

    invoke("start_run", { cfg })
      .then(async (outcome) => {
        updateJobPlaceholder(runId, outcome);
        await loadJobs();
        if (state.jobs.live?.runId === runId) await refreshLiveMeta(runId);
      })
      .catch(async (e) => {
        updateJobPlaceholder(runId, { status: "failed", error: String(e) });
        if (state.jobs.live?.runId === runId) {
          state.jobs.live.status = "failed";
          state.jobs.live.error = String(e);
        }
        await loadJobs();
        toast(`「${plan.name}」启动失败：${e}`, "err", 8000);
      });
  }

  if (withVersion) scheduleSave();
  state.runForm.selectedPlanIds = [];
  state.runForm.launchOpen = false;
  return started;
}

/// 刷新版号：选中的方案都回到第 1 版重新数
export function resetPlanVersions(ids) {
  const set = new Set(ids || []);
  for (const p of state.plans) {
    if (set.has(p.id)) p.version = 1;
  }
  scheduleSave();
}

// ---------- 重点任务星标 ----------
export function isStarred(runId) {
  return state.starredRunIds.includes(runId);
}

export function toggleStar(runId) {
  const i = state.starredRunIds.indexOf(runId);
  if (i >= 0) state.starredRunIds.splice(i, 1);
  else state.starredRunIds.push(runId);
  scheduleSave();
}

function updateJobPlaceholder(runId, outcome) {
  const job = state.jobs.list.find((j) => j.runId === runId);
  if (!job) return;
  job.pending = false;
  if (outcome?.status) job.status = outcome.status;
  if (outcome?.error) job.error = outcome.error;
  if (typeof outcome?.done === "number") job.done = outcome.done;
  if (typeof outcome?.total === "number") job.total = outcome.total;
}

async function refreshLiveMeta(runId) {
  const run = await invoke("load_run", { runId });
  if (run && state.jobs.live?.runId === runId) {
    state.jobs.live.status = run.meta.status;
    state.jobs.live.error = run.meta.error || "";
    mergeLiveMissing(run.results);
    state.jobs.live.done = Math.max(state.jobs.live.results.length, state.jobs.live.done);
    state.jobs.live.total = run.meta.total;
  }
}

async function buildCfgFromJob(job, mode, retryIndices) {
  const plan = planById(job.planId);
  if (!plan) throw new Error("方案已被删除，无法续跑或重跑");
  const ds = datasetMetaById(plan.datasetId);
  if (!ds) throw new Error("方案绑定的数据集已被删除");
  const target = connectionById(plan.targetId);
  const judge = connectionById(plan.judgeId);
  if (!target || !judge) throw new Error("方案绑定的连接已被删除");
  return {
    runId: job.runId,
    title: job.title,
    planId: plan.id,
    planName: plan.name,
    prompts: {
      systemPrompt: plan.systemPrompt,
      evalPrompt: plan.evalPrompt,
      injections: plan.injections,
    },
    datasetId: plan.datasetId,
    datasetName: ds.name,
    target: profileOf(target),
    judge: profileOf(judge),
    concurrency: Number(state.runForm.concurrency) || 1,
    maxRetries: Number(state.runForm.maxRetries) || 0,
    mode,
    retryIndices,
  };
}

export async function cancelJob(runId) {
  const found = await invoke("cancel_run", { runId });
  if (found) {
    toast("正在取消，在途请求已中断", "info");
    const job = state.jobs.list.find((j) => j.runId === runId);
    if (job) job.status = "cancelling";
    if (state.jobs.live?.runId === runId) state.jobs.live.status = "cancelling";
  } else {
    toast("没有找到正在运行的任务，可能已经结束了", "info");
  }
  return found;
}

export async function deleteJob(job) {
  await invoke("delete_run", { runId: job.runId });
  perRun.delete(job.runId);
  if (state.jobs.live?.runId === job.runId) state.jobs.live = null;
  if (state.analysis.runId === job.runId) {
    state.analysis.runId = "";
    state.analysis.live = false;
    state.analysis.liveResults = [];
    analysisPos.clear();
  }
  // 任务没了，星标也一起清掉，免得越攒越多
  const si = state.starredRunIds.indexOf(job.runId);
  if (si >= 0) state.starredRunIds.splice(si, 1);
  await loadJobs();
  toast("已删除这个任务", "ok");
}

export async function resumeJob(job) {
  const loaded = await invoke("load_run", { runId: job.runId });
  if (!loaded) throw new Error("任务结果文件不存在");
  const existing = new Set((loaded.results || []).map((r) => r.index));
  const indices = Array.from({ length: loaded.meta.total || 0 }, (_, i) => i).filter(
    (i) => !existing.has(i)
  );
  if (!indices.length) {
    toast("没有未完成的行", "info");
    return false;
  }
  const cfg = await buildCfgFromJob(job, "resume", indices);
  seedPerRun(job.runId, loaded.results);
  markJobRunning(job);
  invoke("start_run", { cfg })
    .then(async (outcome) => {
      updateJobPlaceholder(job.runId, outcome);
      await loadJobs();
      if (state.jobs.live?.runId === job.runId) await refreshLiveMeta(job.runId);
    })
    .catch(async (e) => {
      updateJobPlaceholder(job.runId, { status: job.status, error: String(e) });
      await loadJobs();
      toast("续跑失败：" + e, "err", 8000);
    });
  return true;
}

/// 需要重跑的判定：调用没通、评测输出没解析出来、或者判了不通过。
/// retryJob 和界面上的条数提示共用这一份，避免两处口径走偏。
function isRetryable(r) {
  return !!(r.targetError || !r.parseOk || !r.pass);
}

export function retryableCount(results) {
  return (results || []).filter(isRetryable).length;
}

export async function retryJob(job) {
  const loaded = await invoke("load_run", { runId: job.runId });
  if (!loaded) throw new Error("任务结果文件不存在");
  const indices = (loaded.results || []).filter(isRetryable).map((r) => r.index);
  if (!indices.length) {
    toast("没有需要重跑的行", "info");
    return false;
  }
  const cfg = await buildCfgFromJob(job, "retry", indices);
  seedPerRun(job.runId, loaded.results);
  markJobRunning(job);
  invoke("start_run", { cfg })
    .then(async (outcome) => {
      updateJobPlaceholder(job.runId, outcome);
      await loadJobs();
      if (state.jobs.live?.runId === job.runId) await refreshLiveMeta(job.runId);
    })
    .catch(async (e) => {
      updateJobPlaceholder(job.runId, { status: job.status, error: String(e) });
      await loadJobs();
      toast("重跑失败：" + e, "err", 8000);
    });
  return true;
}

function markJobRunning(job) {
  job.status = "running";
  job.error = null;
  job.concurrency = Number(state.runForm.concurrency) || 1;
  state.jobs.progress[job.runId] = {
    done: state.jobs.live?.runId === job.runId ? state.jobs.live.results.length : 0,
    total: job.total,
  };
  // 明细面板跟着切到运行中，操作按钮立即从「续跑/重跑」变成「停止」
  if (state.jobs.live?.runId === job.runId) {
    state.jobs.live.status = "running";
    state.jobs.live.error = "";
  }
}

/// 清空某个任务的结果：结果文件截断，任务退回「已中断」，可再续跑（等于全量重跑）。
/// 任务本身、数据集快照和提示词快照都保留。
export async function clearJob(job) {
  await invoke("clear_run", { runId: job.runId });
  perRun.delete(job.runId);
  await loadJobs();
  const live = state.jobs.live;
  if (live?.runId === job.runId) {
    live.results = [];
    live.done = 0;
    live.status = "interrupted";
    live.error = "";
    livePos.clear();
    liveReceived = new Set();
  }
  // 分析页若正实时盯着该任务，同步清空
  if (state.analysis.live && state.analysis.runId === job.runId) {
    state.analysis.liveResults = [];
    state.analysis.liveDone = 0;
    analysisPos.clear();
  }
  toast("已清空这个任务的结果", "ok");
}

// ---------- 实时明细 ----------
const livePos = new Map();
let liveReceived = new Set();

function normalizeResult(raw) {
  const r = markRaw(raw);
  try {
    Object.defineProperty(r, "__hay", {
      value: null,
      writable: true,
      configurable: true,
      enumerable: false,
    });
  } catch {
    /* 忽略 */
  }
  return r;
}

/// 每个运行中任务各自的实时结果缓存：任务列表里的通过率 / 通过失败统计用。
/// 与 state.jobs.live（当前打开明细的任务）互相独立，切明细不影响统计连续性。
const perRun = new Map();

/// 续跑 / 重跑前按磁盘已有结果重建 per-run 缓存，新完成的行再增量合并
function seedPerRun(runId, results) {
  const list = (results || []).map(normalizeResult).sort((a, b) => a.index - b.index);
  const pos = new Map();
  list.forEach((r, i) => pos.set(r.index, i));
  perRun.set(runId, { results: list, pos });
}

/// 把推送的结果合并进该任务自己的实时缓存（按 runId 各配位置表）
function mergePerRun(payload) {
  if (!payload.runId) return;
  let c = perRun.get(payload.runId);
  if (!c) {
    c = { results: [], pos: new Map() };
    perRun.set(payload.runId, c);
  }
  if (typeof payload.done === "number") c.done = payload.done;
  if (typeof payload.total === "number") c.total = payload.total;
  mergeInto(c.results, c.pos, payload.results);
}

function reindexLive() {
  livePos.clear();
  state.jobs.live?.results.forEach((r, i) => livePos.set(r.index, i));
}

export function resetLiveResults() {
  if (!state.jobs.live) return;
  state.jobs.live.results = [];
  livePos.clear();
  liveReceived = new Set();
}

function setLiveResults(list) {
  if (!state.jobs.live) return;
  state.jobs.live.results = (list || [])
    .map(normalizeResult)
    .sort((a, b) => a.index - b.index);
  reindexLive();
  liveReceived = new Set(state.jobs.live.results.map((r) => r.index));
}

function mergeLiveMissing(list) {
  if (!state.jobs.live) return;
  let added = 0;
  for (const raw of list || []) {
    if (livePos.has(raw.index)) continue;
    const r = normalizeResult(raw);
    livePos.set(r.index, state.jobs.live.results.length);
    state.jobs.live.results.push(r);
    added++;
  }
  if (!added) return;
  state.jobs.live.results.sort((a, b) => a.index - b.index);
  reindexLive();
}

/// 把推送的结果合并进目标数组：按 index 去重（后写覆盖先写），排序后重建位置表。
/// 运行页实时明细和分析页实时缓存共用这一套，各配自己的位置表，互不串台。
function mergeInto(target, posMap, rawList) {
  for (const raw of rawList || []) {
    const r = normalizeResult(raw);
    const pos = posMap.get(r.index);
    if (pos === undefined) {
      posMap.set(r.index, target.length);
      target.push(r);
    } else {
      target[pos] = r;
    }
  }
  target.sort((a, b) => a.index - b.index);
  posMap.clear();
  target.forEach((r, i) => posMap.set(r.index, i));
}

function mergeLiveProgress(payload) {
  const live = state.jobs.live;
  if (!live || live.runId !== payload.runId) return;
  if (typeof payload.done === "number") live.done = payload.done;
  if (typeof payload.total === "number") live.total = payload.total;
  for (const raw of payload.results || []) liveReceived.add(raw.index);
  mergeInto(live.results, livePos, payload.results);
}

/// 分析页实时缓存：只合并当前选中的运行中任务，done/total 更新到 analysis 上
const analysisPos = new Map();
function mergeAnalysisLive(payload) {
  const a = state.analysis;
  if (!a.live || a.runId !== payload.runId) return;
  if (typeof payload.done === "number") a.liveDone = payload.done;
  if (typeof payload.total === "number") a.liveTotal = payload.total;
  mergeInto(a.liveResults, analysisPos, payload.results);
}

let runListener = null;
export async function attachRunListener() {
  if (runListener) return;
  runListener = await listen("run://progress", (ev) => {
    const p = ev.payload || {};
    if (p.runId) {
      state.jobs.progress[p.runId] = { done: p.done, total: p.total };
      mergeLiveProgress(p);
      mergePerRun(p);
      mergeAnalysisLive(p);
    }
  });
}

/// 打开某个任务的实时明细；切换任务时清空上一个任务的结果，避免串台
export async function openLive(runId) {
  if (state.jobs.live?.runId === runId) return;
  const job = state.jobs.list.find((j) => j.runId === runId);
  state.jobs.live = {
    runId,
    results: [],
    dataset: null,
    done: 0,
    total: job?.total || 0,
    status: job?.status || "running",
    title: job?.title || "",
    planName: job?.planName || "",
    datasetName: job?.datasetName || "",
    error: job?.error || "",
    loading: true,
  };
  livePos.clear();
  liveReceived = new Set();
  try {
    const run = await invoke("load_run", { runId });
    if (state.jobs.live?.runId !== runId) return;
    if (!run) {
      state.jobs.live.loading = false;
      return;
    }
    setLiveResults(run.results || []);
    state.jobs.live.done = Math.max(state.jobs.live.done, state.jobs.live.results.length);
    state.jobs.live.total = run.meta.total || state.jobs.live.total;
    state.jobs.live.status = run.meta.status;
    state.jobs.live.title = stripDateSuffix(run.meta.title) || state.jobs.live.title;
    state.jobs.live.planName = run.meta.planName || state.jobs.live.planName;
    state.jobs.live.datasetName = run.meta.datasetName || state.jobs.live.datasetName;
    state.jobs.live.dataset = run.dataset || null;
    state.jobs.live.error = run.meta.error || "";
  } catch (e) {
    toast("读取任务明细失败：" + e, "err");
  } finally {
    if (state.jobs.live?.runId === runId) state.jobs.live.loading = false;
  }
}

export const liveStats = computed(() => computeStats(state.jobs.live?.results || []));

export function statusLabelOf(row) {
  if (row && Object.prototype.hasOwnProperty.call(row, "数据组状态")) {
    return String(row["数据组状态"] || "未分组");
  }
  for (const k of ["状态", "分组", "类别", "status", "group", "case"]) {
    if (row && row[k] !== undefined && String(row[k]).trim() !== "") return String(row[k]);
  }
  return "未分组";
}

// ---------- 分析 ----------
export async function selectAnalysisRun(runId, force = false) {
  if (!runId) {
    state.analysis = {
      runId: "",
      meta: null,
      results: [],
      badLines: 0,
      dataset: null,
      loading: false,
      follow: state.analysis.follow,
      live: false,
      liveResults: [],
      liveDone: 0,
      liveTotal: 0,
    };
    resetAnalysisView();
    return;
  }
  if (state.analysis.runId === runId && !force) return;
  state.analysis.loading = true;
  try {
    const run = await invoke("load_run", { runId });
    const job = state.jobs.list.find((j) => j.runId === runId);
    const isLive = !!job && ["running", "cancelling"].includes(job.status);
    if (!run) {
      // 运行中但还没建目录的占位任务：不报错，先进实时模式等事件填充
      if (isLive) {
        state.analysis = {
          runId,
          meta: null,
          results: [],
          badLines: 0,
          dataset: null,
          loading: false,
          follow: state.analysis.follow,
          live: true,
          liveResults: [],
          liveDone: 0,
          liveTotal: job?.total || 0,
        };
        resetAnalysisView();
        return;
      }
      toast("这条任务记录已经不在了", "info");
      state.analysis.runId = "";
      state.analysis.meta = null;
      state.analysis.results = [];
      return;
    }
    const normalized = (run.results || [])
      .map(normalizeResult)
      .sort((a, b) => a.index - b.index);
    state.analysis = {
      runId,
      meta: run.meta,
      results: normalized,
      badLines: run.badLines || 0,
      dataset: run.dataset || null,
      loading: false,
      follow: state.analysis.follow,
      live: isLive,
      liveResults: isLive ? normalized.slice() : [],
      liveDone: isLive ? normalized.length : 0,
      liveTotal: isLive ? run.meta.total || 0 : 0,
    };
    if (isLive) {
      analysisPos.clear();
      state.analysis.liveResults.forEach((r, i) => analysisPos.set(r.index, i));
    }
    resetAnalysisView();
    if (run.badLines > 0) {
      toast(`这条任务有 ${run.badLines} 行结果损坏，已跳过`, "info", 5000);
    }
  } catch (e) {
    state.analysis.loading = false;
    toast("读取任务记录失败：" + e, "err");
  }
}

export function resetAnalysisView() {
  state.view.filterCategory = "";
  state.view.filterStatus = "all";
  state.view.filterGroup = "";
  state.view.query = "";
  state.view.queryApplied = "";
  state.view.page = 1;
}

export function analysisTitle() {
  const meta = state.analysis.meta;
  if (!meta) return "";
  return meta.title || meta.planName || "";
}

export function analysisSourceText() {
  const meta = state.analysis.meta;
  if (!meta) return "";
  const plan = meta.planName || "默认方案";
  const ds = meta.datasetName || "默认数据集";
  return `${plan} · ${ds}`;
}

// ---------- 主题 ----------
function systemTheme() {
  return window.matchMedia?.("(prefers-color-scheme: dark)")?.matches ? "dark" : "light";
}

function applySavedTheme(theme) {
  applyTheme(theme === "dark" || theme === "light" ? theme : systemTheme());
}

// ---------- 持久化 ----------
let saveTimer = null;

function snapshot() {
  const plan = activePlan();
  // 没有方案绑定时退回前两条连接，保证 auto-iterate 脚本在只建了连接时也能读到 target/judge
  const target = connectionById(plan?.targetId) || state.connections[0];
  const judge =
    connectionById(plan?.judgeId) || state.connections[1] || state.connections[0];
  return {
    version: 2,
    connections: plain(state.connections),
    plans: plain(state.plans),
    activeConnectionId: state.activeConnectionId,
    activePlanId: state.activePlanId,
    concurrency: Number(state.runForm.concurrency) || 4,
    maxRetries: Number(state.runForm.maxRetries) || 0,
    versionEnabled: !!state.runForm.versionEnabled,
    starredRunIds: plain(state.starredRunIds),
    theme: state.theme,
    // 任务记录保存范围（天），0 = 永久。后端启动时和任务结束后按它清理
    retentionDays: Number(state.retentionDays) || 0,
    sidebarCollapsed: !!state.sidebarCollapsed,
    liveWidthPct: Number(state.liveWidthPct) || 0.6,
    listWidthPx: Number(state.listWidthPx) || 280,
    listCollapsed: !!state.listCollapsed,
    // auto-iterate 脚本继续读这三个旧字段，镜像当前方案的配置
    settings: {
      target: profileOf(target),
      judge: profileOf(judge),
      concurrency: Number(state.runForm.concurrency) || 4,
      maxRetries: Number(state.runForm.maxRetries) || 0,
    },
    prompts: {
      title: plan?.name || "",
      systemPrompt: plan?.systemPrompt || "",
      evalPrompt: plan?.evalPrompt || "",
      injections: plan?.injections || {
        targetSystem: blankSlot(),
        targetUser: blankSlot(),
        judgeSystem: blankSlot(),
        judgeUser: blankSlot(),
      },
    },
  };
}

async function doSave() {
  if (!state.ready) return false;
  state.save.saving = true;
  try {
    await invoke("save_settings", { settings: snapshot() });
    state.save.at = Date.now();
    state.save.error = "";
    return true;
  } catch (e) {
    state.save.error = String(e);
    toast("保存失败：" + e, "err");
    return false;
  } finally {
    state.save.saving = false;
  }
}

export function scheduleSave() {
  if (!state.ready) return;
  clearTimeout(saveTimer);
  saveTimer = setTimeout(doSave, 600);
}

/// 切换对象 / 切页 / 窗口失焦时立即落盘
export async function flushSave() {
  clearTimeout(saveTimer);
  return doSave();
}

export const savedText = computed(() => {
  if (!state.save.at) return "";
  const d = new Date(state.save.at);
  const p = (n) => String(n).padStart(2, "0");
  return `已保存 ${p(d.getHours())}:${p(d.getMinutes())}`;
});

// ---------- 加载与迁移 ----------
function pickProfile(src) {
  if (!src || typeof src !== "object") return null;
  const merged = { ...blankProfile(), ...src };
  return merged;
}

function sanitizeConnections(list) {
  const used = new Set();
  const out = [];
  for (const raw of Array.isArray(list) ? list : []) {
    if (!raw || typeof raw !== "object") continue;
    let id = String(raw.id || "");
    if (!id || used.has(id)) id = uid("conn");
    used.add(id);
    out.push({
      ...blankProfile(),
      ...raw,
      id,
      name: String(raw.name || "未命名连接"),
    });
  }
  return out;
}

/// 数据注入槽位归一化：新格式的 injections 槽位对象，缺字段则补默认。
/// 评测提示词位置由 promptIn 决定（不再有 evalSlot）。
function slotOf(s) {
  if (!s || typeof s !== "object") return blankSlot();
  return {
    enabled: !!s.enabled,
    text: String(s.text || ""),
    promptIn: !!s.promptIn,
    dataIn: !!s.dataIn,
  };
}

/// 迁移：新格式有 injections 直接读（老版本 injections 缺 promptIn/dataIn 时，
/// 用老 evalSlot 回填评测提示词去向，且默认渲染 input/output）；
/// 老格式用 userTemplate + dataSlot 换算成被测槽位。
/// 老 dataSlot=system → 数据进被测 system；dataSlot=user（或没有）→ 数据进被测 user。
function migrateInjections(raw) {
  const inj = raw && typeof raw.injections === "object" ? raw.injections : {};
  // 老 evalSlot：评测提示词本体发到 system 还是 user（老版本恒渲染 input/output）
  const oldEvalInSystem = (raw?.evalSlot || "user") === "system";
  if (inj.targetSystem || inj.targetUser || inj.judgeSystem || inj.judgeUser) {
    const judgeSystem = slotOf(inj.judgeSystem);
    const judgeUser = slotOf(inj.judgeUser);
    // 老版本 injections 没有 promptIn/dataIn（都是 false），用老 evalSlot 回填评测去向
    if (!("promptIn" in inj.judgeSystem) || !("promptIn" in inj.judgeUser)) {
      if (oldEvalInSystem) {
        judgeSystem.promptIn = true;
        judgeSystem.dataIn = true;
      } else {
        judgeUser.promptIn = true;
        judgeUser.dataIn = true;
      }
    }
    const targetSystem = slotOf(inj.targetSystem);
    // 老版本没有 promptIn：主体提示词当时恒在被测 system，迁移后必须保持发在 system，
    // 否则主体提示词会整段丢失。
    if (!("promptIn" in inj.targetSystem)) targetSystem.promptIn = true;
    return {
      targetSystem,
      targetUser: slotOf(inj.targetUser),
      judgeSystem,
      judgeUser,
    };
  }
  const fresh = {
    targetSystem: blankSlot(),
    targetUser: blankSlot(),
    judgeSystem: blankSlot(),
    judgeUser: blankSlot(),
  };
  // 老版本主体提示词恒在被测 system，迁移后保持
  fresh.targetSystem.promptIn = true;
  const tpl = String(raw?.userTemplate || "");
  if (tpl.trim()) {
    const dataSlot = raw?.dataSlot === "system" ? "system" : "user";
    if (dataSlot === "system") {
      fresh.targetSystem = { enabled: true, text: tpl, promptIn: true, dataIn: false };
    } else {
      fresh.targetUser = { enabled: true, text: tpl, promptIn: false, dataIn: false };
    }
  }
  // 老 evalSlot：评测提示词放到对应槽位，恒渲染 input/output
  if (oldEvalInSystem) {
    fresh.judgeSystem.promptIn = true;
    fresh.judgeSystem.dataIn = true;
  } else {
    fresh.judgeUser.promptIn = true;
    fresh.judgeUser.dataIn = true;
  }
  return fresh;
}

/// 规则 A 迁移：勾了「提示词本体导入」的槽位现在只发提示词本体，
/// 数据模板退化成「声明这个槽位渲染哪几列」，本身不再作为消息内容发出去。
/// 老方案那段数据本来是发出去的，这里按需并进提示词本体：
/// 提示词本体一个都没引用到模板里的列时，把模板原样接到末尾（等于规则 A 之前的发送形态）；
/// 已经引用到的就不接，免得同一份数据注入两遍。
function absorbTemplate(prompt, slot) {
  const tpl = String(slot?.text || "").trim();
  if (!slot?.promptIn || !slot.enabled || !tpl) return prompt;
  const inPrompt = referencedVars(prompt);
  const tplCols = [...referencedVars(tpl)].filter((v) => !RESERVED_VARS.has(v));
  if (tplCols.some((v) => inPrompt.has(v))) return prompt;
  return `${String(prompt).replace(/\s+$/, "")}\n\n${tpl}`;
}

function normalizePlan(raw, id) {
  const injections = migrateInjections(raw);
  // 提示词本体总得有个槽位承载：一侧两个槽位都不认领，就放回 system，
  // 否则整段提示词会静默地不再发送
  if (!injections.targetSystem.promptIn && !injections.targetUser.promptIn) {
    injections.targetSystem.promptIn = true;
  }
  if (!injections.judgeSystem.promptIn && !injections.judgeUser.promptIn) {
    injections.judgeSystem.promptIn = true;
  }
  let systemPrompt = String(raw.systemPrompt || "");
  let evalPrompt = String(raw.evalPrompt || "");
  for (const s of [injections.targetSystem, injections.targetUser]) {
    systemPrompt = absorbTemplate(systemPrompt, s);
  }
  for (const s of [injections.judgeSystem, injections.judgeUser]) {
    evalPrompt = absorbTemplate(evalPrompt, s);
  }
  return {
    id,
    name: String(raw.name || "未命名方案"),
    // 老方案没有版号字段，留 null，等任务列表读回来按累计次数补齐
    version: raw.version == null ? null : Math.max(1, Number(raw.version) || 1),
    systemPrompt,
    evalPrompt,
    // 只有新方案带自动列块；迁移来的方案不去猜哪段是自动写的
    autoCols: Array.isArray(raw.autoCols) ? raw.autoCols : [],
    datasetId: String(raw.datasetId || ""),
    targetId: String(raw.targetId || ""),
    judgeId: String(raw.judgeId || ""),
    injections,
  };
}

function sanitizePlans(list) {
  const used = new Set();
  const out = [];
  for (const raw of Array.isArray(list) ? list : []) {
    if (!raw || typeof raw !== "object") continue;
    let id = String(raw.id || "");
    if (!id || used.has(id)) id = uid("plan");
    used.add(id);
    // 数据注入四槽位：新格式读 injections，老格式从 userTemplate + dataSlot 换算，
    // 评测提示词去向（promptIn）从老 evalSlot 回填
    out.push(normalizePlan(raw, id));
  }
  return out;
}

export async function loadSettings() {
  let src = null;
  try {
    src = await invoke("load_settings");
  } catch (e) {
    // 读失败时不能拿默认值覆盖原文件
    state.loadError = String(e);
    state.ready = false;
    toast("读取配置失败：" + e, "err", 8000);
    return;
  }

  try {
    await loadDatasets();
  } catch (e) {
    console.warn("读取数据集索引失败", e);
    state.datasets = [];
  }

  const conf = src || {};
  const legacySettings = conf.settings || {};
  const legacyPrompts = conf.prompts || {};
  const hasLegacy =
    conf.version !== 2 &&
    (legacySettings.target || legacySettings.judge || legacyPrompts.systemPrompt ||
      legacyPrompts.evalPrompt || legacyPrompts.userTemplate);

  // 连接库
  if (Array.isArray(conf.connections) && conf.connections.length) {
    state.connections = sanitizeConnections(conf.connections);
  } else if (hasLegacy) {
    const targetSrc = pickProfile(legacySettings.target);
    const judgeSrc = pickProfile(legacySettings.judge);
    const conns = [];
    if (targetSrc) {
      conns.push({
        ...targetSrc,
        id: "conn-target",
        name: targetSrc.model?.trim() || "被测连接",
      });
    }
    if (judgeSrc) {
      conns.push({
        ...judgeSrc,
        id: "conn-judge",
        name: judgeSrc.model?.trim() || "评测连接",
      });
    }
    state.connections = sanitizeConnections(conns);
  } else {
    state.connections = [];
  }
  state.activeConnectionId =
    state.connections.some((c) => c.id === conf.activeConnectionId)
      ? conf.activeConnectionId
      : state.connections[0]?.id || "";

  // 数据集：后端已把旧 dataset.json 迁成 ds-default
  if (state.datasets.length) {
    state.activeDatasetId = state.datasets.some((d) => d.id === conf.activeDatasetId)
      ? conf.activeDatasetId
      : state.datasets[0].id;
  } else {
    state.activeDatasetId = "";
  }

  // 方案库
  if (Array.isArray(conf.plans) && conf.plans.length) {
    state.plans = sanitizePlans(conf.plans);
  } else if (hasLegacy) {
    const firstDs = state.datasets[0];
    const cols = firstDs?.columns || [];
    let userTemplate = String(legacyPrompts.userTemplate || "");
    if (!userTemplate.trim() && cols.length) {
      userTemplate = buildUserTemplate(cols);
    }
    const targetId = state.connections.some((c) => c.id === "conn-target")
      ? "conn-target"
      : state.connections[0]?.id || "";
    const judgeId = state.connections.some((c) => c.id === "conn-judge")
      ? "conn-judge"
      : state.connections[0]?.id || "";
    state.plans = [
      // 旧版单方案配置迁移后保持原发送形态：评测提示词走 user，数据走被测 user
      normalizePlan(
        {
          name: String(legacyPrompts.title || "").trim() || "默认方案",
          systemPrompt: String(legacyPrompts.systemPrompt || DEFAULT_SYSTEM_PROMPT),
          evalPrompt: String(legacyPrompts.evalPrompt || DEFAULT_EVAL_PROMPT),
          datasetId: firstDs?.id || "",
          targetId,
          judgeId,
          userTemplate,
          evalSlot: "user",
        },
        uid("plan")
      ),
    ];
  } else {
    state.plans = [];
  }
  state.activePlanId = state.plans.some((p) => p.id === conf.activePlanId)
    ? conf.activePlanId
    : state.plans[0]?.id || "";

  state.runForm.concurrency = Number(conf.concurrency ?? legacySettings.concurrency ?? 4) || 4;
  state.runForm.maxRetries = Number(conf.maxRetries ?? legacySettings.maxRetries ?? 2) || 0;
  state.runForm.versionEnabled = !!conf.versionEnabled;
  state.starredRunIds = Array.isArray(conf.starredRunIds) ? conf.starredRunIds.map(String) : [];

  applySavedTheme(conf.theme);
  if (!conf.theme) applyTheme(systemTheme());
  state.retentionDays = Number(conf.retentionDays) || 0;
  state.sidebarCollapsed = !!conf.sidebarCollapsed;
  state.liveWidthPct = Number(conf.liveWidthPct) || 0.6;
  state.listWidthPx = Number(conf.listWidthPx) || 280;
  state.listCollapsed = !!conf.listCollapsed;

  if (state.activeDatasetId) {
    await loadDatasetPreview(state.activeDatasetId, 0, 50);
  }

  state.ready = true;

  // 任务列表与启动时中断恢复
  await loadDataDir();
  await loadExportDir();
  await loadJobs();
  initPlanVersions();

  const plan = activePlan();
  if (plan?.datasetId) await syncDatasetMirror(plan.datasetId);

  // 迁移只发生一次：有旧数据时立刻写回 v2
  if (!src || conf.version !== 2) {
    await doSave();
  }
}

// ---------- 统计口径 ----------
function textOf(r) {
  return [r.targetError, r.details, ...(r.reasons || [])]
    .filter(Boolean)
    .join(" ");
}

export function isExcluded(r) {
  if (r.targetError) return true;
  if (r.parseOk === false) return true;
  return (r.errorTypes || []).some((t) => EXCLUDED_CATEGORIES.includes(t));
}

export function isExcludedCategory(name) {
  if (!name) return false;
  if (EXCLUDED_CATEGORIES.includes(name) || name === TIMEOUT_LABEL) return true;
  return name.startsWith("模型超时");
}

function excludedKindOf(r) {
  const timeout = TIMEOUT_RE.test(textOf(r));
  if (r.targetError) return timeout ? TIMEOUT_LABEL : TARGET_FAIL;
  if ((r.errorTypes || []).includes(JUDGE_FAIL)) return timeout ? TIMEOUT_LABEL : JUDGE_FAIL;
  return PARSE_FAIL;
}

/// 统计口径只在这里实现一次，运行页和分析页共用
export function computeStats(list) {
  const total = list.length;
  let passed = 0;
  let excluded = 0;
  const counts = {};
  const exCounts = {};

  for (const r of list) {
    if (isExcluded(r)) {
      excluded++;
      const k = excludedKindOf(r);
      exCounts[k] = (exCounts[k] || 0) + 1;
      continue;
    }
    if (r.pass === true) {
      passed++;
      continue;
    }
    const types = (r.errorTypes || []).filter((t) => t && t.trim());
    const keys = types.length ? types : [UNCLASSIFIED];
    for (const t of keys) counts[t] = (counts[t] || 0) + 1;
  }

  const valid = total - excluded;
  const failed = valid - passed;

  const toItems = (src, base, builtinList) =>
    Object.entries(src)
      .map(([name, count]) => ({
        name,
        count,
        ratio: base ? count / base : 0,
        ratioOfTotal: total ? count / total : 0,
        builtin: builtinList ? builtinList.includes(name) : false,
      }))
      .sort((a, b) => b.count - a.count || a.name.localeCompare(b.name));

  const byType = toItems(counts, failed, BUILTIN_CATEGORIES);
  const excludedTypes = toItems(exCounts, excluded);

  return {
    total,
    valid,
    passed,
    failed,
    excluded,
    passRate: valid ? passed / valid : 0,
    byType,
    excludedTypes,
    categories: [...byType, ...excludedTypes].map((x) => x.name),
  };
}

export const viewResults = computed(() =>
  state.analysis.live ? state.analysis.liveResults : state.analysis.results
);
export const viewStats = computed(() => computeStats(viewResults.value));
export const isHistoryView = computed(() => !!state.analysis.runId);

export function bucketsOf(r) {
  if (isExcluded(r)) return [excludedKindOf(r)];
  if (r.pass === true) return [];
  const types = (r.errorTypes || []).filter((t) => t && t.trim());
  return types.length ? types : [UNCLASSIFIED];
}

export const groupOptions = computed(() => {
  const list = [];
  const seen = new Set();
  for (const r of viewResults.value) {
    const g = r.statusLabel || "未分组";
    if (!seen.has(g)) {
      seen.add(g);
      list.push(g);
    }
  }
  return list;
});

function hayOf(r) {
  if (typeof r.__hay === "string") return r.__hay;
  const hay = `${r.input}\n${r.output}\n${(r.reasons || []).join(" ")}\n${r.details}\n${r.statusLabel}`.toLowerCase();
  try {
    r.__hay = hay;
  } catch {
    /* 忽略 */
  }
  return hay;
}

export const filteredResults = computed(() => {
  const q = state.view.queryApplied.trim().toLowerCase();
  return viewResults.value.filter((r) => {
    const ex = isExcluded(r);
    if (state.view.filterStatus === "pass" && (ex || r.pass !== true)) return false;
    if (state.view.filterStatus === "fail" && (ex || r.pass === true)) return false;
    if (state.view.filterStatus === "excluded" && !ex) return false;
    if (state.view.filterGroup && (r.statusLabel || "未分组") !== state.view.filterGroup) {
      return false;
    }
    if (state.view.filterCategory && !bucketsOf(r).includes(state.view.filterCategory)) {
      return false;
    }
    if (q && !hayOf(r).includes(q)) return false;
    return true;
  });
});

export const pageCount = computed(() =>
  Math.max(1, Math.ceil(filteredResults.value.length / Math.max(1, state.view.pageSize)))
);

export const pagedResults = computed(() => {
  const size = Math.max(1, state.view.pageSize);
  const page = Math.min(Math.max(1, state.view.page), pageCount.value);
  const start = (page - 1) * size;
  return filteredResults.value.slice(start, start + size);
});

watch(
  () => [
    state.view.filterStatus,
    state.view.filterCategory,
    state.view.filterGroup,
    state.view.queryApplied,
    state.view.pageSize,
  ],
  () => {
    state.view.page = 1;
  }
);

watch(pageCount, (n) => {
  if (state.view.page > n) state.view.page = n;
});

let queryTimer = null;
watch(
  () => state.view.query,
  (q) => {
    clearTimeout(queryTimer);
    queryTimer = setTimeout(() => {
      state.view.queryApplied = q;
    }, 200);
  }
);

// ---------- 自动保存 ----------
watch(
  () => [
    state.connections,
    state.plans,
    state.activeConnectionId,
    state.activePlanId,
    state.theme,
    state.runForm.concurrency,
    state.runForm.maxRetries,
    state.runForm.versionEnabled,
    state.retentionDays,
  ],
  () => scheduleSave(),
  { deep: true }
);

// 当前方案切换时，把它的数据集镜像给 auto-iterate，并立即落盘
watch(
  () => state.activePlanId,
  async () => {
    const plan = activePlan();
    if (plan?.datasetId) await syncDatasetMirror(plan.datasetId);
    flushSave();
  }
);
