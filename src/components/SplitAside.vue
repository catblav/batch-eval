<script setup>
import { computed, ref } from "vue";
import { state, scheduleSave } from "../store";

// 三个两栏页（API 配置 / 提示词配置 / 数据集）共用的左栏。
// 宽度和收起状态都存 store，切页时不会跳。
const MIN_W = 200;
const MAX_W = 420;

const dragging = ref(false);
const width = computed(() => state.listWidthPx);
const collapsed = computed(() => state.listCollapsed);

function startResize(e) {
  const startX = e.clientX;
  const startW = state.listWidthPx;
  dragging.value = true;
  document.body.style.userSelect = "none";

  const onMove = (ev) => {
    const next = startW + (ev.clientX - startX);
    state.listWidthPx = Math.min(MAX_W, Math.max(MIN_W, next));
  };
  const onUp = () => {
    dragging.value = false;
    document.body.style.userSelect = "";
    window.removeEventListener("mousemove", onMove);
    window.removeEventListener("mouseup", onUp);
    scheduleSave();
  };
  window.addEventListener("mousemove", onMove);
  window.addEventListener("mouseup", onUp);
}
</script>

<template>
  <aside
    v-if="!collapsed"
    class="shrink-0 flex flex-col bg-wash/40 min-h-0"
    :style="{ width: width + 'px' }"
  >
    <slot />
  </aside>

  <!-- 拖这条线调宽度：视觉上是 1px 的分界线，热区左右各铺开 6px -->
  <div
    v-if="!collapsed"
    class="relative w-px shrink-0 cursor-col-resize transition-colors"
    :class="dragging ? 'bg-accent' : 'bg-line/60 hover:bg-accent/60'"
    title="拖动调整宽度"
    @mousedown.prevent="startResize"
  >
    <span class="absolute inset-y-0 -left-1.5 -right-1.5" />
  </div>
</template>
