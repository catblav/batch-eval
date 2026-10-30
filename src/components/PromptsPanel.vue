<script setup>
import { computed, ref } from "vue";
import { state, ensureUserTemplate, toast } from "../store";
import { invoke } from "@tauri-apps/api/core";
import { save, open } from "@tauri-apps/plugin-dialog";

const sysLen = computed(() => state.prompts.systemPrompt.length);
const evalHasVars = computed(
  () => /\{\{\s*input\s*\}\}/.test(state.prompts.evalPrompt) && /\{\{\s*output\s*\}\}/.test(state.prompts.evalPrompt)
);
const hasOutputContract = computed(() => /"pass"|pass\s*[:：]/.test(state.prompts.evalPrompt));

const templateExample = "例如：\n{{列名1}}\n\n{{列名2}}";
const wrapVar = (c) => "{{" + c + "}}";

// ---------- 变量注入预览 ----------
// 与 Rust 端 runner::render 的顺序保持一致：先替换列名，再替换 input / output
const previewOpen = ref(true);
const sampleRow = computed(() => state.dataset.rows[0] || null);

function renderTpl(tpl, row, input, output) {
  let out = String(tpl || "");
  for (const [k, v] of Object.entries(row || {})) {
    const val = typeof v === "string" ? v : JSON.stringify(v);
    out = out.split(`{{${k}}}`).join(val);
  }
  out = out.split("{{input}}").join(input).split("{{ input }}").join(input);
  out = out.split("{{output}}").join(output).split("{{ output }}").join(output);
  return out;
}

const previewUserMsg = computed(() => renderTpl(state.prompts.userTemplate, sampleRow.value, "", ""));

const previewTarget = computed(() => ({
  system: state.prompts.systemPrompt,
  user: previewUserMsg.value,
}));

// 裁判拿到的 {{input}} = 主体提示词 + 已渲染的用户消息，与后端 judge_input_text 一致
const previewJudgeInput = computed(() => {
  const sys = state.prompts.systemPrompt.trim();
  return sys ? `【被评估模型的提示词】\n${sys}\n\n【本条输入】\n${previewUserMsg.value}` : previewUserMsg.value;
});

const previewJudge = computed(() =>
  renderTpl(
    state.prompts.evalPrompt,
    sampleRow.value,
    previewJudgeInput.value,
    "（被测模型的实际输出会出现在这里）"
  )
);

async function importPrompt(which) {
  try {
    const path = await open({ multiple: false, filters: [{ name: "文本", extensions: ["md", "txt", "json"] }] });
    if (!path) return;
    const text = await invoke("read_text_file", { path });
    state.prompts[which] = text;
    toast(`已导入 ${which === "systemPrompt" ? "主体提示词" : "评测提示词"}`, "ok");
  } catch (e) {
    toast(String(e), "err");
  }
}

async function exportPrompt(which) {
  try {
    const path = await save({
      defaultPath: which === "systemPrompt" ? "主体提示词.md" : "评测提示词.md",
      filters: [{ name: "Markdown", extensions: ["md"] }],
    });
    if (!path) return;
    await invoke("save_text_file", { path, content: state.prompts[which] });
    toast("已导出", "ok");
  } catch (e) {
    toast(String(e), "err");
  }
}

function fillTemplate() {
  ensureUserTemplate();
  toast("已按所选列生成用户消息模板", "ok");
}
</script>

