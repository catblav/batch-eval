<script setup>
import { computed } from "vue";

const props = defineProps({
  modelValue: { type: String, default: "" },
  label: { type: String, default: "" },
  hint: { type: String, default: "" },
  placeholder: { type: String, default: "" },
  type: { type: String, default: "text" },
  mono: { type: Boolean, default: false },
});
const emit = defineEmits(["update:modelValue"]);

const isNumber = computed(() => props.type === "number");

function onInput(e) {
  if (isNumber.value) {
    const raw = e.target.value;
    emit("update:modelValue", raw === "" ? null : Number(raw));
  } else {
    emit("update:modelValue", e.target.value);
  }
}
</script>

<template>
  <label class="block">
    <span v-if="label" class="label">{{ label }}</span>
    <input
      :type="type"
      :value="modelValue === null || modelValue === undefined ? '' : modelValue"
      :placeholder="placeholder"
      :class="mono ? 'field-mono' : 'field'"
      @input="onInput"
    />
    <span v-if="hint" class="hint mt-1 block">{{ hint }}</span>
  </label>
</template>
