<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import {
  state,
  toast,
  PAGES,
  toggleTheme,
  toggleSidebar,
  usePage,
  applyDataDir,
  openDataDir,
  applyExportDir,
  openExportDir,
} from "../store";
import { open } from "@tauri-apps/plugin-dialog";
import Icon from "./Icon.vue";

const collapsed = computed(() => state.sidebarCollapsed);
const runningCount = computed(() => state.jobs.list.filter((j) => j.status === "running").length);

const themeLabel = computed(() => (state.theme === "dark" ? "亮色模式" : "暗色模式"));

// ---------- 数据保存位置 ----------
const dataMenu = ref(false);
const busy = ref(false);
// 选完文件夹先停在这，等用户在弹窗里选「复制」还是「不复制」再真正切
const pendingDir = ref("");

function toggleDataMenu() {
  dataMenu.value = !dataMenu.value;
}
function closeDataMenu() {
  dataMenu.value = false;
}

async function pickDataDir() {
  if (busy.value) return;
  // 任务写到一半换目录会让结果分叉，直接拦下来
  if (state.jobs.list.some((j) => ["running", "cancelling"].includes(j.status))) {
    toast("有任务正在运行，等跑完再改数据位置", "info", 5000);
    return;
  }
  let dir = null;
  try {
    dir = await open({ directory: true, title: "选择数据保存文件夹" });
  } catch (e) {
    toast("打开文件夹选择器失败：" + e, "err", 8000);
    return;
  }
  if (!dir) return;
  dataMenu.value = false;
  pendingDir.value = String(dir);
}

function cancelSwitch() {
  if (busy.value) return;
  pendingDir.value = "";
}

async function doSwitch(copy) {
  const dir = pendingDir.value;
  if (!dir || busy.value) return;
  busy.value = true;
  try {
    const r = await applyDataDir(dir, copy);
    const copied = r.runs || r.datasets ? `，复制了 ${r.runs} 条任务记录、${r.datasets} 个数据集` : "";
    toast(`数据位置已改为 ${dir}${copied}`, "ok", 6000);
    pendingDir.value = "";
  } catch (e) {
    toast("切换数据位置失败：" + e, "err", 8000);
  } finally {
    busy.value = false;
  }
}

// 恢复默认位置也走同一个弹窗：换回去一样会把现在这堆数据落在原处看不见，
// 得让用户自己选要不要复制
function resetDataDir() {
  if (busy.value) return;
  dataMenu.value = false;
  pendingDir.value = state.dataDir.defaultDir;
}

async function doOpenDataDir() {
  dataMenu.value = false;
  await openDataDir();
}

// ---------- 评测报告导出位置 ----------
// 和数据位置分开：导出目录只是落盘位置，不涉及任务和数据集，切错了也不影响历史数据
async function pickExportDir() {
  if (busy.value) return;
  let dir = null;
  try {
    dir = await open({ directory: true, title: "选择评测报告导出文件夹" });
  } catch (e) {
    toast("打开文件夹选择器失败：" + e, "err", 8000);
    return;
  }
  if (!dir) return;
  busy.value = true;
  try {
    await applyExportDir(String(dir));
    toast(`导出位置已改为 ${dir}`, "ok", 5000);
  } catch (e) {
    toast("设置导出位置失败：" + e, "err", 8000);
  } finally {
    busy.value = false;
  }
}

async function clearExportDir() {
  if (busy.value) return;
  busy.value = true;
  try {
    await applyExportDir("");
    toast("已恢复：每次导出自己选保存位置", "ok", 5000);
  } catch (e) {
    toast("恢复失败：" + e, "err", 8000);
  } finally {
    busy.value = false;
  }
}

async function doOpenExportDir() {
  dataMenu.value = false;
  await openExportDir();
}

onMounted(() => window.addEventListener("click", closeDataMenu));
onBeforeUnmount(() => window.removeEventListener("click", closeDataMenu));
</script>

