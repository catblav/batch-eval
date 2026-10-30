<script setup>
import { ref, computed, watch } from "vue";
import { state, toast, ensureUserTemplate, statusLabelOf } from "../store";
import { invoke } from "@tauri-apps/api/core";
import { open, ask } from "@tauri-apps/plugin-dialog";
import Pager from "./Pager.vue";

const pasteOpen = ref(false);
const pasteText = ref("");
const pasteFormat = ref("auto");
const loading = ref(false);

const columns = computed(() => state.dataset.columns);
const rows = computed(() => state.dataset.rows);
const statusCol = computed(() => columns.value.includes("数据组状态"));
const skipped = computed(() => Number(state.dataset.skipped) || 0);
const largeData = computed(() => rows.value.length > 20000);

// 预览分页：数据集可能上万条，只渲染当前页
const page = ref(1);
const pageSize = ref(50);
const pageCount = computed(() => Math.max(1, Math.ceil(rows.value.length / Math.max(1, pageSize.value))));
const pagedRows = computed(() => {
  const size = Math.max(1, pageSize.value);
  const p = Math.min(Math.max(1, page.value), pageCount.value);
  const start = (p - 1) * size;
  return rows.value.slice(start, start + size);
});
const startNo = computed(
  () => (Math.min(Math.max(1, page.value), pageCount.value) - 1) * pageSize.value
);

// 换数据集或改每页条数后回到第一页
watch(
  () => rows.value.length,
  () => {
    page.value = 1;
  }
);
function onPageSize(n) {
  pageSize.value = n;
  page.value = 1;
}

watch(columns, (cols) => {
  if (!cols.length) return;
  // 默认注入所有列，但状态列除外
  state.selectedColumns = cols.filter((c) => c !== "数据组状态");
  ensureUserTemplate();
});

async function importFile() {
  try {
    const path = await open({
      multiple: false,
      filters: [{ name: "数据集", extensions: ["csv", "tsv", "xlsx", "xlsm", "json", "jsonl", "txt", "md"] }],
    });
    if (!path) return;
    loading.value = true;
    const ds = await invoke("parse_dataset_file", { path });
    await apply(ds);
  } catch (e) {
    toast(String(e), "err");
  } finally {
    loading.value = false;
  }
}

async function applyPaste() {
  if (!pasteText.value.trim()) return;
  loading.value = true;
  try {
    const ds = await invoke("parse_dataset_text", {
      text: pasteText.value,
      format: pasteFormat.value === "auto" ? null : pasteFormat.value,
    });
    await apply(ds);
    pasteOpen.value = false;
    pasteText.value = "";
  } catch (e) {
    toast(String(e), "err");
  } finally {
    loading.value = false;
  }
}

async function apply(ds) {
  const n = ds.rows.length;
  // 十万条以上大概率是误选了整个导出表，先让用户确认一次
  if (n > 100000) {
    const ok = await ask(
      `这份数据有 ${n.toLocaleString()} 条，单批跑完耗时极长、内存占用也大。确定要载入吗？`,
      { title: "数据量过大", kind: "warning", okLabel: "载入", cancelLabel: "取消" }
    );
    if (!ok) return;
  }
  state.dataset = ds;
  toast(`已载入 ${n.toLocaleString()} 条数据，${ds.columns.length} 列`, "ok");
}

function toggleColumn(col) {
  const set = new Set(state.selectedColumns);
  if (set.has(col)) set.delete(col);
  else set.add(col);
  state.selectedColumns = columns.value.filter((c) => set.has(c));
  state.prompts.userTemplate = state.selectedColumns.map((c) => `{{${c}}}`).join("\n\n");
}

function clear() {
  state.dataset = { columns: [], rows: [], source: "", skipped: 0 };
  state.selectedColumns = [];
  toast("已清空数据集", "ok");
}

function sample(row, col) {
  const v = String(row[col] ?? "");
  return v.length > 90 ? v.slice(0, 90) + "…" : v;
}
</script>

