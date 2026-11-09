<script setup>
defineProps({
  items: { type: Array, default: () => [] }, // {name, count, ratio}
  total: { type: Number, default: 0 },
  color: { type: String, default: "rgb(var(--fail))" },
});
</script>

<template>
  <div class="space-y-2">
    <div v-for="it in items" :key="it.name" class="group">
      <div class="flex items-center gap-2 text-[12px]">
        <span class="text-main truncate max-w-[260px]" :title="it.name">{{ it.name }}</span>
        <span class="flex-1" />
        <span class="text-sub tabular-nums">{{ it.count }}</span>
        <span class="text-weak tabular-nums w-11 text-right">{{ (it.ratio * 100).toFixed(0) }}%</span>
      </div>
      <div class="h-1.5 bg-line rounded-full overflow-hidden mt-1">
        <div
          class="h-full rounded-full transition-all duration-500"
          :style="{ width: (it.ratio * 100).toFixed(1) + '%', background: color }"
        />
      </div>
    </div>
    <p v-if="!items.length" class="hint">暂无分类数据</p>
  </div>
</template>
