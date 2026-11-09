import { reactive, computed, watch, markRaw } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  DEFAULT_EVAL_PROMPT,
  DEFAULT_SYSTEM_PROMPT,
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

export function blankPlan(name, datasetId = "", columns = []) {
  return {
    id: uid("plan"),
    name,
    systemPrompt: DEFAULT_SYSTEM_PROMPT,
    userTemplate: defaultUserTemplate(columns),
    evalPrompt: DEFAULT_EVAL_PROMPT,
    datasetId,
    targetId: "",
    judgeId: "",
  };
}

export const state = reactive({
  version: 2,
  ready: false,
  loadError: "",
  page: "run",
  theme: "light",
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
    titlePrefix: "",
    concurrency: 4,
    maxRetries: 2,
    launchOpen: false,
  },

  analysis: {
    runId: "",
    meta: null,
    results: [],
    badLines: 0,
    dataset: null,
    loading: false,
  },

  view: {
    filterCategory: "",
    filterStatus: "all",
    filterGroup: "",
    query: "",
    queryApplied: "",
    page: 1,
    pageSize: 50,
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
  const used = new Set([
    ...referencedVars(plan?.systemPrompt),
    ...referencedVars(plan?.userTemplate),
  ]);
  const colSet = new Set(cols);
  const missing = cols.filter((c) => !used.has(c));
  const unknown = [...used].filter((v) => !RESERVED_VARS.has(v) && !colSet.has(v));
  return { missing, unknown, ok: !missing.length && !unknown.length };
});

