<script setup>
import { ref, computed, watch, nextTick, onMounted, onBeforeUnmount } from "vue";
import {
  state,
  toast,
  activePlan,
  planProblems,
  templateVarCheck,
  datasetMetaById,
  datasetColumnsOf,
  connectionById,
  createPlan,
  duplicatePlan,
  deletePlan,
  setPlanDataset,
  usePlanForRun,
  usePage,
  savedText,
  flushSave,
  statusLabelOf,
  toggleList,
  referencedVars,
} from "../store";
import {
  DEFAULT_SYSTEM_PROMPT,
  DEFAULT_EVAL_PROMPT,
  DEFAULT_NOTES,
  autoColsBlock,
  defaultInjectedColumns,
  defaultUserTemplate,
} from "../defaultPrompts";
import { invoke } from "@tauri-apps/api/core";
import { open, save, ask } from "@tauri-apps/plugin-dialog";
import Icon from "./Icon.vue";
import EmptyState from "./EmptyState.vue";
import HighlightEditor from "./HighlightEditor.vue";
import SplitAside from "./SplitAside.vue";
import NameInput from "./NameInput.vue";

const search = ref("");
const tab = ref("system");
const menuId = ref("");
const helpOpen = ref(true);
const defaultDialog = ref(null);
const nameInput = ref(null);
const previewIndex = ref(1);
const sampleRow = ref(null);
const sampleLoading = ref(false);

const TABS = [
  { key: "system", name: "主体提示词", field: "systemPrompt" },
  { key: "eval", name: "评测提示词", field: "evalPrompt" },
  { key: "inject", name: "数据注入", field: "injections" },
  { key: "preview", name: "预览" },
];

/// 四个消息槽位：勾选启用 + 数据模板。列名填进哪个槽位就注入哪个槽位。
const SLOT_DEFS = [
  { key: "targetSystem", label: "被测 system" },
  { key: "targetUser", label: "被测 user" },
  { key: "judgeSystem", label: "评测 system" },
  { key: "judgeUser", label: "评测 user" },
];
/// 当前「列 chips 填入」的目标槽位：默认被测 system，点编辑器会切换
const activeSlot = ref("targetSystem");
const targetSlotDefs = SLOT_DEFS.filter((d) => d.key.startsWith("target"));
const judgeSlotDefs = SLOT_DEFS.filter((d) => d.key.startsWith("judge"));

/// 返回槽位对象（reactive 引用，直接改 enabled/text 会写进方案）
function slotRow(key) {
  const inj = active.value?.injections || {};
  return inj[key] || { enabled: false, text: "", promptIn: false, dataIn: false };
}
function slotLabel(key) {
  return SLOT_DEFS.find((d) => d.key === key)?.label || key;
}
/// 槽位未启用 / 模板为空时预览的占位文案
function slotEmptyHint(key) {
  const s = slotRow(key);
  if (s?.promptIn) return "（空）提示词为空";
  if (!s?.enabled) return `（空）未勾选${slotLabel(key)}`;
  if (!String(s?.text || "").trim()) return `（空）模板为空`;
  return "";
}

/// 槽位第二个标签：勾了提示词本体导入时，数据模板只声明这个槽位渲染哪几列，
/// 本身不构成消息内容；没勾时模板渲染后就是这条消息
function slotDataChip(key) {
  const s = slotRow(key);
  if (!s?.enabled) return "";
  return s.promptIn ? "列声明" : "数据模板";
}

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

/// 与 Rust runner::render_slot 一致：只替换「注入本槽位的列」，{{input}}/{{output}} 仅当 dataIn。
/// 提示词里引用但没注入本槽位的变量保持字面——数据只渲染在它被填入的槽位。
function renderSlotTpl(tpl, row, slotCols, dataIn, inp, outp) {
  let out = String(tpl || "");
  for (const [k, v] of Object.entries(row || {})) {
    if (!slotCols.has(k)) continue;
    const val = typeof v === "string" ? v : JSON.stringify(v);
    out = out.split(`{{${k}}}`).join(val);
  }
  if (dataIn) {
    out = out.split("{{input}}").join(inp).split("{{ input }}").join(inp);
    out = out.split("{{output}}").join(outp).split("{{ output }}").join(outp);
  }
  return out;
}

