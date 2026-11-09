<script setup>
import { ref, computed, watch, nextTick, onMounted, onBeforeUnmount } from "vue";
import {
  state,
  toast,
  connectionById,
  profileOf,
  createConnection,
  duplicateConnection,
  deleteConnection,
  planById,
  savedText,
  jsonObjectError,
} from "../store";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import Icon from "./Icon.vue";

const search = ref("");
const menuId = ref("");
const showKey = ref(false);
const showAdvanced = ref(false);
const testing = ref(false);
const probing = ref(false);
const result = ref("");
const resultOk = ref(false);
const probeText = ref("");
const probeVerdict = ref("");
const nameInput = ref(null);

const active = computed(() => connectionById(state.activeConnectionId));
const filtered = computed(() => {
  const q = search.value.trim().toLowerCase();
  if (!q) return state.connections;
  return state.connections.filter((c) =>
    [c.name, c.model, c.baseUrl, c.protocol].join(" ").toLowerCase().includes(q)
  );
});
const usedBy = computed(() =>
  active.value ? state.plans.filter((p) => p.targetId === active.value.id || p.judgeId === active.value.id) : []
);
const bodyError = computed(() => jsonObjectError(active.value?.extraBody));
const headerError = computed(() => jsonObjectError(active.value?.extraHeaders));

watch(
  () => state.activeConnectionId,
  () => {
    showKey.value = false;
    showAdvanced.value = false;
    result.value = "";
    probeText.value = "";
    probeVerdict.value = "";
    menuId.value = "";
  }
);

const PRESETS = {
  openai: { protocol: "openai", baseUrl: "https://api.openai.com/v1", model: "gpt-4o" },
  deepseek: { protocol: "openai", baseUrl: "https://api.deepseek.com/v1", model: "deepseek-chat" },
  dashscope: {
    protocol: "openai",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    model: "qwen-plus",
  },
  anthropic: {
    protocol: "anthropic",
    baseUrl: "https://api.anthropic.com",
    model: "claude-sonnet-5-5",
  },
  local: { protocol: "openai", baseUrl: "http://127.0.0.1:11434/v1", model: "qwen2.5:14b" },
};

function applyPreset(key) {
  if (!active.value || !PRESETS[key]) return;
  Object.assign(active.value, PRESETS[key]);
  result.value = "";
  probeText.value = "";
}

async function create() {
  createConnection();
  await nextTick();
  focusName();
}

async function focusName() {
  await nextTick();
  nameInput.value?.focus();
  nameInput.value?.select();
}

function openMenu(id) {
  menuId.value = menuId.value === id ? "" : id;
}

function rename(conn) {
  state.activeConnectionId = conn.id;
  menuId.value = "";
  focusName();
}

function copy(conn) {
  duplicateConnection(conn.id);
  menuId.value = "";
}

function usedByRunning(id) {
  return state.jobs.list.some((j) => {
    if (j.status !== "running") return false;
    const plan = planById(j.planId);
    return plan && (plan.targetId === id || plan.judgeId === id);
  });
}

async function remove(conn) {
  menuId.value = "";
  if (usedByRunning(conn.id)) {
    toast("有任务正在用这个连接运行，先取消或等它结束", "err");
    return;
  }
  const affected = state.plans
    .filter((p) => p.targetId === conn.id || p.judgeId === conn.id)
    .map((p) => p.name);
  const msg = affected.length
    ? `删除「${conn.name}」？\n\n以下方案会失去这个连接：${affected.join("、")}`
    : `删除「${conn.name}」？`;
  const ok = await ask(msg, {
    title: "删除连接",
    kind: "warning",
    okLabel: "删除",
    cancelLabel: "取消",
  });
  if (!ok) return;
  deleteConnection(conn.id);
  toast("已删除连接", "ok");
}

