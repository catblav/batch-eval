<script setup>
import { ref, computed, watch, onMounted, onBeforeUnmount } from "vue";
import {
  state,
  toast,
  launchJobs,
  cancelJob,
  deleteJob,
  resumeJob,
  retryJob,
  clearJob,
  retryableCount,
  openLive,
  jobPassRate,
  runStats,
  planById,
  planProblems,
  datasetMetaById,
  connectionById,
  liveStats,
  isExcluded,
  scheduleSave,
  previewRetention,
  applyRetention,
  retentionLabelOf,
  retentionShortOf,
  RETENTION_OPTIONS,
  isStarred,
  toggleStar,
  resetPlanVersions,
} from "../store";
import { ask } from "@tauri-apps/plugin-dialog";
import Icon from "./Icon.vue";
import Pager from "./Pager.vue";
import DetailDrawer from "./DetailDrawer.vue";
import EmptyState from "./EmptyState.vue";
import HoverTip from "./HoverTip.vue";
import { catColor, catSoft, catLine } from "../utils/catColors";

const now = ref(Date.now());
let tick = null;

const selected = computed(() => state.runForm.selectedPlanIds);
const selectedPlans = computed(() => selected.value.map(planById).filter(Boolean));
const runningJobs = computed(() =>
  state.jobs.list.filter((j) => j.status === "running" || j.status === "cancelling")
);
const finishedJobs = computed(() =>
  state.jobs.list.filter((j) => !["running", "cancelling"].includes(j.status))
);
const concurrency = computed(() => Number(state.runForm.concurrency) || 1);
const totalConcurrency = computed(() => selectedPlans.value.length * concurrency.value);
/// 勾了「自带版号」才显示数字：只选一个方案就显示它的下一版，多选时说明各算各的
const versionPreview = computed(() => {
  if (!state.runForm.versionEnabled) return "";
  if (!selectedPlans.value.length) return "";
  if (selectedPlans.value.length === 1)
    return `第${Math.max(1, Number(selectedPlans.value[0].version) || 1)}版`;
  return "各按方案版号";
});
const canLaunch = computed(() => {
  if (!selectedPlans.value.length) return false;
  if (selectedPlans.value.some((p) => planProblems(p).length)) return false;
  return runningJobs.value.length + selectedPlans.value.length <= 4;
});
const launchHint = computed(() => {
  if (!selectedPlans.value.length) return "勾选要运行的方案";
  if (runningJobs.value.length + selectedPlans.value.length > 4)
    return `最多同时运行 4 个任务，当前已有 ${runningJobs.value.length} 个`;
  if (selectedPlans.value.some((p) => planProblems(p).length)) return "有方案还没配置完整";
  return "";
});

const live = computed(() => state.jobs.live);
const liveOnlyFail = ref(false);
const liveQuery = ref("");
const livePage = ref(1);
const livePageSize = ref(20);
const liveFollow = ref(true);
const detailIndex = ref(-1);

const liveFiltered = computed(() => {
  const q = liveQuery.value.trim().toLowerCase();
  return (live.value?.results || []).filter((r) => {
    if (liveOnlyFail.value && r.pass === true) return false;
    if (!q) return true;
    return `${r.index + 1}\n${r.input}\n${r.output}\n${(r.reasons || []).join(" ")}\n${r.statusLabel}`
      .toLowerCase()
      .includes(q);
  });
});
const livePageCount = computed(() =>
  Math.max(1, Math.ceil(liveFiltered.value.length / Math.max(1, livePageSize.value)))
);
const liveRows = computed(() => {
  const size = Math.max(1, livePageSize.value);
  const p = Math.min(Math.max(1, livePage.value), livePageCount.value);
  return liveFiltered.value.slice((p - 1) * size, p * size);
});
const detailRow = computed(() => {
  if (detailIndex.value < 0) return null;
  return (live.value?.results || []).find((r) => r.index === detailIndex.value) || null;
});

/// 已完成任务的分页：任务攒多了全渲染出来又长又卡，按页切。
const donePage = ref(1);
const donePageSize = ref(20);
const donePageCount = computed(() =>
  Math.max(1, Math.ceil(finishedJobs.value.length / Math.max(1, donePageSize.value)))
);
const doneRows = computed(() => {
  const size = Math.max(1, donePageSize.value);
  const p = Math.min(Math.max(1, donePage.value), donePageCount.value);
  return finishedJobs.value.slice((p - 1) * size, p * size);
});
// 任务被删掉 / 清理后，当前页可能越界，往回收一收
watch([donePageCount, donePageSize], () => {
  if (donePage.value > donePageCount.value) donePage.value = donePageCount.value;
});
const detailDataRow = computed(() => {
  if (detailIndex.value < 0) return null;
  return live.value?.dataset?.rows?.[detailIndex.value] || null;
});

// 明细面板里的任务操作：直接用任务列表里的对象，续跑 / 重跑都要求它还在
const liveJob = computed(() => {
  const id = live.value?.runId;
  if (!id) return null;
  return state.jobs.list.find((j) => j.runId === id) || null;
});
const liveRunning = computed(() => ["running", "cancelling"].includes(live.value?.status));
const liveCanResume = computed(
  () => !!liveJob.value && ["interrupted", "cancelled", "failed"].includes(live.value?.status)
);
const liveRetryCount = computed(() => retryableCount(live.value?.results));
const liveCanRetry = computed(
  () => !!liveJob.value && !liveRunning.value && liveRetryCount.value > 0
);
const liveCanClear = computed(
  () => !!liveJob.value && !liveRunning.value && (live.value?.results?.length || 0) > 0
);
const liveHasActions = computed(
  () => liveRunning.value || liveCanResume.value || liveCanRetry.value || liveCanClear.value
);