/// 某槽位注入的列集合：槽位启用且模板非空时，取模板里引用的列。其余为空。
function slotColsOf(key) {
  const s = slotRow(key);
  if (!s?.enabled || !String(s?.text || "").trim()) return new Set();
  return referencedVars(s.text);
}

/// 与 Rust runner::build_slot 一致。
/// 导入了提示词本体的槽位：消息 = 提示词本体（按本槽位声明的列渲染），
/// 数据模板在这里只声明注入哪几列，本身不作为文本发出。
/// 没导入提示词本体的槽位：消息 = 数据模板渲染后的内容。
function buildSlotPreview(key, prompt, inp, outp) {
  const s = slotRow(key);
  const cols = slotColsOf(key);
  if (s?.promptIn) {
    return renderSlotTpl(prompt, sampleRow.value, cols, !!s?.dataIn, inp, outp);
  }
  if (s?.enabled && String(s?.text || "").trim()) {
    return renderSlotTpl(s.text, sampleRow.value, cols, !!s?.dataIn, inp, outp);
  }
  return "";
}

/// 被测模型实际 system：主体提示词本体（若勾选）
const previewSystem = computed(() =>
  buildSlotPreview("targetSystem", active.value?.systemPrompt, "", "")
);
/// 被测模型实际 user：同理，被测 user 也能导入主体提示词本体
const previewUser = computed(() =>
  buildSlotPreview("targetUser", active.value?.systemPrompt, "", "")
);
/// 与 Rust runner::judge_input_text 一致：空的那条消息不出现，也就不留空标题
const previewJudgeInput = computed(() => {
  const sys = previewSystem.value.trim();
  const user = previewUser.value.trim();
  if (!sys) return user;
  if (!user) return `【被评估模型的提示词】\n${sys}`;
  return `【被评估模型的提示词】\n${sys}\n\n【本条输入】\n${user}`;
});
const OUTPUT_PLACEHOLDER = "（被测模型的实际输出会出现在这里）";

