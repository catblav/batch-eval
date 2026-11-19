<script setup>
import Icon from "./Icon.vue";

defineProps({
  icon: { type: String, default: "file" },
  title: { type: String, required: true },
  hint: { type: String, default: "" },
  // "md" 撑满容器居中，"sm" 给侧栏列表这类窄区域
  size: { type: String, default: "md" },
});
</script>

<template>
  <div
    class="flex flex-col items-center justify-center text-center"
    :class="size === 'sm' ? 'gap-2.5 py-10 px-4' : 'gap-3 h-full px-6'"
  >
    <span class="empty-icon" :class="size === 'sm' ? '!w-9 !h-9' : ''">
      <Icon :name="icon" :size="size === 'sm' ? 16 : 20" :stroke-width="1.5" />
    </span>
    <div class="space-y-1">
      <p class="text-[13px] font-medium text-main">{{ title }}</p>
      <p v-if="hint" class="hint max-w-[300px]">{{ hint }}</p>
    </div>
    <slot />
  </div>
</template>