export function buildUserTemplate(columns) {
  if (!columns?.length) return "";
  return columns.map((c) => `{{${c}}}`).join("\n\n");
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

export function jobPassRate(job) {
  if (job.runId === state.jobs.live?.runId) {
    const live = liveStats.value;
    if (live.total) return live.passRate;
  }
  const s = job.summary;
  if (!s?.valid) return null;
  return s.passed / s.valid;
}

export async function loadJobs() {
  try {
    const list = await invoke("list_runs");
    const arr = Array.isArray(list) ? list : [];
    // 保留正在启动、后端还没写入目录的占位任务
    const runningPlaceholders = state.jobs.list.filter(
      (j) => j.pending && !arr.some((x) => x.runId === j.runId)
    );
    state.jobs.list = [...runningPlaceholders, ...arr];
  } catch (e) {
    console.warn("读取任务列表失败", e);
  }
}

function dateLabel() {
  const d = new Date();
  const p = (n) => String(n).padStart(2, "0");
  return `${d.getMonth() + 1}月${p(d.getDate())}日 ${p(d.getHours())}:${p(d.getMinutes())}`;
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
  const prefix = state.runForm.titlePrefix.trim();
  const now = Date.now();

  let started = 0;
  for (const [i, plan] of plans.entries()) {
    const runId = `run-${now}-${i}-${Math.random().toString(36).slice(2, 6)}`;
    const title = prefix
      ? `${prefix} · ${plan.name}`
      : `${plan.name} · ${dateLabel()}`;
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
        userTemplate: plan.userTemplate,
        evalPrompt: plan.evalPrompt,
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

  state.runForm.selectedPlanIds = [];
  state.runForm.launchOpen = false;
  return started;
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
      userTemplate: plan.userTemplate,
      evalPrompt: plan.evalPrompt,
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
  } else {
    toast("没有找到正在运行的任务，可能已经结束了", "info");
  }
  return found;
}

export async function deleteJob(job) {
  await invoke("delete_run", { runId: job.runId });
  if (state.jobs.live?.runId === job.runId) state.jobs.live = null;
  if (state.analysis.runId === job.runId) state.analysis.runId = "";
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

export async function retryJob(job) {
  const loaded = await invoke("load_run", { runId: job.runId });
  if (!loaded) throw new Error("任务结果文件不存在");
  const indices = (loaded.results || [])
    .filter((r) => r.targetError || !r.parseOk || !r.pass)
    .map((r) => r.index);
  if (!indices.length) {
    toast("没有需要重跑的行", "info");
    return false;
  }
  const cfg = await buildCfgFromJob(job, "retry", indices);
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
  state.jobs.progress[job.runId] = {
    done: state.jobs.live?.runId === job.runId ? state.jobs.live.results.length : 0,
    total: job.total,
  };
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

function mergeLiveProgress(payload) {
  const live = state.jobs.live;
  if (!live || live.runId !== payload.runId) return;
  if (typeof payload.done === "number") live.done = payload.done;
  if (typeof payload.total === "number") live.total = payload.total;
  for (const raw of payload.results || []) {
    const r = normalizeResult(raw);
    liveReceived.add(r.index);
    const pos = livePos.get(r.index);
    if (pos === undefined) {
      livePos.set(r.index, live.results.length);
      live.results.push(r);
    } else {
      live.results[pos] = r;
    }
  }
  live.results.sort((a, b) => a.index - b.index);
  reindexLive();
}

let runListener = null;
export async function attachRunListener() {
  if (runListener) return;
  runListener = await listen("run://progress", (ev) => {
    const p = ev.payload || {};
    if (p.runId) {
      state.jobs.progress[p.runId] = { done: p.done, total: p.total };
      mergeLiveProgress(p);
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
    state.jobs.live.title = run.meta.title || state.jobs.live.title;
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
export async function selectAnalysisRun(runId) {
  if (!runId) {
    state.analysis = {
      runId: "",
      meta: null,
      results: [],
      badLines: 0,
      dataset: null,
      loading: false,
    };
    resetAnalysisView();
    return;
  }
  if (state.analysis.runId === runId) return;
  state.analysis.loading = true;
  try {
    const run = await invoke("load_run", { runId });
    if (!run) {
      toast("这条任务记录已经不在了", "info");
      state.analysis.runId = "";
      state.analysis.meta = null;
      state.analysis.results = [];
      return;
    }
    state.analysis = {
      runId,
      meta: run.meta,
      results: (run.results || [])
        .map(normalizeResult)
        .sort((a, b) => a.index - b.index),
      badLines: run.badLines || 0,
      dataset: run.dataset || null,
      loading: false,
    };
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
    theme: state.theme,
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
      userTemplate: plan?.userTemplate || "",
      evalPrompt: plan?.evalPrompt || "",
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

function sanitizePlans(list) {
  const used = new Set();
  const out = [];
  for (const raw of Array.isArray(list) ? list : []) {
    if (!raw || typeof raw !== "object") continue;
    let id = String(raw.id || "");
    if (!id || used.has(id)) id = uid("plan");
    used.add(id);
    out.push({
      id,
      name: String(raw.name || "未命名方案"),
      systemPrompt: String(raw.systemPrompt || ""),
      userTemplate: String(raw.userTemplate || ""),
      evalPrompt: String(raw.evalPrompt || ""),
      datasetId: String(raw.datasetId || ""),
      targetId: String(raw.targetId || ""),
      judgeId: String(raw.judgeId || ""),
    });
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
      {
        id: uid("plan"),
        name: String(legacyPrompts.title || "").trim() || "默认方案",
        systemPrompt: String(legacyPrompts.systemPrompt || DEFAULT_SYSTEM_PROMPT),
        userTemplate,
        evalPrompt: String(legacyPrompts.evalPrompt || DEFAULT_EVAL_PROMPT),
        datasetId: firstDs?.id || "",
        targetId,
        judgeId,
      },
    ];
  } else {
    state.plans = [];
  }
  state.activePlanId = state.plans.some((p) => p.id === conf.activePlanId)
    ? conf.activePlanId
    : state.plans[0]?.id || "";

  state.runForm.concurrency = Number(conf.concurrency ?? legacySettings.concurrency ?? 4) || 4;
  state.runForm.maxRetries = Number(conf.maxRetries ?? legacySettings.maxRetries ?? 2) || 0;

  applySavedTheme(conf.theme);
  if (!conf.theme) applyTheme(systemTheme());

  if (state.activeDatasetId) {
    await loadDatasetPreview(state.activeDatasetId, 0, 50);
  }

  state.ready = true;

  // 任务列表与启动时中断恢复
  await loadJobs();

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

export const viewResults = computed(() => state.analysis.results);
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
