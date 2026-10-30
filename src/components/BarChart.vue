<script setup>
const props = defineProps({
  items: { type: Array, default: () => [] }, // {name, count, ratio}
  total: { type: Number, default: 0 },
  color: { type: String, default: "#d9534f" },
});
</script>

<template>
  <div class="space-y-1.5">
    <div v-for="it in items" :key="it.name" class="group">
      <div class="flex items-center gap-2 text-[12px]">
        <span class="text-ink-300 truncate max-w-[240px]" :title="it.name">{{ it.name }}</span>
        <span class="flex-1" />
        <span class="text-ink-500 tabular-nums">{{ it.count }}</span>
        <span class="text-ink-600 tabular-nums w-12 text-right">{{ (it.ratio * 100).toFixed(0) }}%</span>
      </div>
      <div class="h-1.5 bg-ink-800 rounded-full overflow-hidden">
        <div class="h-full rounded-full transition-all duration-500"
          :style="{ width: (it.ratio * 100).toFixed(1) + '%', background: color, opacity: 0.9 }" />
      </div>
    </div>
    <p v-if="!items.length" class="hint">暂无分类数据</p>
  </div>
</template>
