<script setup>
import { computed, ref } from "vue";
import Icon from "./Icon.vue";

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
const rangeEnd = computed(() => Math.min(props.total, cur.value * props.pageSize));

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
  <div
    v-if="total"
    class="flex items-center gap-2 px-2.5 py-2 border-t border-line/60 bg-wash/30
      text-[12px] text-sub flex-wrap shrink-0"
  >
    <span class="tabular-nums">共 {{ total }} 条 · {{ rangeStart }}–{{ rangeEnd }}</span>
    <span class="flex-1" />
    <!-- 宽度要按最长选项「200 条/页」给够，否则文字被下拉箭头挤掉 -->
    <select
      class="field w-[108px] !px-2 text-[12px]"
      title="每页条数"
      :value="pageSize"
      @change="emit('update:pageSize', Number($event.target.value))"
    >
      <option v-for="s in sizes" :key="s" :value="s">{{ s }} 条/页</option>
    </select>
    <button class="icon-btn" :disabled="cur <= 1" title="上一页" @click="go(cur - 1)">
      <Icon name="chevron-left" :size="15" />
    </button>
    <span class="tabular-nums text-main min-w-[64px] text-center">{{ cur }} / {{ pageCount }}</span>
    <button class="icon-btn" :disabled="cur >= pageCount" title="下一页" @click="go(cur + 1)">
      <Icon name="chevron-right" :size="15" />
    </button>
    <!-- number 输入框自带上下箭头，宽度得留出它那一份，否则「跳页」两字被截 -->
    <input
      v-model="jump"
      type="number"
      min="1"
      :max="pageCount"
      class="field w-[80px] !px-2 text-[12px]"
      placeholder="跳页"
      @keyup.enter="goJump"
    />
  </div>
</template>
