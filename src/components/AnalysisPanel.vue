<script setup>
import { computed, ref } from "vue";
import { state, stats, filteredResults, pagedResults, pageCount, groupOptions, toast, isExcluded, isExcludedCategory } from "../store";
import BarChart from "./BarChart.vue";
import Donut from "./Donut.vue";
import DetailDrawer from "./DetailDrawer.vue";
import { exportCsv, exportJson, exportFullReport } from "../utils/exporters";

const s = stats;
const drawerIndex = ref(-1);
const exporting = ref(false);

const hasFilter = computed(
  () =>
    state.view.filterStatus !== "all" ||
    !!state.view.filterCategory ||
    !!state.view.filterGroup ||
    !!state.view.query
);

function reset() {
  state.view.filterCategory = "";
  state.view.filterStatus = "all";
  state.view.filterGroup = "";
  state.view.query = "";
  state.view.queryApplied = "";
  state.view.page = 1;
}

// 页码始终按实际总页数收口，避免筛选后停在空页
const page = computed(() => Math.min(Math.max(1, state.view.page), pageCount.value));
const rangeStart = computed(() =>
  filteredResults.value.length ? (page.value - 1) * state.view.pageSize + 1 : 0
);
const rangeEnd = computed(() =>
  Math.min(filteredResults.value.length, page.value * state.view.pageSize)
);

const jump = ref("");
function goJump() {
  const n = Number(jump.value);
  if (Number.isFinite(n) && n >= 1) state.view.page = Math.min(Math.floor(n), pageCount.value);
  jump.value = "";
}
function go(p) {
  state.view.page = Math.min(Math.max(1, p), pageCount.value);
}

function open(i) {
  drawerIndex.value = i;
}

function onExport(kind) {
  if (!state.run.results.length) {
    toast("还没有结果可导出", "info");
    return;
  }
  if (kind === "csv") exportCsv(state.run.results);
  else exportJson(state.run.results, state.dataset.columns);
}

