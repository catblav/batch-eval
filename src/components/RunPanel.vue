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
  openLive,
  jobPassRate,
  planById,
  planProblems,
  datasetMetaById,
  connectionById,
  liveStats,
  isExcluded,
} from "../store";
import { ask } from "@tauri-apps/plugin-dialog";
import Icon from "./Icon.vue";
import Pager from "./Pager.vue";
import DetailDrawer from "./DetailDrawer.vue";

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
const livePageSize = ref(50);
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
const detailDataRow = computed(() => {
  if (detailIndex.value < 0) return null;
  return live.value?.dataset?.rows?.[detailIndex.value] || null;
});

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
  if (s === "running") return "text-accent border-accent/30 bg-accent/5";
  if (s === "finished") return "text-pass border-pass/30 bg-pass/5";
  if (s === "failed") return "text-fail border-fail/30 bg-fail/5";
  if (s === "interrupted" || s === "cancelling") return "text-warn border-warn/30 bg-warn/5";
  return "text-sub border-line bg-wash";
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

onMounted(() => {
  tick = setInterval(() => (now.value = Date.now()), 1000);
});
onBeforeUnmount(() => clearInterval(tick));
</script>

<template>
  <div class="h-full flex flex-col bg-page">
    <div class="h-14 px-5 flex items-center gap-2 shrink-0 bg-page">
      <h1 class="text-[15px] font-semibold">运行</h1>
      <span class="text-[12px] text-weak tabular-nums">
        {{ runningJobs.length ? `${runningJobs.length} 个进行中` : "没有进行中的任务" }}
      </span>
      <span class="flex-1" />
      <button class="btn btn-sm" :class="{ 'bg-wash border-weak/40': state.runForm.launchOpen }"
        @click="state.runForm.launchOpen = !state.runForm.launchOpen">
        <Icon :name="state.runForm.launchOpen ? 'close' : 'plus'" :size="14" />
        {{ state.runForm.launchOpen ? "收起" : "新建任务" }}
      </button>
    </div>

    <!-- 启动面板 -->
    <div v-if="state.runForm.launchOpen" class="shrink-0 border-y border-line bg-panel">
      <div class="px-5 py-3 border-b border-line flex items-center gap-2">
        <span class="text-[13px] font-medium">选择方案</span>
        <span class="hint">每个方案启动一个独立任务，各自使用自己的数据集和连接</span>
        <span class="flex-1" />
        <button class="btn-ghost btn-sm" @click="selectAllRunnable">全选可运行</button>
        <button class="btn-ghost btn-sm" :disabled="!selected.length" @click="state.runForm.selectedPlanIds = []">
          清空
        </button>
      </div>

      <div class="max-h-[240px] overflow-y-auto">
        <div v-if="!state.plans.length" class="px-5 py-8 text-center hint">
          还没有方案，先去方案页新建一个。
        </div>
        <label
          v-for="p in state.plans"
          :key="p.id"
          class="flex items-center gap-3 px-5 h-11 border-b border-line/70 cursor-pointer"
          :class="planProblems(p).length ? 'opacity-60 cursor-not-allowed' : 'hover:bg-wash/70'"
        >
          <input
            type="checkbox"
            class="w-3.5 h-3.5"
            :checked="selected.includes(p.id)"
            :disabled="planProblems(p).length > 0"
            @change="togglePlan(p)"
          />
          <span class="w-[150px] shrink-0 text-[13px] font-medium truncate">{{ p.name }}</span>
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

      <div class="px-5 py-3 flex items-end gap-3 flex-wrap">
        <div>
          <label class="label">名称前缀</label>
          <input v-model="state.runForm.titlePrefix" class="field w-[220px]"
            placeholder="留空为「方案名 · 日期」" />
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

    <div class="flex-1 min-h-0 flex flex-col">
      <div class="flex-1 min-h-0 overflow-auto" :class="{ 'max-h-[46%] shrink-0': live }">
        <div v-if="!state.jobs.list.length" class="h-full flex flex-col items-center justify-center gap-2">
          <p class="text-[13px] text-sub">还没有任务</p>
          <p class="hint">勾选一个或多个方案，同时跑多组互不影响的评测。</p>
          <button class="btn-primary mt-2" @click="state.runForm.launchOpen = true">新建任务</button>
        </div>

        <template v-else>
          <div v-if="runningJobs.length">
            <div class="px-5 pt-3 pb-1.5 text-[12px] text-sub">运行中 · {{ runningJobs.length }}</div>
            <table class="data-table">
              <thead>
                <tr>
                  <th class="w-[22%]">方案</th>
                  <th class="w-[15%]">数据集</th>
                  <th class="w-[19%]">被测 → 评测</th>
                  <th class="w-[17%]">进度</th>
                  <th class="w-14 text-right">通过率</th>
                  <th class="w-16 text-right">耗时</th>
                  <th class="w-[104px]" />
                </tr>
              </thead>
              <tbody>
                <tr v-for="j in runningJobs" :key="j.runId" @click="view(j)">
                  <td>
                    <div class="flex items-center gap-1.5 min-w-0">
                      <span class="font-medium truncate">{{ j.planName || j.title }}</span>
                      <span class="chip !h-5 !px-1.5 shrink-0" :class="statusClass(j.status)">
                        {{ statusText(j.status) }}
                      </span>
                    </div>
                  </td>
                  <td class="text-sub truncate">{{ j.datasetName || "—" }}</td>
                  <td class="text-weak truncate">{{ modelsOf(j) }}</td>
                  <td>
                    <div class="flex items-center gap-2">
                      <div class="flex-1 h-1.5 rounded-full bg-line overflow-hidden min-w-[48px]">
                        <div class="h-full bg-accent transition-all"
                          :style="{ width: percentOf(j).toFixed(1) + '%' }" />
                      </div>
                      <span class="text-[11.5px] text-sub tabular-nums shrink-0">
                        {{ progressOf(j).done }}/{{ progressOf(j).total }}
                      </span>
                    </div>
                  </td>
                  <td class="text-right tabular-nums">{{ rateText(j) }}</td>
                  <td class="text-right text-sub tabular-nums">{{ elapsedOf(j) }}</td>
                  <td>
                    <div class="flex items-center justify-end gap-0.5" @click.stop>
                      <button class="icon-btn" title="查看实时" @click="view(j)">
                        <Icon name="eye" :size="15" />
                      </button>
                      <button v-if="j.status === 'running'" class="icon-btn text-fail" title="取消任务"
                        @click="stop(j)">
                        <Icon name="stop" :size="14" />
                      </button>
                    </div>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>

          <div>
            <div class="px-5 pt-4 pb-1.5 text-[12px] text-sub">
              已完成 · {{ finishedJobs.length }}
              <span v-if="!finishedJobs.length" class="text-weak font-normal">（完成任务后出现在这里）</span>
            </div>
            <table v-if="finishedJobs.length" class="data-table">
              <thead>
                <tr>
                  <th class="w-[22%]">方案</th>
                  <th class="w-[15%]">数据集</th>
                  <th class="w-[17%]">被测 → 评测</th>
                  <th class="w-16 text-right">通过率</th>
                  <th class="w-[104px] text-right">通过 / 失败</th>
                  <th class="w-16 text-right">耗时</th>
                  <th class="w-[132px]" />
                </tr>
              </thead>
              <tbody>
                <tr v-for="j in finishedJobs" :key="j.runId" @click="view(j)">
                  <td>
                    <div class="flex items-center gap-1.5 min-w-0">
                      <span class="font-medium truncate">{{ j.title || j.planName }}</span>
                      <span class="chip !h-5 !px-1.5 shrink-0" :class="statusClass(j.status)">
                        {{ statusText(j.status) }}
                      </span>
                    </div>
                    <div class="text-[11px] text-weak truncate">{{ j.planName }}</div>
                  </td>
                  <td class="text-sub truncate">{{ j.datasetName || "—" }}</td>
                  <td class="text-weak truncate">{{ modelsOf(j) }}</td>
                  <td class="text-right tabular-nums font-medium">{{ rateText(j) }}</td>
                  <td class="text-right text-[12px] tabular-nums whitespace-nowrap">
                    <span class="text-pass">{{ j.summary?.passed ?? 0 }}</span>
                    <span class="text-weak"> / </span>
                    <span class="text-fail">{{ j.summary?.failed ?? 0 }}</span>
                    <span v-if="j.summary?.excluded" class="text-warn"> · {{ j.summary.excluded }} 未计入</span>
                  </td>
                  <td class="text-right text-sub tabular-nums">{{ elapsedOf(j) }}</td>
                  <td>
                    <div class="flex items-center justify-end gap-0.5" @click.stop>
                      <button class="icon-btn" title="查看结果" @click="view(j)">
                        <Icon name="eye" :size="15" />
                      </button>
                      <button v-if="['interrupted', 'cancelled', 'failed'].includes(j.status)"
                        class="icon-btn" title="断点续跑" @click="resume(j)">
                        <Icon name="play" :size="13" />
                      </button>
                      <button v-if="j.summary?.failed || j.summary?.excluded" class="icon-btn"
                        title="重跑失败项" @click="retry(j)">
                        <Icon name="refresh" :size="14" />
                      </button>
                      <button class="icon-btn text-fail" title="删除任务" @click="remove(j)">
                        <Icon name="trash" :size="14" />
                      </button>
                    </div>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </template>
      </div>

      <!-- 实时明细 -->
      <div v-if="live" class="flex-1 min-h-0 flex flex-col border-t border-line bg-panel">
        <div class="h-11 px-4 flex items-center gap-2 border-b border-line shrink-0">
          <span class="text-[13px] font-medium truncate">{{ live.title || live.planName || "任务明细" }}</span>
          <span class="chip !h-5 !px-1.5" :class="statusClass(live.status)">{{ statusText(live.status) }}</span>
          <span class="text-[11.5px] text-weak truncate">
            {{ live.planName }}<span v-if="live.datasetName"> · {{ live.datasetName }}</span>
          </span>
          <span class="flex-1" />
          <span class="text-[11.5px] text-sub tabular-nums">
            {{ live.done }} / {{ live.total || "—" }}
          </span>
          <span class="text-[11.5px] text-pass tabular-nums">通过 {{ liveStats.passed }}</span>
          <span class="text-[11.5px] text-fail tabular-nums">失败 {{ liveStats.failed }}</span>
          <span v-if="liveStats.excluded" class="text-[11.5px] text-warn tabular-nums">未计 {{ liveStats.excluded }}</span>
          <span class="text-[11.5px] text-main tabular-nums">
            通过率 {{ liveStats.valid ? ((liveStats.passed / liveStats.valid) * 100).toFixed(1) + "%" : "—" }}
          </span>
          <button class="icon-btn" title="关闭明细" @click="closeLive">
            <Icon name="close" :size="15" />
          </button>
        </div>

        <div v-if="live.error" class="shrink-0 px-4 py-1.5 text-[12px] text-fail bg-fail/5 border-b border-line truncate"
          :title="live.error">
          {{ live.error }}
        </div>

        <div class="h-9 px-4 flex items-center gap-3 border-b border-line shrink-0">
          <label class="flex items-center gap-1.5 text-[12px] text-sub cursor-pointer">
            <input v-model="liveOnlyFail" type="checkbox" class="w-3.5 h-3.5" />
            只看未通过
          </label>
          <div class="relative">
            <Icon name="search" :size="13"
              class="absolute left-2 top-1/2 -translate-y-1/2 text-weak pointer-events-none" />
            <input v-model="liveQuery" class="field pl-7 !h-7 w-48" placeholder="搜索输出或原因" />
          </div>
          <span class="text-[11.5px] text-weak tabular-nums">{{ liveFiltered.length }} 条</span>
          <span class="flex-1" />
          <button v-if="live.status === 'running' && !liveFollow" class="btn-ghost btn-sm"
            @click="liveFollow = true; livePage = livePageCount">
            回到最新
          </button>
        </div>

        <div class="flex-1 min-h-0 overflow-auto">
          <table class="data-table">
            <thead>
              <tr>
                <th class="w-12 text-right pr-3">#</th>
                <th class="w-20">结果</th>
                <th class="w-28">分组</th>
                <th class="w-[28%]">分类</th>
                <th>原因</th>
                <th class="w-16 text-right">用时</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="r in liveRows" :key="r.index" :class="{ 'row-active': detailIndex === r.index }"
                @click="openDetail(r)">
                <td class="text-right pr-3 text-weak tabular-nums">{{ r.index + 1 }}</td>
                <td>
                  <span v-if="isExcluded(r)" class="chip !h-5 !px-1.5 text-warn border-warn/30 bg-warn/5">未计入</span>
                  <span v-else-if="r.pass === true" class="chip !h-5 !px-1.5 text-pass border-pass/30 bg-pass/5">通过</span>
                  <span v-else class="chip !h-5 !px-1.5 text-fail border-fail/30 bg-fail/5">不通过</span>
                </td>
                <td class="text-sub truncate">{{ r.statusLabel }}</td>
                <td>
                  <span v-for="t in r.errorTypes || []" :key="t" class="chip !h-5 !px-1.5 mr-1">{{ t }}</span>
                </td>
                <td class="text-sub truncate max-w-[360px]">{{ (r.reasons || []).join("；") }}</td>
                <td class="text-right text-sub tabular-nums">
                  {{ (((r.targetLatencyMs || 0) + (r.judgeLatencyMs || 0)) / 1000).toFixed(1) }}s
                </td>
              </tr>
              <tr v-if="!liveFiltered.length">
                <td colspan="6" class="text-center text-weak py-8">
                  {{ live.loading ? "读取中…" : live.status === "running" ? "等待结果…" : "没有结果" }}
                </td>
              </tr>
            </tbody>
          </table>
        </div>

        <Pager :total="liveFiltered.length" :page="livePage" :page-size="livePageSize"
          @update:page="liveFollow = false; livePage = $event"
          @update:page-size="livePageSize = $event; liveFollow = false" />
      </div>
    </div>

    <DetailDrawer :row="detailRow" :data-row="detailDataRow" @close="detailIndex = -1" />
  </div>
</template>
