<script setup>
import { computed, ref } from "vue";

const props = defineProps({
  total: { type: Number, required: true },
  page: { type: Number, required: true },
  pageSize: { type: Number, required: true },
  sizes: { type: Array, default: () => [50, 100, 200] },
});
const emit = defineEmits(["update:page", "update:pageSize"]);

const pageCount = computed(() =>
  Math.max(1, Math.ceil(props.total / Math.max(1, props.pageSize)))
);
const cur = computed(() => Math.min(Math.max(1, props.page), pageCount.value));
const rangeStart = computed(() =>
  props.total ? (cur.value - 1) * props.pageSize + 1 : 0
);
const rangeEnd = computed(() =>
  Math.min(props.total, cur.value * props.pageSize)
);

const jump = ref("");
function go(p) {
  emit("update:page", Math.min(Math.max(1, p), pageCount.value));
}
function goJump() {
  const n = Number(jump.value);
  if (Number.isFinite(n) && n >= 1) go(Math.floor(n));
  jump.value = "";
}
</script>

<template>
  <div v-if="total" class="flex items-center gap-2 px-4 py-2 border-t border-ink-700/60 text-[12px] text-ink-400 flex-wrap">
    <span class="tabular-nums">共 {{ total }} 条 · 第 {{ rangeStart }}–{{ rangeEnd }} 条</span>
    <span class="flex-1" />
    <select class="field w-[96px] text-[12px]" title="每页条数"
      :value="pageSize" @change="emit('update:pageSize', Number($event.target.value))">
      <option v-for="s in sizes" :key="s" :value="s">{{ s }} 条/页</option>
    </select>
    <button class="btn-ghost text-[12px]" :disabled="cur <= 1" @click="go(1)">首页</button>
    <button class="btn-ghost text-[12px]" :disabled="cur <= 1" @click="go(cur - 1)">上一页</button>
    <span class="tabular-nums text-ink-300">{{ cur }} / {{ pageCount }}</span>
    <button class="btn-ghost text-[12px]" :disabled="cur >= pageCount" @click="go(cur + 1)">下一页</button>
    <button class="btn-ghost text-[12px]" :disabled="cur >= pageCount" @click="go(pageCount)">末页</button>
    <input v-model="jump" type="number" min="1" :max="pageCount" class="field w-16 text-[12px]"
      placeholder="跳页" @keyup.enter="goJump" />
  </div>
</template>
