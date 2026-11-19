<script setup>
import { onMounted, onBeforeUnmount, computed } from "vue";
import { state, loadSettings, attachRunListener, flushSave } from "./store";
import Sidebar from "./components/Sidebar.vue";
import StatusBar from "./components/StatusBar.vue";
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
  <div class="h-full flex flex-col bg-page text-main">
    <div class="flex-1 flex min-h-0">
      <Sidebar />

      <div class="flex-1 min-w-0 flex flex-col">
        <div
          v-if="state.loadError"
          class="shrink-0 mx-2.5 mt-2.5 flex items-center gap-2 px-3 h-9 rounded-md
            bg-fail/10 border border-fail/30 text-fail text-[12.5px]"
        >
          <Icon name="alert" :size="14" />
          <span class="truncate">读取配置失败，未自动保存以免覆盖原文件：{{ state.loadError }}</span>
        </div>

        <!-- 主区留 10px 边距，里面每个面板自己是一张卡片，浮在页面底色上 -->
        <main class="flex-1 min-h-0 overflow-hidden p-2.5">
          <transition name="page" mode="out-in">
            <ConnectionsPanel v-if="state.page === 'connections'" key="connections" />
            <PromptsPanel v-else-if="state.page === 'plans'" key="plans" />
            <DatasetPanel v-else-if="state.page === 'datasets'" key="datasets" />
            <RunPanel v-else-if="state.page === 'run'" key="run" />
            <AnalysisPanel v-else-if="state.page === 'analysis'" key="analysis" />
          </transition>
        </main>
      </div>
    </div>

    <StatusBar />

    <transition name="toast">
      <div
        v-if="toastVisible"
        class="fixed right-5 bottom-5 z-[60] flex items-start gap-2 max-w-[420px]
          px-3.5 py-2.5 rounded-lg border bg-panel shadow-pop text-[12.5px] leading-relaxed"
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
/* 面板切换只用透明度：一旦加 transform，面板内的 fixed 遮罩
   （详情抽屉、提示词弹窗）会改为相对面板定位并被卡片裁掉 */
.page-enter-active,
.page-leave-active {
  transition: opacity 0.12s ease;
}
.page-enter-from,
.page-leave-to {
  opacity: 0;
}

.toast-enter-active {
  transition:
    opacity 0.22s cubic-bezier(0.2, 0.8, 0.2, 1),
    transform 0.22s cubic-bezier(0.2, 0.8, 0.2, 1);
}
.toast-leave-active {
  transition:
    opacity 0.16s ease,
    transform 0.16s ease;
}
.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateY(8px) scale(0.98);
}
</style>
