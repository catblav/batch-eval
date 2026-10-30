<script setup>
import { ref, computed, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import {
  state,
  stats,
  toast,
  statusLabelOf,
  ensureUserTemplate,
  resetRunResults,
  mergeMissingResults,
  startRunSession,
  finishRunSession,
  sessionReceivedCount,
  clearRun,
} from "../store";
import Field from "./Field.vue";
import Pager from "./Pager.vue";

const err = ref("");

const sel = state.settings;
const run = state.run;

const ready = computed(() => {
  const problems = [];
  if (!state.prompts.systemPrompt.trim()) problems.push("主体提示词为空");
  if (!state.prompts.userTemplate.trim() && !state.dataset.columns.length) problems.push("用户消息模板为空");
  if (!state.prompts.evalPrompt.trim()) problems.push("评测提示词为空");
  if (!state.dataset.rows.length) problems.push("数据集为空");
  if (!sel.target.baseUrl.trim()) problems.push("被测模型 Base URL 为空");
  if (!sel.target.model.trim()) problems.push("被测模型名为空");
  if (!sel.judge.baseUrl.trim()) problems.push("评测模型 Base URL 为空");
  if (!sel.judge.model.trim()) problems.push("评测模型名为空");
  return problems;
});

// 实时结果分页：运行中默认跟随最新一页，手动翻页后不再自动跳
const page = ref(1);
const pageSize = ref(50);
const follow = ref(true);
const pageCount = computed(() => Math.max(1, Math.ceil(run.results.length / Math.max(1, pageSize.value))));
const pagedResults = computed(() => {
  const size = Math.max(1, pageSize.value);
  const p = Math.min(Math.max(1, page.value), pageCount.value);
  const start = (p - 1) * size;
  return run.results.slice(start, start + size);
});
watch(pageCount, (n) => {
  if (follow.value) page.value = n;
});
watch(
  () => run.running,
  (v) => {
    if (v) {
      follow.value = true;
    }
  }
);
function goPage(p) {
  follow.value = false;
  page.value = p;
}
function toLatest() {
  follow.value = true;
  page.value = pageCount.value;
}
function onPageSize(n) {
  pageSize.value = n;
  page.value = follow.value ? pageCount.value : 1;
}

const progress = computed(() => (run.total ? run.done / run.total : 0));
const elapsedText = computed(() => {
  const ms = run.elapsed || 0;
  const s = Math.floor(ms / 1000);
  return `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
});

async function start(mode = "new", onlyIndices = null) {
  err.value = "";
  ensureUserTemplate();
  if (ready.value.length) {
    err.value = "还有没填好的地方：" + ready.value.join("、");
    return;
  }

  let indices = onlyIndices;
  if (mode === "new") {
    resetRunResults();
    run.runId = `run-${Date.now()}`;
    page.value = 1;
    follow.value = true;
  } else if (mode === "retry") {
    indices = run.results.filter((r) => r.targetError || !r.parseOk || !r.pass).map((r) => r.index);
    if (!indices.length) {
      toast("没有需要重跑的行", "info");
      return;
    }
  } else if (mode === "resume") {
    indices = Array.from({ length: state.dataset.rows.length }, (_, i) => i).filter(
      (i) => !run.results.some((r) => r.index === i)
    );
    if (!indices.length) {
      toast("没有未完成的行", "info");
      return;
    }
  }
  // 重跑 / 续跑沿用当前 runId，结果追加到同一份 results.jsonl
  if (!run.runId) run.runId = `run-${Date.now()}`;

  const labels = state.dataset.rows.map((r) => statusLabelOf(r));
  const sessionTotal = indices ? indices.length : state.dataset.rows.length;
  startRunSession(sessionTotal);

  let status = "finished";
  try {
    const outcome = await invoke("start_run", {
      cfg: {
        systemPrompt: state.prompts.systemPrompt,
        userTemplate: state.prompts.userTemplate,
        evalPrompt: state.prompts.evalPrompt,
        target: sel.target,
        judge: sel.judge,
        concurrency: Number(sel.concurrency) || 1,
        maxRetries: Number(sel.maxRetries) || 0,
        mode,
        retryIndices: indices || null,
        runId: run.runId,
      },
      rows: state.dataset.rows,
      labels,
    });

    status = outcome?.status || "finished";
    if (outcome?.error) {
      err.value = outcome.error;
      run.error = outcome.error;
      toast(outcome.error, "err", 8000);
    }

    // 事件是异步投递的，跑得快时可能还没全部到达；这里只补界面上缺的 index，
    // 而且要确认磁盘上就是本次运行，绝不把上一次的结果并进来
    if (outcome && outcome.done > sessionReceivedCount()) {
      const loaded = await invoke("load_last_run");
      if (loaded?.meta?.runId === run.runId) mergeMissingResults(loaded.results);
    }

    if (status === "cancelled") toast("已取消，已跑完的结果已保留", "info");
  } catch (e) {
    status = "failed";
    err.value = String(e);
    run.error = String(e);
    toast(String(e), "err");
  } finally {
    finishRunSession(status);
  }
}

async function clearAll() {
  if (run.running) return;
  const ok = await ask(
    `清空这次的 ${run.results.length} 条运行结果？磁盘上的结果文件也会删除，无法恢复。`,
    { title: "清空结果", kind: "warning", okLabel: "清空", cancelLabel: "取消" }
  );
  if (!ok) return;
  try {
    await clearRun();
    page.value = 1;
    follow.value = true;
    toast("已清空运行结果", "ok");
  } catch (e) {
    toast(String(e), "err");
  }
}

async function cancel() {
  try {
    await invoke("cancel_run", { runId: run.runId });
    toast("已停止发起新请求，在途请求会自然结束", "info");
  } catch (e) {
    toast(String(e), "err");
  }
}

const canResume = computed(
  () => run.results.length > 0 && run.results.length < state.dataset.rows.length && !run.running
);
const failedCount = computed(
  () => run.results.filter((r) => r.targetError || !r.parseOk || !r.pass).length
);

const statusText = computed(() => {
  if (run.running) return "运行中…";
  if (run.status === "cancelled") return "已取消";
  if (run.status === "failed") return "运行失败";
  if (run.status === "interrupted") return "上次运行被中断";
  if (run.status === "finished") return "已完成";
  return "";
});
</script>

<template>
  <div class="space-y-4">
    <div class="card p-4">
      <div class="flex items-center gap-2 mb-3">
        <span class="text-[13px] font-semibold">运行参数</span>
        <span class="flex-1" />
        <span class="hint">并发太高容易被限流，4~8 比较稳</span>
      </div>
      <div class="grid grid-cols-4 gap-3">
        <Field label="并发数" v-model="sel.concurrency" type="number" />
        <Field label="失败重试次数" v-model="sel.maxRetries" type="number" />
        <div class="col-span-2 flex items-end gap-2">
          <div class="flex-1">
            <span class="label">数据量</span>
            <div class="field text-ink-400">{{ state.dataset.rows.length }} 条 · 每条调用被测模型 1 次 + 评测模型 1 次</div>
          </div>
        </div>
      </div>
    </div>

    <div class="card p-4">
      <div class="flex items-center gap-2 mb-3">
        <span class="text-[13px] font-semibold">进度</span>
        <span class="flex-1" />
        <span class="hint">已用 {{ elapsedText }}</span>
      </div>

      <div class="h-2 rounded-full bg-ink-800 overflow-hidden">
        <div class="h-full bg-accent transition-all duration-300" :style="{ width: (progress * 100).toFixed(1) + '%' }" />
      </div>
      <div class="flex items-center gap-3 mt-2 text-[12px] text-ink-400">
        <span>{{ run.done }} / {{ run.total || state.dataset.rows.length }}</span>
        <span v-if="run.results.length">
          通过 {{ stats.passed }} · 不通过 {{ stats.failed }}
        </span>
        <span class="flex-1" />
        <span v-if="statusText" :class="run.running ? 'text-accent' : run.status === 'failed' ? 'text-fail' : 'text-ink-400'">
          {{ statusText }}
        </span>
      </div>

      <div class="flex gap-2 mt-4">
        <button class="btn-primary" :disabled="run.running" @click="start('new')">开始批量评测</button>
        <button v-if="run.running" class="btn-danger" @click="cancel">取消</button>
        <button class="btn" :disabled="run.running || !canResume" @click="start('resume')">断点续跑</button>
        <button class="btn" :disabled="run.running || !failedCount" @click="start('retry')">
          重跑失败项（{{ failedCount }}）
        </button>
        <span class="flex-1" />
        <button class="btn-ghost" :disabled="run.running || !run.results.length" @click="clearAll">
          清空结果
        </button>
      </div>

      <div v-if="err" class="mt-3 text-[12px] text-fail bg-fail/10 border border-fail/30 rounded px-3 py-2 whitespace-pre-wrap">
        {{ err }}
      </div>
      <div v-else-if="run.error"
        class="mt-3 text-[12px] text-fail bg-fail/10 border border-fail/30 rounded px-3 py-2 whitespace-pre-wrap">
        <span class="font-semibold">本次运行没有跑通：</span>{{ run.error }}
      </div>
      <div v-if="ready.length" class="mt-3 hint">
        待补齐：{{ ready.join("、") }}
      </div>
    </div>

    <div class="card overflow-hidden">
      <div class="flex items-center gap-2 px-4 py-2 border-b border-ink-700/60">
        <span class="text-[13px] font-semibold">实时结果</span>
        <span v-if="run.results.length" class="hint">共 {{ run.results.length }} 条</span>
        <span class="flex-1" />
        <button v-if="run.running && !follow" class="btn-ghost text-[12px]" @click="toLatest">回到最新</button>
      </div>
      <div class="overflow-auto max-h-[340px]">
        <table class="w-full text-[12px]">
          <thead class="sticky top-0 bg-ink-850 text-ink-400">
            <tr>
              <th class="px-3 py-2 text-left font-medium w-12">#</th>
              <th class="px-3 py-2 text-left font-medium w-24">状态</th>
              <th class="px-3 py-2 text-left font-medium w-28">分组</th>
              <th class="px-3 py-2 text-left font-medium">分类</th>
              <th class="px-3 py-2 text-left font-medium">原因</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="r in pagedResults" :key="r.index"
              class="border-t border-ink-800 hover:bg-ink-850/60">
              <td class="px-3 py-1.5 text-ink-500">{{ r.index + 1 }}</td>
              <td class="px-3 py-1.5">
                <span class="chip" :class="r.pass === true ? 'border-pass/40 text-pass bg-pass/10' : 'border-fail/40 text-fail bg-fail/10'">
                  {{ r.pass === true ? "通过" : "不通过" }}
                </span>
              </td>
              <td class="px-3 py-1.5 text-ink-400">{{ r.statusLabel }}</td>
              <td class="px-3 py-1.5">
                <span v-for="t in r.errorTypes" :key="t" class="chip border-ink-700 text-ink-300 mr-1">{{ t }}</span>
              </td>
              <td class="px-3 py-1.5 text-ink-400 truncate max-w-[420px]">{{ (r.reasons || []).join("；") }}</td>
            </tr>
            <tr v-if="!run.results.length">
              <td colspan="5" class="px-3 py-10 text-center text-ink-500">还没开始跑</td>
            </tr>
          </tbody>
        </table>
      </div>
      <Pager :total="run.results.length" :page="page" :page-size="pageSize"
        @update:page="goPage" @update:page-size="onPageSize" />
    </div>
  </div>
</template>
