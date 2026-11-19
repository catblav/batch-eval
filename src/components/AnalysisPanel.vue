<script setup>
import { computed, onMounted, ref, watch } from "vue";
import { ask } from "@tauri-apps/plugin-dialog";
import {
  state,
  toast,
  viewStats,
  viewResults,
  filteredResults,
  pagedResults,
  pageCount,
  groupOptions,
  isExcluded,
  isExcludedCategory,
  loadJobs,
  selectAnalysisRun,
  analysisTitle,
  analysisSourceText,
  deleteJob,
  usePage,
} from "../store";
import Donut from "./Donut.vue";
import Pager from "./Pager.vue";
import DetailDrawer from "./DetailDrawer.vue";
import Icon from "./Icon.vue";
import EmptyState from "./EmptyState.vue";
import { exportCsv, exportJson, exportFullReport, slotDataGetter } from "../utils/exporters";
import { catColor, catSoft, catLine } from "../utils/catColors";

const s = viewStats;
const drawerIndex = ref(-1);
const exporting = ref(false);
const deleting = ref(false);
/// 明细聚焦：隐藏 KPI 卡和图表，明细占满整页，再点一下返回
const fullDetail = ref(false);

const analyzable = computed(() =>
  state.jobs.list.slice().sort((a, b) => (b.startedAt || 0) - (a.startedAt || 0))
);
/// 运行中的任务可以实时查看，但不能删除
const isRunning = computed(() =>
  ["running", "cancelling"].includes(currentJob.value?.status)
);
const currentJob = computed(() =>
  state.jobs.list.find((j) => j.runId === state.analysis.runId) || null
);
const currentIndex = computed(() =>
  analyzable.value.findIndex((j) => j.runId === state.analysis.runId)
);
const canPrev = computed(() => currentIndex.value > 0);
const canNext = computed(
  () => currentIndex.value >= 0 && currentIndex.value < analyzable.value.length - 1
);
const title = computed(() => analysisTitle());
const sourceText = computed(() => analysisSourceText());

const page = computed(() => Math.min(Math.max(1, state.view.page), pageCount.value));
const avgLatency = computed(() => {
  const list = viewResults.value.filter((r) => r.targetLatencyMs);
  if (!list.length) return 0;
  return Math.round(list.reduce((a, r) => a + r.targetLatencyMs, 0) / list.length);
});

const promptCats = computed(() => s.value.byType.filter((t) => !t.builtin));
const builtinCats = computed(() => s.value.byType.filter((t) => t.builtin));
const hasFilter = computed(
  () =>
    state.view.filterStatus !== "all" ||
    !!state.view.filterCategory ||
    !!state.view.filterGroup ||
    !!state.view.query
);

// KPI 指标卡：图标 + 大数字 + 说明沉底。tone 决定图标底色 / 数字颜色
const pct = (x) => (Number.isFinite(x) ? (x * 100).toFixed(1) : "—");
const kpiCards = computed(() => {
  const v = s.value;
  return [
    { key: "valid", label: "有效样本", icon: "database", tone: "neutral",
      value: v.valid, unit: "条", sub: `共 ${v.total} 条结果` },
    { key: "pass", label: "通过", icon: "check", tone: "pass",
      value: v.passed, unit: "条", sub: `占有效 ${pct(v.passRate)}%` },
    { key: "fail", label: "不通过", icon: "alert", tone: "fail",
      value: v.failed, unit: "条", sub: `占有效 ${pct(1 - v.passRate)}%` },
    { key: "excluded", label: "未计入", icon: "clock", tone: "warn",
      value: v.excluded, unit: "条", sub: `占总 ${pct(v.excluded / v.total)}%` },
    { key: "latency", label: "平均响应", icon: "play", tone: "neutral",
      value: (avgLatency.value / 1000).toFixed(1), unit: "s", sub: "被测模型每行" },
  ];
});
const tones = {
  pass: { bg: "bg-pass/10", fg: "text-pass", num: "text-pass" },
  fail: { bg: "bg-fail/10", fg: "text-fail", num: "text-fail" },
  warn: { bg: "bg-warn/10", fg: "text-warn", num: "text-warn" },
  neutral: { bg: "bg-wash", fg: "text-sub", num: "text-main" },
};

