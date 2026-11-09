<script setup>
import { ref, computed, watch, onMounted, onBeforeUnmount } from "vue";
import {
  state,
  toast,
  datasetMetaById,
  loadDatasetPreview,
  importDatasetFile,
  importDatasetText,
  replaceDataset,
  renameDataset,
  removeDataset,
} from "../store";
import { open, ask } from "@tauri-apps/plugin-dialog";
import Icon from "./Icon.vue";
import Pager from "./Pager.vue";

const search = ref("");
const pasteOpen = ref(false);
const pasteText = ref("");
const pasteName = ref("");
const pasteFormat = ref("auto");
const busy = ref(false);
const page = ref(1);
const pageSize = ref(50);
const menuId = ref("");
const nameDraft = ref("");
const nameInput = ref(null);

const active = computed(() => datasetMetaById(state.activeDatasetId));
const preview = computed(() => state.datasetPreview);
const total = computed(() => preview.value.id === state.activeDatasetId ? preview.value.total : 0);
const rows = computed(() => preview.value.id === state.activeDatasetId ? preview.value.rows : []);
const columns = computed(() => preview.value.id === state.activeDatasetId ? preview.value.columns : []);
const offset = computed(() => (page.value - 1) * pageSize.value);
const large = computed(() => total.value > 20000);
const usedBy = computed(() =>
  active.value ? state.plans.filter((p) => p.datasetId === active.value.id) : []
);
const statusCol = computed(() => columns.value.includes("数据组状态"));

const filtered = computed(() => {
  const q = search.value.trim().toLowerCase();
  if (!q) return state.datasets;
  return state.datasets.filter((d) =>
    [d.name, d.source].join(" ").toLowerCase().includes(q)
  );
});

watch(
  () => state.activeDatasetId,
  async (id) => {
    page.value = 1;
    nameDraft.value = active.value?.name || "";
    if (id) await loadDatasetPreview(id, 0, pageSize.value);
  },
  { immediate: true }
);

watch(
  () => active.value?.name,
  (n) => {
    if (n && nameDraft.value === "") nameDraft.value = n;
  }
);

async function loadPage(p) {
  if (!active.value) return;
  page.value = p;
  await loadDatasetPreview(active.value.id, (p - 1) * pageSize.value, pageSize.value);
}

async function onPageSize(n) {
  pageSize.value = n;
  page.value = 1;
  if (active.value) await loadDatasetPreview(active.value.id, 0, n);
}

async function commitName() {
  if (!active.value) return;
  const name = nameDraft.value.trim();
  if (!name) {
    nameDraft.value = active.value.name;
    return;
  }
  if (name === active.value.name) return;
  try {
    await renameDataset(active.value.id, name);
    toast("已重命名", "ok");
  } catch (e) {
    nameDraft.value = active.value.name;
    toast("重命名失败：" + e, "err");
  }
}

async function chooseFile() {
  try {
    const path = await open({
      multiple: false,
      filters: [
        {
          name: "数据集",
          extensions: ["csv", "tsv", "xlsx", "xlsm", "json", "jsonl", "txt", "md"],
        },
      ],
    });
    if (!path) return;
    busy.value = true;
    await importDatasetFile(path);
    page.value = 1;
  } catch (e) {
    toast("导入失败：" + e, "err");
  } finally {
    busy.value = false;
  }
}

async function submitPaste() {
  if (!pasteText.value.trim()) {
    toast("先粘贴数据", "info");
    return;
  }
  busy.value = true;
  try {
    await importDatasetText(
      pasteText.value,
      pasteFormat.value === "auto" ? null : pasteFormat.value,
      pasteName.value.trim() || null
    );
    pasteOpen.value = false;
    pasteText.value = "";
    pasteName.value = "";
    page.value = 1;
  } catch (e) {
    toast("解析失败：" + e, "err");
  } finally {
    busy.value = false;
  }
}

function openMenu() {
  menuId.value = menuId.value ? "" : "main";
}

async function replace() {
  menuId.value = "";
  if (!active.value) return;
  const path = await open({
    multiple: false,
    filters: [
      {
        name: "数据集",
        extensions: ["csv", "tsv", "xlsx", "xlsm", "json", "jsonl", "txt", "md"],
      },
    ],
  });
  if (!path) return;
  const ok = await ask(
    `用新文件替换「${active.value.name}」的全部数据？\n\n已完成的评测保留自己的数据快照，不受影响；正在运行或中断的任务可能无法继续。`,
    { title: "替换数据", kind: "warning", okLabel: "替换", cancelLabel: "取消" }
  );
  if (!ok) return;
  busy.value = true;
  try {
    await replaceDataset(active.value.id, path);
    page.value = 1;
  } catch (e) {
    toast("替换失败：" + e, "err");
  } finally {
    busy.value = false;
  }
}