<template>
  <div class="space-y-4">
    <div class="card p-4">
      <div class="flex items-center gap-2 mb-2">
        <span class="text-[13px] font-semibold">主体提示词</span>
        <span class="hint">被测模型的 system prompt</span>
        <span class="flex-1" />
        <span class="hint">{{ sysLen }} 字</span>
        <button class="btn-ghost text-[12px]" @click="importPrompt('systemPrompt')">导入文件</button>
        <button class="btn-ghost text-[12px]" @click="exportPrompt('systemPrompt')">导出</button>
      </div>
      <textarea v-model="state.prompts.systemPrompt" rows="12" class="field-mono w-full"
        placeholder="粘贴你的主体提示词（被测的提示词）…" />
    </div>

    <div class="card p-4">
      <div class="flex items-center gap-2 mb-2">
        <span class="text-[13px] font-semibold">用户消息模板</span>
        <span class="hint">用 <code class="font-mono">&#123;&#123;列名&#125;&#125;</code> 注入数据列，跑测时逐行替换</span>
        <span class="flex-1" />
        <button class="btn-ghost text-[12px]" @click="fillTemplate">按数据集列生成</button>
      </div>
      <textarea v-model="state.prompts.userTemplate" rows="6" class="field-mono w-full"
        :placeholder="templateExample" />
      <div v-if="state.selectedColumns.length" class="flex flex-wrap gap-1 mt-2">
        <span class="hint mr-1">可用变量：</span>
        <code v-for="c in state.selectedColumns" :key="c"
          class="chip border-ink-700 text-ink-300 bg-ink-850 font-mono">{{ wrapVar(c) }}</code>
      </div>
    </div>

    <div class="card p-4">
      <div class="flex items-center gap-2 mb-2">
        <span class="text-[13px] font-semibold">评测提示词</span>
        <span class="hint">裁判模型逐条调用，需输出 pass 与分类</span>
        <span class="flex-1" />
        <span class="chip" :class="evalHasVars ? 'border-pass/40 text-pass bg-pass/10' : 'border-warn/40 text-warn bg-warn/10'">
          {{ evalHasVars ? "变量齐全" : "缺少 input / output 变量" }}
        </span>
        <span class="chip" :class="hasOutputContract ? 'border-pass/40 text-pass bg-pass/10' : 'border-warn/40 text-warn bg-warn/10'">
          {{ hasOutputContract ? "含 pass 字段" : "未见 pass 字段" }}
        </span>
        <button class="btn-ghost text-[12px]" @click="importPrompt('evalPrompt')">导入文件</button>
        <button class="btn-ghost text-[12px]" @click="exportPrompt('evalPrompt')">导出</button>
      </div>
      <textarea v-model="state.prompts.evalPrompt" rows="20" class="field-mono w-full" />
      <div class="mt-3 panel px-3 py-2.5 space-y-1.5">
        <div class="text-[12px] font-semibold text-ink-300">变量怎么注入</div>
        <div class="grid grid-cols-[92px_1fr] gap-x-3 gap-y-1 text-[12px] text-ink-400">
          <code class="font-mono text-ink-200">&#123;&#123;input&#125;&#125;</code>
          <span>主体提示词 + 本条输入（用户消息模板按该行数据渲染后的结果），两段带小标题拼接</span>
          <code class="font-mono text-ink-200">&#123;&#123;output&#125;&#125;</code>
          <span>被测模型对这一条的实际输出（含截断标记，如果超了 max_tokens）</span>
          <code class="font-mono text-ink-200">&#123;&#123;列名&#125;&#125;</code>
          <span>该行对应列的原始值，评测提示词里也可以直接用</span>
        </div>
        <p class="hint pt-1">
          不写变量也能跑，但裁判就看不到输入和输出，判定会失真。错误类型写成封闭列表，跑完本工具会按这份列表聚合。
        </p>
      </div>
    </div>

    <div class="card p-4">
      <div class="flex items-center gap-2 mb-2">
        <span class="text-[13px] font-semibold">跑测时实际发送的内容</span>
        <span class="hint">
          {{ sampleRow ? `用数据集第 1 条实时渲染：${Object.values(sampleRow).join(" / ").slice(0, 40)}…` : "还没导入数据集，这里先用空数据渲染" }}
        </span>
        <span class="flex-1" />
        <button class="btn-ghost text-[12px]" @click="previewOpen = !previewOpen">
          {{ previewOpen ? "收起" : "展开" }}
        </button>
      </div>

      <div v-show="previewOpen" class="space-y-3">
        <div>
          <div class="hint mb-1">被测模型 · system</div>
          <pre class="bg-ink-950 border border-ink-800 rounded p-3 text-[12px] font-mono whitespace-pre-wrap max-h-40 overflow-auto text-ink-300">{{ previewTarget.system || "（空）" }}</pre>
        </div>
        <div>
          <div class="hint mb-1">被测模型 · user（用户消息模板渲染后）</div>
          <pre class="bg-ink-950 border border-ink-800 rounded p-3 text-[12px] font-mono whitespace-pre-wrap max-h-40 overflow-auto text-ink-300">{{ previewTarget.user || "（空）" }}</pre>
        </div>
        <div>
          <div class="hint mb-1">评测模型 · user（评测提示词渲染后）</div>
          <pre class="bg-ink-950 border border-ink-800 rounded p-3 text-[12px] font-mono whitespace-pre-wrap max-h-64 overflow-auto text-ink-200">{{ previewJudge }}</pre>
        </div>
      </div>
    </div>
  </div>
</template>
