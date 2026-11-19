<script setup>
import { computed } from "vue";

const props = defineProps({
  total: { type: Number, default: 0 },
  passed: { type: Number, default: 0 },
  failed: { type: Number, default: 0 },
  excluded: { type: Number, default: 0 },
});

const R = 54;
const C = 2 * Math.PI * R;

// 分母是有效样本：调用失败 / 超时 / 解析失败不参与通过率
const valid = computed(() => Math.max(0, props.total - props.excluded));
const rate = computed(() => (valid.value ? props.passed / valid.value : 0));
const dash = computed(() => `${(rate.value * C).toFixed(1)} ${C.toFixed(1)}`);
const color = computed(() => {
  if (!valid.value) return "rgb(var(--text-weak))";
  if (rate.value >= 0.9) return "rgb(var(--pass))";
  if (rate.value >= 0.7) return "rgb(var(--warn))";
  return "rgb(var(--fail))";
});
</script>

<template>
  <!-- 百分比放进圆环中心：数字和图形合成一个单元，旁边不再堆一列重复的文字 -->
  <div class="relative w-[132px] h-[132px] shrink-0">
    <svg viewBox="0 0 140 140" class="w-full h-full -rotate-90">
      <circle cx="70" cy="70" :r="R" fill="none" stroke="rgb(var(--line))" stroke-width="11" />
      <circle
        cx="70"
        cy="70"
        :r="R"
        fill="none"
        :stroke="color"
        stroke-width="11"
        stroke-linecap="round"
        :stroke-dasharray="dash"
        class="transition-all duration-700"
      />
    </svg>
    <div class="absolute inset-0 flex flex-col items-center justify-center">
      <div class="flex items-baseline" :style="{ color }">
        <span class="text-[24px] font-semibold tabular-nums leading-none">{{ (rate * 100).toFixed(1) }}</span>
        <span class="text-[12px] font-medium ml-0.5">%</span>
      </div>
      <div class="text-[10.5px] text-weak mt-1">通过率</div>
    </div>
  </div>
</template>