function blockedByJob() {
  return state.jobs.list.find(
    (j) =>
      j.datasetId === active.value?.id &&
      ["running", "cancelling", "interrupted"].includes(j.status)
  );
}

async function remove() {
  menuId.value = "";
  if (!active.value) return;
  const job = blockedByJob();
  if (job) {
    toast(`任务「${job.title}」正在使用这个数据集，先取消或等它结束`, "err");
    return;
  }
  const affected = usedBy.value.map((p) => p.name);
  const msg = affected.length
    ? `删除「${active.value.name}」？\n\n以下方案会失去数据集：${affected.join("、")}`
    : `删除「${active.value.name}」？`;
  const ok = await ask(msg, {
    title: "删除数据集",
    kind: "warning",
    okLabel: "删除",
    cancelLabel: "取消",
  });
  if (!ok) return;
  try {
    await removeDataset(active.value.id);
    toast("已删除数据集", "ok");
  } catch (e) {
    toast("删除失败：" + e, "err");
  }
}

function sample(v) {
  const s = String(v ?? "");
  return s.length > 120 ? s.slice(0, 120) + "…" : s;
}

function closeMenu() {
  menuId.value = "";
}

onMounted(() => window.addEventListener("click", closeMenu));
onBeforeUnmount(() => window.removeEventListener("click", closeMenu));
</script>