/// 被测 / 评测各自取已完成行的中位耗时，再按并发折算剩余时间。
/// 中位数比平均值稳，不会被个别超时行带偏；只取最近 200 条，万级数据下不拖慢进度刷新。
const liveTiming = computed(() => {
  const sample = (live.value?.results || []).slice(-200);
  const median = (arr) => {
    if (!arr.length) return 0;
    const s = [...arr].sort((a, b) => a - b);
    const m = s.length >> 1;
    return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
  };
  const pick = (key) => sample.map((r) => r[key] || 0).filter((v) => v > 0);
  const targetMs = median(pick("targetLatencyMs"));
  const judgeMs = median(pick("judgeLatencyMs"));
  const perRowMs = targetMs + judgeMs;
  const conc = liveJob.value?.concurrency || Number(state.runForm.concurrency) || 1;
  const remain = Math.max(0, (live.value?.total || 0) - (live.value?.done || 0));
  const etaMs = perRowMs > 0 && remain > 0 ? perRowMs * Math.ceil(remain / conc) : 0;
  return { targetMs, judgeMs, perRowMs, etaMs };
});

// 左右两栏的宽度分配：拖中间的分隔条调整，两边都留最小宽度，避免拖到没法看
const MIN_LIVE_PX = 420;
const MIN_LIST_PX = 480;
const splitBox = ref(null);
const dragging = ref(false);

function startResize(e) {
  const box = splitBox.value;
  if (!box) return;
  const total = box.clientWidth;
  if (!total) return;
  const startX = e.clientX;
  const startPct = state.liveWidthPct;
  // 两侧各自的最小宽度换算成占比，再夹一层硬边界，拖不出合理区间
  const minPct = Math.max(0.3, MIN_LIVE_PX / total);
  const maxPct = Math.min(0.78, 1 - MIN_LIST_PX / total);
  dragging.value = true;
  document.body.style.userSelect = "none";

  const onMove = (ev) => {
    const next = startPct + (startX - ev.clientX) / total;
    state.liveWidthPct = Math.min(maxPct, Math.max(minPct, next));
  };
  const onUp = () => {
    dragging.value = false;
    document.body.style.userSelect = "";
    window.removeEventListener("mousemove", onMove);
    window.removeEventListener("mouseup", onUp);
    scheduleSave();
  };
  window.addEventListener("mousemove", onMove);
  window.addEventListener("mouseup", onUp);
}

watch(livePageCount, (n) => {
  if (liveFollow.value) livePage.value = n;
});

watch(
  () => live.value?.runId,
  () => {
    liveOnlyFail.value = false;
    liveQuery.value = "";
    detailIndex.value = -1;
    livePageSize.value = 50;
    liveFollow.value = live.value?.status === "running";
    livePage.value = liveFollow.value ? livePageCount.value : 1;
  }
);

watch(
  () => [liveOnlyFail.value, liveQuery.value, livePageSize.value],
  () => {
    livePage.value = 1;
  }
);

function togglePlan(p) {
  if (planProblems(p).length) return;
  const set = new Set(selected.value);
  if (set.has(p.id)) set.delete(p.id);
  else set.add(p.id);
  state.runForm.selectedPlanIds = state.plans.filter((x) => set.has(x.id)).map((x) => x.id);
}

function selectAllRunnable() {
  state.runForm.selectedPlanIds = state.plans
    .filter((p) => !planProblems(p).length)
    .map((p) => p.id);
}

function launch() {
  const n = launchJobs();
  if (n > 0) toast(`已启动 ${n} 个任务，各自独立运行`, "ok");
}

function resetVersions() {
  if (!selected.value.length) return;
  resetPlanVersions(selected.value);
  toast("选中方案的版号已重置为第 1 版", "ok");
}

function statusText(s) {
  return {
    running: "运行中",
    cancelling: "取消中",
    finished: "已完成",
    cancelled: "已取消",
    failed: "失败",
    interrupted: "已中断",
  }[s] || s;
}

function statusClass(s) {
  if (s === "running") return "chip-accent";
  if (s === "finished") return "chip-pass";
  if (s === "failed") return "chip-fail";
  if (s === "interrupted" || s === "cancelling") return "chip-warn";
  return "chip-muted";
}

function progressOf(job) {
  const total = job.total || job.summary?.total || 0;
  const p = state.jobs.progress[job.runId];
  let done = p?.done;
  if (done === undefined) {
    done = job.summary?.total || 0;
  }
  return { done: Math.min(done, total), total };
}

function percentOf(job) {
  const { done, total } = progressOf(job);
  return total ? Math.min(100, (done / total) * 100) : 0;
}

function rateText(job) {
  const r = jobPassRate(job);
  if (r === null || r === undefined) return "—";
  return `${(r * 100).toFixed(r === 1 || r === 0 ? 0 : 1)}%`;
}

/// 运行中任务的通过 / 失败数字：走 runStats 实时统计，结果一到就跟着变
const statsOf = (j) => runStats(j);

