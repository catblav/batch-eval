<script setup>
import { computed, onMounted, ref } from "vue";
import { state, savedText } from "../store";

const version = ref("");
const runningCount = computed(
  () => state.jobs.list.filter((j) => j.status === "running").length,
);
const saveText = computed(() => {
  if (state.save.error) return "保存失败";
  if (state.save.saving) return "保存中…";
  return savedText.value || "已同步";
});
const saveDot = computed(() =>
  state.save.error
    ? "bg-fail"
    : state.save.saving
      ? "bg-warn animate-pulse"
      : "bg-pass/70",
);

onMounted(async () => {
  try {
    const { getVersion } = await import("@tauri-apps/api/app");
    version.value = await getVersion();
  } catch {
    version.value = "";
  }
});
</script>

<template>
  <footer
    class="h-6 shrink-0 flex items-center gap-3 px-2.5 bg-sidebar border-t border-line/70
      text-[11.5px] text-weak select-none"
  >
    <span class="flex items-center gap-1.5">
      <span class="dot" :class="saveDot" />
      {{ saveText }}
    </span>
    <span v-if="runningCount" class="flex items-center gap-1.5 text-sub">
      <span class="dot bg-pass animate-pulse" />
      运行中 {{ runningCount }}
    </span>
    <span class="flex-1" />
    <span v-if="version">黑猫评测台 v{{ version }}</span>
  </footer>
</template>
