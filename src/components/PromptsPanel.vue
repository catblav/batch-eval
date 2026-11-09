<script setup>
import { ref, computed, watch, nextTick, onMounted, onBeforeUnmount } from "vue";
import {
  state,
  toast,
  activePlan,
  planProblems,
  templateVarCheck,
  buildUserTemplate,
  datasetMetaById,
  datasetColumnsOf,
  connectionById,
  createPlan,
  duplicatePlan,
  deletePlan,
  usePlanForRun,
  usePage,
  savedText,
  statusLabelOf,
} from "../store";
import {
  DEFAULT_SYSTEM_PROMPT,
  DEFAULT_EVAL_PROMPT,
  DEFAULT_NOTES,
  defaultUserTemplate,
} from "../defaultPrompts";
import { invoke } from "@tauri-apps/api/core";
import { open, save, ask } from "@tauri-apps/plugin-dialog";
import Icon from "./Icon.vue";

const search = ref("");
const tab = ref("system");
const menuId = ref("");
const helpOpen = ref(true);
const defaultDialog = ref(null);
const nameInput = ref(null);
const userRef = ref(null);
const previewIndex = ref(1);
const sampleRow = ref(null);
const sampleLoading = ref(false);

const TABS = [
  { key: "system", name: "主体提示词", field: "systemPrompt" },
  { key: "user", name: "用户消息", field: "userTemplate" },
  { key: "eval", name: "评测提示词", field: "evalPrompt" },
  { key: "preview", name: "预览" },
];

const active = computed(() => activePlan());
const dataset = computed(() => datasetMetaById(active.value?.datasetId));
const columns = computed(() => datasetColumnsOf(active.value?.datasetId));
const problems = computed(() => planProblems(active.value));
const totalRows = computed(() => dataset.value?.rows || 0);
const filtered = computed(() => {
  const q = search.value.trim().toLowerCase();
  if (!q) return state.plans;
  return state.plans.filter((p) => p.name.toLowerCase().includes(q));
});
const evalHasVars = computed(
  () =>
    /\{\{\s*input\s*\}\}/.test(active.value?.evalPrompt || "") &&
    /\{\{\s*output\s*\}\}/.test(active.value?.evalPrompt || "")
);
const hasOutputContract = computed(() => /"pass"|pass\s*[:：]/.test(active.value?.evalPrompt || ""));
const judgeConn = computed(() => connectionById(active.value?.judgeId));
const judgeWarn = computed(
  () => !!judgeConn.value && Number(judgeConn.value.temperature) !== 0
);

watch(
  () => state.activePlanId,
  () => {
    menuId.value = "";
    previewIndex.value = 1;
    sampleRow.value = null;
  }
);

watch(
  () => active.value?.datasetId,
  () => {
    previewIndex.value = 1;
    sampleRow.value = null;
  }
);

const wrapVar = (c) => `{{${c}}}`;
const input = "{{input}}";
const output = "{{output}}";
const columnVar = "{{列名}}";

// 与 Rust runner::render 顺序一致：先列名，再 input / output
function renderTpl(tpl, row, input, output) {
  let out = String(tpl || "");
  for (const [k, v] of Object.entries(row || {})) {
    const val = typeof v === "string" ? v : JSON.stringify(v);
    out = out.split(`{{${k}}}`).join(val);
  }
  out = out.split("{{input}}").join(input).split("{{ input }}").join(input);
  out = out.split("{{output}}").join(output).split("{{ output }}").join(output);
  return out;
}

const previewSystem = computed(() => renderTpl(active.value?.systemPrompt, sampleRow.value, "", ""));
const previewUser = computed(() => renderTpl(active.value?.userTemplate, sampleRow.value, "", ""));
const previewJudgeInput = computed(() => {
  const sys = previewSystem.value.trim();
  return sys
    ? `【被评估模型的提示词】\n${sys}\n\n【本条输入】\n${previewUser.value}`
    : previewUser.value;
});
const previewJudge = computed(() =>
  renderTpl(
    active.value?.evalPrompt,
    sampleRow.value,
    previewJudgeInput.value,
    "（被测模型的实际输出会出现在这里）"
  )
);
const sampleLabel = computed(() =>
  sampleRow.value ? statusLabelOf(sampleRow.value) : ""
);