// 通过率颜色：60% 及以上绿色，以下红色，没有有效样本时灰色
function rateTone(rate) {
  if (rate === null || rate === undefined) return "text-weak";
  return rate >= 0.6 ? "text-pass" : "text-fail";
}

// 通过率那根横条的宽度：扫一眼就能看出哪几行跑得好，不用逐行读数字
function ratePct(job) {
  const r = jobPassRate(job);
  return r === null || r === undefined ? 0 : Math.round(r * 100);
}

function fmtDuration(ms) {
  if (!ms || ms < 0) return "—";
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m${String(s % 60).padStart(2, "0")}s`;
  return `${Math.floor(m / 60)}h${String(m % 60).padStart(2, "0")}m`;
}

function elapsedOf(job) {
  const end = job.finishedAt || now.value;
  return fmtDuration(end - (job.startedAt || end));
}

/// 时间列：日期和时间同格显示。今天 / 昨天用相对说法加具体时间，再往前写具体日期。
/// 今天绿、昨天橙——扫一眼就能把新任务和旧任务分开。
function timeCellOf(job) {
  const ts = job.finishedAt || job.startedAt;
  if (!ts) return { text: "—", tone: "text-weak" };
  const d = new Date(ts);
  const p = (n) => String(n).padStart(2, "0");
  const hm = `${p(d.getHours())}:${p(d.getMinutes())}`;
  const t = new Date(now.value);
  const dayStart = new Date(t.getFullYear(), t.getMonth(), t.getDate()).getTime();
  if (ts >= dayStart) return { text: `今天 ${hm}`, tone: "text-pass" };
  if (ts >= dayStart - 86400000) return { text: `昨天 ${hm}`, tone: "text-warn" };
  const md = `${d.getMonth() + 1}月${d.getDate()}日`;
  return {
    text: d.getFullYear() === t.getFullYear() ? `${md} ${hm}` : `${d.getFullYear()}年${md} ${hm}`,
    tone: "text-sub",
  };
}

function modelsOf(job) {
  const target = job.targetProfile?.model || connectionById(planById(job.planId)?.targetId)?.model || "—";
  const judge = job.judgeProfile?.model || connectionById(planById(job.planId)?.judgeId)?.model || "—";
  return `${target} → ${judge}`;
}

async function view(job) {
  detailIndex.value = -1;
  await openLive(job.runId);
}

async function stop(job) {
  try {
    await cancelJob(job.runId);
  } catch (e) {
    toast("取消失败：" + e, "err");
  }
}

async function resume(job) {
  try {
    await resumeJob(job);
  } catch (e) {
    toast("续跑失败：" + e, "err", 8000);
  }
}

async function retry(job) {
  try {
    await retryJob(job);
  } catch (e) {
    toast("重跑失败：" + e, "err", 8000);
  }
}

async function stopLive() {
  if (!liveJob.value) return;
  await stop(liveJob.value);
}

async function resumeLive() {
  if (!liveJob.value) return;
  await resume(liveJob.value);
}

async function retryLive() {
  if (!liveJob.value) return;
  await retry(liveJob.value);
}

async function clearLive() {
  const job = liveJob.value;
  if (!job) return;
  const n = live.value?.results?.length || 0;
  const ok = await ask(
    `清空「${job.title || job.planName}」的 ${n} 条结果？\n\n结果文件会清空，任务回到「已中断」，之后可以续跑重来。任务记录、数据集快照和提示词快照都保留。`,
    { title: "清空结果", kind: "warning", okLabel: "清空", cancelLabel: "取消" }
  );
  if (!ok) return;
  try {
    await clearJob(job);
  } catch (e) {
    toast("清空失败：" + e, "err", 8000);
  }
}

async function remove(job) {
  const ok = await ask(`删除任务「${job.title || job.planName}」？\n\n它的结果文件和数据集快照会一起删除，无法恢复。`, {
    title: "删除任务",
    kind: "warning",
    okLabel: "删除",
    cancelLabel: "取消",
  });
  if (!ok) return;
  try {
    await deleteJob(job);
  } catch (e) {
    toast("删除失败：" + e, "err");
  }
}

function closeLive() {
  state.jobs.live = null;
  detailIndex.value = -1;
}

function openDetail(r) {
  detailIndex.value = r.index;
}

// ---------- 保存范围 ----------
const retentionOpen = ref(false);
const retentionBusy = ref(false);
const retention = computed(() => Number(state.retentionDays) || 0);

function toggleRetention() {
  retentionOpen.value = !retentionOpen.value;
}
function closeRetention() {
  retentionOpen.value = false;
}

async function pickRetention(days) {
  retentionOpen.value = false;
  if (retentionBusy.value) return;
  const next = Number(days) || 0;
  if (next === retention.value) return;
  retentionBusy.value = true;
  try {
    if (next > 0) {
      const n = await previewRetention(next);
      if (n > 0) {
        const ok = await ask(
          `改为「${retentionLabelOf(next)}」会立刻清理 ${n} 条超出范围的任务记录，无法恢复。`,
          { title: "清理任务记录", kind: "warning", okLabel: "清理并保存", cancelLabel: "取消" }
        );
        if (!ok) return;
      }
    }
    const removed = await applyRetention(next);
    toast(
      `保存范围已改为「${retentionLabelOf(next)}」` +
        (removed > 0 ? `，清理了 ${removed} 条过期记录` : ""),
      "info",
      5000
    );
  } finally {
    retentionBusy.value = false;
  }
}

onMounted(() => {
  tick = setInterval(() => (now.value = Date.now()), 1000);
  window.addEventListener("click", closeRetention);
});
onBeforeUnmount(() => {
  clearInterval(tick);
  window.removeEventListener("click", closeRetention);
});
</script>

<template>
  <div class="h-full flex flex-col card">
    <div class="h-14 px-2.5 flex items-center gap-2 shrink-0 border-b border-line/60">
      <h1 class="page-title">运行</h1>
      <span class="text-[12px] text-weak tabular-nums">
        {{ runningJobs.length ? `${runningJobs.length} 个进行中` : "没有进行中的任务" }}
      </span>
      <span class="flex-1" />
      <div class="relative">
        <button class="btn-ghost btn-sm" :disabled="retentionBusy" @click.stop="toggleRetention">
          <Icon name="clock" :size="13" />
          保存范围：{{ retentionShortOf(retention) }}
        </button>
        <div
          v-if="retentionOpen"
          class="absolute right-0 top-8 z-20 w-72 py-1 rounded border border-line bg-panel shadow-sm"
          @click.stop
        >
          <button
            v-for="o in RETENTION_OPTIONS"
            :key="o.days"
            class="w-full text-left px-3 h-7 text-[12.5px] flex items-center gap-1.5 hover:bg-wash"
            @click="pickRetention(o.days)"
          >
            <Icon v-if="o.days === retention" name="check" :size="12" class="text-accent shrink-0" />
            <span v-else class="w-3 shrink-0" />
            {{ o.label }}
          </button>
          <div class="my-1 border-t border-line/60"></div>
          <p class="px-3 pb-1.5 text-[11.5px] leading-relaxed text-weak">
            超出范围的记录会在应用启动时、以及每次任务结束后自动清理一次。清理后运行页和分析页都看不到，无法恢复。正在运行的任务不受影响。
          </p>
        </div>
      </div>
      <button class="btn btn-sm" :class="{ 'bg-wash border-weak/40': state.runForm.launchOpen }"
        @click="state.runForm.launchOpen = !state.runForm.launchOpen">
        <Icon :name="state.runForm.launchOpen ? 'close' : 'plus'" :size="14" />
        {{ state.runForm.launchOpen ? "收起" : "新建任务" }}
      </button>
    </div>

    <!-- 启动面板 -->
    <div v-if="state.runForm.launchOpen" class="shrink-0 border-b border-line/60 bg-wash/40">
      <div class="px-2.5 py-3 border-b border-line/60 flex items-center gap-2">
        <Icon name="sliders" :size="14" class="text-weak shrink-0" />
        <span class="text-[13px] font-medium">选择方案</span>
        <span class="hint">每个方案启动一个独立任务，各自使用自己的数据集和连接</span>
        <span class="flex-1" />
        <button class="btn-ghost btn-sm" @click="selectAllRunnable">
          <Icon name="check" :size="12" />
          全选可运行
        </button>
        <button class="btn-ghost btn-sm" :disabled="!selected.length" @click="state.runForm.selectedPlanIds = []">
          <Icon name="close" :size="12" />
          清空
        </button>
      </div>

      <div class="max-h-[240px] overflow-y-auto">
        <EmptyState
          v-if="!state.plans.length"
          size="sm"
          icon="sliders"
          title="还没有方案"
          hint="先去提示词配置页新建一个，再回来启动任务。"
        />
        <label
          v-for="p in state.plans"
          :key="p.id"
          class="flex items-center gap-3 px-2.5 h-11 border-b border-line/50 cursor-pointer"
          :class="planProblems(p).length ? 'opacity-60 cursor-not-allowed' : 'hover:bg-accent/[0.04]'"
        >
          <input
            type="checkbox"
            class="w-3.5 h-3.5"
            :checked="selected.includes(p.id)"
            :disabled="planProblems(p).length > 0"
            @change="togglePlan(p)"
          />
          <span class="w-[172px] shrink-0 flex items-center gap-1.5 min-w-0">
            <span class="text-[13px] font-medium truncate">{{ p.name }}</span>
            <span v-if="state.runForm.versionEnabled && selected.includes(p.id)"
              class="chip !h-5 !px-1.5 shrink-0 tabular-nums">
              第{{ p.version || 1 }}版
            </span>
          </span>
          <span class="w-[170px] shrink-0 text-[12px] text-sub truncate">
            {{ datasetMetaById(p.datasetId)?.name || "未选数据集" }}
          </span>
          <span class="flex-1 min-w-0 text-[12px] text-weak truncate">
            {{ connectionById(p.targetId)?.model || "未选被测" }}
            →
            {{ connectionById(p.judgeId)?.model || "未选评测" }}
          </span>
          <span v-if="planProblems(p).length" class="text-[11.5px] text-warn text-right max-w-[260px] truncate"
            :title="planProblems(p).join('、')">
            {{ planProblems(p).join("、") }}
          </span>
          <span v-else class="text-[11.5px] text-weak shrink-0 tabular-nums">
            {{ datasetMetaById(p.datasetId)?.rows?.toLocaleString() || 0 }} 行
          </span>
        </label>
      </div>

      <div class="px-2.5 py-3 flex items-end gap-3 flex-wrap">
        <div>
          <label class="label">名称后缀</label>
          <input v-model="state.runForm.titleSuffix" class="field w-[200px]"
            placeholder="留空为「方案名」" />
        </div>
        <div>
          <label class="label">自带版号</label>
          <div class="flex items-center gap-1.5">
            <label class="flex items-center gap-1.5 cursor-pointer select-none">
              <input type="checkbox" class="w-3.5 h-3.5" v-model="state.runForm.versionEnabled" />
              <span class="inline-flex items-center justify-center h-[30px] w-[92px] rounded-md
                  border border-line bg-panel text-[12.5px] tabular-nums whitespace-nowrap"
                :class="versionPreview ? 'text-main' : 'text-weak'">
                {{ versionPreview || "第  版" }}
              </span>
            </label>
            <HoverTip text="版号重置为第 1 版" :min-length="0" :delay="400">
              <button class="icon-btn" :disabled="!selected.length" @click="resetVersions">
                <Icon name="refresh" :size="13" />
              </button>
            </HoverTip>
          </div>
        </div>
        <div>
          <label class="label">并发</label>
          <input v-model.number="state.runForm.concurrency" type="number" min="1" max="32"
            class="field w-16 tabular-nums" />
        </div>
        <div>
          <label class="label">失败重试</label>
          <input v-model.number="state.runForm.maxRetries" type="number" min="0" max="10"
            class="field w-16 tabular-nums" />
        </div>
        <div class="pb-1.5 text-[12px]">
          <span :class="totalConcurrency > 8 ? 'text-warn' : 'text-sub'">
            {{ selectedPlans.length }} 个任务 × {{ concurrency }} 并发 = {{ totalConcurrency }} 路并发
          </span>
          <span v-if="totalConcurrency > 8" class="text-weak"> · 容易被上游限流</span>
        </div>
        <span class="flex-1" />
        <span v-if="launchHint" class="pb-2 text-[11.5px] text-warn">{{ launchHint }}</span>
        <button class="btn-primary" :disabled="!canLaunch" @click="launch">
          <Icon name="play" :size="14" />
          启动 {{ selectedPlans.length || "" }}
        </button>
      </div>
    </div>

    <div ref="splitBox" class="flex-1 min-h-0 flex">
      <div class="flex-1 min-w-0 min-h-0 flex flex-col">
        <EmptyState
          v-if="!state.jobs.list.length"
          class="flex-1 min-h-0"
          icon="play"
          title="还没有任务"
          hint="勾选一个或多个方案，同时跑多组互不影响的评测。"
        >
          <button class="btn-primary" @click="state.runForm.launchOpen = true">
            <Icon name="plus" :size="14" />
            新建任务
          </button>
        </EmptyState>

        <!-- 任务列表滚动，分页条钉在这一栏底部 -->
        <div v-else class="flex-1 min-h-0 overflow-auto">
          <div v-if="runningJobs.length">
            <div class="px-2.5 pt-4 pb-1.5 flex items-center gap-1.5">
              <span class="dot bg-pass animate-pulse" />
              <span class="group-title">运行中 · {{ runningJobs.length }}</span>
            </div>
            <table class="data-table table-fixed">
              <!-- 表头行拉到 44px，和右侧明细的头部条一样高 -->
              <thead class="[&_th]:h-11">
                <tr>
                  <th :class="live ? 'w-[28%]' : 'w-[20%]'">方案</th>
                  <th v-if="!live" class="w-[15%]">数据集</th>
                  <th v-if="!live" class="w-[19%]">被测 → 评测</th>
                  <th v-if="!live" class="w-[92px]">开始时间</th>
                  <th class="w-[24%]">进度</th>
                  <th class="w-20">通过率</th>
                  <th class="w-[92px]">通过 / 失败</th>
                  <th v-if="!live" class="w-16">耗时</th>
                  <th class="text-right" :class="!live && 'w-[102px]'">操作</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="j in runningJobs" :key="j.runId" @click="view(j)"
                  :class="{ 'row-starred': isStarred(j.runId) }">
                  <td>
                    <div class="flex items-center gap-1.5 min-w-0">
                      <span class="font-medium truncate">{{ j.title || j.planName }}</span>
                      <span class="chip !h-5 !px-1.5 shrink-0" :class="statusClass(j.status)">
                        {{ statusText(j.status) }}
                      </span>
                    </div>
                  </td>
                  <td v-if="!live" class="text-sub truncate">{{ j.datasetName || "—" }}</td>
                  <td v-if="!live" class="text-weak truncate">{{ modelsOf(j) }}</td>
                  <td v-if="!live" class="text-[12px] tabular-nums whitespace-nowrap" :class="timeCellOf(j).tone">
                    {{ timeCellOf(j).text }}
                  </td>
                  <td>
                    <div class="flex items-center gap-2">
                      <div class="flex-1 h-1.5 rounded-full bg-line overflow-hidden min-w-[48px]">
                        <div class="h-full bg-accent transition-all"
                          :class="{ 'progress-live': j.status === 'running' }"
                          :style="{ width: percentOf(j).toFixed(1) + '%' }" />
                      </div>
                      <span class="text-[11.5px] text-sub tabular-nums shrink-0">
                        {{ progressOf(j).done }}/{{ progressOf(j).total }}
                      </span>
                    </div>
                  </td>
                  <td class="text-right">
                    <span class="text-[13px] font-semibold tabular-nums" :class="rateTone(jobPassRate(j))">
                      {{ rateText(j) }}
                    </span>
                  </td>
                  <td class="text-[12px] tabular-nums whitespace-nowrap">
                    <span class="text-pass">{{ statsOf(j).passed }}</span>
                    <span class="text-weak"> / </span>
                    <span class="text-fail">{{ statsOf(j).failed }}</span>
                    <span v-if="statsOf(j).excluded" class="text-warn"> · {{ statsOf(j).excluded }} 未计入</span>
                  </td>
                  <td v-if="!live" class="text-sub tabular-nums">{{ elapsedOf(j) }}</td>
                  <td class="whitespace-nowrap">
                    <div class="flex items-center justify-start gap-0.5" @click.stop>
                      <HoverTip :text="isStarred(j.runId) ? '取消重点' : '标为重点'" :min-length="0" :delay="400">
                        <button class="icon-btn" :class="isStarred(j.runId) ? 'text-warn' : 'text-weak'"
                          @click="toggleStar(j.runId)">
                          <Icon :name="isStarred(j.runId) ? 'star-filled' : 'star'" :size="14" />
                        </button>
                      </HoverTip>
                      <HoverTip text="查看明细" :min-length="0" :delay="400">
                        <button class="icon-btn" @click="view(j)">
                          <Icon name="eye" :size="15" />
                        </button>
                      </HoverTip>
                      <HoverTip v-if="j.status === 'running'" text="停止任务" :min-length="0" :delay="400">
                        <button class="icon-btn text-fail" @click="stop(j)">
                          <Icon name="stop" :size="14" />
                        </button>
                      </HoverTip>
                    </div>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>

          <!-- 上面有「运行中」那一段时才需要这段间距。没有的时候加了会变成表头和标题栏之间
               一段 16px 的空白，表头文字看着就贴着下边那条线 -->
          <div v-if="finishedJobs.length" :class="{ 'pt-4': runningJobs.length }">
            <table class="data-table table-fixed">
              <thead class="[&_th]:h-11">
                <tr>
                  <th :class="live ? 'w-[34%]' : 'w-[22%]'">方案</th>
                  <th v-if="!live" class="w-[15%]">数据集</th>
                  <th v-if="!live" class="w-[17%]">被测 → 评测</th>
                  <th v-if="!live" class="w-[92px]">完成时间</th>
                  <th class="w-20">通过率</th>
                  <th :class="live ? 'w-[92px]' : 'w-[104px]'">通过 / 失败</th>
                  <th v-if="!live" class="w-16">耗时</th>
                  <th class="text-right" :class="!live && 'w-[158px]'">操作</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="j in doneRows" :key="j.runId" @click="view(j)"
                  :class="{ 'row-starred': isStarred(j.runId) }">
                  <td>
                    <div class="flex items-center gap-1.5 min-w-0">
                      <span class="font-medium truncate">{{ j.title || j.planName }}</span>
                      <span class="chip !h-5 !px-1.5 shrink-0" :class="statusClass(j.status)">
                        {{ statusText(j.status) }}
                      </span>
                    </div>
                    <div v-if="j.title && j.title !== j.planName" class="text-[11px] text-weak truncate">{{ j.planName }}</div>
                  </td>
                  <td v-if="!live" class="text-sub truncate">{{ j.datasetName || "—" }}</td>
                  <td v-if="!live" class="text-weak truncate">{{ modelsOf(j) }}</td>
                  <td v-if="!live" class="text-[12px] tabular-nums whitespace-nowrap" :class="timeCellOf(j).tone">
                    {{ timeCellOf(j).text }}
                  </td>
                  <td>
                    <div class="text-[13px] font-semibold tabular-nums leading-tight" :class="rateTone(jobPassRate(j))">
                      {{ rateText(j) }}
                    </div>
                    <div class="bar mt-1.5 w-12">
                      <div class="bar-fill bg-pass" :style="{ width: ratePct(j) + '%' }" />
                    </div>
                  </td>
                  <td class="text-[12px] tabular-nums whitespace-nowrap">
                    <span class="text-pass">{{ j.summary?.passed ?? 0 }}</span>
                    <span class="text-weak"> / </span>
                    <span class="text-fail">{{ j.summary?.failed ?? 0 }}</span>
                    <span v-if="j.summary?.excluded" class="text-warn"> · {{ j.summary.excluded }} 未计入</span>
                  </td>
                  <td v-if="!live" class="text-sub tabular-nums">{{ elapsedOf(j) }}</td>
                  <td class="whitespace-nowrap">
                    <div class="flex items-center justify-start gap-0.5" @click.stop>
                      <HoverTip :text="isStarred(j.runId) ? '取消重点' : '标为重点'" :min-length="0" :delay="400">
                        <button class="icon-btn" :class="isStarred(j.runId) ? 'text-warn' : 'text-weak'"
                          @click="toggleStar(j.runId)">
                          <Icon :name="isStarred(j.runId) ? 'star-filled' : 'star'" :size="14" />
                        </button>
                      </HoverTip>
                      <HoverTip text="查看明细" :min-length="0" :delay="400">
                        <button class="icon-btn" @click="view(j)">
                          <Icon name="eye" :size="15" />
                        </button>
                      </HoverTip>
                      <HoverTip v-if="['interrupted', 'cancelled', 'failed'].includes(j.status)"
                        text="断点续跑" :min-length="0" :delay="400">
                        <button class="icon-btn text-accent hover:text-accent" @click="resume(j)">
                          <Icon name="play" :size="13" />
                        </button>
                      </HoverTip>
                      <HoverTip v-if="j.summary?.failed || j.summary?.excluded"
                        :text="`重跑失败项（${(j.summary?.failed || 0) + (j.summary?.excluded || 0)} 条）`"
                        :min-length="0" :delay="400">
                        <button class="icon-btn text-accent hover:text-accent" @click="retry(j)">
                          <Icon name="refresh" :size="14" />
                        </button>
                      </HoverTip>
                      <HoverTip text="删除任务" :min-length="0" :delay="400">
                        <button class="icon-btn text-fail" @click="remove(j)">
                          <Icon name="trash" :size="14" />
                        </button>
                      </HoverTip>
                    </div>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
          <p v-if="!finishedJobs.length" class="px-2.5 pt-4 text-[11.5px] text-weak">
            完成任务后出现在这里
          </p>
        </div>

        <!-- 不满一页也显示：页签数量和总数本身就是信息，隐藏了反而不知道一共多少条 -->
        <Pager
          :total="finishedJobs.length"
          :page="donePage"
          :page-size="donePageSize"
          :sizes="[10, 20, 50]"
          @update:page="donePage = $event"
          @update:page-size="donePageSize = $event; donePage = 1"
        />
      </div>

      <!-- 拖这个分隔条调整左右两栏的占比：视觉上是一条 1px 的线，热区左右各铺开 6px -->
      <div v-if="live" class="relative w-px shrink-0 cursor-col-resize transition-colors"
        :class="dragging ? 'bg-accent' : 'bg-line hover:bg-accent/60'" title="拖动调整宽度"
        @mousedown.prevent="startResize">
        <span class="absolute inset-y-0 -left-1.5 -right-1.5" />
      </div>

      <!-- 实时明细：从右侧并排展开，任务列表让出宽度 -->
      <div v-if="live" class="shrink-0 min-h-0 flex flex-col"
        :style="{ width: (state.liveWidthPct * 100).toFixed(2) + '%' }">
        <!-- 和左边分组标题同一行、同样的内边距，两栏从「方案 / 通过率」那一行开始齐平。
             退出键绝对定位，不参与这一行的高度计算，否则这一栏会比左边高一截，两边的表头就错开了 -->
        <div class="relative px-2.5 pt-4 pb-1.5 shrink-0 flex items-center gap-1.5">
          <span class="group-title">实时明细</span>
          <button class="icon-btn absolute right-1.5 top-2.5" title="关闭明细" @click="closeLive">
            <Icon name="close" :size="15" />
          </button>
        </div>
        <!-- 右侧留出滚动条那 10px，否则这一条会比下面的表格宽出一截 -->
        <div class="h-11 pl-2.5 pr-[12.5px] flex items-center gap-2 border-b border-line/60 shrink-0">
          <span class="text-[13px] font-medium truncate min-w-0 flex-1"
            :title="`${live.planName}${live.datasetName ? ' · ' + live.datasetName : ''}`">
            {{ live.title || live.planName || "任务明细" }}
          </span>
          <span class="chip !h-5 !px-1.5 shrink-0" :class="statusClass(live.status)">
            {{ statusText(live.status) }}
          </span>
          <span class="text-[11.5px] text-sub tabular-nums shrink-0">
            {{ live.done }} / {{ live.total || "—" }}
          </span>
          <span v-if="liveRunning && liveTiming.perRowMs"
            class="hidden xl:inline text-[11.5px] text-weak tabular-nums shrink-0"
            :title="`单条耗时中位：被测 ${(liveTiming.targetMs / 1000).toFixed(1)}s + 评测 ${(liveTiming.judgeMs / 1000).toFixed(1)}s`">
            单条 {{ (liveTiming.perRowMs / 1000).toFixed(1) }}s
          </span>
          <span v-if="liveRunning && liveTiming.etaMs"
            class="hidden xl:inline text-[11.5px] text-weak tabular-nums shrink-0">
            预计还需 {{ fmtDuration(liveTiming.etaMs) }}
          </span>
          <span class="text-[11.5px] text-pass tabular-nums shrink-0">通过 {{ liveStats.passed }}</span>
          <span class="text-[11.5px] text-fail tabular-nums shrink-0">失败 {{ liveStats.failed }}</span>
          <span v-if="liveStats.excluded" class="text-[11.5px] text-warn tabular-nums shrink-0">
            未计 {{ liveStats.excluded }}
          </span>
          <span class="text-[11.5px] tabular-nums shrink-0"
            :class="liveStats.valid
              ? (liveStats.passed / liveStats.valid >= 0.6 ? 'text-pass' : 'text-fail')
              : 'text-weak'">
            通过率 {{ liveStats.valid ? ((liveStats.passed / liveStats.valid) * 100).toFixed(1) + "%" : "—" }}
          </span>
        </div>

        <div v-if="live.error" class="shrink-0 px-2.5 py-1.5 text-[12px] text-fail bg-fail/5 border-b border-line/60 truncate"
          :title="live.error">
          {{ live.error }}
        </div>

        <!-- 面板宽度可拖，窄的时候让这一行换行，不硬挤成一团 -->
        <div class="min-h-9 pl-2.5 pr-[12.5px] py-1.5 flex items-center gap-3 flex-wrap border-b border-line/60 shrink-0">
          <label class="flex items-center gap-1.5 text-[12px] text-sub cursor-pointer shrink-0">
            <input v-model="liveOnlyFail" type="checkbox" class="w-3.5 h-3.5" />
            只看未通过
          </label>
          <div class="relative shrink-0">
            <Icon name="search" :size="13"
              class="absolute left-2 top-1/2 -translate-y-1/2 text-weak pointer-events-none" />
            <input v-model="liveQuery" class="field pl-7 !h-7 w-40" placeholder="搜索输出或原因" />
          </div>
          <span class="text-[11.5px] text-weak tabular-nums shrink-0">{{ liveFiltered.length }} 条</span>
          <span class="flex-1 min-w-2" />
          <button v-if="live.status === 'running' && !liveFollow" class="btn-ghost btn-sm shrink-0"
            @click="liveFollow = true; livePage = livePageCount">
            回到最新
          </button>
          <template v-if="liveHasActions">
            <span class="w-px h-4 bg-line shrink-0" />
            <template v-if="liveCanResume || liveCanRetry">
              <label class="flex items-center gap-1.5 text-[11.5px] text-sub whitespace-nowrap shrink-0">
                并发
                <input v-model.number="state.runForm.concurrency" type="number" min="1" max="32"
                  class="field !h-7 w-14 px-2 tabular-nums" />
              </label>
              <label class="flex items-center gap-1.5 text-[11.5px] text-sub whitespace-nowrap shrink-0"
                title="失败项自动重试次数">
                失败重试
                <input v-model.number="state.runForm.maxRetries" type="number" min="0" max="10"
                  class="field !h-7 w-14 px-2 tabular-nums" />
              </label>
            </template>
            <button v-if="liveRunning" class="btn btn-danger btn-sm shrink-0"
              :disabled="live.status === 'cancelling'" @click="stopLive">
              <Icon name="stop" :size="11" />
              {{ live.status === "cancelling" ? "取消中…" : "停止" }}
            </button>
            <button v-if="liveCanResume" class="btn-blue btn-sm shrink-0" @click="resumeLive">
              <Icon name="play" :size="12" />
              续跑
            </button>
            <button v-if="liveCanRetry" class="btn-blue btn-sm shrink-0" @click="retryLive">
              <Icon name="refresh" :size="12" />
              重跑失败项（{{ liveRetryCount }}）
            </button>
            <button v-if="liveCanClear" class="btn-danger btn-sm shrink-0" @click="clearLive">
              <Icon name="trash" :size="11" />
              清空结果
            </button>
          </template>
        </div>

        <!-- 滚动条始终占位，表格宽度恒定，和上面的头部条、工具栏对齐 -->
        <div class="flex-1 min-h-0 overflow-auto [scrollbar-gutter:stable]">
          <table class="data-table table-fixed min-w-[460px]">
            <thead>
              <tr>
                <th class="w-12">#</th>
                <th class="w-20">结果</th>
                <th class="w-24">分组</th>
                <th class="w-[132px]">分类</th>
                <th>原因</th>
                <th class="w-16">用时</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="r in liveRows" :key="r.index" :class="{ 'row-active': detailIndex === r.index }"
                @click="openDetail(r)">
                <td class="text-weak tabular-nums">{{ r.index + 1 }}</td>
                <td>
                  <span v-if="isExcluded(r)" class="chip-warn !h-5 !px-1.5">未计入</span>
                  <span v-else-if="r.pass === true" class="chip-pass !h-5 !px-1.5">通过</span>
                  <span v-else class="chip-fail !h-5 !px-1.5">不通过</span>
                </td>
                <td class="text-sub truncate">{{ r.statusLabel }}</td>
                <td>
                  <span v-for="t in r.errorTypes || []" :key="t" class="chip !h-5 !px-1.5 mr-1"
                    :style="{ color: catColor(t), background: catSoft(t), borderColor: catLine(t) }">{{ t }}</span>
                </td>
                <td class="text-sub truncate" :title="(r.reasons || []).join('；')">
                  {{ (r.reasons || []).join("；") }}
                </td>
                <td class="text-sub tabular-nums">
                  {{ (((r.targetLatencyMs || 0) + (r.judgeLatencyMs || 0)) / 1000).toFixed(1) }}s
                </td>
              </tr>
              <tr v-if="!liveFiltered.length">
                <td colspan="6" class="py-3">
                  <template v-if="live.loading">
                    <span v-for="n in 3" :key="n" class="skeleton block h-3 mb-2 last:mb-0" />
                  </template>
                  <p v-else class="text-center text-weak py-5">
                    {{ live.status === "running" ? "等待结果…" : "没有结果" }}
                  </p>
                </td>
              </tr>
            </tbody>
          </table>
        </div>

        <Pager :total="liveFiltered.length" :page="livePage" :page-size="livePageSize"
          :sizes="[10, 20, 50]"
          @update:page="liveFollow = false; livePage = $event"
          @update:page-size="livePageSize = $event; liveFollow = false" />
      </div>
    </div>

    <DetailDrawer :row="detailRow" :data-row="detailDataRow" @close="detailIndex = -1" />
  </div>
</template>
