<script setup>
import { onMounted, computed } from "vue";
import { state, loadSettings, attachRunListener, stats, toggleTheme } from "./store";
import ApiProfileForm from "./components/ApiProfileForm.vue";
import PromptsPanel from "./components/PromptsPanel.vue";
import DatasetPanel from "./components/DatasetPanel.vue";
import RunPanel from "./components/RunPanel.vue";
import AnalysisPanel from "./components/AnalysisPanel.vue";

const steps = [
  { key: "api", name: "API 配置", icon: "①" },
  { key: "prompt", name: "提示词", icon: "②" },
  { key: "data", name: "数据集", icon: "③" },
  { key: "run", name: "运行", icon: "④" },
  { key: "analysis", name: "分析", icon: "⑤" },
];

const toastVisible = computed(() => !!state.toast.text);

const toastColor = computed(() => {
  const k = state.toast.kind;
  if (k === "ok") return "border-pass/50 text-pass bg-pass/10";
  if (k === "err") return "border-fail/50 text-fail bg-fail/10";
  return "border-accent/50 text-accent bg-accent/10";
});

onMounted(async () => {
  // 进度监听放在全局：切到其他页时 RunPanel 会卸载，事件不能跟着丢
  await attachRunListener();
  await loadSettings();
});
</script>

<template>
  <div class="h-full flex flex-col">
    <header class="flex items-center gap-3 px-5 h-14 border-b border-ink-800 shrink-0">
      <div class="flex items-center gap-2">
        <div class="w-6 h-6 rounded bg-accent/20 border border-accent/40 flex items-center justify-center">
          <span class="text-accent text-[12px] font-bold">评</span>
        </div>
        <span class="text-[14px] font-semibold tracking-wide">批量评测台</span>
      </div>
      <span class="hint">提示词 · 数据集 · 分类聚合</span>
      <span class="flex-1" />
      <button class="btn-ghost text-[12px] px-2" :title="state.theme === 'dark' ? '切换到亮色' : '切换到暗色'"
        @click="toggleTheme">
        {{ state.theme === "dark" ? "☀ 亮色" : "☾ 暗色" }}
      </button>
      <div class="flex gap-1 bg-ink-850 border border-ink-700/60 p-0.5 rounded-md">
        <button v-for="(s, i) in steps" :key="s.key" class="tab flex items-center gap-1.5"
          :class="{ 'tab-active': state.step === i }" @click="state.step = i">
          <span class="opacity-60 text-[11px]">{{ s.icon }}</span>{{ s.name }}
        </button>
      </div>
    </header>

    <main class="flex-1 overflow-auto p-5">
      <div class="max-w-[1280px] mx-auto">
        <template v-if="state.step === 0">
          <div class="space-y-4">
            <ApiProfileForm kind="target" />
            <ApiProfileForm kind="judge" />
            <p class="hint px-1">
              被测模型跑出结果，评测模型拿评测提示词逐条判定。两个可以是同一个 API，也可以分开；判定要稳定时，评测模型建议用 temperature 0。
            </p>
          </div>
        </template>

        <template v-else-if="state.step === 1">
          <PromptsPanel />
        </template>

        <template v-else-if="state.step === 2">
          <DatasetPanel />
        </template>

        <template v-else-if="state.step === 3">
          <RunPanel />
        </template>

        <template v-else>
          <AnalysisPanel />
        </template>
      </div>
    </main>

    <transition name="fade">
      <div v-if="toastVisible"
        class="fixed bottom-5 left-1/2 -translate-x-1/2 px-4 py-2 rounded-md border text-[12.5px] shadow-lg backdrop-blur z-[60] max-w-[70vw] truncate"
        :class="toastColor">
        {{ state.toast.text }}
      </div>
    </transition>
  </div>
</template>

<style scoped>
.fade-enter-active, .fade-leave-active { transition: opacity 0.2s; }
.fade-enter-from, .fade-leave-to { opacity: 0; }
</style>