async function loadSample() {
  if (!active.value?.datasetId || !totalRows.value) {
    sampleRow.value = null;
    return;
  }
  const idx = Math.min(Math.max(1, previewIndex.value), totalRows.value) - 1;
  previewIndex.value = idx + 1;
  sampleLoading.value = true;
  try {
    sampleRow.value = await invoke("get_dataset_row", {
      id: active.value.datasetId,
      index: idx,
    });
  } catch (e) {
    sampleRow.value = null;
    toast("读取数据行失败：" + e, "err");
  } finally {
    sampleLoading.value = false;
  }
}

watch(
  () => [tab.value, active.value?.id, active.value?.datasetId, previewIndex.value],
  () => {
    if (tab.value === "preview") loadSample();
  }
);

async function create() {
  createPlan();
  await nextTick();
  nameInput.value?.focus();
  nameInput.value?.select();
}

function usedByRunning(id) {
  return state.jobs.list.some((j) => j.status === "running" && j.planId === id);
}

async function duplicate() {
  menuId.value = "";
  if (!active.value) return;
  duplicatePlan(active.value.id);
  toast("已复制方案", "ok");
}

async function remove() {
  menuId.value = "";
  if (!active.value) return;
  if (usedByRunning(active.value.id)) {
    toast("有任务正在用这个方案运行，先取消或等它结束", "err");
    return;
  }
  const ok = await ask(`删除方案「${active.value.name}」？\n\n它的提示词和绑定会一起删除，不影响已完成的任务。`, {
    title: "删除方案",
    kind: "warning",
    okLabel: "删除",
    cancelLabel: "取消",
  });
  if (!ok) return;
  const id = active.value.id;
  state.runForm.selectedPlanIds = state.runForm.selectedPlanIds.filter((x) => x !== id);
  deletePlan(id);
  toast("已删除方案", "ok");
}

function insertVar(col) {
  if (!active.value) return;
  const token = wrapVar(col);
  const el = userRef.value;
  if (el && document.activeElement === el) {
    const start = el.selectionStart ?? active.value.userTemplate.length;
    const end = el.selectionEnd ?? start;
    active.value.userTemplate =
      active.value.userTemplate.slice(0, start) + token + active.value.userTemplate.slice(end);
    nextTick(() => {
      el.focus();
      const pos = start + token.length;
      el.setSelectionRange(pos, pos);
    });
  } else {
    const base = active.value.userTemplate.trimEnd();
    active.value.userTemplate = base ? `${base}\n\n${token}` : token;
  }
}

function fillTemplate() {
  if (!active.value) return;
  if (!columns.value.length) {
    toast("先给方案绑定一个有列的数据集", "info");
    return;
  }
  active.value.userTemplate = buildUserTemplate(columns.value);
  toast("已按数据集列生成用户消息", "ok");
}

function syncVars() {
  if (!active.value) return;
  const missing = templateVarCheck.value.missing;
  if (!missing.length) return;
  const tokens = missing.map((c) => wrapVar(c)).join("\n\n");
  const base = active.value.userTemplate.trimEnd();
  active.value.userTemplate = base ? `${base}\n\n${tokens}` : tokens;
  toast(`已补入 ${missing.length} 个未引用的列`, "ok");
}

function baseNameOf(path) {
  const name = String(path || "").split(/[\\/]/).pop() || "";
  const dot = name.lastIndexOf(".");
  return (dot > 0 ? name.slice(0, dot) : name).trim().slice(0, 60);
}

async function importPrompt(which) {
  try {
    const path = await open({
      multiple: false,
      filters: [{ name: "文本", extensions: ["md", "txt"] }],
    });
    if (!path) return;
    const text = await invoke("read_text_file", { path });
    active.value[which] = text;
    toast(`已导入${which === "systemPrompt" ? "主体提示词" : which === "evalPrompt" ? "评测提示词" : "用户消息"}`);
  } catch (e) {
    toast("导入失败：" + e, "err");
  }
}

async function exportPrompt(which) {
  try {
    const label =
      which === "systemPrompt" ? "主体提示词" : which === "evalPrompt" ? "评测提示词" : "用户消息";
    const path = await save({
      defaultPath: `${active.value.name}_${label}.md`,
      filters: [{ name: "Markdown", extensions: ["md"] }],
    });
    if (!path) return;
    await invoke("save_text_file", { path, content: active.value[which] });
    toast("已导出", "ok");
  } catch (e) {
    toast("导出失败：" + e, "err");
  }
}

const TITLES = {
  systemPrompt: "主体提示词",
  userTemplate: "用户消息模板",
  evalPrompt: "评测提示词",
};

