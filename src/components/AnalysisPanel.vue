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
import BarChart from "./BarChart.vue";
import Donut from "./Donut.vue";
import Pager from "./Pager.vue";
import DetailDrawer from "./DetailDrawer.vue";
import Icon from "./Icon.vue";
import { exportCsv, exportJson, exportFullReport } from "../utils/exporters";

const s = viewStats;
const drawerIndex = ref(-1);
const exporting = ref(false);
const deleting = ref(false);

const analyzable = computed(() =>
  state.jobs.list
    .filter((j) => !["running", "cancelling"].includes(j.status))
    .slice()
    .sort((a, b) => (b.startedAt || 0) - (a.startedAt || 0))
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
    finished: "已完成",
    cancelled: "已取消",
    failed: "失败",
    interrupted: "已中断",
  }[st] || st;
}

function statusClass(st) {
  if (st === "finished") return "text-pass border-pass/30 bg-pass/5";
  if (st === "failed") return "text-fail border-fail/30 bg-fail/5";
  return "text-warn border-warn/30 bg-warn/5";
}

function choose(id) {
  if (!id) return;
  selectAnalysisRun(id);
}

function goPrev() {
  if (canPrev.value) selectAnalysisRun(analyzable.value[currentIndex.value - 1].runId);
}

function goNext() {
  if (canNext.value) selectAnalysisRun(analyzable.value[currentIndex.value + 1].runId);
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

function onExport(kind) {
  if (!viewResults.value.length) {
    toast("还没有结果可导出", "info");
    return;
  }
  if (kind === "csv") exportCsv(viewResults.value, title.value);
  else exportJson(viewResults.value, state.analysis.dataset?.columns || [], title.value);
}

function buildReportPayload() {
  const meta = state.analysis.meta || {};
  const v = s.value;
  const rows = viewResults.value.map((r) => {
    const ex = isExcluded(r);
    return {
      index: r.index + 1,
      group: r.statusLabel || "未分组",
      verdict: ex ? "未计入" : r.pass === true ? "通过" : "不通过",
      excluded: ex,
      categories: (r.errorTypes || []).join(" / "),
      reasons: (r.reasons || []).join("；"),
      details: r.details || "",
      output: r.output || "",
      input: r.input || "",
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
  if (!state.analysis.runId) {
    const first = analyzable.value[0];
    if (first) await selectAnalysisRun(first.runId);
  }
});
</script>

<template>
  <div class="h-full flex flex-col bg-page">
    <div class="h-14 px-5 flex items-center gap-2 shrink-0">
      <h1 class="text-[15px] font-semibold shrink-0">分析</h1>
      <div class="flex items-center gap-1 ml-2 min-w-0">
        <button class="icon-btn" title="上一条" :disabled="!canPrev" @click="goPrev">
          <Icon name="chevron-left" :size="15" />
        </button>
        <select
          class="field w-[380px] min-w-0 text-[12.5px]"
          :value="state.analysis.runId"
          :disabled="!analyzable.length"
          @change="choose($event.target.value)"
        >
          <option v-if="!analyzable.length" value="">还没有可分析的任务</option>
          <option v-for="j in analyzable" :key="j.runId" :value="j.runId">
            {{ optionLabel(j) }}
          </option>
        </select>
        <button class="icon-btn" title="下一条" :disabled="!canNext" @click="goNext">
          <Icon name="chevron-right" :size="15" />
        </button>
      </div>
      <span v-if="currentJob" class="chip !h-5 !px-1.5 ml-1 shrink-0" :class="statusClass(currentJob.status)">
        {{ statusText(currentJob.status) }}
      </span>
      <div class="min-w-0 flex-1 ml-1 hidden xl:block">
        <div class="text-[12px] text-sub truncate">{{ title }}</div>
        <div class="text-[11px] text-weak truncate">{{ sourceText }}</div>
      </div>
      <span class="flex-1 xl:hidden" />
      <button class="icon-btn text-fail" title="删除这条记录" :disabled="!currentJob || deleting" @click="onDelete">
        <Icon name="trash" :size="15" />
      </button>
    </div>

    <div class="flex-1 min-h-0 overflow-y-auto">
      <div v-if="!analyzable.length" class="h-full flex flex-col items-center justify-center gap-2">
        <p class="text-[13px] text-sub">还没有可分析的任务</p>
        <p class="hint">先去运行页跑一批；运行中的任务可以在那里看实时明细。</p>
        <button class="btn-primary mt-2" @click="usePage('run')">去运行页</button>
      </div>

      <div v-else-if="!state.analysis.runId" class="h-full flex flex-col items-center justify-center gap-2">
        <p class="text-[13px] text-sub">选择一条任务记录</p>
        <p class="hint">上面的下拉里只列已完成、已取消和已中断的任务。</p>
      </div>

      <div v-else-if="state.analysis.loading" class="h-full flex items-center justify-center">
        <p class="hint">读取中…</p>
      </div>

      <div v-else-if="!s.total" class="h-full flex flex-col items-center justify-center gap-2">
        <p class="text-[13px] text-sub">这条记录没有结果</p>
        <p class="hint">可能刚开始就被中断了。换一条记录，或到运行页续跑。</p>
      </div>

      <template v-else>
        <section class="bg-panel border-y border-line px-5 py-4 flex items-center gap-8">
          <Donut :total="s.total" :passed="s.passed" :failed="s.failed" :excluded="s.excluded" />
          <div class="grid grid-cols-4 gap-x-8 gap-y-2 flex-1 min-w-0">
            <div>
              <div class="hint">有效样本</div>
              <div class="stat-number mt-1">{{ s.valid }}<span class="text-[13px] text-weak ml-1">/ {{ s.total }}</span></div>
            </div>
            <div>
              <div class="hint">通过</div>
              <div class="stat-number mt-1 text-pass">{{ s.passed }}</div>
            </div>
            <div>
              <div class="hint">未计入</div>
              <div class="stat-number mt-1 text-warn">{{ s.excluded }}</div>
              <div class="hint truncate mt-1" :title="s.excludedTypes[0]?.name">
                {{ s.excludedTypes[0]?.name || "—" }}
              </div>
            </div>
            <div>
              <div class="hint">平均响应</div>
              <div class="stat-number mt-1">{{ (avgLatency / 1000).toFixed(1) }}<span class="text-[13px] text-weak ml-0.5">s</span></div>
            </div>
          </div>
          <button class="btn shrink-0" :disabled="exporting" @click="onExportReport"
            title="通过率、分类分布、未计入明细、分组汇总与全部明细，导成一个多 sheet 的 Excel">
            <Icon name="download" :size="14" />
            {{ exporting ? "生成中…" : "导出完整报告" }}
          </button>
        </section>

        <section class="bg-panel border-b border-line px-5 py-4 grid gap-8"
          :class="s.excluded ? 'grid-cols-2' : 'grid-cols-1'">
          <div>
            <div class="flex items-center gap-2 mb-2.5">
              <span class="text-[13px] font-medium">错误分类分布</span>
              <span class="hint">按 errorType 聚合，占不通过的 {{ s.failed }} 条，点击筛选</span>
            </div>
            <div v-if="promptCats.length">
              <div class="hint mb-1.5">评测提示词定义的分类</div>
              <BarChart :items="promptCats" :total="s.failed" />
            </div>
            <div v-if="builtinCats.length" :class="promptCats.length ? 'mt-4 pt-3 border-t border-line' : ''">
              <div class="hint mb-1.5">工具兜底分类</div>
              <BarChart :items="builtinCats" :total="s.failed" color="rgb(var(--warn))" />
            </div>
            <p v-if="!s.byType.length" class="hint">全部通过，没有错误分类</p>
            <div class="flex flex-wrap gap-1.5 mt-3">
              <button
                v-for="t in s.byType"
                :key="t.name"
                class="chip"
                :class="state.view.filterCategory === t.name
                  ? 'border-accent/60 text-accent bg-accent/5'
                  : t.builtin
                    ? 'border-warn/40 text-warn'
                    : 'hover:text-main'"
                @click="toggleCat(t.name)"
              >
                {{ t.name }} <span class="tabular-nums text-weak">{{ t.count }}</span>
              </button>
            </div>
          </div>

          <div v-if="s.excluded">
            <div class="flex items-center gap-2 mb-2.5">
              <span class="text-[13px] font-medium text-warn">未计入通过率 · {{ s.excluded }}</span>
              <span class="hint">调用失败、超时、解析失败，不参与通过率</span>
            </div>
            <BarChart :items="s.excludedTypes" :total="s.excluded" color="rgb(var(--warn))" />
            <div class="flex flex-wrap gap-1.5 mt-3">
              <button
                v-for="t in s.excludedTypes"
                :key="t.name"
                class="chip text-warn border-warn/40"
                :class="state.view.filterCategory === t.name ? 'bg-warn/5' : ''"
                @click="toggleCat(t.name)"
              >
                {{ t.name }} <span class="tabular-nums opacity-70">{{ t.count }}</span>
              </button>
            </div>
          </div>
        </section>

        <section class="bg-panel border-b border-line">
          <div class="px-5 py-2.5 flex items-center gap-2 flex-wrap border-b border-line">
            <span class="text-[13px] font-medium">明细</span>
            <span class="hint tabular-nums">{{ filteredResults.length }} / {{ s.total }} 条</span>
            <span class="flex-1" />
            <input v-model="state.view.query" class="field w-52 !h-7 text-[12px]"
              placeholder="搜索输入 / 输出 / 原因" />
            <button v-if="hasFilter" class="btn-ghost btn-sm" @click="reset">重置筛选</button>
            <span class="w-px h-4 bg-line mx-1" />
            <button class="btn-ghost btn-sm" :disabled="!viewResults.length" @click="onExport('csv')">
              明细 CSV
            </button>
            <button class="btn-ghost btn-sm" :disabled="!viewResults.length" @click="onExport('json')">
              JSON
            </button>
          </div>

          <div class="overflow-auto max-h-[460px]">
            <table class="data-table">
              <thead>
                <tr>
                  <th class="w-12 text-right pr-3">#</th>
                  <th class="w-32">
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
                  <th class="w-40">
                    <div class="flex items-center gap-1.5">
                      <span>分组</span>
                      <select v-model="state.view.filterGroup"
                        class="bg-transparent border border-line rounded px-1 py-0.5 text-[11px] text-sub max-w-[92px] cursor-pointer"
                        :title="state.view.filterGroup || '按分组筛选'">
                        <option value="">全部</option>
                        <option v-for="g in groupOptions" :key="g" :value="g">{{ g }}</option>
                      </select>
                    </div>
                  </th>
                  <th class="w-56">
                    <div class="flex items-center gap-1.5">
                      <span>分类</span>
                      <select v-model="state.view.filterCategory"
                        class="bg-transparent border border-line rounded px-1 py-0.5 text-[11px] text-sub max-w-[120px] cursor-pointer"
                        :title="state.view.filterCategory || '按分类筛选'">
                        <option value="">全部</option>
                        <option v-for="c in s.categories" :key="c" :value="c">{{ c }}</option>
                      </select>
                    </div>
                  </th>
                  <th>被测模型输出</th>
                  <th>原因</th>
                  <th class="w-16 text-right">耗时</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="r in pagedResults" :key="r.index" @click="drawerIndex = r.index">
                  <td class="text-right pr-3 text-weak tabular-nums">{{ r.index + 1 }}</td>
                  <td>
                    <span v-if="isExcluded(r)" class="chip !h-5 !px-1.5 text-warn border-warn/30 bg-warn/5">未计入</span>
                    <span v-else-if="r.pass === true" class="chip !h-5 !px-1.5 text-pass border-pass/30 bg-pass/5">通过</span>
                    <span v-else class="chip !h-5 !px-1.5 text-fail border-fail/30 bg-fail/5">不通过</span>
                  </td>
                  <td class="text-sub truncate max-w-[160px]" :title="r.statusLabel">{{ r.statusLabel }}</td>
                  <td>
                    <span v-for="t in r.errorTypes || []" :key="t" class="chip !h-5 !px-1.5 mr-1 mb-0.5">{{ t }}</span>
                    <span v-if="!r.errorTypes?.length" class="text-weak">—</span>
                  </td>
                  <td class="text-main max-w-[360px]" :title="r.output">
                    <span class="line-clamp-3 whitespace-pre-wrap">{{ r.output || (r.targetError ? "（调用失败）" : "—") }}</span>
                  </td>
                  <td class="text-sub max-w-[320px]">
                    <span class="line-clamp-3">{{ (r.reasons || []).join("；") || r.details || "—" }}</span>
                  </td>
                  <td class="text-right text-weak tabular-nums">
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
            @update:page="state.view.page = $event"
            @update:page-size="state.view.pageSize = $event" />
        </section>
      </template>
    </div>

    <DetailDrawer :row="detailRow" :data-row="detailDataRow" @close="drawerIndex = -1" />
  </div>
</template>
