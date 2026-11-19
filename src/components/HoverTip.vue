<script setup>
import { nextTick, ref } from "vue";

const props = defineProps({
  text: { type: String, default: "" },
  // 短于这个长度就不弹，免得满屏小气泡
  minLength: { type: Number, default: 20 },
  // 悬停多久才弹，和系统 tooltip 的节奏一致
  delay: { type: Number, default: 1200 },
});

const show = ref(false);
const left = ref(0);
const top = ref(0);
const pop = ref(null);
let hideTimer = null;
let showTimer = null;

function place(x, y) {
  const el = pop.value;
  if (!el) return;
  const pad = 12;
  const w = el.offsetWidth;
  const h = el.offsetHeight;
  let l = x + 14;
  let t = y + 16;
  // 贴到右/下边缘就翻到另一侧，别被窗口切掉
  if (l + w > window.innerWidth - pad) l = Math.max(pad, x - w - 14);
  if (t + h > window.innerHeight - pad) t = Math.max(pad, window.innerHeight - h - pad);
  left.value = l;
  top.value = t;
}

function open(e) {
  if ((props.text || "").length < props.minLength) return;
  clearTimeout(hideTimer);
  clearTimeout(showTimer);
  const { clientX: x, clientY: y } = e;
  showTimer = setTimeout(async () => {
    show.value = true;
    await nextTick();
    place(x, y);
  }, props.delay);
}

// 留一点延迟，鼠标能从触发元素移到浮层上滚动长文本
function close() {
  clearTimeout(showTimer);
  hideTimer = setTimeout(() => {
    show.value = false;
  }, 120);
}

function keep() {
  clearTimeout(hideTimer);
}
</script>

<template>
  <span @mouseenter="open" @mouseleave="close">
    <slot />
    <Teleport to="body">
      <div
        v-if="show"
        ref="pop"
        class="hover-pop"
        :style="{ left: left + 'px', top: top + 'px' }"
        @mouseenter="keep"
        @mouseleave="close"
      >{{ text }}</div>
    </Teleport>
  </span>
</template>

<style scoped>
.hover-pop {
  position: fixed;
  z-index: 80;
  max-width: min(520px, 60vw);
  max-height: min(420px, 70vh);
  overflow: auto;
  padding: 8px 10px;
  border-radius: 8px;
  border: 1px solid rgb(var(--card-line));
  background: rgb(var(--panel));
  color: rgb(var(--text-main));
  box-shadow: var(--shadow-pop);
  font-size: 12px;
  line-height: 1.65;
  white-space: pre-wrap;
  overflow-wrap: break-word;
}
</style>