async function test() {
  if (!active.value) return;
  testing.value = true;
  result.value = "";
  try {
    const msg = await invoke("test_connection", { profile: profileOf(active.value) });
    result.value = msg;
    resultOk.value = true;
  } catch (e) {
    result.value = String(e);
    resultOk.value = false;
  } finally {
    testing.value = false;
  }
}

async function probe() {
  if (!active.value) return;
  probing.value = true;
  probeText.value = "";
  probeVerdict.value = "";
  try {
    const calls = await invoke("probe_api", { profile: profileOf(active.value) });
    const lat = calls.map((c) => c.latencyMs);
    const tok = calls.map((c) => c.completionTokens || 0);
    const sorted = [...lat].sort((a, b) => a - b);
    const med = sorted[Math.floor(sorted.length / 2)] || 0;
    const medTok = [...tok].sort((a, b) => a - b)[Math.floor(tok.length / 2)] || 0;
    const tps = med ? (medTok / (med / 1000)).toFixed(1) : "—";
    probeText.value =
      `3 次：${lat.map((x) => (x / 1000).toFixed(1) + "s").join(" / ")}` +
      ` · 中位 ${(med / 1000).toFixed(1)}s · 输出 ${medTok} tok · ${tps} tok/s`;
    probeVerdict.value =
      med > 15000
        ? "极短输入都这么慢，问题在模型或上游排队，不是本工具"
        : med > 5000
          ? "偏慢，多半是上游排队或网络，换时段再测对比"
          : "接口正常，批量跑得慢是因为单条请求本身耗时长";
  } catch (e) {
    probeText.value = String(e);
  } finally {
    probing.value = false;
  }
}

function closeMenu() {
  menuId.value = "";
}

onMounted(() => {
  window.addEventListener("click", closeMenu);
});
onBeforeUnmount(() => window.removeEventListener("click", closeMenu));
</script>

