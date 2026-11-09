<script setup>
import { onMounted, onBeforeUnmount, computed } from "vue";
import { state, loadSettings, attachRunListener, flushSave } from "./store";
import Sidebar from "./components/Sidebar.vue";
import Icon from "./components/Icon.vue";
import ConnectionsPanel from "./components/ConnectionsPanel.vue";
import PromptsPanel from "./components/PromptsPanel.vue";
import DatasetPanel from "./components/DatasetPanel.vue";
import RunPanel from "./components/RunPanel.vue";
import AnalysisPanel from "./components/AnalysisPanel.vue";

const toastVisible = computed(() => !!state.toast.text);
const toastStyle = computed(() => {
  const k = state.toast.kind;
  if (k === "ok") return "border-pass/40 text-pass";
  if (k === "err") return "border-fail/40 text-fail";
  return "border-line text-sub";
});

function onBlur() {
  flushSave();
}
function onVisibility() {
  if (document.visibilityState === "hidden") flushSave();
}

onMounted(async () => {
  await attachRunListener();
  await loadSettings();
  window.addEventListener("blur", onBlur);
  document.addEventListener("visibilitychange", onVisibility);
});

onBeforeUnmount(() => {
  window.removeEventListener("blur", onBlur);
  document.removeEventListener("visibilitychange", onVisibility);
});
</script>

<template>
  <div class="h-full flex bg-page text-main">
    <Sidebar />

    <div class="flex-1 min-w-0 flex flex-col">
      <div
        v-if="state.loadError"
        class="shrink-0 flex items-center gap-2 px-5 h-9 bg-fail/10 border-b border-fail/30 text-fail text-[12.5px]"
      >
        <Icon name="alert" :size="14" />
        <span class="truncate">读取配置失败，未自动保存以免覆盖原文件：{{ state.loadError }}</span>
      </div>

      <main class="flex-1 min-h-0 overflow-hidden">
        <ConnectionsPanel v-if="state.page === 'connections'" />
        <PromptsPanel v-else-if="state.page === 'plans'" />
        <DatasetPanel v-else-if="state.page === 'datasets'" />
        <RunPanel v-else-if="state.page === 'run'" />
        <AnalysisPanel v-else-if="state.page === 'analysis'" />
      </main>
    </div>

    <transition name="fade">
      <div
        v-if="toastVisible"
        class="fixed right-5 bottom-5 z-[60] flex items-start gap-2 max-w-[420px]
          px-3.5 py-2.5 rounded border bg-panel shadow-sm text-[12.5px] leading-relaxed"
        :class="toastStyle"
      >
        <Icon :name="state.toast.kind === 'ok' ? 'check' : state.toast.kind === 'err' ? 'alert' : 'clock'"
          :size="14" class="mt-0.5" />
        <span class="text-main">{{ state.toast.text }}</span>
      </div>
    </transition>
  </div>
</template>

<style scoped>
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.16s ease;
}
.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
</style>
