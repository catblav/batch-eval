<script setup>
import { computed, onMounted, onBeforeUnmount } from "vue";
import { isExcluded } from "../store";
import Icon from "./Icon.vue";

const props = defineProps({
  row: { type: Object, default: null },
  dataRow: { type: Object, default: null },
});
const emit = defineEmits(["close"]);

const judgeTps = computed(() => {
  const r = props.row;
  if (!r || !r.judgeCompletionTokens || !r.judgeLatencyMs) return "—";
  return (r.judgeCompletionTokens / (r.judgeLatencyMs / 1000)).toFixed(1);
});

function copy(text) {
  navigator.clipboard.writeText(text || "");
}

function onKey(e) {
  if (e.key === "Escape") emit("close");
}
onMounted(() => window.addEventListener("keydown", onKey));
onBeforeUnmount(() => window.removeEventListener("keydown", onKey));
</script>

<template>
  <div v-if="row" class="fixed inset-0 z-50 flex justify-end">
    <div class="absolute inset-0 bg-black/35" @click="emit('close')" />
    <div class="relative w-[min(760px,92vw)] h-full bg-panel border-l border-line flex flex-col slide-in">
      <div class="h-12 px-4 flex items-center gap-2 border-b border-line shrink-0">
        <span class="text-[13px] font-medium tabular-nums">第 {{ row.index + 1 }} 条</span>
        <span v-if="isExcluded(row)" class="chip text-warn border-warn/40 bg-warn/5">未计入通过率</span>
        <span v-else-if="row.pass === true" class="chip text-pass border-pass/40 bg-pass/5">通过</span>
        <span v-else class="chip text-fail border-fail/40 bg-fail/5">不通过</span>
        <span class="chip">{{ row.statusLabel || "未分组" }}</span>
        <span class="flex-1" />
        <button class="btn-ghost btn-sm" @click="copy(row.output)">复制输出</button>
        <button class="icon-btn" title="关闭" @click="emit('close')">
          <Icon name="close" :size="15" />
        </button>
      </div>

      <div class="flex-1 overflow-auto p-4 space-y-4">
        <div class="flex flex-wrap gap-1">
          <span v-for="t in row.errorTypes || []" :key="t" class="chip text-fail border-fail/40 bg-fail/5">{{ t }}</span>
          <span v-if="row.truncated" class="chip text-warn border-warn/40 bg-warn/5">输入被截断</span>
          <span v-if="!row.parseOk" class="chip text-warn border-warn/40 bg-warn/5">评测输出非标准结构</span>
        </div>

        <div>
          <div class="label">分类原因</div>
          <ul v-if="row.reasons?.length" class="space-y-1.5">
            <li
              v-for="(r, i) in row.reasons"
              :key="i"
              class="text-[12.5px] text-main bg-wash border border-line rounded px-3 py-2 whitespace-pre-wrap"
            >
              {{ r }}
            </li>
          </ul>
          <p v-else-if="row.details" class="text-[12.5px] text-main whitespace-pre-wrap">{{ row.details }}</p>
          <p v-else class="hint">无</p>
        </div>

        <div
          v-if="row.targetError"
          class="bg-fail/10 border border-fail/30 rounded px-3 py-2 text-[12px] text-fail whitespace-pre-wrap"
        >
          {{ row.targetError }}
        </div>

        <div class="rounded border border-line bg-page px-3 py-2.5 grid grid-cols-2 gap-x-6 gap-y-1.5 text-[11.5px] text-sub">
          <div>被测耗时 <span class="text-main tabular-nums">{{ ((row.targetLatencyMs || 0) / 1000).toFixed(2) }}s</span></div>
          <div>评测耗时 <span class="text-main tabular-nums">{{ ((row.judgeLatencyMs || 0) / 1000).toFixed(2) }}s</span></div>
          <div>
            被测 tokens
            <span class="text-main tabular-nums">{{ row.promptTokens ?? "—" }} → {{ row.completionTokens ?? "—" }}</span>
          </div>
          <div>
            裁判 tokens
            <span class="text-main tabular-nums">{{ row.judgePromptTokens ?? "—" }} → {{ row.judgeCompletionTokens ?? "—" }}</span>
          </div>
          <div v-if="row.judgeCompletionTokens">裁判生成速度 <span class="text-main tabular-nums">{{ judgeTps }} tok/s</span></div>
        </div>

        <div>
          <div class="label">原始数据</div>
          <div class="bg-page border border-line rounded p-3 text-[12px] font-mono whitespace-pre-wrap max-h-40 overflow-auto text-sub">
            <template v-if="dataRow">
              <div v-for="(v, k) in dataRow" :key="k" class="mb-1">
                <span class="text-weak">{{ k }}：</span>{{ v }}
              </div>
            </template>
            <span v-else class="text-weak">这条记录没有保存原始数据行</span>
          </div>
        </div>

        <div>
          <div class="label">实际发给被测模型的用户消息</div>
          <pre class="bg-page border border-line rounded p-3 text-[12px] font-mono whitespace-pre-wrap max-h-64 overflow-auto text-main">{{ row.input || "—" }}</pre>
        </div>

        <div>
          <div class="label">被测模型输出</div>
          <pre class="bg-page border border-line rounded p-3 text-[12px] font-mono whitespace-pre-wrap max-h-80 overflow-auto text-main">{{ row.output || "—" }}</pre>
        </div>

        <div>
          <div class="label">评测模型原始输出</div>
          <pre class="bg-page border border-line rounded p-3 text-[12px] font-mono whitespace-pre-wrap max-h-64 overflow-auto text-sub">{{ row.judgeRaw || "—" }}</pre>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.slide-in {
  animation: slideIn 0.16s ease-out;
}
@keyframes slideIn {
  from {
    transform: translateX(20px);
    opacity: 0.5;
  }
  to {
    transform: translateX(0);
    opacity: 1;
  }
}
</style>