<template>
  <aside
    class="shrink-0 flex flex-col bg-sidebar border-r border-line transition-[width] duration-150"
    :class="collapsed ? 'w-[56px]' : 'w-[204px]'"
  >
    <div
      class="h-14 flex items-center border-b border-line/70 shrink-0"
      :class="collapsed ? 'justify-center px-0' : 'px-4 gap-2.5'"
    >
      <span
        class="w-7 h-7 rounded-md bg-btn text-btnfg flex items-center justify-center shrink-0 shadow-sm"
      >
        <Icon name="cat" :size="17" :stroke-width="1.7" />
      </span>
      <div v-if="!collapsed" class="min-w-0">
        <div class="text-[13.5px] font-semibold leading-tight truncate">黑猫评测台</div>
        <div class="text-[11.5px] text-weak leading-tight">批量评测</div>
      </div>
      <button
        v-if="!collapsed"
        class="icon-btn ml-auto"
        title="收起侧栏"
        @click="toggleSidebar"
      >
        <Icon name="chevron-left" :size="15" />
      </button>
    </div>

    <nav class="p-2 space-y-0.5">
      <button
        v-for="p in PAGES"
        :key="p.key"
        class="nav-item"
        :class="{ 'nav-item-active': state.page === p.key, 'justify-center px-0': collapsed }"
        :title="collapsed ? p.name : undefined"
        @click="usePage(p.key)"
      >
        <Icon :name="p.icon" :size="16" />
        <span v-if="!collapsed">{{ p.name }}</span>
        <span
          v-if="!collapsed && p.key === 'run' && runningCount"
          class="ml-auto flex items-center gap-1 text-[11px] text-sub"
        >
          <span class="dot bg-pass animate-pulse" />
          {{ runningCount }}
        </span>
        <span
          v-else-if="collapsed && p.key === 'run' && runningCount"
          class="absolute top-1.5 right-1.5 w-1.5 h-1.5 rounded-full bg-pass animate-pulse"
        />
      </button>

      <button
        v-if="collapsed"
        class="nav-item justify-center px-0"
        title="展开侧栏"
        @click="toggleSidebar"
      >
        <Icon name="chevron-right" :size="16" />
      </button>
    </nav>

    <div class="mt-auto border-t border-line/70 p-2 space-y-0.5">
      <div class="relative">
        <button
          class="nav-item h-8"
          :class="{ 'justify-center px-0': collapsed }"
          :title="collapsed ? state.dataDir.current || '数据位置' : undefined"
          @click.stop="toggleDataMenu"
        >
          <Icon name="folder" :size="15" />
          <span v-if="!collapsed">数据位置</span>
          <Icon
            v-if="!collapsed"
            name="chevron-down"
            :size="12"
            class="ml-auto text-weak"
            :class="{ 'rotate-180': dataMenu }"
          />
        </button>

        <!-- 侧栏在最底部，菜单往上弹 -->
        <div
          v-if="dataMenu"
          class="absolute bottom-9 left-0 z-30 w-[300px] p-2.5 rounded-lg border border-line bg-panel shadow-pop"
          @click.stop
        >
          <div class="text-[12px] font-medium mb-1">数据保存位置</div>
          <div class="text-[11.5px] leading-relaxed break-all select-text text-sub mb-1.5">
            {{ state.dataDir.current }}
          </div>
          <div class="text-[11.5px] text-weak mb-2">
            {{ state.dataDir.runs }} 条任务记录 · {{ state.dataDir.datasets }} 个数据集
            <span v-if="state.dataDir.isDefault">（默认位置）</span>
          </div>
          <p v-if="!state.dataDir.exists" class="mb-2 text-[11.5px] leading-relaxed text-fail">
            这个文件夹现在打不开，可能是被删了或者盘没插。选个新位置，或者恢复默认。
          </p>
          <div class="flex items-center gap-1.5">
            <button class="btn btn-sm" :disabled="busy" @click="pickDataDir">选择文件夹</button>
            <button class="btn-ghost btn-sm" @click="doOpenDataDir">打开文件夹</button>
          </div>
          <button
            v-if="!state.dataDir.isDefault"
            class="btn-ghost btn-sm w-full mt-1"
            :disabled="busy"
            @click="resetDataDir"
          >
            恢复默认位置
          </button>
          <div class="mt-2.5 pt-2.5 border-t border-line/60">
            <div class="text-[12px] font-medium mb-1">评测报告导出位置</div>
            <div
              v-if="state.exportDir.set"
              class="text-[11.5px] leading-relaxed break-all select-text text-sub mb-1.5"
            >
              {{ state.exportDir.current }}
            </div>
            <div v-else class="text-[11.5px] text-weak mb-1.5">
              未设置，导出时弹窗选择保存位置
            </div>
            <p
              v-if="state.exportDir.set && !state.exportDir.exists"
              class="mb-2 text-[11.5px] leading-relaxed text-fail"
            >
              这个文件夹现在打不开，导出时会退回弹窗选择。
            </p>
            <div class="flex items-center gap-1.5 flex-wrap">
              <button class="btn btn-sm" :disabled="busy" @click="pickExportDir">选择文件夹</button>
              <button
                v-if="state.exportDir.set"
                class="btn-ghost btn-sm"
                @click="doOpenExportDir"
              >
                打开文件夹
              </button>
              <button
                v-if="state.exportDir.set"
                class="btn-ghost btn-sm"
                :disabled="busy"
                @click="clearExportDir"
              >
                恢复弹窗
              </button>
            </div>
          </div>
        </div>
      </div>

      <button
        class="nav-item h-8"
        :class="{ 'justify-center px-0': collapsed }"
        :title="collapsed ? themeLabel : undefined"
        @click="toggleTheme"
      >
        <Icon :name="state.theme === 'dark' ? 'sun' : 'moon'" :size="15" />
        <span v-if="!collapsed">{{ themeLabel }}</span>
      </button>
    </div>
  </aside>

  <!-- 换位置前把「复制 / 不复制」摆明白再动手。原生 ask 紧接着系统文件夹选择器弹出来会被
       上一次点击直接点掉，看不到就换过去了 -->
  <div
    v-if="pendingDir"
    class="fixed inset-0 z-50 flex items-center justify-center p-6 bg-black/35"
    @click.self="cancelSwitch"
  >
    <div class="w-[460px] max-w-full rounded-xl border border-line bg-panel shadow-pop">
      <div class="px-4 py-3 border-b border-line/60 flex items-center gap-2">
        <Icon name="folder" :size="15" class="text-weak shrink-0" />
        <span class="text-[13.5px] font-medium">切换数据保存位置</span>
        <span class="flex-1" />
        <button class="icon-btn" title="取消" :disabled="busy" @click="cancelSwitch">
          <Icon name="close" :size="15" />
        </button>
      </div>

      <div class="px-4 py-3">
        <div class="text-[11.5px] text-weak mb-1">新位置</div>
        <div class="text-[12px] break-all select-text mb-3">{{ pendingDir }}</div>
        <p class="text-[12px] text-sub leading-relaxed">
          现在有 {{ state.dataDir.runs }} 条任务记录、{{ state.dataDir.datasets }} 个数据集。
          换到新位置之后它们怎么办？
        </p>
      </div>

      <div class="px-4 pb-4 space-y-1.5">
        <button
          class="w-full text-left rounded-lg border border-line hover:border-accent hover:bg-wash px-3 py-2.5 transition-colors disabled:opacity-50"
          :disabled="busy"
          @click="doSwitch(true)"
        >
          <div class="text-[12.5px] font-medium mb-0.5">全部复制过去</div>
          <div class="text-[11.5px] text-weak leading-relaxed">
            新位置能看到全部历史数据。原位置的文件保留不删，确认没问题后可以自己清理。
          </div>
        </button>
        <button
          class="w-full text-left rounded-lg border border-line hover:border-accent hover:bg-wash px-3 py-2.5 transition-colors disabled:opacity-50"
          :disabled="busy"
          @click="doSwitch(false)"
        >
          <div class="text-[12.5px] font-medium mb-0.5">不复制，只换位置</div>
          <div class="text-[11.5px] text-weak leading-relaxed">
            老数据留在原位置，应用之后只读新位置，会看不到它们，用到这些数据集的方案也跑不了。
          </div>
        </button>
      </div>
    </div>
  </div>
</template>
