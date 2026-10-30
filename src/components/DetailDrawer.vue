<script setup>
import { computed } from "vue";
import { state } from "../store";

const props = defineProps({ index: { type: Number, default: -1 } });
const emit = defineEmits(["close"]);

const row = computed(() =>
  props.index >= 0 ? state.run.results.find((r) => r.index === props.index) : null
);
const dataRow = computed(() => (props.index >= 0 ? state.dataset.rows[props.index] : null));

function copy(text) {
  navigator.clipboard.writeText(text || "");
}
</script>

<template>
  <div v-if="row" class="fixed inset-0 z-50 flex justify-end">
    <div class="absolute inset-0 bg-black/50" @click="emit('close')" />
    <div class="relative w-[min(860px,92vw)] h-full bg-ink-900 border-l border-ink-700 flex flex-col slide-in">
      <div class="flex items-center gap-2 px-4 py-3 border-b border-ink-700/60">
        <span class="text-[13px] font-semibold">第 {{ row.index + 1 }} 条</span>
        <span class="chip" :class="row.pass === true ? 'border-pass/40 text-pass bg-pass/10' : 'border-fail/40 text-fail bg-fail/10'">
          {{ row.pass === true ? "通过" : "不通过" }}
        </span>
        <span class="chip border-ink-700 text-ink-400">{{ row.statusLabel }}</span>
        <span class="flex-1" />
        <button class="btn-ghost text-[12px]" @click="copy(row.output)">复制输出</button>
        <button class="btn-ghost text-[12px]" @click="emit('close')">关闭</button>
      </div>

      <div class="flex-1 overflow-auto p-4 space-y-4">
        <div class="flex flex-wrap gap-1">
          <span v-for="t in row.errorTypes" :key="t" class="chip border-fail/40 text-fail bg-fail/10">{{ t }}</span>
          <span v-if="row.truncated" class="chip border-warn/40 text-warn bg-warn/10">输入被截断</span>
          <span v-if="!row.parseOk" class="chip border-warn/40 text-warn bg-warn/10">评测输出非标准结构</span>
        </div>

        <div>
          <div class="label">分类原因</div>
          <ul v-if="row.reasons.length" class="space-y-1.5">
            <li v-for="(r, i) in row.reasons" :key="i"
              class="text-[12.5px] text-ink-300 bg-fail/5 border border-fail/20 rounded px-3 py-2 whitespace-pre-wrap">
              {{ r }}
            </li>
          </ul>
          <p v-else-if="row.details" class="text-[12.5px] text-ink-300 whitespace-pre-wrap">{{ row.details }}</p>
          <p v-else class="hint">无</p>
        </div>

        <div v-if="row.targetError" class="bg-fail/10 border border-fail/30 rounded px-3 py-2 text-[12px] text-fail whitespace-pre-wrap">
          {{ row.targetError }}
        </div>

        <div class="grid grid-cols-4 gap-2 text-[11.5px] text-ink-500">
          <div>被测耗时：<span class="text-ink-300">{{ (row.targetLatencyMs / 1000).toFixed(2) }}s</span></div>
          <div>评测耗时：<span class="text-ink-300">{{ (row.judgeLatencyMs / 1000).toFixed(2) }}s</span></div>
          <div>输入 tokens：<span class="text-ink-300">{{ row.promptTokens ?? "—" }}</span></div>
          <div>输出 tokens：<span class="text-ink-300">{{ row.completionTokens ?? "—" }}</span></div>
        </div>

        <div>
          <div class="label">原始数据</div>
          <div class="bg-ink-950 border border-ink-800 rounded p-3 text-[12px] font-mono whitespace-pre-wrap max-h-40 overflow-auto text-ink-400">
            <template v-if="dataRow">
              <div v-for="(v, k) in dataRow" :key="k" class="mb-1">
                <span class="text-ink-600">{{ k }}：</span>{{ v }}
              </div>
            </template>
            <span v-else>—</span>
          </div>
        </div>

        <div>
          <div class="label">实际发给被测模型的用户消息</div>
          <pre class="bg-ink-950 border border-ink-800 rounded p-3 text-[12px] font-mono whitespace-pre-wrap max-h-64 overflow-auto text-ink-300">{{ row.input }}</pre>
        </div>

        <div>
          <div class="label">被测模型输出</div>
          <pre class="bg-ink-950 border border-ink-800 rounded p-3 text-[12px] font-mono whitespace-pre-wrap max-h-80 overflow-auto text-ink-200">{{ row.output || "—" }}</pre>
        </div>

        <div>
          <div class="label">评测模型原始输出</div>
          <pre class="bg-ink-950 border border-ink-800 rounded p-3 text-[12px] font-mono whitespace-pre-wrap max-h-64 overflow-auto text-ink-400">{{ row.judgeRaw || "—" }}</pre>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.slide-in {
  animation: slideIn 0.18s ease-out;
}
@keyframes slideIn {
  from { transform: translateX(24px); opacity: 0.4; }
  to { transform: translateX(0); opacity: 1; }
}
::highlight(x) {}
</style>