/// 评测 system：promptIn 决定评测提示词本体是否进这里，dataIn 决定 input/output 是否渲染
const previewJudgeSystem = computed(() =>
  buildSlotPreview("judgeSystem", active.value?.evalPrompt, previewJudgeInput.value, OUTPUT_PLACEHOLDER)
);
/// 评测 user：同上，独立开关
const previewJudgeUser = computed(() =>
  buildSlotPreview("judgeUser", active.value?.evalPrompt, previewJudgeInput.value, OUTPUT_PLACEHOLDER)
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

function escapeRegEx(s) {
  return String(s).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/// 某列是否已写入指定槽位的数据模板（运行时才会注入）
function slotReferenced(key, c) {
  return referencedVars(slotRow(key)?.text).has(c);
}

/// 列勾选开关：勾上 = 焦点槽位模板末尾接上 {{列}}；取消 = 从该槽位删掉 {{列}}。
/// 追加用顿号直接接着放，不换行——列 chips 只是标记要注入哪些列。
function toggleSlotCol(c) {
  const key = activeSlot.value;
  const s = slotRow(key);
  if (!active.value || !s) return;
  const token = wrapVar(c);
  if (slotReferenced(key, c)) {
    const re = new RegExp(`\\{\\{\\s*${escapeRegEx(c)}\\s*\\}\\}`, "g");
    s.text = s.text.replace(re, "");
  } else {
    const base = s.text.trimEnd();
    s.text = base ? `${base}、${token}` : token;
  }
}

/// 一键把全部列（自动排除「序号」）填入焦点槽位
function fillSlotCols() {
  const key = activeSlot.value;
  const s = slotRow(key);
  if (!active.value || !s) return;
  if (!columns.value.length) {
    toast("先给方案绑定一个有列的数据集", "info");
    return;
  }
  s.text = defaultUserTemplate(columns.value);
  toast(`已按数据集列生成数据模板（填入${slotLabel(key)}）`, "ok");
}

/// 立即把当前方案落盘（平时有 600ms 自动保存，这里是手动确认一次）
async function saveNow() {
  const ok = await flushSave();
  if (ok) toast("已保存", "ok");
}

/// 把未引用的列补入焦点槽位
function syncVars() {
  if (!active.value) return;
  const missing = templateVarCheck.value.missing;
  if (!missing.length) return;
  const key = activeSlot.value;
  const s = slotRow(key);
  if (!s) return;
  const tokens = missing.map((c) => wrapVar(c)).join("、");
  const base = s.text.trimEnd();
  s.text = base ? `${base}、${tokens}` : tokens;
  toast(`已把 ${missing.length} 个未引用列补入${slotLabel(key)}`, "ok");
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
    toast(`已导入${which === "systemPrompt" ? "主体提示词" : "评测提示词"}`);
  } catch (e) {
    toast("导入失败：" + e, "err");
  }
}

async function exportPrompt(which) {
  try {
    const label = which === "systemPrompt" ? "主体提示词" : "评测提示词";
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
  evalPrompt: "评测提示词",
};

/// 主体提示词的默认模板要带上自动列块：默认布局下数据靠提示词里的 {{列}} 渲染，
/// 只给一段不引用列的提示词，填进去等于把数据断了
function defaultTextOf(which) {
  if (which === "systemPrompt") {
    return DEFAULT_SYSTEM_PROMPT + autoColsBlock(defaultInjectedColumns(columns.value));
  }
  if (which === "evalPrompt") return DEFAULT_EVAL_PROMPT;
  return "";
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
  // 记下这段列块是自动写的，换数据集时好定位替换
  if (d.which === "systemPrompt") {
    active.value.autoCols = defaultInjectedColumns(columns.value);
  }
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
  <div class="h-full flex card">
    <SplitAside>
      <div class="h-14 px-2.5 flex items-center gap-2 shrink-0">
        <h1 class="page-title">提示词配置</h1>
        <span class="text-[12px] text-weak tabular-nums">{{ state.plans.length }}</span>
        <span class="flex-1" />
        <button class="icon-btn" title="新建方案" @click="create">
          <Icon name="plus" :size="16" />
        </button>
        <button class="icon-btn" title="收起列表" @click="toggleList">
          <Icon name="chevron-left" :size="15" />
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
        <EmptyState
          v-if="!state.plans.length"
          size="sm"
          icon="sliders"
          title="还没有方案"
          hint="方案 = 提示词 + 数据集 + 一对连接。"
        >
          <button class="btn-primary btn-sm" @click="create">
            <Icon name="plus" :size="13" />
            新建方案
          </button>
        </EmptyState>
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
    </SplitAside>

    <section class="flex-1 min-w-0 flex flex-col bg-panel">
      <template v-if="active">
        <div class="h-14 px-2.5 flex items-center gap-2.5 border-b border-line/60 shrink-0">
          <button v-if="state.listCollapsed" class="icon-btn shrink-0" title="展开列表"
            @click="toggleList">
            <Icon name="chevron-right" :size="15" />
          </button>
          <NameInput ref="nameInput" v-model="active.name" placeholder="方案名称" />
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

        <div class="shrink-0 px-2.5 py-3 border-b border-line/60 flex items-center gap-4 flex-wrap">
          <div class="flex items-center gap-2">
            <span class="group-title">数据集</span>
            <select
              :value="active.datasetId"
              class="field w-[190px]"
              @change="setPlanDataset(active, $event.target.value)"
            >
              <option value="">未选择</option>
              <option v-for="d in state.datasets" :key="d.id" :value="d.id">
                {{ d.name }} · {{ d.rows }} 行
              </option>
            </select>
          </div>
          <div class="flex items-center gap-2">
            <span class="group-title">被测</span>
            <select v-model="active.targetId" class="field w-[180px]">
              <option value="">未选择</option>
              <option v-for="c in state.connections" :key="c.id" :value="c.id">
                {{ c.name }} · {{ c.model || "未填模型" }}
              </option>
            </select>
          </div>
          <div class="flex items-center gap-2">
            <span class="group-title">评测</span>
            <select v-model="active.judgeId" class="field w-[180px]">
              <option value="">未选择</option>
              <option v-for="c in state.connections" :key="c.id" :value="c.id">
                {{ c.name }} · {{ c.model || "未填模型" }}
              </option>
            </select>
            <button class="chip-btn" :disabled="!active.targetId" title="评测连接与被测相同"
              @click="active.judgeId = active.targetId">同被测</button>
            <span v-if="judgeWarn" class="text-[11.5px] text-warn whitespace-nowrap">建议 temperature 0</span>
          </div>
        </div>

        <div v-if="problems.length"
          class="shrink-0 px-2.5 h-8 flex items-center gap-1.5 border-b border-line/60 bg-warn/5 text-[12px] text-warn">
          <Icon name="alert" :size="13" />
          <span class="truncate">还不能运行：{{ problems.join("、") }}</span>
          <span class="flex-1" />
          <button class="btn-ghost btn-sm text-warn !h-6" @click="usePage('connections')">
            <Icon name="arrow-right" :size="12" />
            去检查连接
          </button>
        </div>

        <div class="h-9 px-2.5 flex items-center border-b border-line/60 shrink-0">
          <button
            v-for="t in TABS"
            :key="t.key"
            class="editor-tab"
            :class="{ 'editor-tab-active': tab === t.key }"
            @click="tab = t.key"
          >
            {{ t.name }}
            <span v-if="t.key === 'inject' && columns.length" class="ml-1 text-[11px] text-weak tabular-nums">
              {{ templateVarCheck.missing.length ? templateVarCheck.missing.length + " 列未用" : "变量齐全" }}
            </span>
          </button>
          <span class="flex-1" />
          <!-- 操作放右上角：和标签栏同一行，不再占底部一整条厚横杠 -->
          <div v-if="tab === 'system'" class="flex items-center gap-1">
            <button class="btn-ghost btn-sm" @click="openDefault('systemPrompt')">
              <Icon name="refresh" :size="12" />
              恢复默认
            </button>
            <button class="btn-ghost btn-sm" @click="importPrompt('systemPrompt')">
              <Icon name="upload" :size="12" />
              导入
            </button>
            <button class="btn-blue btn-sm" @click="exportPrompt('systemPrompt')">
              <Icon name="download" :size="12" />
              导出
            </button>
          </div>
          <div v-else-if="tab === 'eval'" class="flex items-center gap-1">
            <button class="btn-ghost btn-sm" @click="openDefault('evalPrompt')">
              <Icon name="refresh" :size="12" />
              恢复默认
            </button>
            <button class="btn-ghost btn-sm" @click="importPrompt('evalPrompt')">
              <Icon name="upload" :size="12" />
              导入
            </button>
            <button class="btn-blue btn-sm" @click="exportPrompt('evalPrompt')">
              <Icon name="download" :size="12" />
              导出
            </button>
          </div>
        </div>

        <!-- 编辑器套一层框：不套的话从 tab 到底部是一整片白，看不出这块是个独立单元 -->
        <div v-if="tab === 'system'" class="flex-1 min-h-0 flex flex-col p-2.5">
          <div class="flex-1 min-h-0 flex flex-col rounded-md border border-line/60 overflow-hidden">
            <HighlightEditor
              v-model="active.systemPrompt"
              :known-vars="columns"
              placeholder="被测模型的 system prompt…"
            />
            <!-- 字数放底部留白里，和编辑器同底色、不加分隔线 -->
            <div class="h-7 shrink-0 px-2.5 flex items-center justify-end text-[11.5px] text-weak tabular-nums">
              {{ (active.systemPrompt || "").length.toLocaleString() }} 字
            </div>
          </div>
        </div>

        <div v-else-if="tab === 'inject'" class="flex-1 min-h-0 flex flex-col">
          <div class="shrink-0 px-2.5 py-1.5 border-b border-line/60 flex items-center gap-2">
            <span class="text-[11.5px] text-sub">数据注入：决定数据渲染在哪个槽位</span>
            <span class="flex-1" />
            <span class="text-[11.5px] text-weak shrink-0">{{ savedText }}</span>
            <button class="btn-ghost btn-sm shrink-0" :disabled="state.save.saving" @click="saveNow">
              {{ state.save.saving ? "保存中…" : "保存" }}
            </button>
          </div>
          <div class="shrink-0 px-2.5 py-2 border-b border-line/60">
            <!-- 注入列：点击填入当前编辑的槽位 -->
            <div class="flex items-center gap-1.5 flex-wrap">
              <span class="text-[11.5px] text-weak mr-0.5">注入列</span>
              <button
                class="chip cursor-pointer transition-colors hover:border-accent/50 hover:text-accent"
                title="把全部列（自动排除「序号」）填入当前编辑的槽位"
                @click="fillSlotCols"
              >全部</button>
              <span class="text-[11.5px] text-sub">填入：{{ slotLabel(activeSlot) }}</span>
              <button
                v-for="c in columns"
                :key="c"
                class="chip font-mono cursor-pointer transition-colors"
                :class="slotReferenced(activeSlot, c) ? 'chip-accent' : 'chip-muted hover:border-accent/50 hover:text-accent'"
                :title="slotReferenced(activeSlot, c) ? `已在${slotLabel(activeSlot)}：点击移除这一列` : `未在${slotLabel(activeSlot)}：点击填入这一列`"
                @click="toggleSlotCol(c)"
              >
                {{ wrapVar(c) }}
              </button>
              <span v-if="!columns.length" class="hint">选择数据集后显示可用的列</span>
            </div>
            <div v-if="templateVarCheck.unknown.length" class="text-[11.5px] text-warn">
              数据集里没有：
              <code v-for="v in templateVarCheck.unknown" :key="v" class="font-mono mr-1">{{ wrapVar(v) }}</code>
              运行时会原样发给模型
            </div>
            <div v-else-if="templateVarCheck.missing.length" class="flex items-center gap-2 text-[11.5px] text-weak">
              <span>{{ templateVarCheck.missing.length }} 列没有被引用，这些数据不会进入请求</span>
              <button class="text-accent hover:underline" @click="syncVars">补入模板</button>
            </div>
          </div>
          <div class="flex-1 min-h-0 overflow-y-auto p-2.5 space-y-3">
            <!-- 被测组：system / user 两个槽位 -->
            <div class="space-y-2">
              <div class="text-[12px] font-medium text-sub">被测</div>
              <div
                v-for="d in targetSlotDefs"
                :key="d.key"
                class="rounded-md border p-2 transition-colors"
                :class="activeSlot === d.key ? 'border-accent/60' : 'border-line/60'"
                title="点这个编辑器，注入列的点击会填入当前槽位"
                @focusin="activeSlot = d.key"
              >
                <div class="flex items-center gap-1.5 select-none">
                  <input
                    type="checkbox"
                    class="shrink-0"
                    :checked="slotRow(d.key).enabled"
                    @change="slotRow(d.key).enabled = $event.target.checked"
                  />
                  <span class="text-[12px] font-medium whitespace-nowrap"
                    :class="slotRow(d.key).enabled ? 'text-main' : 'text-weak'">
                    {{ d.label }}
                  </span>
                  <label
                    class="flex items-center gap-0.5 ml-2 cursor-pointer select-none"
                    title="主体提示词本体发到被测模型的这个槽位。勾了它，这个槽位发出去的就是提示词本体，下面的数据模板只用来声明渲染哪几列，本身不作为消息内容；不勾，下面的模板渲染后就是这条消息。system 与 user 都不勾就不发提示词。"
                  >
                    <input
                      type="checkbox"
                      class="shrink-0"
                      :checked="slotRow(d.key).promptIn"
                      @change="slotRow(d.key).promptIn = $event.target.checked"
                    />
                    <span class="text-[11.5px] text-sub whitespace-nowrap">提示词本体导入</span>
                  </label>
                </div>
                <div class="mt-1.5 h-24 flex flex-col rounded-md border border-line/60 overflow-hidden">
                  <HighlightEditor
                    v-model="slotRow(d.key).text"
                    :known-vars="columns"
                    :placeholder="slotRow(d.key).promptIn
                      ? '这个槽位渲染哪几列：{{列名}}、{{列名}}…（上面勾了提示词本体导入，这里只声明列）'
                      : '数据模板：引导语 + {{列名}} 引用数据列…'"
                  />
                </div>
                <div class="h-5 px-1 mt-1 flex items-center justify-between text-[11px] text-weak">
                  <span>引用 {{ referencedVars(slotRow(d.key).text).size }} 列</span>
                  <span class="tabular-nums">{{ (slotRow(d.key).text || "").length }} 字</span>
                </div>
              </div>
            </div>
            <!-- 评测组：system / user 两个槽位 -->
            <div class="space-y-2">
              <div class="text-[12px] font-medium text-sub">评测</div>
              <div
                v-for="d in judgeSlotDefs"
                :key="d.key"
                class="rounded-md border p-2 transition-colors"
                :class="activeSlot === d.key ? 'border-accent/60' : 'border-line/60'"
                title="点这个编辑器，注入列的点击会填入当前槽位"
                @focusin="activeSlot = d.key"
              >
                <div class="flex items-center gap-1.5 select-none">
                  <input
                    type="checkbox"
                    class="shrink-0"
                    :checked="slotRow(d.key).enabled"
                    @change="slotRow(d.key).enabled = $event.target.checked"
                  />
                  <span class="text-[12px] font-medium whitespace-nowrap"
                    :class="slotRow(d.key).enabled ? 'text-main' : 'text-weak'">
                    {{ d.label }}
                  </span>
                  <label
                    class="flex items-center gap-0.5 ml-2 cursor-pointer select-none"
                    title="评测提示词本体（含 {{input}} / {{output}}）发到评测模型的这个槽位。勾了它，这个槽位发出去的就是评测提示词本体，下面的数据模板只用来声明渲染哪几列，本身不作为消息内容；不勾，下面的模板渲染后就是这条消息。"
                  >
                    <input
                      type="checkbox"
                      class="shrink-0"
                      :checked="slotRow(d.key).promptIn"
                      @change="slotRow(d.key).promptIn = $event.target.checked"
                    />
                    <span class="text-[11.5px] text-sub whitespace-nowrap">提示词本体导入</span>
                  </label>
                  <label
                    class="flex items-center gap-0.5 ml-2 cursor-pointer select-none"
                    title="本槽位里评测提示词 / 数据模板的 {{input}}、{{output}} 是否渲染成实际被测数据。不勾选则保留字面花括号。"
                  >
                    <input
                      type="checkbox"
                      class="shrink-0"
                      :checked="slotRow(d.key).dataIn"
                      @change="slotRow(d.key).dataIn = $event.target.checked"
                    />
                    <span class="text-[11.5px] text-sub whitespace-nowrap">被测数据导入</span>
                    <span
                      class="cursor-help text-[11.5px] text-weak"
                      title="被测数据 = {{input}} + {{output}}，含义见「评测提示词」页的变量说明"
                    >?</span>
                  </label>
                </div>
                <div class="mt-1.5 h-24 flex flex-col rounded-md border border-line/60 overflow-hidden">
                  <HighlightEditor
                    v-model="slotRow(d.key).text"
                    :known-vars="columns"
                    :placeholder="slotRow(d.key).promptIn
                      ? '这个槽位渲染哪几列：{{列名}}、{{列名}}…（上面勾了提示词本体导入，这里只声明列）'
                      : '数据模板：引导语 + {{列名}} 引用数据列…'"
                  />
                </div>
                <div class="h-5 px-1 mt-1 flex items-center justify-between text-[11px] text-weak">
                  <span>引用 {{ referencedVars(slotRow(d.key).text).size }} 列</span>
                  <span class="tabular-nums">{{ (slotRow(d.key).text || "").length }} 字</span>
                </div>
              </div>
            </div>
          </div>
        </div>

        <div v-else-if="tab === 'eval'" class="flex-1 min-h-0 flex flex-col">
          <div class="shrink-0 px-2.5 py-1.5 border-b border-line/60 flex items-center gap-2 flex-wrap">
            <span class="text-[11.5px] text-sub">这份提示词本体发到评测模型哪个槽位，由「数据注入」页评测组的「提示词本体导入」勾选决定</span>
            <span class="flex-1" />
            <button class="btn-ghost btn-sm" @click="helpOpen = !helpOpen">
              {{ helpOpen ? "收起变量说明" : "变量说明" }}
            </button>
          </div>
          <div class="shrink-0 h-8 px-2.5 flex items-center gap-2 border-b border-line/60">
            <span :class="evalHasVars ? 'chip-pass' : 'chip-warn'">
              {{ evalHasVars ? "变量齐全" : "缺少 input / output" }}
            </span>
            <span :class="hasOutputContract ? 'chip-pass' : 'chip-warn'">
              {{ hasOutputContract ? "含 pass 字段" : "未见 pass 字段" }}
            </span>
          </div>
          <div v-if="helpOpen" class="shrink-0 px-2.5 py-2 border-b border-line/60 bg-wash/40">
            <div class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-[11.5px] text-sub">
              <code class="font-mono text-main">{{ input }}</code>
              <span>主体提示词 + 本条输入（数据注入模板按该行数据渲染后）</span>
              <code class="font-mono text-main">{{ output }}</code>
              <span>被测模型对这一条的实际输出</span>
              <code class="font-mono text-main">{{ columnVar }}</code>
              <span>该行对应列的原始值，评测提示词里也能直接用</span>
            </div>
          </div>
          <div class="flex-1 min-h-0 flex flex-col p-2.5">
            <div class="flex-1 min-h-0 flex flex-col rounded-md border border-line/60 overflow-hidden">
              <HighlightEditor
                v-model="active.evalPrompt"
                :known-vars="columns"
                placeholder="裁判模型的提示词…"
              />
              <div class="h-7 shrink-0 px-2.5 flex items-center justify-end text-[11.5px] text-weak tabular-nums">
                {{ (active.evalPrompt || "").length.toLocaleString() }} 字
              </div>
            </div>
          </div>
        </div>

        <div v-else class="flex-1 min-h-0 flex flex-col">
          <div class="shrink-0 h-10 px-2.5 flex items-center gap-2 border-b border-line/60">
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
          <div v-else class="flex-1 min-h-0 grid grid-rows-4 gap-2 p-3">
            <div class="min-h-0 flex flex-col rounded border border-line">
              <div class="h-7 px-2.5 flex items-center text-[11.5px] text-sub border-b border-line shrink-0">
                被测模型 · system
                <span v-if="slotRow('targetSystem').promptIn" class="chip ml-1.5">主体提示词</span>
                <span v-if="slotDataChip('targetSystem')" class="chip ml-1.5">{{ slotDataChip('targetSystem') }}</span>
              </div>
              <pre class="flex-1 min-h-0 overflow-auto p-2.5 text-[12px] whitespace-pre-wrap text-main">{{ sampleLoading ? "读取中…" : previewSystem || "（空）" }}</pre>
            </div>
            <div class="min-h-0 flex flex-col rounded border border-line">
              <div class="h-7 px-2.5 flex items-center text-[11.5px] text-sub border-b border-line shrink-0">
                被测模型 · user
                <span v-if="slotRow('targetUser').promptIn" class="chip ml-1.5">主体提示词</span>
                <span v-if="slotDataChip('targetUser')" class="chip ml-1.5">{{ slotDataChip('targetUser') }}</span>
              </div>
              <pre class="flex-1 min-h-0 overflow-auto p-2.5 text-[12px] whitespace-pre-wrap" :class="previewUser ? 'text-main' : 'text-weak'">{{ sampleLoading ? "读取中…" : previewUser || slotEmptyHint("targetUser") }}</pre>
            </div>
            <div class="min-h-0 flex flex-col rounded border border-line">
              <div class="h-7 px-2.5 flex items-center text-[11.5px] text-sub border-b border-line shrink-0">
                评测模型 · system
                <span v-if="slotRow('judgeSystem').promptIn" class="chip ml-1.5">评测提示词</span>
                <span v-if="slotDataChip('judgeSystem')" class="chip ml-1.5">{{ slotDataChip('judgeSystem') }}</span>
              </div>
              <pre class="flex-1 min-h-0 overflow-auto p-2.5 text-[12px] whitespace-pre-wrap" :class="previewJudgeSystem ? 'text-main' : 'text-weak'">{{ sampleLoading ? "读取中…" : previewJudgeSystem || slotEmptyHint("judgeSystem") }}</pre>
            </div>
            <div class="min-h-0 flex flex-col rounded border border-line">
              <div class="h-7 px-2.5 flex items-center text-[11.5px] text-sub border-b border-line shrink-0">
                评测模型 · user
                <span v-if="slotRow('judgeUser').promptIn" class="chip ml-1.5">评测提示词</span>
                <span v-if="slotDataChip('judgeUser')" class="chip ml-1.5">{{ slotDataChip('judgeUser') }}</span>
              </div>
              <pre class="flex-1 min-h-0 overflow-auto p-2.5 text-[12px] whitespace-pre-wrap" :class="previewJudgeUser ? 'text-main' : 'text-weak'">{{ sampleLoading ? "读取中…" : previewJudgeUser || slotEmptyHint("judgeUser") }}</pre>
            </div>
          </div>
        </div>
      </template>

      <EmptyState
        v-else
        icon="sliders"
        title="还没有方案"
        hint="一套提示词加上数据集和连接，就是一个可以反复运行的方案。"
      >
        <button class="btn-primary" @click="create">
          <Icon name="plus" :size="14" />
          新建方案
        </button>
      </EmptyState>
    </section>

    <div v-if="defaultDialog" class="fixed inset-0 z-50 flex items-center justify-center p-4">
      <div class="absolute inset-0 bg-black/40" @click="defaultDialog = null" />
      <div
        class="relative w-[min(780px,94vw)] max-h-[86vh] bg-panel border border-cardline rounded-lg
          shadow-pop flex flex-col overflow-hidden"
      >
        <div class="h-11 px-2.5 flex items-center gap-2 border-b border-line/60">
          <span class="text-[13px] font-medium">默认模板 · {{ defaultDialog.title }}</span>
          <span class="flex-1" />
          <button class="btn-ghost btn-sm" @click="copyDefault">复制</button>
          <button class="icon-btn" title="关闭" @click="defaultDialog = null">
            <Icon name="close" :size="15" />
          </button>
        </div>
        <div class="flex-1 overflow-auto p-2.5 space-y-3">
          <p class="hint">{{ defaultDialog.note }}</p>
          <pre class="bg-wash/40 border border-line/60 rounded-md p-3 text-[12px] whitespace-pre-wrap text-main">{{ defaultDialog.text }}</pre>
          <p class="text-[12px] text-warn">填入会覆盖编辑框里当前的内容。</p>
        </div>
        <div class="h-12 px-2.5 flex items-center justify-end gap-2 border-t border-line/60">
          <button class="btn btn-sm" @click="defaultDialog = null">取消</button>
          <button class="btn-primary btn-sm" @click="applyDefault">
            <Icon name="check" :size="12" />
            填入编辑器
          </button>
        </div>
      </div>
    </div>
  </div>
</template>
