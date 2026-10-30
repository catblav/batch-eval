import { save } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "../store";

const HEADERS = [
  "序号",
  "数据组状态",
  "结果",
  "分类",
  "分类原因",
  "细节",
  "被测模型输出",
  "用户消息",
  "被测耗时ms",
  "评测耗时ms",
  "输入tokens",
  "输出tokens",
  "评测原始输出",
  "错误",
];

// 分块大小：前端每次只拼这么多行/条，内存峰值与总量无关
const CSV_CHUNK = 500;
const JSON_CHUNK = 200;

function esc(v) {
  const s = v === null || v === undefined ? "" : String(v);
  if (/[",\n\r]/.test(s)) return '"' + s.replace(/"/g, '""') + '"';
  return s;
}

/// 一行结果 → CSV 字段数组。列名不再参与，数据列本就来自每条结果自身的字段。
function rowOf(r) {
  return [
    r.index + 1,
    r.statusLabel,
    r.pass ? "通过" : "不通过",
    (r.errorTypes || []).join(" / "),
    (r.reasons || []).join("；"),
    r.details || "",
    r.output || "",
    r.input || "",
    r.targetLatencyMs,
    r.judgeLatencyMs,
    r.promptTokens ?? "",
    r.completionTokens ?? "",
    r.judgeRaw || "",
    r.targetError || "",
  ].map(esc);
}

export async function exportCsv(results) {
  try {
    const path = await save({
      defaultPath: "评测结果.csv",
      filters: [{ name: "CSV", extensions: ["csv"] }],
    });
    if (!path) return;

    // BOM 让 Excel 正确识别 UTF-8 中文；首块带表头，顺带截断旧文件
    await invoke("save_text_file", {
      path,
      content: "﻿" + HEADERS.map(esc).join(",") + "\r\n",
    });
    for (let i = 0; i < results.length; i += CSV_CHUNK) {
      const lines = results.slice(i, i + CSV_CHUNK).map((r) => rowOf(r).join(","));
      await invoke("append_text_file", { path, content: lines.join("\r\n") + "\r\n" });
    }
    toast(`已导出 ${results.length} 条 CSV，可直接用 Excel 打开`, "ok");
  } catch (e) {
    toast("导出失败：" + e, "err");
  }
}

export async function exportJson(results, columns) {
  try {
    const path = await save({
      defaultPath: "评测结果.json",
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (!path) return;

    // 手写流式结构，避免 1 万条时一次性 stringify 出几十 MB 的大字符串
    const head =
      "{\n" +
      `  "exportedAt": ${JSON.stringify(new Date().toISOString())},\n` +
      `  "columns": ${JSON.stringify(columns || [])},\n` +
      `  "results": [`;
    await invoke("save_text_file", { path, content: head });

    for (let i = 0; i < results.length; i += JSON_CHUNK) {
      const items = results.slice(i, i + JSON_CHUNK).map((r) => JSON.stringify(r, null, 2));
      const body = (i > 0 ? ",\n" : "\n") + items.join(",\n");
      await invoke("append_text_file", { path, content: body });
    }

    await invoke("append_text_file", { path, content: "\n  ]\n}\n" });
    toast(`已导出 ${results.length} 条 JSON`, "ok");
  } catch (e) {
    toast("导出失败：" + e, "err");
  }
}