const detailRow = computed(() => {
  if (drawerIndex.value < 0) return null;
  return viewResults.value.find((r) => r.index === drawerIndex.value) || null;
});
const detailDataRow = computed(() => {
  if (drawerIndex.value < 0) return null;
  return state.analysis.dataset?.rows?.[drawerIndex.value] || null;
});

watch(
  () => state.analysis.runId,
  () => (drawerIndex.value = -1)
);

/// 当前任务在「运行中↔终态」之间切换时（跑完、取消、重跑、续跑），强制重载一次：
/// 终态切到磁盘完整数据，运行中切回实时模式。loadJobs 里 status 值不变时浅比较不会触发，无多余重载
watch(
  () => {
    const j = currentJob.value;
    return j?.status || "";
  },
  async (st, prev) => {
    if (!state.analysis.runId) return;
    const liveNow = st === "running" || st === "cancelling";
    const wasLive = prev === "running" || prev === "cancelling";
    if (liveNow !== wasLive) await selectAnalysisRun(state.analysis.runId, true);
  }
);

function pad2(n) {
  return String(n).padStart(2, "0");
}

function fmtTime(ts) {
  if (!ts) return "";
  const d = new Date(ts);
  const now = new Date();
  const dayOf = (x) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((dayOf(now) - dayOf(d)) / 86400000);
  const hm = `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
  if (diff <= 0) return `今天 ${hm}`;
  if (diff === 1) return `昨天 ${hm}`;
  return `${d.getMonth() + 1}月${pad2(d.getDate())}日 ${hm}`;
}

function rateOf(job) {
  const sum = job.summary;
  if (!sum?.valid) return "—";
  return `${Math.round((sum.passed / sum.valid) * 100)}%`;
}

function optionLabel(j) {
  return `${j.title || j.planName || "未命名任务"} · ${j.datasetName || "未记录数据集"} · ${fmtTime(j.startedAt)} · ${rateOf(j)}`;
}

function statusText(st) {
  return {
    running: "运行中",
    cancelling: "取消中",
    finished: "已完成",
    cancelled: "已取消",
    failed: "失败",
    interrupted: "已中断",
  }[st] || st;
}

function statusClass(st) {
  if (st === "running" || st === "cancelling") return "chip-accent";
  if (st === "finished") return "chip-pass";
  if (st === "failed") return "chip-fail";
  return "chip-warn";
}

/// 手动选过任务就不再自动跟最新，直到又跑完一个新任务
function choose(id) {
  if (!id) return;
  state.analysis.follow = false;
  selectAnalysisRun(id);
}

function goPrev() {
  if (!canPrev.value) return;
  state.analysis.follow = false;
  selectAnalysisRun(analyzable.value[currentIndex.value - 1].runId);
}

function goNext() {
  if (!canNext.value) return;
  state.analysis.follow = false;
  selectAnalysisRun(analyzable.value[currentIndex.value + 1].runId);
}

function reset() {
  state.view.filterCategory = "";
  state.view.filterStatus = "all";
  state.view.filterGroup = "";
  state.view.query = "";
  state.view.queryApplied = "";
  state.view.page = 1;
}

function toggleCat(name) {
  state.view.filterCategory = state.view.filterCategory === name ? "" : name;
  if (!state.view.filterCategory) return;
  state.view.filterStatus = isExcludedCategory(name) ? "excluded" : "fail";
}

/// 导出里的 system / user 放该行注入到两个槽位的原始数据列，不是渲染后的整段消息
function exportDataOf() {
  return slotDataGetter(state.analysis.meta, state.analysis.dataset?.rows || []);
}

function onExport(kind) {
  if (!viewResults.value.length) {
    toast("还没有结果可导出", "info");
    return;
  }
  const dataOf = exportDataOf();
  if (kind === "csv") exportCsv(viewResults.value, title.value, dataOf);
  else {
    // 任务快照只存了数据行，列名从首行取
    const columns =
      state.analysis.dataset?.columns ||
      Object.keys(state.analysis.dataset?.rows?.[0] || {});
    exportJson(viewResults.value, columns, title.value, dataOf);
  }
}

function buildReportPayload() {
  const meta = state.analysis.meta || {};
  const v = s.value;
  const dataOf = exportDataOf();
  const rows = viewResults.value.map((r) => {
    const ex = isExcluded(r);
    const data = dataOf(r);
    return {
      index: r.index + 1,
      group: r.statusLabel || "未分组",
      verdict: ex ? "未计入" : r.pass === true ? "通过" : "不通过",
      excluded: ex,
      categories: (r.errorTypes || []).join(" / "),
      reasons: (r.reasons || []).join("；"),
      details: r.details || "",
      output: r.output || "",
      system: data.system,
      input: data.user,
      targetError: r.targetError || "",
      targetLatencyMs: r.targetLatencyMs,
      judgeLatencyMs: r.judgeLatencyMs,
      promptTokens: r.promptTokens ?? null,
      completionTokens: r.completionTokens ?? null,
      judgeRaw: r.judgeRaw || "",
    };
  });
  const profile = (p) => ({
    protocol: p?.protocol || "",
    model: p?.model || "",
    contextWindow: Number(p?.contextWindow) || 0,
    maxTokens: Number(p?.maxTokens) || 0,
    temperature: Number(p?.temperature) || 0,
    topP: Number(p?.topP) || 0,
    timeoutSecs: Number(p?.timeoutSecs) || 0,
    extraBody: p?.extraBody || "",
    extraHeaders: p?.extraHeaders || "",
  });
  return {
    title: title.value,
    planName: meta.planName || "",
    datasetName: meta.datasetName || "",
    generatedAt: new Date(meta.finishedAt || meta.startedAt || Date.now()).toISOString(),
    datasetRows: state.analysis.dataset?.rows?.length || viewResults.value.length,
    targetProfile: profile(meta.targetProfile),
    judgeProfile: profile(meta.judgeProfile),
    summary: {
      total: v.total,
      valid: v.valid,
      passed: v.passed,
      failed: v.failed,
      excluded: v.excluded,
      passRate: v.passRate,
      avgTargetLatencyMs: avgLatency.value,
    },
    categories: v.byType.map((t) => ({ name: t.name, count: t.count, ratio: t.ratio })),
    rows,
  };
}

async function onExportReport() {
  if (!viewResults.value.length) {
    toast("还没有结果可导出", "info");
    return;
  }
  if (exporting.value) return;
  exporting.value = true;
  try {
    await exportFullReport(buildReportPayload());
  } finally {
    exporting.value = false;
  }
}

async function onDelete() {
  const job = currentJob.value;
  if (!job || deleting.value) return;
  const ok = await ask(
    `删除任务「${job.title || job.planName}」？\n\n它的结果文件和数据集快照会一起删除，无法恢复。`,
    { title: "删除任务", kind: "warning", okLabel: "删除", cancelLabel: "取消" }
  );
  if (!ok) return;
  deleting.value = true;
  try {
    await deleteJob(job);
    const next = analyzable.value[0];
    if (next) await selectAnalysisRun(next.runId);
    else await selectAnalysisRun("");
  } finally {
    deleting.value = false;
  }
}

onMounted(async () => {
  await loadJobs();
  if (
    state.analysis.runId &&
    !state.jobs.list.some((j) => j.runId === state.analysis.runId)
  ) {
    await selectAnalysisRun("");
  }
  const latest = analyzable.value[0];
  // 跑完新任务后自动跟到最新一条；手动选过老任务的保持不动
  if (state.analysis.follow) {
    if (latest && latest.runId !== state.analysis.runId) {
      await selectAnalysisRun(latest.runId);
    }
  } else if (!state.analysis.runId && latest) {
    await selectAnalysisRun(latest.runId);
  }
});
</script>

<template>
  <div class="h-full flex flex-col card">
    <div class="h-14 px-2.5 flex items-center gap-2 shrink-0 border-b border-line/60">
      <h1 class="page-title shrink-0">分析</h1>
      <!-- 选择框吃满中间剩余宽度：任务名很长，固定宽度只能截断 -->
      <div class="flex items-center gap-1 flex-1 min-w-0 ml-2">
        <button class="icon-btn shrink-0" title="上一条" :disabled="!canPrev" @click="goPrev">
          <Icon name="chevron-left" :size="15" />
        </button>
        <select
          class="field flex-1 min-w-0 text-[12.5px]"
          :value="state.analysis.runId"
          :disabled="!analyzable.length"
          @change="choose($event.target.value)"
        >
          <option v-if="!analyzable.length" value="">还没有可分析的任务</option>
          <option v-for="j in analyzable" :key="j.runId" :value="j.runId">
            {{ optionLabel(j) }}
          </option>
        </select>
        <button class="icon-btn shrink-0" title="下一条" :disabled="!canNext" @click="goNext">
          <Icon name="chevron-right" :size="15" />
        </button>
      </div>
      <span v-if="currentJob" class="chip !h-5 !px-1.5 shrink-0" :class="statusClass(currentJob.status)">
        {{ statusText(currentJob.status) }}
      </span>
      <span v-if="isRunning && state.analysis.live" class="text-[11.5px] text-sub tabular-nums shrink-0">
        {{ state.analysis.liveDone }} / {{ state.analysis.liveTotal || "—" }}
      </span>
      <div class="min-w-0 max-w-[240px] text-right shrink-0 hidden xl:block">
        <div class="text-[12px] text-sub truncate" :title="title">{{ title }}</div>
        <div class="text-[11px] text-weak truncate" :title="sourceText">{{ sourceText }}</div>
      </div>
      <button class="btn btn-sm shrink-0" :disabled="exporting || !viewResults.length"
        title="通过率、分类分布、未计入明细与全部明细，导成一个多 sheet 的 Excel"
        @click="onExportReport">
        <Icon name="download" :size="14" />
        {{ exporting ? "生成中…" : "导出完整报告" }}
      </button>
      <button class="icon-btn text-fail shrink-0" title="删除这条记录"
        :disabled="!currentJob || deleting || isRunning" @click="onDelete">
        <Icon name="trash" :size="15" />
      </button>
    </div>

    <!-- 主体可滚动；明细卡片撑满剩余高度，分页条才贴得到卡片底边 -->
    <div class="flex-1 min-h-0 overflow-y-auto flex flex-col">
      <EmptyState
        v-if="!analyzable.length"
        icon="chart"
        title="还没有可分析的任务"
        hint="先去运行页跑一批任务；运行中的任务会实时出现在这里。"
      >
        <button class="btn-primary" @click="usePage('run')">去运行页</button>
      </EmptyState>

      <EmptyState
        v-else-if="!state.analysis.runId"
        icon="search"
        title="选择一条任务记录"
        hint="上面的下拉里只列已完成、已取消和已中断的任务。"
      />

      <div
        v-else-if="state.analysis.loading"
        class="h-full flex flex-col items-center justify-center gap-3 px-2.5"
      >
        <span v-for="n in 5" :key="n" class="skeleton h-3 w-full max-w-[560px]" />
      </div>

      <EmptyState
        v-else-if="!s.total"
        icon="file"
        :title="state.analysis.live ? '等待第一批结果' : '这条记录没有结果'"
        :hint="state.analysis.live
          ? '任务正在运行，结果到达后这里会实时更新。'
          : '可能刚开始就被中断了。换一条记录，或到运行页续跑。'"
      />

      <template v-else>
        <!-- KPI 指标卡行：图标 + 大数字 + 说明沉底，先给结论再给证据 -->
        <template v-if="!fullDetail">
          <div class="px-2.5 pt-2.5 grid grid-cols-2 md:grid-cols-3 xl:grid-cols-5 gap-2.5 shrink-0">
          <div
            v-for="k in kpiCards"
            :key="k.key"
            class="card px-3 py-2.5 flex flex-col justify-between gap-2"
          >
            <div class="flex items-center justify-between">
              <span class="w-7 h-7 rounded-md flex items-center justify-center shrink-0"
                :class="tones[k.tone].bg">
                <Icon :name="k.icon" :size="13" :class="tones[k.tone].fg" />
              </span>
              <span class="text-[11.5px] text-weak">{{ k.label }}</span>
            </div>
            <div class="text-[30px] font-semibold tabular-nums leading-none"
              :class="tones[k.tone].num">
              {{ k.value }}<span v-if="k.unit" class="text-[15px] text-weak font-normal ml-0.5">{{ k.unit }}</span>
            </div>
            <div class="text-[11px] text-weak text-right truncate">{{ k.sub }}</div>
          </div>
        </div>

        <!-- 图表区：通过率环图 + 错误分类分布（点击筛选明细） -->
        <div class="px-2.5 pt-2.5 grid grid-cols-1 xl:grid-cols-5 gap-2.5 shrink-0">
          <div class="card p-3 xl:col-span-2 flex items-center gap-4 justify-center">
            <Donut :total="s.total" :passed="s.passed" :failed="s.failed" :excluded="s.excluded" />
            <div class="flex flex-col gap-1 min-w-0">
              <span class="group-title">通过率</span>
              <span class="text-[26px] font-semibold tabular-nums leading-none text-main">
                {{ pct(s.passRate) }}<span class="text-[15px] text-weak ml-0.5">%</span>
              </span>
              <span class="text-[11px] text-weak">平均响应 {{ (avgLatency / 1000).toFixed(1) }}s</span>
            </div>
          </div>

          <div class="card p-3 xl:col-span-3 flex flex-col min-h-0">
            <div class="flex items-center justify-between shrink-0">
              <span class="group-title">错误分类分布</span>
              <span class="text-[10.5px] text-weak">{{ s.byType.length ? "点击筛选明细" : "" }}</span>
            </div>

            <div class="flex-1 min-h-0 overflow-y-auto overflow-x-hidden mt-2 space-y-2.5">
              <!-- 评测提示词定义的分类 -->
              <button
                v-for="t in promptCats"
                :key="t.name"
                class="block w-full text-left"
                :title="`${t.name} · ${t.count} 条`"
                @click="toggleCat(t.name)"
              >
                <div
                  class="flex items-center gap-2 text-[12px] rounded px-1 -mx-1 transition-colors"
                  :class="state.view.filterCategory === t.name ? '' : 'hover:bg-wash/60'"
                  :style="state.view.filterCategory === t.name
                    ? { color: catColor(t.name), background: catSoft(t.name) }
                    : undefined"
                >
                  <span class="w-1.5 h-1.5 rounded-full shrink-0" :style="{ background: catColor(t.name) }" />
                  <span class="truncate flex-1">{{ t.name }}</span>
                  <span class="tabular-nums text-weak shrink-0">{{ t.count }}</span>
                  <span class="tabular-nums w-9 text-right text-weak shrink-0">{{ (t.ratio * 100).toFixed(0) }}%</span>
                </div>
                <div class="bar mt-1">
                  <div class="bar-fill" :style="{ width: (t.ratio * 100).toFixed(1) + '%', background: catColor(t.name) }" />
                </div>
              </button>

              <!-- 工具兜底分类：调用失败 / 解析失败等 -->
              <template v-if="builtinCats.length">
                <div class="flex items-center gap-2 pt-2 border-t border-line/50 mt-1">
                  <span class="group-title">工具兜底</span>
                </div>
                <button
                  v-for="t in builtinCats"
                  :key="t.name"
                  class="block w-full text-left"
                  :title="`${t.name} · ${t.count} 条`"
                  @click="toggleCat(t.name)"
                >
                  <div
                    class="flex items-center gap-2 text-[12px] rounded px-1 -mx-1 transition-colors"
                    :class="state.view.filterCategory === t.name ? '' : 'hover:bg-wash/60'"
                    :style="state.view.filterCategory === t.name
                      ? { color: catColor(t.name), background: catSoft(t.name) }
                      : undefined"
                  >
                    <span class="w-1.5 h-1.5 rounded-full shrink-0" :style="{ background: catColor(t.name) }" />
                    <span class="truncate flex-1">{{ t.name }}</span>
                    <span class="tabular-nums text-weak shrink-0">{{ t.count }}</span>
                    <span class="tabular-nums w-9 text-right text-weak shrink-0">{{ (t.ratio * 100).toFixed(0) }}%</span>
                  </div>
                  <div class="bar mt-1">
                    <div class="bar-fill" :style="{ width: (t.ratio * 100).toFixed(1) + '%', background: catColor(t.name) }" />
                  </div>
                </button>
              </template>

              <!-- 未计入细分：分母不同，单独一组 -->
              <template v-if="s.excluded">
                <div class="flex items-center justify-between pt-2 border-t border-line/50 mt-1">
                  <span class="group-title text-warn">未计入分类 · {{ s.excluded }}</span>
                </div>
                <button
                  v-for="t in s.excludedTypes"
                  :key="t.name"
                  class="block w-full text-left"
                  :title="`${t.name} · ${t.count} 条`"
                  @click="toggleCat(t.name)"
                >
                  <div
                    class="flex items-center gap-2 text-[12px] rounded px-1 -mx-1 transition-colors"
                    :class="state.view.filterCategory === t.name ? '' : 'hover:bg-wash/60'"
                    :style="state.view.filterCategory === t.name
                      ? { color: catColor(t.name), background: catSoft(t.name) }
                      : undefined"
                  >
                    <span class="w-1.5 h-1.5 rounded-full shrink-0" :style="{ background: catColor(t.name) }" />
                    <span class="truncate flex-1">{{ t.name }}</span>
                    <span class="tabular-nums text-weak shrink-0">{{ t.count }}</span>
                    <span class="tabular-nums w-9 text-right text-weak shrink-0">{{ (t.ratio * 100).toFixed(0) }}%</span>
                  </div>
                  <div class="bar mt-1">
                    <div class="bar-fill" :style="{ width: (t.ratio * 100).toFixed(1) + '%', background: catColor(t.name) }" />
                  </div>
                </button>
                <p class="hint">调用失败、超时、解析失败，不参与通过率</p>
              </template>

              <p v-if="!promptCats.length && !builtinCats.length" class="hint">全部通过，没有错误分类</p>
            </div>
          </div>
        </div>
        </template>

        <!-- 明细卡片：撑满剩余高度，表格内部滚动；full 模式占满整页 -->
        <div class="flex-1 flex flex-col px-2.5 py-2.5 min-w-0"
          :class="fullDetail ? 'min-h-0' : 'min-h-[240px]'">
          <div class="card flex-1 min-h-0 flex flex-col">
            <div class="px-2.5 h-11 flex items-center gap-2 flex-wrap border-b border-line/60 shrink-0">
              <span class="group-title">明细</span>
              <span class="text-[11.5px] text-weak tabular-nums">
                {{ filteredResults.length }} / {{ s.total }}
              </span>
              <button class="btn-ghost btn-sm" :title="fullDetail ? '返回指标与图表概览' : '明细占满整页，便于逐条核对'"
                @click="fullDetail = !fullDetail">
                <Icon :name="fullDetail ? 'close' : 'eye'" :size="13" />
                {{ fullDetail ? "返回概览" : "查看全部明细" }}
              </button>
              <span class="flex-1" />
              <!-- 结果 / 分组 / 分类三个筛选挪到表头里了，这里只留搜索 -->
              <div class="relative">
                <Icon name="search" :size="13"
                  class="absolute left-2 top-1/2 -translate-y-1/2 text-weak pointer-events-none" />
                <input v-model="state.view.query" class="field pl-7 !h-7 w-72 text-[12px]"
                  placeholder="搜索输入 / 输出 / 原因" />
              </div>
              <button class="btn-ghost btn-sm" :disabled="!hasFilter" @click="reset">重置</button>
              <span class="w-px h-4 bg-line mx-0.5" />
              <button class="btn-ghost btn-sm" :disabled="!viewResults.length" @click="onExport('csv')">
                CSV
              </button>
              <button class="btn-ghost btn-sm" :disabled="!viewResults.length" @click="onExport('json')">
                JSON
              </button>
            </div>

            <div class="flex-1 min-h-0 overflow-auto">
              <table class="data-table">
                <thead>
                  <tr>
                    <th class="w-12">#</th>
                    <th class="w-24">
                      <div class="flex items-center gap-1.5">
                        <span>结果</span>
                        <select v-model="state.view.filterStatus"
                          class="bg-transparent border border-line rounded px-1 py-0.5 text-[11px] text-sub cursor-pointer">
                          <option value="all">全部</option>
                          <option value="pass">通过</option>
                          <option value="fail">不通过</option>
                          <option value="excluded">未计入</option>
                        </select>
                      </div>
                    </th>
                    <th class="w-36">
                      <div class="flex items-center gap-1.5">
                        <span>分组</span>
                        <select v-model="state.view.filterGroup"
                          class="bg-transparent border border-line rounded px-1 py-0.5 text-[11px] text-sub max-w-[84px] cursor-pointer"
                          :title="state.view.filterGroup || '按分组筛选'">
                          <option value="">全部</option>
                          <option v-for="g in groupOptions" :key="g" :value="g">{{ g }}</option>
                        </select>
                      </div>
                    </th>
                    <th class="w-52">
                      <div class="flex items-center gap-1.5">
                        <span>分类</span>
                        <select v-model="state.view.filterCategory"
                          class="bg-transparent border border-line rounded px-1 py-0.5 text-[11px] text-sub max-w-[110px] cursor-pointer"
                          :title="state.view.filterCategory || '按分类筛选'">
                          <option value="">全部</option>
                          <option v-for="c in s.categories" :key="c" :value="c">{{ c }}</option>
                        </select>
                      </div>
                    </th>
                    <th>被测模型输出</th>
                    <th>原因</th>
                    <th class="w-16">耗时</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="r in pagedResults" :key="r.index" @click="drawerIndex = r.index">
                    <td class="text-weak tabular-nums">{{ r.index + 1 }}</td>
                    <td>
                      <span v-if="isExcluded(r)" class="chip-warn !h-5 !px-1.5">未计入</span>
                      <span v-else-if="r.pass === true" class="chip-pass !h-5 !px-1.5">通过</span>
                      <span v-else class="chip-fail !h-5 !px-1.5">不通过</span>
                    </td>
                    <td class="text-sub truncate max-w-[140px]" :title="r.statusLabel">{{ r.statusLabel }}</td>
                    <td>
                      <span v-for="t in r.errorTypes || []" :key="t" class="chip !h-5 !px-1.5 mr-1 mb-0.5"
                        :style="{ color: catColor(t), background: catSoft(t), borderColor: catLine(t) }">{{ t }}</span>
                      <span v-if="!r.errorTypes?.length" class="text-weak">—</span>
                    </td>
                    <td class="text-main max-w-[360px]" :title="r.output">
                      <span class="line-clamp-3 whitespace-pre-wrap">{{ r.output || (r.targetError ? "（调用失败）" : "—") }}</span>
                    </td>
                    <td class="text-sub max-w-[320px]">
                      <span class="line-clamp-3">{{ (r.reasons || []).join("；") || r.details || "—" }}</span>
                    </td>
                    <td class="text-weak tabular-nums">
                      {{ (((r.targetLatencyMs || 0) + (r.judgeLatencyMs || 0)) / 1000).toFixed(1) }}s
                    </td>
                  </tr>
                  <tr v-if="!filteredResults.length">
                    <td colspan="7" class="text-center text-weak py-10">没有符合条件的数据</td>
                  </tr>
                </tbody>
              </table>
            </div>

            <Pager :total="filteredResults.length" :page="page" :page-size="state.view.pageSize"
              :sizes="[5, 10, 20]"
              @update:page="state.view.page = $event"
              @update:page-size="state.view.pageSize = $event" />
          </div>
        </div>
      </template>
    </div>

    <DetailDrawer :row="detailRow" :data-row="detailDataRow" @close="drawerIndex = -1" />
  </div>
</template>
