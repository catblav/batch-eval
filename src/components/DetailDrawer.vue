<script setup>
import { computed, onMounted, onBeforeUnmount, ref, watch } from "vue";
import { isExcluded } from "../store";
import Icon from "./Icon.vue";
import { catColor, catSoft, catLine } from "../utils/catColors";
import { prettyJson } from "../utils/prettyJson";

const props = defineProps({
  row: { type: Object, default: null },
  dataRow: { type: Object, default: null },
});
const emit = defineEmits(["close"]);

/// 五个消息区块的展开状态。默认展开原始数据、被测输出、评测输出：
/// 数据行对照原始输入，两端输出是判定的依据；被测两侧消息先收起。
const DEFAULT_OPEN = { data: true, system: false, user: false, output: true, judge: true };
const open = ref({ ...DEFAULT_OPEN });
watch(
  () => props.row?.index,
  () => {
    open.value = { ...DEFAULT_OPEN };
  }
);
function toggle(key) {
  open.value = { ...open.value, [key]: !open.value[key] };
}

/// 五个区块按数据流排列：数据集原样 → 被测收到的两条消息 → 被测回复 → 评测回复原文。
/// kv 有值时按「列名：值」逐行渲染，否则按纯文本渲染。
const sections = computed(() => {
  const r = props.row;
  if (!r) return [];
  const dr = props.dataRow;
  const kvText = dr
    ? Object.entries(dr)
        .map(([k, v]) => `${k}：${v}`)
        .join("\n")
    : "";
  return [
    {
      key: "data",
      title: "原始数据",
      kv: dr || null,
      text: kvText,
      empty: "这条记录没有保存原始数据行",
      muted: true,
    },
    {
      key: "system",
      title: "被测system",
      text: r.system || "",
      empty: "（空）被测模型没收到 system 消息；旧版结果文件不记录这个字段",
    },
    {
      key: "user",
      title: "被测user",
      text: r.input || "",
      empty: "（空）被测模型没收到 user 消息",
    },
    {
      key: "output",
      title: "被测输出",
      text: prettyJson(r.output),
      count: (r.output || "").length,
      empty: "—",
    },
    {
      key: "judge",
      title: "评测输出",
      text: prettyJson(r.judgeRaw),
      count: (r.judgeRaw || "").length,
      empty: "—",
      muted: true,
    },
  ].map((s) => ({ count: s.text.length, ...s }));
});

const judgeTps = computed(() => {
  const r = props.row;
  if (!r || !r.judgeCompletionTokens || !r.judgeLatencyMs) return "—";
  return (r.judgeCompletionTokens / (r.judgeLatencyMs / 1000)).toFixed(1);
});

function copy(text) {
  navigator.clipboard.writeText(text || "");
}

/// 区块复制：复制该区块展示的文本，按钮短暂显示勾选反馈。
const copiedKey = ref("");
function copySection(key) {
  const s = sections.value.find((x) => x.key === key);
  copy(s?.text || "");
  copiedKey.value = key;
  setTimeout(() => {
    if (copiedKey.value === key) copiedKey.value = "";
  }, 1200);
}

function onKey(e) {
  if (e.key === "Escape") emit("close");
}
onMounted(() => window.addEventListener("keydown", onKey));
onBeforeUnmount(() => window.removeEventListener("keydown", onKey));
</script>