<template>
  <div class="h-full flex bg-page">
    <aside class="w-[280px] shrink-0 flex flex-col border-r border-line min-h-0">
      <div class="h-14 px-4 flex items-center gap-2 shrink-0">
        <h1 class="text-[15px] font-semibold">API 配置</h1>
        <span class="text-[12px] text-weak tabular-nums">{{ state.connections.length }}</span>
        <span class="flex-1" />
        <button class="icon-btn" title="新建连接" @click="create">
          <Icon name="plus" :size="16" />
        </button>
      </div>
      <div class="px-3 pb-2 shrink-0">
        <div class="relative">
          <Icon name="search" :size="14"
            class="absolute left-2 top-1/2 -translate-y-1/2 text-weak pointer-events-none" />
          <input v-model="search" class="field pl-7 h-8" placeholder="搜索名称或模型" />
        </div>
      </div>
      <div class="flex-1 min-h-0 overflow-y-auto px-2 pb-3 space-y-0.5">
        <div v-if="!state.connections.length" class="px-3 py-10 text-center">
          <p class="text-[13px] text-sub">还没有连接</p>
          <p class="hint mt-1.5">新建一条，或从模板填入。</p>
          <button class="btn-primary btn-sm mt-3" @click="create">新建连接</button>
        </div>
        <p v-else-if="!filtered.length" class="px-3 py-8 text-center hint">没有匹配的连接</p>

        <div v-for="c in filtered" :key="c.id" class="relative group">
          <button
            class="list-item w-full"
            :class="{ 'list-item-active': c.id === state.activeConnectionId }"
            @click="state.activeConnectionId = c.id"
          >
            <div class="flex items-center gap-2 pr-7">
              <span class="text-[13px] font-medium truncate">{{ c.name }}</span>
              <span v-if="state.plans.some((p) => p.targetId === c.id || p.judgeId === c.id)"
                class="text-[10.5px] text-pass border border-pass/30 bg-pass/10 rounded px-1 py-px shrink-0">使用中</span>
            </div>
            <div class="mt-0.5 text-[11.5px] text-weak truncate">
              {{ c.protocol === "anthropic" ? "Anthropic" : "OpenAI" }} · {{ c.model || "未填模型" }}
            </div>
          </button>
          <button
            class="icon-btn absolute right-1.5 top-2 opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
            title="更多"
            @click.stop="openMenu(c.id)"
          >
            <Icon name="more" :size="15" />
          </button>
          <div
            v-if="menuId === c.id"
            class="absolute right-1.5 top-9 z-20 w-28 py-1 rounded border border-line bg-panel shadow-sm"
            @click.stop
          >
            <button class="w-full text-left px-3 h-7 text-[12.5px] hover:bg-wash" @click="rename(c)">重命名</button>
            <button class="w-full text-left px-3 h-7 text-[12.5px] hover:bg-wash" @click="copy(c)">复制</button>
            <button class="w-full text-left px-3 h-7 text-[12.5px] text-fail hover:bg-fail/10" @click="remove(c)">删除</button>
          </div>
        </div>
      </div>
    </aside>

    <section class="flex-1 min-w-0 flex flex-col bg-panel">
      <template v-if="active">
        <div class="h-14 px-5 flex items-center gap-2.5 border-b border-line shrink-0">
          <input
            ref="nameInput"
            v-model="active.name"
            class="min-w-0 w-[240px] h-8 px-1.5 rounded border border-transparent bg-transparent
              text-[15px] font-semibold text-main hover:border-line focus:border-accent transition-colors"
            placeholder="连接名称"
          />
          <span class="text-[11.5px] text-weak shrink-0">{{ savedText }}</span>
          <span class="flex-1" />
          <select class="field w-[104px] h-7 text-[12px] text-sub" value=""
            @change="applyPreset($event.target.value); $event.target.value = ''">
            <option value="">填入模板…</option>
            <option value="openai">OpenAI</option>
            <option value="deepseek">DeepSeek</option>
            <option value="dashscope">通义</option>
            <option value="anthropic">Anthropic</option>
            <option value="local">本地</option>
          </select>
          <button class="btn btn-sm" :disabled="probing" @click="probe">
            {{ probing ? "测速中…" : "测速" }}
          </button>
          <button class="btn btn-sm" :disabled="testing" @click="test">
            {{ testing ? "测试中…" : "测试连接" }}
          </button>
        </div>

        <div class="flex-1 min-h-0 overflow-y-auto">
          <div class="max-w-[760px] px-6 py-5 space-y-6">
            <div class="text-[12px] text-weak">
              {{ active.protocol === "anthropic" ? "Anthropic 协议" : "OpenAI 兼容协议" }}
              <span v-if="active.model"> · {{ active.model }}</span>
              <span v-if="usedBy.length"> · 被 {{ usedBy.length }} 个方案使用：{{ usedBy.map((p) => p.name).join("、") }}</span>
            </div>

            <div class="grid grid-cols-2 gap-x-4 gap-y-4">
              <div class="col-span-2">
                <label class="label">Base URL</label>
                <input v-model="active.baseUrl" class="field-mono" placeholder="https://api.example.com/v1" />
              </div>
              <div>
                <label class="label">模型名</label>
                <input v-model="active.model" class="field-mono" placeholder="gpt-4o-mini" />
              </div>
              <div>
                <label class="label">协议</label>
                <select v-model="active.protocol" class="field">
                  <option value="openai">OpenAI 兼容（/chat/completions）</option>
                  <option value="anthropic">Anthropic（/v1/messages）</option>
                </select>
              </div>
              <div class="col-span-2">
                <label class="label">API Key</label>
                <div class="relative">
                  <input
                    v-model="active.apiKey"
                    :type="showKey ? 'text' : 'password'"
                    class="field-mono pr-9"
                    placeholder="sk-..."
                    autocomplete="off"
                  />
                  <button
                    class="absolute right-1 top-1/2 -translate-y-1/2 icon-btn"
                    :title="showKey ? '隐藏' : '显示'"
                    @click="showKey = !showKey"
                  >
                    <Icon :name="showKey ? 'eye-off' : 'eye'" :size="15" />
                  </button>
                </div>
                <p class="hint mt-1">只保存在本机配置里，不会出现在报告、导出或错误信息中。</p>
              </div>
            </div>

            <div>
              <div class="flex items-center gap-2 mb-3">
                <span class="text-[13px] font-medium">生成参数</span>
                <span class="hint">评测连接建议 temperature 0，判定更稳定</span>
              </div>
              <div class="grid grid-cols-4 gap-3">
                <div>
                  <label class="label">temperature</label>
                  <input v-model.number="active.temperature" type="number" step="0.1" min="0" max="2" class="field tabular-nums" />
                </div>
                <div>
                  <label class="label">top_p</label>
                  <input v-model.number="active.topP" type="number" step="0.05" min="0" max="1" class="field tabular-nums" />
                </div>
                <div>
                  <label class="label">max_tokens</label>
                  <input v-model.number="active.maxTokens" type="number" min="1" class="field tabular-nums" />
                </div>
                <div>
                  <label class="label">超时（秒）</label>
                  <input v-model.number="active.timeoutSecs" type="number" min="10" class="field tabular-nums" />
                </div>
              </div>
            </div>

            <div>
              <button class="btn-ghost btn-sm -ml-2.5" @click="showAdvanced = !showAdvanced">
                <Icon :name="showAdvanced ? 'chevron-down' : 'chevron-right'" :size="13" />
                高级：上下文长度 / 自定义请求体与请求头
              </button>
              <div v-if="showAdvanced" class="mt-3 grid grid-cols-2 gap-4">
                <div>
                  <label class="label">上下文长度（tokens）</label>
                  <input v-model.number="active.contextWindow" type="number" min="512" class="field tabular-nums" />
                  <p class="hint mt-1">超出的输入按头尾保留截断。</p>
                </div>
                <div />
                <div>
                  <label class="label">自定义请求体（JSON 对象，合并进 body）</label>
                  <textarea v-model="active.extraBody" rows="4" class="field-mono"
                    :class="bodyError ? 'border-fail focus:border-fail focus:ring-fail/20' : ''"
                    placeholder='{"seed": 1}' />
                  <p class="hint mt-1" :class="bodyError ? 'text-fail' : ''">
                    {{ bodyError || "敏感键在写入任务记录和报告前会被脱敏。" }}
                  </p>
                </div>
                <div>
                  <label class="label">自定义请求头（JSON 对象）</label>
                  <textarea v-model="active.extraHeaders" rows="4" class="field-mono"
                    :class="headerError ? 'border-fail focus:border-fail focus:ring-fail/20' : ''"
                    placeholder='{"X-Org-Id": "abc"}' />
                  <p class="hint mt-1" :class="headerError ? 'text-fail' : ''">
                    {{ headerError || "不要在这里填 API Key，用上面的输入框。" }}
                  </p>
                </div>
              </div>
            </div>

            <div v-if="result || probeText" class="rounded border border-line bg-page px-3.5 py-3 space-y-1.5">
              <div v-if="result" class="text-[12px] leading-relaxed break-all" :class="resultOk ? 'text-pass' : 'text-fail'">
                {{ result }}
              </div>
              <div v-if="probeText" class="text-[12px] leading-relaxed break-all"
                :class="probeText.startsWith('3 次') ? 'text-main' : 'text-fail'">
                {{ probeText }}
              </div>
              <div v-if="probeVerdict" class="hint">{{ probeVerdict }}</div>
            </div>
          </div>
        </div>
      </template>

      <div v-else class="flex-1 flex flex-col items-center justify-center gap-2">
        <p class="text-[13px] text-sub">连接库还是空的</p>
        <p class="hint">一条连接就是一套 API 地址、密钥和生成参数，可以被多个方案复用。</p>
        <button class="btn-primary mt-2" @click="create">新建连接</button>
      </div>
    </section>
  </div>
</template>
