<script setup>
import { computed, ref } from "vue";

const props = defineProps({
  modelValue: { type: String, default: "" },
  placeholder: { type: String, default: "" },
  // 数据集当前可用的列名，用来区分「认得的变量」和「数据集里没有的」
  knownVars: { type: Array, default: () => [] },
});
const emit = defineEmits(["update:modelValue"]);

const ta = ref(null);
const layer = ref(null);

function esc(s) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

// 双花括号变量染色。只加背景和字色，不加 padding，
// 否则高亮层和 textarea 的换行位置会错开
const html = computed(() => {
  const src = props.modelValue || "";
  const re = /\{\{\s*([^{}]+?)\s*\}\}/g;
  let out = "";
  let last = 0;
  let m;
  while ((m = re.exec(src))) {
    out += esc(src.slice(last, m.index));
    const known = props.knownVars.includes(m[1]);
    out += `<span class="${known ? "hl-ok" : "hl-bad"}">${esc(m[0])}</span>`;
    last = m.index + m[0].length;
  }
  // 末尾补一个换行，最后一行是空行时高度才对得上
  return out + esc(src.slice(last)) + "\n";
});

function onScroll() {
  if (layer.value && ta.value) {
    layer.value.scrollTop = ta.value.scrollTop;
    layer.value.scrollLeft = ta.value.scrollLeft;
  }
}

defineExpose({ el: ta });
</script>

<template>
  <div class="hl-wrap">
    <div ref="layer" class="hl-layer" aria-hidden="true" v-html="html" />
    <textarea
      ref="ta"
      class="hl-input"
      :value="modelValue"
      :placeholder="placeholder"
      spellcheck="false"
      @input="emit('update:modelValue', $event.target.value)"
      @scroll="onScroll"
    />
  </div>
</template>

<style scoped>
.hl-wrap {
  position: relative;
  flex: 1;
  min-height: 0;
}

/* 两层必须逐项对齐：同字体、同字号、同行高、同内边距、同换行规则。
   textarea 常驻滚动条，所以高亮层也常驻一条隐形滚动条占同样的宽度 */
.hl-layer,
.hl-input {
  position: absolute;
  inset: 0;
  padding: 16px;
  font-family: inherit;
  font-size: 12.5px;
  line-height: 1.625;
  white-space: pre-wrap;
  overflow-wrap: break-word;
  tab-size: 2;
}

.hl-layer {
  overflow-y: scroll;
  overflow-x: hidden;
  pointer-events: none;
  color: rgb(var(--text-main));
  scrollbar-width: none;
}
.hl-layer::-webkit-scrollbar {
  display: none;
}

.hl-input {
  overflow-y: scroll;
  resize: none;
  border: 0;
  background: transparent;
  color: transparent;
  caret-color: rgb(var(--text-main));
  outline: none;
}
.hl-input::placeholder {
  color: rgb(var(--text-weak));
}

.hl-layer :deep(.hl-ok) {
  color: rgb(var(--accent));
  background: rgb(var(--accent) / 0.14);
  border-radius: 3px;
}
.hl-layer :deep(.hl-bad) {
  color: rgb(var(--warn));
  background: rgb(var(--warn) / 0.16);
  border-radius: 3px;
}
</style>
