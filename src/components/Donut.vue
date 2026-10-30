<script setup>
import { computed } from "vue";

const props = defineProps({
  total: { type: Number, default: 0 },
  passed: { type: Number, default: 0 },
  failed: { type: Number, default: 0 },
});

const R = 54;
const C = 2 * Math.PI * R;
const rate = computed(() => (props.total ? props.passed / props.total : 0));
const dash = computed(() => `${(rate.value * C).toFixed(1)} ${C.toFixed(1)}`);
const color = computed(() => {
  if (!props.total) return "#39424f";
  if (rate.value >= 0.9) return "#3fae7a";
  if (rate.value >= 0.7) return "#d9a13f";
  return "#d9534f";
});
</script>

<template>
  <div class="flex items-center gap-5">
    <svg viewBox="0 0 140 140" class="w-[140px] h-[140px] -rotate-90">
      <circle cx="70" cy="70" :r="R" fill="none" stroke="#262d38" stroke-width="13" />
      <circle cx="70" cy="70" :r="R" fill="none" :stroke="color" stroke-width="13"
        stroke-linecap="round" :stroke-dasharray="dash" class="transition-all duration-700" />
    </svg>
    <div>
      <div class="text-[30px] font-semibold tabular-nums leading-none" :style="{ color }">
        {{ (rate * 100).toFixed(1) }}<span class="text-[15px] text-ink-500 ml-0.5">%</span>
      </div>
      <div class="hint mt-1">通过率</div>
      <div class="flex gap-3 mt-3 text-[12px]">
        <span class="text-pass">通过 {{ passed }}</span>
        <span class="text-fail">不通过 {{ failed }}</span>
        <span class="text-ink-500">共 {{ total }}</span>
      </div>
    </div>
  </div>
</template>
