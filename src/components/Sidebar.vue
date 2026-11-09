<script setup>
import { computed } from "vue";
import { state, PAGES, toggleTheme, savedText, usePage } from "../store";
import Icon from "./Icon.vue";

const saveText = computed(() => {
  if (state.save.error) return "保存失败";
  if (state.save.saving) return "保存中…";
  return savedText.value || "自动保存";
});
</script>

<template>
  <aside class="w-[204px] shrink-0 flex flex-col bg-sidebar border-r border-line">
    <div class="h-14 px-4 flex items-center gap-2.5 border-b border-line">
      <span class="w-7 h-7 rounded bg-btn text-btnfg flex items-center justify-center">
        <Icon name="cat" :size="17" :stroke-width="1.7" />
      </span>
      <div class="min-w-0">
        <div class="text-[13.5px] font-semibold leading-tight truncate">黑猫评测台</div>
        <div class="text-[11.5px] text-weak leading-tight">批量评测</div>
      </div>
    </div>

    <nav class="p-2 space-y-0.5">
      <button
        v-for="p in PAGES"
        :key="p.key"
        class="nav-item"
        :class="{ 'nav-item-active': state.page === p.key }"
        @click="usePage(p.key)"
      >
        <Icon :name="p.icon" :size="16" />
        <span>{{ p.name }}</span>
        <span
          v-if="p.key === 'run' && state.jobs.list.some((j) => j.status === 'running')"
          class="ml-auto flex items-center gap-1 text-[11px] text-sub"
        >
          <span class="dot bg-pass animate-pulse" />
          {{ state.jobs.list.filter((j) => j.status === "running").length }}
        </span>
      </button>
    </nav>

    <div class="mt-auto border-t border-line p-2.5 space-y-1">
      <div class="px-1 text-[11.5px] truncate" :class="state.save.error ? 'text-fail' : 'text-weak'">
        {{ saveText }}
      </div>
      <button class="nav-item h-8" @click="toggleTheme">
        <Icon :name="state.theme === 'dark' ? 'sun' : 'moon'" :size="15" />
        <span>{{ state.theme === "dark" ? "亮色模式" : "暗色模式" }}</span>
      </button>
    </div>
  </aside>
</template>
