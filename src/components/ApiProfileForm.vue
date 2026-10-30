<script setup>
import { ref, computed, watch } from "vue";
import { state, toast, saveNow } from "../store";
import { invoke } from "@tauri-apps/api/core";
import Field from "./Field.vue";

const props = defineProps({ kind: { type: String, required: true } });
// 用 computed 而非固定引用，避免读配置后拿到已被替换的旧对象
const profile = computed(() => state.settings[props.kind]);
const testing = ref(false);
const saving = ref(false);
const result = ref("");
const showAdvanced = ref(false);

const preset = (name) => {
  const map = {
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
  Object.assign(profile.value, map[name]);
  result.value = "";
};

// 同步成功后在按钮旁常驻提示；被测模型配置再被改动即失效
const synced = ref(false);
watch(profile, () => (synced.value = false), { deep: true });

// 整份配置（含 API Key 与全部参数）复制到评测模型，两张卡片一起保存
const applyToJudge = async () => {
  Object.assign(state.settings.judge, JSON.parse(JSON.stringify(profile.value)));
  const ok = await saveNow();
  synced.value = ok;
};

async function save() {
  saving.value = true;
  try {
    if (await saveNow(props.kind)) toast("配置已保存", "ok");
  } finally {
    saving.value = false;
  }
}

// 保存成功后一直显示，不再自动消失
const savedText = computed(() => {
  const at = state.lastSavedAtByKind[props.kind];
  if (!at) return "";
  const d = new Date(at);
  const p = (n) => String(n).padStart(2, "0");
  return `已保存 ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
});

async function test() {
  testing.value = true;
  result.value = "";
  try {
    const msg = await invoke("test_connection", { profile: profile.value });
    result.value = msg;
    toast(msg, "ok");
  } catch (e) {
    result.value = String(e);
    toast(String(e), "err");
  } finally {
    testing.value = false;
  }
}
</script>

<template>
  <div class="card p-4 space-y-3">
    <div class="flex items-center gap-2 flex-wrap">
      <span class="text-[13px] font-semibold text-ink-200">
        {{ kind === "target" ? "被测模型" : "评测模型（裁判）" }}
      </span>
      <span class="hint">{{ kind === "target" ? "被评的 API" : "给出分类与原因的 API" }}</span>
      <span class="flex-1" />
      <span v-if="savedText" class="hint">{{ savedText }}</span>
      <template v-if="kind === 'target'">
        <span v-if="synced" class="text-[12px] text-pass">已同步</span>
        <button class="btn-ghost text-[12px]" @click="applyToJudge">
          同步到评测模型
        </button>
      </template>
      <button class="btn text-[12px]" :disabled="testing" @click="test">
        {{ testing ? "测试中…" : "测试连接" }}
      </button>
      <button class="btn-primary text-[12px]" :disabled="saving" @click="save">
        {{ saving ? "保存中…" : "保存配置" }}
      </button>
    </div>

    <div class="flex flex-wrap gap-1">
      <button v-for="p in ['openai', 'deepseek', 'dashscope', 'anthropic', 'local']"
        :key="p" class="btn-ghost text-[11.5px]" @click="preset(p)">{{ p }}</button>
    </div>

    <div class="grid grid-cols-2 gap-3">
      <Field label="Base URL" v-model="profile.baseUrl" mono placeholder="https://api.example.com/v1"
        hint="留空则不可运行；填 OpenAI 兼容入口或 Anthropic 根地址" />
      <Field label="模型名" v-model="profile.model" mono placeholder="gpt-4o-mini" />
      <Field label="API Key" v-model="profile.apiKey" type="password" mono placeholder="sk-..." />
      <div>
        <span class="label">协议</span>
        <select v-model="profile.protocol" class="field">
          <option value="openai">OpenAI 兼容（/chat/completions）</option>
          <option value="anthropic">Anthropic（/v1/messages）</option>
        </select>
      </div>
      <Field label="上下文长度 (tokens)" v-model="profile.contextWindow" type="number"
        hint="超出的输入会被头尾保留截断，避免报错" />
      <Field label="max_tokens" v-model="profile.maxTokens" type="number" />
      <Field label="temperature" v-model="profile.temperature" type="number" />
      <Field label="top_p" v-model="profile.topP" type="number" />
    </div>

    <button class="btn-ghost text-[12px] -ml-2" @click="showAdvanced = !showAdvanced">
      {{ showAdvanced ? "收起高级参数" : "高级参数（超时 / 自定义请求体 / 请求头）" }}
    </button>
    <div v-if="showAdvanced" class="grid grid-cols-2 gap-3">
      <Field label="超时（秒）" v-model="profile.timeoutSecs" type="number" />
      <div />
      <div>
        <span class="label">自定义请求体（JSON 对象，合并进 body）</span>
        <textarea v-model="profile.extraBody" rows="4" class="field-mono"
          placeholder='{"seed": 1, "presence_penalty": 0.2}' />
      </div>
      <div>
        <span class="label">自定义请求头（JSON 对象）</span>
        <textarea v-model="profile.extraHeaders" rows="4" class="field-mono"
          placeholder='{"X-Org-Id": "abc"}' />
      </div>
    </div>

    <p v-if="result" class="hint break-all whitespace-pre-wrap" :class="result.startsWith('连接成功') ? 'text-pass' : 'text-fail'">
      {{ result }}
    </p>
  </div>
</template>