<template>
  <div class="h-full flex bg-page">
    <aside class="w-[280px] shrink-0 flex flex-col border-r border-line min-h-0">
      <div class="h-14 px-4 flex items-center gap-2 shrink-0">
        <h1 class="text-[15px] font-semibold">数据集</h1>
        <span class="text-[12px] text-weak tabular-nums">{{ state.datasets.length }}</span>
        <span class="flex-1" />
        <button class="icon-btn" title="导入数据集" :disabled="busy" @click="chooseFile">
          <Icon name="plus" :size="16" />
        </button>
      </div>
      <div class="px-3 pb-2 shrink-0">
        <div class="relative">
          <Icon name="search" :size="14"
            class="absolute left-2 top-1/2 -translate-y-1/2 text-weak pointer-events-none" />
          <input v-model="search" class="field pl-7 h-8" placeholder="搜索数据集" />
        </div>
      </div>
      <div class="flex-1 min-h-0 overflow-y-auto px-2 pb-3 space-y-0.5">
        <div v-if="!state.datasets.length" class="px-3 py-10 text-center">
          <p class="text-[13px] text-sub">还没有数据集</p>
          <p class="hint mt-1.5">支持 CSV / Excel / JSON / 纯文本。</p>
        </div>
        <p v-else-if="!filtered.length" class="px-3 py-8 text-center hint">没有匹配的数据集</p>
        <button
          v-for="d in filtered"
          :key="d.id"
          class="list-item w-full"
          :class="{ 'list-item-active': d.id === state.activeDatasetId }"
          @click="state.activeDatasetId = d.id"
        >
          <div class="text-[13px] font-medium truncate">{{ d.name }}</div>
          <div class="mt-0.5 text-[11.5px] text-weak tabular-nums">
            {{ d.rows.toLocaleString() }} 行 · {{ d.columns.length }} 列
          </div>
        </button>
      </div>
    </aside>

    <section class="flex-1 min-w-0 flex flex-col bg-panel">
      <template v-if="active">
        <div class="h-14 px-5 flex items-center gap-2.5 border-b border-line shrink-0">
          <input
            ref="nameInput"
            v-model="nameDraft"
            class="min-w-0 w-[240px] h-8 px-1.5 rounded border border-transparent bg-transparent
              text-[15px] font-semibold text-main hover:border-line focus:border-accent transition-colors"
            placeholder="数据集名称"
            @blur="commitName"
            @keydown.enter.prevent="$event.target.blur()"
            @keydown.esc="nameDraft = active.name; $event.target.blur()"
          />
          <span class="text-[11.5px] text-weak shrink-0">
            {{ usedBy.length ? `被 ${usedBy.length} 个方案使用` : "暂无方案使用" }}
          </span>
          <span class="flex-1" />
          <button class="btn btn-sm" @click="pasteOpen = !pasteOpen">粘贴数据</button>
          <button class="btn-primary btn-sm" :disabled="busy" @click="chooseFile">
            {{ busy ? "处理中…" : "导入文件" }}
          </button>
          <div class="relative">
            <button class="icon-btn" title="更多操作" @click.stop="openMenu">
              <Icon name="more" :size="16" />
            </button>
            <div
              v-if="menuId"
              class="absolute right-0 top-8 z-20 w-28 py-1 rounded border border-line bg-panel shadow-sm"
              @click.stop
            >
              <button class="w-full text-left px-3 h-7 text-[12.5px] hover:bg-wash" @click="replace">
                替换数据
              </button>
              <button class="w-full text-left px-3 h-7 text-[12.5px] text-fail hover:bg-fail/10" @click="remove">
                删除数据集
              </button>
            </div>
          </div>
        </div>

        <div v-if="pasteOpen" class="shrink-0 px-5 py-3 border-b border-line bg-page space-y-2">
          <div class="flex items-center gap-2">
            <input v-model="pasteName" class="field w-52" placeholder="名称（留空为「粘贴数据」）" />
            <select v-model="pasteFormat" class="field w-36">
              <option value="auto">自动识别</option>
              <option value="json">JSON</option>
              <option value="csv">CSV / TSV</option>
              <option value="text">纯文本块</option>
            </select>
            <span class="flex-1" />
            <button class="btn-ghost btn-sm" @click="pasteOpen = false">取消</button>
            <button class="btn-primary btn-sm" :disabled="busy" @click="submitPaste">
              {{ busy ? "解析中…" : "导入" }}
            </button>
          </div>
          <textarea
            v-model="pasteText"
            rows="6"
            class="field-mono"
            placeholder="JSON 数组 / 每行一个 JSON / CSV 表头加数据 / 空行分隔的文本块"
          />
        </div>

        <div class="shrink-0 px-5 py-3 border-b border-line">
          <div class="flex items-center gap-3 text-[12px] text-sub flex-wrap">
            <span class="tabular-nums">{{ total.toLocaleString() }} 行 · {{ columns.length }} 列</span>
            <span v-if="active.source" class="text-weak truncate max-w-[320px]">来源 {{ active.source }}</span>
            <span v-if="preview.skipped" class="text-warn">跳过 {{ preview.skipped }} 行坏数据</span>
            <span v-if="statusCol" class="chip text-pass border-pass/30">有分组列</span>
            <span class="flex-1" />
          </div>
          <div class="mt-2 flex flex-wrap gap-1.5 max-h-[52px] overflow-y-auto">
            <span v-for="c in columns" :key="c" class="chip font-mono">
              {{ c }}
              <span v-if="c === '数据组状态'" class="text-warn">分组</span>
            </span>
          </div>
        </div>

        <div v-if="large" class="shrink-0 px-5 py-2 border-b border-line text-[12px] text-warn bg-warn/5">
          数据量较大（{{ total.toLocaleString() }} 行）。建议并发 4–8、超时不低于 180 秒，先用少量数据试跑。
        </div>

        <div class="flex-1 min-h-0 overflow-auto">
          <table class="data-table">
            <thead>
              <tr>
                <th class="w-12 text-right pr-3">#</th>
                <th v-for="c in columns" :key="c">{{ c }}</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="(r, i) in rows" :key="offset + i">
                <td class="text-right pr-3 text-weak tabular-nums">{{ offset + i + 1 }}</td>
                <td v-for="c in columns" :key="c" class="font-mono text-[12px]">
                  <span :title="String(r[c] ?? '')">{{ sample(r[c]) }}</span>
                </td>
              </tr>
              <tr v-if="!rows.length && preview.loading">
                <td :colspan="columns.length + 1" class="text-center text-weak py-10">读取中…</td>
              </tr>
            </tbody>
          </table>
        </div>

        <Pager
          :total="total"
          :page="page"
          :page-size="pageSize"
          @update:page="loadPage"
          @update:page-size="onPageSize"
        />
      </template>

      <div v-else class="flex-1 flex flex-col items-center justify-center gap-2">
        <p class="text-[13px] text-sub">还没有选择数据集</p>
        <p class="hint">导入一份 CSV、Excel 或 JSON，方案会引用它来逐行评测。</p>
        <div class="flex items-center gap-2 mt-2">
          <button class="btn" @click="pasteOpen = !pasteOpen">粘贴数据</button>
          <button class="btn-primary" :disabled="busy" @click="chooseFile">导入文件</button>
        </div>
      </div>
    </section>
  </div>
</template>
