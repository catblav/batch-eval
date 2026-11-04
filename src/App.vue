<script setup>
import { onMounted, computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { state, loadSettings, attachRunListener, stats, toggleTheme, toast } from "./store";
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

// 自动迭代脚本编译在应用内，导出成一份可直接交给 agent 的工作目录
const packExporting = ref(false);
const packPath = ref("");

async function exportPack() {
  const dir = await open({ directory: true, multiple: false, title: "选择导出到哪个文件夹" });
  if (!dir) return;
  packExporting.value = true;
  try {
    packPath.value = await invoke("export_script_pack", { dest: dir });
    toast("脚本包已导出", "ok");
  } catch (e) {
    toast(String(e), "err");
  } finally {
    packExporting.value = false;
  }
}

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
          <span class="text-accent text-[12px] font-bold">猫</span>
        </div>
        <span class="text-[14px] font-semibold tracking-wide">黑猫-自动评测台</span>
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

            <div class="card p-4">
              <div class="flex items-start justify-between gap-4">
                <div>
                  <div class="text-[13px] font-medium">自动迭代脚本</div>
                  <p class="hint mt-1 max-w-[820px]">
                    给 Claude、Codex 这类 agent 用。导出一个工作目录，里面有脚本、使用说明和标准文件夹；把这个文件夹交给 agent，它就能按你的数据和判定标准反复跑测试、改提示词。
                  </p>
                </div>
                <button class="btn-primary text-[12px] shrink-0" :disabled="packExporting" @click="exportPack">
                  {{ packExporting ? "导出中…" : "导出脚本包" }}
                </button>
              </div>
              <p v-if="packPath" class="hint mt-2">已导出到 {{ packPath }}，打开里面的「怎么用.txt」。</p>
            </div>
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
