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

// 自动迭代脚本随应用安装，用户自己建工作目录，这里只给路径和可复制的引导语
const iteratePaths = ref(null);
const guideText = ref("");

async function loadIteratePaths() {
  try {
    iteratePaths.value = await invoke("auto_iterate_paths");
  } catch (e) {
    iteratePaths.value = null;
  }
}

async function pickWorkdir() {
  const dir = await open({ directory: true, multiple: false, title: "选择放着提示词和数据集的工作目录" });
  if (!dir) return;
  guideText.value = `请读取 ${iteratePaths.value.guide}，按里面的说明帮我评测数据和改进提示词。工作目录：${dir}`;
  const ok = await copyText(guideText.value);
  toast(ok ? "引导语已复制" : "已生成引导语，请手动复制", ok ? "ok" : "info");
}

async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch (e) {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    let ok = false;
    try {
      ok = document.execCommand("copy");
    } catch (_) {}
    document.body.removeChild(ta);
    return ok;
  }
}

onMounted(async () => {
  // 进度监听放在全局：切到其他页时 RunPanel 会卸载，事件不能跟着丢
  await attachRunListener();
  await loadSettings();
  await loadIteratePaths();
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

            <div class="card p-4 space-y-2">
              <div class="flex items-start justify-between gap-4">
                <div>
                  <div class="text-[13px] font-medium">自动迭代脚本</div>
                  <p class="hint mt-1 max-w-[820px]">
                    给 Claude、Codex 这类 agent 用。脚本和说明随应用安装。选一个工作目录，把被测提示词、评测提示词和数据集放进去，再把下面这句话发给 agent；它会建好子目录并接着跑。
                  </p>
                  <p v-if="iteratePaths" class="hint mt-1 break-all">脚本：{{ iteratePaths.script }}</p>
                  <p v-else class="hint mt-1">读取脚本路径失败，请重试或重新安装应用。</p>
                </div>
                <button class="btn-primary text-[12px] shrink-0" :disabled="!iteratePaths" @click="pickWorkdir">
                  选择工作目录
                </button>
              </div>
              <textarea v-if="guideText" readonly :value="guideText" rows="3"
                class="w-full text-[12px] bg-ink-850 border border-ink-700/60 rounded p-2 resize-none"
                @focus="$event.target.select()"></textarea>
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