<template>
  <div v-if="row" class="fixed inset-0 z-50 flex justify-end">
    <div class="absolute inset-0 bg-black/40" @click="emit('close')" />
    <div
      class="relative w-[min(760px,92vw)] h-full bg-panel border-l border-line/60 shadow-pop
        flex flex-col slide-in"
    >
      <div class="h-12 px-2.5 flex items-center gap-2 border-b border-line/60 shrink-0">
        <span class="text-[13px] font-medium tabular-nums">第 {{ row.index + 1 }} 条</span>
        <span v-if="isExcluded(row)" class="chip-warn">未计入通过率</span>
        <span v-else-if="row.pass === true" class="chip-pass">通过</span>
        <span v-else class="chip-fail">不通过</span>
        <span class="chip-muted">{{ row.statusLabel || "未分组" }}</span>
        <span class="flex-1" />
        <button class="icon-btn" title="关闭" @click="emit('close')">
          <Icon name="close" :size="15" />
        </button>
      </div>

      <div class="flex-1 overflow-auto p-2.5 space-y-4">
        <div
          v-if="row.errorTypes?.length || row.truncated || !row.parseOk"
          class="flex flex-wrap gap-1"
        >
          <span v-for="t in row.errorTypes || []" :key="t" class="chip"
            :style="{ color: catColor(t), background: catSoft(t), borderColor: catLine(t) }">{{ t }}</span>
          <span v-if="row.truncated" class="chip-warn">输入被截断</span>
          <span v-if="!row.parseOk" class="chip-warn">评测输出非标准结构</span>
        </div>

        <!-- 评测调用失败或返回解析不出来时，评测输出是空的，原因只能在这里看；
             正常解析出结果的，评测输出原文里就有 reason，不再重复一遍 -->
        <div v-if="!row.parseOk && !row.targetError">
          <div class="label">未计入通过率的原因</div>
          <ul v-if="row.reasons?.length" class="space-y-1.5">
            <li
              v-for="(r, i) in row.reasons"
              :key="i"
              class="text-[12.5px] text-main bg-wash/70 border border-line/60 rounded-md px-3 py-2 whitespace-pre-wrap"
            >
              {{ r }}
            </li>
          </ul>
          <p v-else-if="row.details" class="text-[12.5px] text-main whitespace-pre-wrap">{{ row.details }}</p>
          <p v-else class="hint">无</p>
        </div>

        <div
          v-if="row.targetError"
          class="bg-fail/10 border border-fail/30 rounded-md px-3 py-2 text-[12px] text-fail whitespace-pre-wrap"
        >
          {{ row.targetError }}
        </div>

        <div class="rounded-md border border-line/60 bg-wash/40 px-3 py-2.5 grid grid-cols-2 gap-x-6 gap-y-1.5 text-[11.5px] text-sub">
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

        <div class="space-y-2">
          <div v-for="s in sections" :key="s.key" class="border border-line/60 rounded-md overflow-hidden">
            <div
              class="w-full flex items-center gap-1.5 px-3 py-2 bg-wash/40 hover:bg-wash/80 transition-colors text-left cursor-pointer"
              role="button"
              :aria-expanded="open[s.key]"
              @click="toggle(s.key)"
            >
              <Icon
                :name="open[s.key] ? 'chevron-down' : 'chevron-right'"
                :size="13"
                class="text-weak shrink-0"
              />
              <span class="text-[12px] text-sub font-medium">{{ s.title }}</span>
              <span class="flex-1" />
              <button
                class="icon-btn shrink-0"
                :title="`复制${s.title}`"
                @click.stop="copySection(s.key)"
              >
                <Icon :name="copiedKey === s.key ? 'check' : 'copy'" :size="12" />
              </button>
              <span class="text-[11px] text-weak tabular-nums shrink-0 ml-1">
                {{ s.count ? `${s.count.toLocaleString()} 字` : "空" }}
              </span>
            </div>
            <div v-show="open[s.key]" class="border-t border-line/60 px-3 py-2.5">
              <div
                v-if="s.kv"
                class="text-[12px] font-mono whitespace-pre-wrap max-h-72 overflow-auto text-sub"
              >
                <div v-for="(v, k) in s.kv" :key="k" class="mb-1">
                  <span class="text-weak">{{ k }}：</span>{{ v }}
                </div>
              </div>
              <pre
                v-else
                class="text-[12px] font-mono whitespace-pre-wrap max-h-72 overflow-auto"
                :class="s.muted ? 'text-sub' : 'text-main'"
              >{{ s.text || s.empty }}</pre>
            </div>
          </div>
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