/// 完整报告：图上看到的一切 + 全部明细，落成一个多 sheet 的 xlsx
async function onExportReport() {
  if (!state.run.results.length) {
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

function fmtDuration(ms) {
  if (!ms) return "—";
  const sec = Math.round(ms / 1000);
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const secLeft = sec % 60;
  if (h) return `${h} 小时 ${m} 分`;
  if (m) return `${m} 分 ${secLeft} 秒`;
  return `${secLeft} 秒`;
}

const avgLatency = computed(() => {
  const list = state.run.results.filter((r) => r.targetLatencyMs);
  if (!list.length) return 0;
  return Math.round(list.reduce((a, r) => a + r.targetLatencyMs, 0) / list.length);
});

/// 报告只带渲染需要的东西，不把整个 reactive 对象丢给 Rust
function buildReportPayload() {
  const v = s.value;
  const rows = state.run.results.map((r) => {
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

  // 模型参数：只取展示用的字段，apiKey 和接口地址不进报告
  const profileOf = (p) => ({
    protocol: p.protocol || "",
    model: p.model || "",
    contextWindow: Number(p.contextWindow) || 0,
    maxTokens: Number(p.maxTokens) || 0,
    temperature: Number(p.temperature) || 0,
    topP: Number(p.topP) || 0,
    timeoutSecs: Number(p.timeoutSecs) || 0,
    extraBody: p.extraBody || "",
    extraHeaders: p.extraHeaders || "",
  });

  return {
    generatedAt: new Date().toISOString(),
    datasetRows: state.dataset.rows.length,
    targetProfile: profileOf(state.settings.target),
    judgeProfile: profileOf(state.settings.judge),
    summary: {
      total: v.total,
      valid: v.valid,
      passed: v.passed,
      failed: v.failed,
      excluded: v.excluded,
      passRate: v.passRate,
      avgTargetLatencyMs: avgLatency.value,
    },
    categories: s.value.byType.map((t) => ({ name: t.name, count: t.count, ratio: t.ratio })),
    rows,
  };
}

// 评测提示词定义的分类 / 工具兜底分类，分开展示
const promptCats = computed(() => stats.value.byType.filter((t) => !t.builtin));
const builtinCats = computed(() => stats.value.byType.filter((t) => t.builtin));

function toggleCat(name) {
  state.view.filterCategory = state.view.filterCategory === name ? "" : name;
  if (!state.view.filterCategory) return;
  // 未计入与不通过是互斥的两组，筛选要跟着分类走，否则点了什么都筛不出来
  state.view.filterStatus = isExcludedCategory(name) ? "excluded" : "fail";
}
</script>

<template>
  <div v-if="!s.total" class="card p-10 text-center">
    <p class="text-ink-400 text-[13px]">还没有跑测结果</p>
    <p class="hint mt-2">先去「运行」页跑一批，这里会出现通过率和分类分布。</p>
  </div>

  <div v-else class="space-y-4">
    <div class="grid grid-cols-4 gap-4">
      <div class="card p-4 col-span-2 flex items-center justify-between gap-3">
        <Donut :total="s.total" :passed="s.passed" :failed="s.failed" :excluded="s.excluded" />
        <button class="btn shrink-0" :disabled="exporting" @click="onExportReport" title="通过率、分类分布、未计入明细、分组汇总与全部明细，导成一个多 sheet 的 Excel">
          {{ exporting ? "生成中…" : "导出完整报告" }}
        </button>
      </div>
      <div class="card p-4 flex flex-col justify-center gap-2">
        <button class="text-left group" @click="state.view.filterStatus = 'pass'; state.view.filterCategory = ''">
          <div class="hint">通过（pass = true）</div>
          <div class="text-[26px] font-semibold tabular-nums text-pass leading-none mt-0.5 group-hover:underline">
            {{ s.passed }}<span class="text-[13px] text-ink-500 ml-1">/ {{ s.valid }}</span>
          </div>
        </button>
        <button class="text-left group" @click="state.view.filterStatus = 'fail'; state.view.filterCategory = ''">
          <div class="hint">不通过（pass = false）</div>
          <div class="text-[26px] font-semibold tabular-nums text-fail leading-none mt-0.5 group-hover:underline">
            {{ s.failed }}<span class="text-[13px] text-ink-500 ml-1">/ {{ s.valid }}</span>
          </div>
        </button>
      </div>
      <div class="card p-4">
        <button class="text-left group w-full" @click="state.view.filterStatus = 'excluded'; state.view.filterCategory = ''"
          title="调用失败 / 超时 / 解析失败，不参与通过率">
          <div class="hint">未计入通过率</div>
          <div class="text-[26px] font-semibold tabular-nums text-warn leading-none mt-0.5 group-hover:underline">
            {{ s.excluded }}<span class="text-[13px] text-ink-500 ml-1">/ {{ s.total }}</span>
          </div>
        </button>
        <div class="hint mt-1 truncate" :title="s.excludedTypes[0]?.name">
          主要：{{ s.excludedTypes[0]?.name || "—" }}
        </div>
        <div class="hint mt-2">平均响应 {{ (avgLatency / 1000).toFixed(1) }}s</div>
      </div>
    </div>

    <div class="card p-4">
      <div class="flex items-center gap-2 mb-3">
        <span class="text-[13px] font-semibold">错误分类分布</span>
        <span class="hint">按 errorType 聚合，占不通过的 {{ s.failed }} 条，点击筛选</span>
      </div>

      <div v-if="promptCats.length">
        <div class="hint mb-1.5">评测提示词定义的分类</div>
        <BarChart :items="promptCats" :total="s.failed" />
      </div>

      <div v-if="builtinCats.length" :class="promptCats.length ? 'mt-4 pt-3 border-t border-ink-700/60' : ''">
        <div class="hint mb-1.5">工具兜底分类（裁判判了不通过却没给分类，不是调用异常）</div>
        <BarChart :items="builtinCats" :total="s.failed" color="rgb(var(--warn))" />
      </div>

      <p v-if="!s.byType.length" class="hint">全部通过，没有错误分类</p>

      <div class="flex flex-wrap gap-1 mt-3">
        <button v-for="t in s.byType" :key="t.name" class="chip"
          :class="state.view.filterCategory === t.name
            ? 'border-accent/60 text-accent bg-accent/10'
            : t.builtin
              ? 'border-warn/40 text-warn/90 hover:text-warn'
              : 'border-ink-700 text-ink-400 hover:text-ink-200'"
          @click="toggleCat(t.name)">
          {{ t.name }} <span class="tabular-nums opacity-70">{{ t.count }}</span>
        </button>
      </div>
    </div>

    <div v-if="s.excluded" class="card p-4 border-warn/30">
      <div class="flex items-center gap-2 mb-3">
        <span class="text-[13px] font-semibold text-warn/90">未计入通过率 · {{ s.excluded }} 条</span>
        <span class="hint">调用失败、超时、解析失败，不是模型答错，不参与通过率计算</span>
      </div>
      <BarChart :items="s.excludedTypes" :total="s.excluded" color="rgb(var(--warn))" />
      <div class="flex flex-wrap gap-1 mt-3">
        <button v-for="t in s.excludedTypes" :key="t.name" class="chip border-warn/40 text-warn/90 hover:text-warn"
          :class="state.view.filterCategory === t.name ? 'bg-warn/10' : ''"
          @click="toggleCat(t.name)">
          {{ t.name }} <span class="tabular-nums opacity-70">{{ t.count }}</span>
        </button>
      </div>
    </div>

    <div class="card overflow-hidden">
      <div class="flex items-center gap-2 px-4 py-2.5 border-b border-ink-700/60 flex-wrap">
        <span class="text-[13px] font-semibold">明细</span>
        <span class="hint">{{ filteredResults.length }} / {{ s.total }} 条</span>
        <span class="flex-1" />
        <input v-model="state.view.query" class="field w-52 text-[12px]" placeholder="搜索输入 / 输出 / 原因" />
        <button v-if="hasFilter" class="btn-ghost text-[12px]" @click="reset">重置筛选</button>
        <button class="btn-ghost text-[12px]" @click="onExport('csv')" title="只导明细表，CSV 格式">导出明细 CSV</button>
        <button class="btn-ghost text-[12px]" @click="onExport('json')" title="原始结果逐条导出">导出 JSON</button>
        <button class="btn text-[12px]" :disabled="exporting" @click="onExportReport">导出完整报告</button>
      </div>

      <div class="overflow-auto max-h-[520px]">
        <table class="w-full text-[12px]">
          <thead class="sticky top-0 bg-ink-850 text-ink-400 z-10">
            <tr>
              <th class="px-3 py-2 text-left font-medium w-12">#</th>
              <th class="px-3 py-2 text-left font-medium w-32">
                <div class="flex items-center gap-1.5">
                  <span>结果</span>
                  <select v-model="state.view.filterStatus" class="col-filter" title="按结果筛选">
                    <option value="all">全部</option>
                    <option value="pass">通过</option>
                    <option value="fail">不通过</option>
                    <option value="excluded">未计入</option>
                  </select>
                </div>
              </th>
              <th class="px-3 py-2 text-left font-medium w-40">
                <div class="flex items-center gap-1.5">
                  <span>分组</span>
                  <select v-model="state.view.filterGroup" class="col-filter max-w-[88px]"
                    :title="state.view.filterGroup || '按分组筛选'">
                    <option value="">全部</option>
                    <option v-for="g in groupOptions" :key="g" :value="g">{{ g }}</option>
                  </select>
                </div>
              </th>
              <th class="px-3 py-2 text-left font-medium w-56">
                <div class="flex items-center gap-1.5">
                  <span>分类</span>
                  <select v-model="state.view.filterCategory" class="col-filter max-w-[110px]"
                    :title="state.view.filterCategory || '按分类筛选'">
                    <option value="">全部</option>
                    <option v-for="c in s.categories" :key="c" :value="c">{{ c }}</option>
                  </select>
                </div>
              </th>
              <th class="px-3 py-2 text-left font-medium">被测模型输出</th>
              <th class="px-3 py-2 text-left font-medium">原因</th>
              <th class="px-3 py-2 text-left font-medium w-16">耗时</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="r in pagedResults" :key="r.index"
              class="border-t border-ink-800 hover:bg-ink-850/70 cursor-pointer" @click="open(r.index)">
              <td class="px-3 py-1.5 text-ink-500 align-top">{{ r.index + 1 }}</td>
              <td class="px-3 py-1.5 align-top">
                <span v-if="isExcluded(r)" class="chip border-warn/40 text-warn/90 bg-warn/10">未计入</span>
                <span v-else-if="r.pass === true" class="chip border-pass/40 text-pass bg-pass/10">通过</span>
                <span v-else class="chip border-fail/40 text-fail bg-fail/10">不通过</span>
              </td>
              <td class="px-3 py-1.5 text-ink-400 truncate max-w-[120px] align-top" :title="r.statusLabel">{{ r.statusLabel }}</td>
              <td class="px-3 py-1.5 align-top">
                <span v-for="t in r.errorTypes" :key="t" class="chip border-ink-700 text-ink-300 mr-1 mb-0.5">{{ t }}</span>
                <span v-if="!r.errorTypes.length" class="text-ink-600">—</span>
              </td>
              <td class="px-3 py-1.5 text-ink-300 align-top max-w-[360px]" :title="r.output">
                <span class="line-clamp-3 whitespace-pre-wrap">{{ r.output || (r.targetError ? "（调用失败）" : "—") }}</span>
              </td>
              <td class="px-3 py-1.5 text-ink-400 align-top max-w-[320px]">
                <span class="line-clamp-3">{{ (r.reasons || []).join("；") || r.details || "—" }}</span>
              </td>
              <td class="px-3 py-1.5 text-ink-500 tabular-nums align-top">{{ (r.targetLatencyMs / 1000).toFixed(1) }}s</td>
            </tr>
            <tr v-if="!filteredResults.length">
              <td colspan="7" class="px-3 py-10 text-center text-ink-500">没有符合条件的数据</td>
            </tr>
          </tbody>
        </table>
      </div>

      <div v-if="filteredResults.length" class="flex items-center gap-2 px-4 py-2 border-t border-ink-700/60 text-[12px] text-ink-400 flex-wrap">
        <span class="tabular-nums">共 {{ filteredResults.length }} 条 · 第 {{ rangeStart }}–{{ rangeEnd }} 条</span>
        <span class="flex-1" />
        <select v-model.number="state.view.pageSize" class="field w-[96px] text-[12px]" title="每页条数">
          <option :value="50">50 条/页</option>
          <option :value="100">100 条/页</option>
          <option :value="200">200 条/页</option>
        </select>
        <button class="btn-ghost text-[12px]" :disabled="page <= 1" @click="go(1)">首页</button>
        <button class="btn-ghost text-[12px]" :disabled="page <= 1" @click="go(page - 1)">上一页</button>
        <span class="tabular-nums text-ink-300">{{ page }} / {{ pageCount }}</span>
        <button class="btn-ghost text-[12px]" :disabled="page >= pageCount" @click="go(page + 1)">下一页</button>
        <button class="btn-ghost text-[12px]" :disabled="page >= pageCount" @click="go(pageCount)">末页</button>
        <input v-model="jump" type="number" min="1" :max="pageCount" class="field w-16 text-[12px]"
          placeholder="跳页" @keyup.enter="goJump" />
      </div>
    </div>
  </div>

  <DetailDrawer :index="drawerIndex" @close="drawerIndex = -1" />
</template>