<template>
  <div class="space-y-4">
    <div class="card p-4">
      <div class="flex items-center gap-2 mb-3">
        <span class="text-[13px] font-semibold">数据集</span>
        <span v-if="rows.length" class="hint">{{ rows.length }} 条 · {{ columns.length }} 列 · {{ state.dataset.source }}</span>
        <span v-else class="hint">支持 CSV / TSV / Excel / JSON / JSONL / 文本，中文编码自动识别</span>
        <span class="flex-1" />
        <button class="btn text-[12px]" @click="pasteOpen = !pasteOpen">粘贴数据</button>
        <button class="btn-primary text-[12px]" :disabled="loading" @click="importFile">
          {{ loading ? "解析中…" : "选择文件" }}
        </button>
        <button v-if="rows.length" class="btn-ghost text-[12px]" @click="clear">清空</button>
      </div>

      <div v-if="skipped" class="mb-3 text-[12px] text-warn bg-warn/10 border border-warn/30 rounded px-3 py-2">
        {{ skipped }} 行格式错误（JSONL 不是合法对象 / CSV 记录残缺），已跳过，请核对源文件。
      </div>
      <div v-if="largeData" class="mb-3 text-[12px] text-accent bg-accent/10 border border-accent/30 rounded px-3 py-2">
        数据量较大（{{ rows.length.toLocaleString() }} 条），建议把并发控制在 4~8、超时不低于 180s，先用少量数据试跑一轮再全量。
        运行结果会实时落盘，中途取消或崩溃都能续跑。
      </div>

      <div v-if="!rows.length" class="border border-dashed border-ink-700 rounded-lg p-10 text-center">
        <p class="text-ink-400 text-[13px]">还没有数据</p>
        <p class="hint mt-2">
          可以从 Excel / CSV / JSON 导入，也可以直接粘贴表格或 JSON 数组。
        </p>
        <p class="hint mt-1">首列或任意列名为「数据组状态」时，分析页会按它做交叉热力图。</p>
      </div>

      <div v-if="pasteOpen" class="mt-3 space-y-2">
        <textarea v-model="pasteText" rows="8" class="field-mono w-full"
          placeholder="支持：JSON 数组 / 每行一条 JSON / CSV 表头加数据 / 空行分隔的文本块" />
        <div class="flex items-center gap-2">
          <select v-model="pasteFormat" class="field w-40">
            <option value="auto">自动识别</option>
            <option value="json">JSON</option>
            <option value="csv">CSV / TSV</option>
            <option value="text">纯文本块</option>
          </select>
          <button class="btn-primary" @click="applyPaste">解析粘贴内容</button>
          <button class="btn-ghost" @click="pasteOpen = false">取消</button>
        </div>
      </div>
    </div>

    <div v-if="rows.length" class="card p-4">
      <div class="flex items-center gap-2 mb-2">
        <span class="text-[13px] font-semibold">列映射</span>
        <span class="hint">勾选的列会以 <code class="font-mono">&#123;&#123;列名&#125;&#125;</code> 注入用户消息模板</span>
        <span class="flex-1" />
        <button class="btn-ghost text-[12px]" @click="state.prompts.userTemplate = state.selectedColumns.map(c => '{{' + c + '}}').join('\n\n')">
          重新生成模板
        </button>
      </div>
      <div class="flex flex-wrap gap-2">
        <label v-for="c in columns" :key="c"
          class="flex items-center gap-1.5 px-2 py-1 rounded border cursor-pointer text-[12px] transition"
          :class="state.selectedColumns.includes(c) ? 'border-accent/50 bg-accent/10 text-ink-200' : 'border-ink-700 text-ink-400 hover:border-ink-600'">
          <input type="checkbox" class="accent-accent" :checked="state.selectedColumns.includes(c)"
            :disabled="c === '数据组状态'" @change="toggleColumn(c)" />
          <span class="font-mono">{{ c }}</span>
          <span v-if="c === '数据组状态'" class="chip border-warn/40 text-warn bg-warn/10">分组</span>
        </label>
      </div>
    </div>

    <div v-if="rows.length" class="card overflow-hidden">
      <div class="flex items-center gap-2 px-4 py-2 border-b border-ink-700/60">
        <span class="text-[13px] font-semibold">数据预览</span>
        <span class="hint">共 {{ rows.length }} 条</span>
        <span class="flex-1" />
        <span v-if="statusCol" class="chip border-pass/40 text-pass bg-pass/10">识别到分组列</span>
        <span v-else class="chip border-ink-700 text-ink-400">无分组列</span>
      </div>
      <div class="overflow-auto max-h-[420px]">
        <table class="w-full text-[12px]">
          <thead class="sticky top-0 bg-ink-850 text-ink-400">
            <tr>
              <th class="px-3 py-2 text-left font-medium w-10">#</th>
              <th v-for="c in columns" :key="c" class="px-3 py-2 text-left font-medium whitespace-nowrap">{{ c }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(r, i) in pagedRows" :key="startNo + i" class="border-t border-ink-800 hover:bg-ink-850/60">
              <td class="px-3 py-1.5 text-ink-500">{{ startNo + i + 1 }}</td>
              <td v-for="c in columns" :key="c" class="px-3 py-1.5 text-ink-300 align-top">
                <span class="font-mono">{{ sample(r, c) }}</span>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <Pager :total="rows.length" :page="page" :page-size="pageSize"
        @update:page="page = $event" @update:page-size="onPageSize" />
    </div>
  </div>
</template>