function defaultTextOf(which) {
  if (which === "systemPrompt") return DEFAULT_SYSTEM_PROMPT;
  if (which === "evalPrompt") return DEFAULT_EVAL_PROMPT;
  return defaultUserTemplate(columns.value);
}

function openDefault(which) {
  defaultDialog.value = {
    which,
    title: TITLES[which],
    note: DEFAULT_NOTES[which],
    text: defaultTextOf(which),
  };
}

function applyDefault() {
  const d = defaultDialog.value;
  if (!d || !active.value) return;
  active.value[d.which] = d.text;
  defaultDialog.value = null;
  toast(`已填入默认${d.title}`, "ok");
}

function copyDefault() {
  navigator.clipboard.writeText(defaultDialog.value?.text || "");
  toast("已复制默认模板", "ok");
}

function runThis() {
  if (!active.value || problems.value.length) return;
  usePlanForRun(active.value.id);
}

function closeMenu() {
  menuId.value = "";
}

onMounted(() => window.addEventListener("click", closeMenu));
onBeforeUnmount(() => window.removeEventListener("click", closeMenu));
</script>

<template>
  <div class="h-full flex bg-page">
    <aside class="w-[280px] shrink-0 flex flex-col border-r border-line min-h-0">
      <div class="h-14 px-4 flex items-center gap-2 shrink-0">
        <h1 class="text-[15px] font-semibold">提示词配置</h1>
        <span class="text-[12px] text-weak tabular-nums">{{ state.plans.length }}</span>
        <span class="flex-1" />
        <button class="icon-btn" title="新建方案" @click="create">
          <Icon name="plus" :size="16" />
        </button>
      </div>
      <div class="px-3 pb-2 shrink-0">
        <div class="relative">
          <Icon name="search" :size="14"
            class="absolute left-2 top-1/2 -translate-y-1/2 text-weak pointer-events-none" />
          <input v-model="search" class="field pl-7 h-8" placeholder="搜索方案" />
        </div>
      </div>
      <div class="flex-1 min-h-0 overflow-y-auto px-2 pb-3 space-y-0.5">
        <div v-if="!state.plans.length" class="px-3 py-10 text-center">
          <p class="text-[13px] text-sub">还没有方案</p>
          <p class="hint mt-1.5">方案 = 提示词 + 数据集 + 一对连接。</p>
          <button class="btn-primary btn-sm mt-3" @click="create">新建方案</button>
        </div>
        <p v-else-if="!filtered.length" class="px-3 py-8 text-center hint">没有匹配的方案</p>

        <button
          v-for="p in filtered"
          :key="p.id"
          class="list-item w-full"
          :class="{ 'list-item-active': p.id === state.activePlanId }"
          @click="state.activePlanId = p.id"
        >
          <div class="flex items-center gap-1.5">
            <span class="text-[13px] font-medium truncate">{{ p.name }}</span>
            <span
              v-if="planProblems(p).length"
              class="dot bg-warn shrink-0"
              :title="planProblems(p).join('、')"
            />
          </div>
          <div class="mt-0.5 text-[11.5px] text-weak truncate">
            {{ datasetMetaById(p.datasetId)?.name || "未选数据集" }}
          </div>
          <div class="mt-0.5 text-[11.5px] text-weak truncate">
            {{ connectionById(p.targetId)?.model || "未选被测" }}
            →
            {{ connectionById(p.judgeId)?.model || "未选评测" }}
          </div>
        </button>
      </div>
    </aside>

    <section class="flex-1 min-w-0 flex flex-col bg-panel">
      <template v-if="active">
        <div class="h-14 px-5 flex items-center gap-2.5 border-b border-line shrink-0">
          <input
            ref="nameInput"
            v-model="active.name"
            class="min-w-0 w-[260px] h-8 px-1.5 rounded border border-transparent bg-transparent
              text-[15px] font-semibold text-main hover:border-line focus:border-accent transition-colors"
            placeholder="方案名称"
          />
          <span class="text-[11.5px] text-weak shrink-0">{{ savedText }}</span>
          <span class="flex-1" />
          <button class="btn-primary btn-sm" :disabled="problems.length > 0"
            :title="problems.length ? problems.join('、') : '去运行页启动这个方案'" @click="runThis">
            <Icon name="play" :size="13" />
            运行此方案
          </button>
          <div class="relative">
            <button class="icon-btn" title="更多操作" @click.stop="menuId = menuId ? '' : 'main'">
              <Icon name="more" :size="16" />
            </button>
            <div
              v-if="menuId"
              class="absolute right-0 top-8 z-20 w-28 py-1 rounded border border-line bg-panel shadow-sm"
              @click.stop
            >
              <button class="w-full text-left px-3 h-7 text-[12.5px] hover:bg-wash" @click="duplicate">
                复制方案
              </button>
              <button class="w-full text-left px-3 h-7 text-[12.5px] text-fail hover:bg-fail/10" @click="remove">
                删除方案
              </button>
            </div>
          </div>
        </div>

        <div class="shrink-0 px-5 py-3 border-b border-line flex items-center gap-4 flex-wrap">
          <div class="flex items-center gap-2">
            <span class="text-[12px] text-sub">数据集</span>
            <select v-model="active.datasetId" class="field w-[190px]">
              <option value="">未选择</option>
              <option v-for="d in state.datasets" :key="d.id" :value="d.id">
                {{ d.name }} · {{ d.rows }} 行
              </option>
            </select>
          </div>
          <div class="flex items-center gap-2">
            <span class="text-[12px] text-sub">被测</span>
            <select v-model="active.targetId" class="field w-[180px]">
              <option value="">未选择</option>
              <option v-for="c in state.connections" :key="c.id" :value="c.id">
                {{ c.name }} · {{ c.model || "未填模型" }}
              </option>
            </select>
          </div>
          <div class="flex items-center gap-2">
            <span class="text-[12px] text-sub">评测</span>
            <select v-model="active.judgeId" class="field w-[180px]">
              <option value="">未选择</option>
              <option v-for="c in state.connections" :key="c.id" :value="c.id">
                {{ c.name }} · {{ c.model || "未填模型" }}
              </option>
            </select>
            <button class="btn-ghost btn-sm" :disabled="!active.targetId" title="评测连接与被测相同"
              @click="active.judgeId = active.targetId">同被测</button>
            <span v-if="judgeWarn" class="text-[11.5px] text-warn whitespace-nowrap">建议 temperature 0</span>
          </div>
        </div>

        <div v-if="problems.length"
          class="shrink-0 px-5 h-8 flex items-center gap-1.5 border-b border-line bg-warn/5 text-[12px] text-warn">
          <Icon name="alert" :size="13" />
          <span class="truncate">还不能运行：{{ problems.join("、") }}</span>
          <span class="flex-1" />
          <button class="btn-ghost btn-sm text-warn !h-6" @click="usePage('connections')">去检查连接</button>
        </div>

        <div class="h-9 px-3 flex items-center border-b border-line shrink-0">
          <button
            v-for="t in TABS"
            :key="t.key"
            class="editor-tab"
            :class="{ 'editor-tab-active': tab === t.key }"
            @click="tab = t.key"
          >
            {{ t.name }}
            <span v-if="t.key === 'user' && columns.length" class="ml-1 text-[11px] text-weak tabular-nums">
              {{ templateVarCheck.missing.length ? templateVarCheck.missing.length + " 列未用" : "变量齐全" }}
            </span>
          </button>
          <span class="flex-1" />
          <span v-if="tab !== 'preview'" class="text-[11.5px] text-weak tabular-nums">
            {{ (active[TABS.find((x) => x.key === tab)?.field] || "").length.toLocaleString() }} 字
          </span>
        </div>

        <div v-if="tab === 'system'" class="flex-1 min-h-0 flex flex-col">
          <textarea
            v-model="active.systemPrompt"
            class="flex-1 min-h-0 w-full resize-none border-0 rounded-none focus:ring-0
              font-mono text-[12.5px] leading-relaxed bg-panel text-main p-4"
            placeholder="被测模型的 system prompt…"
          />
          <div class="h-10 shrink-0 px-4 flex items-center gap-2 border-t border-line">
            <span class="flex-1" />
            <button class="btn-ghost btn-sm" @click="openDefault('systemPrompt')">恢复默认</button>
            <button class="btn-ghost btn-sm" @click="importPrompt('systemPrompt')">导入</button>
            <button class="btn-ghost btn-sm" @click="exportPrompt('systemPrompt')">导出</button>
          </div>
        </div>

        <div v-else-if="tab === 'user'" class="flex-1 min-h-0 flex flex-col">
          <div class="shrink-0 px-4 py-2 border-b border-line">
            <div class="flex items-center gap-1.5 flex-wrap">
              <span class="text-[11.5px] text-weak mr-0.5">点击插入</span>
              <button
                v-for="c in columns"
                :key="c"
                class="chip font-mono hover:border-accent/50 hover:text-main"
                :class="templateVarCheck.missing.includes(c) ? 'border-warn/40 text-warn bg-warn/5' : ''"
                :title="templateVarCheck.missing.includes(c) ? '这个列还没有被引用' : '已引用'"
                @click="insertVar(c)"
              >
                {{ wrapVar(c) }}
              </button>
              <span v-if="!columns.length" class="hint">选择数据集后显示可用的列</span>
            </div>
            <div v-if="templateVarCheck.unknown.length" class="mt-1.5 text-[11.5px] text-warn">
              数据集里没有：
              <code v-for="v in templateVarCheck.unknown" :key="v" class="font-mono mr-1">{{ wrapVar(v) }}</code>
              运行时会原样发给模型
            </div>
            <div v-else-if="templateVarCheck.missing.length" class="mt-1.5 flex items-center gap-2 text-[11.5px] text-weak">
              <span>{{ templateVarCheck.missing.length }} 列没有被引用，这些数据不会进入请求</span>
              <button class="text-accent hover:underline" @click="syncVars">补入模板</button>
            </div>
          </div>
          <textarea
            ref="userRef"
            v-model="active.userTemplate"
            class="flex-1 min-h-0 w-full resize-none border-0 rounded-none focus:ring-0
              font-mono text-[12.5px] leading-relaxed bg-panel text-main p-4"
            placeholder="用户消息模板，用双花括号加列名注入数据…"
          />
          <div class="h-10 shrink-0 px-4 flex items-center gap-2 border-t border-line">
            <span class="flex-1" />
            <button class="btn-ghost btn-sm" @click="fillTemplate">按数据集列生成</button>
            <button class="btn-ghost btn-sm" @click="openDefault('userTemplate')">恢复默认</button>
            <button class="btn-ghost btn-sm" @click="importPrompt('userTemplate')">导入</button>
            <button class="btn-ghost btn-sm" @click="exportPrompt('userTemplate')">导出</button>
          </div>
        </div>

        <div v-else-if="tab === 'eval'" class="flex-1 min-h-0 flex flex-col">
          <div class="shrink-0 h-9 px-4 flex items-center gap-2 border-b border-line">
            <span class="chip" :class="evalHasVars ? 'border-pass/40 text-pass bg-pass/5' : 'border-warn/40 text-warn bg-warn/5'">
              {{ evalHasVars ? "变量齐全" : "缺少 input / output" }}
            </span>
            <span class="chip" :class="hasOutputContract ? 'border-pass/40 text-pass bg-pass/5' : 'border-warn/40 text-warn bg-warn/5'">
              {{ hasOutputContract ? "含 pass 字段" : "未见 pass 字段" }}
            </span>
            <span class="flex-1" />
            <button class="btn-ghost btn-sm" @click="helpOpen = !helpOpen">
              {{ helpOpen ? "收起变量说明" : "变量说明" }}
            </button>
          </div>
          <div v-if="helpOpen" class="shrink-0 px-4 py-2 border-b border-line bg-page">
            <div class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-[11.5px] text-sub">
              <code class="font-mono text-main">{{ input }}</code>
              <span>主体提示词 + 本条输入（用户消息按该行数据渲染后）</span>
              <code class="font-mono text-main">{{ output }}</code>
              <span>被测模型对这一条的实际输出</span>
              <code class="font-mono text-main">{{ columnVar }}</code>
              <span>该行对应列的原始值，评测提示词里也能直接用</span>
            </div>
          </div>
          <textarea
            v-model="active.evalPrompt"
            class="flex-1 min-h-0 w-full resize-none border-0 rounded-none focus:ring-0
              font-mono text-[12.5px] leading-relaxed bg-panel text-main p-4"
            placeholder="裁判模型的提示词…"
          />
          <div class="h-10 shrink-0 px-4 flex items-center gap-2 border-t border-line">
            <span class="flex-1" />
            <button class="btn-ghost btn-sm" @click="openDefault('evalPrompt')">恢复默认</button>
            <button class="btn-ghost btn-sm" @click="importPrompt('evalPrompt')">导入</button>
            <button class="btn-ghost btn-sm" @click="exportPrompt('evalPrompt')">导出</button>
          </div>
        </div>

        <div v-else class="flex-1 min-h-0 flex flex-col">
          <div class="shrink-0 h-10 px-4 flex items-center gap-2 border-b border-line">
            <span class="text-[12px] text-sub">数据行</span>
            <button class="icon-btn" :disabled="previewIndex <= 1" title="上一条"
              @click="previewIndex = Math.max(1, previewIndex - 1)">
              <Icon name="chevron-left" :size="14" />
            </button>
            <input v-model.number="previewIndex" type="number" min="1" :max="totalRows || 1"
              class="field w-16 text-center tabular-nums" />
            <button class="icon-btn" :disabled="previewIndex >= totalRows" title="下一条"
              @click="previewIndex = Math.min(totalRows, previewIndex + 1)">
              <Icon name="chevron-right" :size="14" />
            </button>
            <span class="text-[11.5px] text-weak tabular-nums">
              {{ totalRows ? `/ ${totalRows}` : "" }}
            </span>
            <span v-if="sampleLabel" class="chip">{{ sampleLabel }}</span>
            <span class="flex-1" />
            <span class="text-[11.5px] text-weak">
              {{ dataset ? dataset.name : "先选择数据集" }}
            </span>
          </div>

          <div v-if="!dataset" class="flex-1 flex items-center justify-center">
            <p class="hint">先给方案选择数据集，才能预览实际发送的内容。</p>
          </div>
          <div v-else-if="!totalRows" class="flex-1 flex items-center justify-center">
            <p class="hint">这个数据集没有数据行。</p>
          </div>
          <div v-else class="flex-1 min-h-0 grid grid-rows-3 gap-2 p-3">
            <div class="min-h-0 flex flex-col rounded border border-line">
              <div class="h-7 px-2.5 flex items-center text-[11.5px] text-sub border-b border-line shrink-0">
                被测模型 · system（已渲染）
              </div>
              <pre class="flex-1 min-h-0 overflow-auto p-2.5 font-mono text-[12px] whitespace-pre-wrap text-main">{{ sampleLoading ? "读取中…" : previewSystem || "（空）" }}</pre>
            </div>
            <div class="min-h-0 flex flex-col rounded border border-line">
              <div class="h-7 px-2.5 flex items-center text-[11.5px] text-sub border-b border-line shrink-0">
                被测模型 · user（模板渲染后）
              </div>
              <pre class="flex-1 min-h-0 overflow-auto p-2.5 font-mono text-[12px] whitespace-pre-wrap text-main">{{ sampleLoading ? "读取中…" : previewUser || "（空）" }}</pre>
            </div>
            <div class="min-h-0 flex flex-col rounded border border-line">
              <div class="h-7 px-2.5 flex items-center text-[11.5px] text-sub border-b border-line shrink-0">
                评测模型 · user（评测提示词渲染后）
              </div>
              <pre class="flex-1 min-h-0 overflow-auto p-2.5 font-mono text-[12px] whitespace-pre-wrap text-main">{{ sampleLoading ? "读取中…" : previewJudge }}</pre>
            </div>
          </div>
        </div>
      </template>

      <div v-else class="flex-1 flex flex-col items-center justify-center gap-2">
        <p class="text-[13px] text-sub">还没有方案</p>
        <p class="hint">一套提示词加上数据集和连接，就是一个可以反复运行的方案。</p>
        <button class="btn-primary mt-2" @click="create">新建方案</button>
      </div>
    </section>

    <div v-if="defaultDialog" class="fixed inset-0 z-50 flex items-center justify-center p-4">
      <div class="absolute inset-0 bg-black/40" @click="defaultDialog = null" />
      <div class="relative w-[min(780px,94vw)] max-h-[86vh] bg-panel border border-line rounded-md flex flex-col">
        <div class="h-11 px-4 flex items-center gap-2 border-b border-line">
          <span class="text-[13px] font-medium">默认模板 · {{ defaultDialog.title }}</span>
          <span class="flex-1" />
          <button class="btn-ghost btn-sm" @click="copyDefault">复制</button>
          <button class="icon-btn" title="关闭" @click="defaultDialog = null">
            <Icon name="close" :size="15" />
          </button>
        </div>
        <div class="flex-1 overflow-auto p-4 space-y-3">
          <p class="hint">{{ defaultDialog.note }}</p>
          <pre class="bg-page border border-line rounded p-3 text-[12px] font-mono whitespace-pre-wrap text-main">{{ defaultDialog.text }}</pre>
          <p class="text-[12px] text-warn">填入会覆盖编辑框里当前的内容。</p>
        </div>
        <div class="h-12 px-4 flex items-center justify-end gap-2 border-t border-line">
          <button class="btn btn-sm" @click="defaultDialog = null">取消</button>
          <button class="btn-primary btn-sm" @click="applyDefault">填入编辑器</button>
        </div>
      </div>
    </div>
  </div>
</template>
