<script setup>
import { ref } from "vue";

// 宽度跟着文字走的名称输入框。
// 固定宽度的话，名字短了右边空一大片，紧跟其后的状态文字会被推得很远。
// 做法：同一份文字用隐形 span 排一遍，由它把外层撑到实际文字宽度，input 再铺满外层。
defineProps({
  modelValue: { type: String, default: "" },
  placeholder: { type: String, default: "" },
  maxWidth: { type: Number, default: 360 },
});
const emit = defineEmits(["update:modelValue", "commit", "cancel"]);

const el = ref(null);

defineExpose({
  el,
  focus: () => el.value?.focus(),
  select: () => el.value?.select(),
});
</script>

<template>
  <span
    class="relative inline-flex h-8 shrink-0"
    :style="{ maxWidth: maxWidth + 'px', minWidth: '96px' }"
  >
    <!-- 隐形测量尺：字体、字号、字重、内边距、边框都要和 input 一致，量出来才准 -->
    <span
      class="invisible whitespace-pre border border-transparent px-1.5 text-[15px] font-semibold"
      aria-hidden="true"
    >{{ modelValue || placeholder }}</span>
    <input
      ref="el"
      class="absolute inset-0 w-full border border-transparent rounded px-1.5 bg-transparent
        text-[15px] font-semibold text-main placeholder:text-weak
        hover:border-line focus:border-accent transition-colors"
      :value="modelValue"
      :placeholder="placeholder"
      spellcheck="false"
      @input="emit('update:modelValue', $event.target.value)"
      @blur="emit('commit', $event)"
      @keydown.enter.prevent="$event.target.blur()"
      @keydown.esc="emit('cancel', $event); $event.target.blur()"
    />
  </span>
</template>
