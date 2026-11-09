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

/// 导出文件名的时间后缀：YYYYMMDD_HHmm，避免多次导出互相覆盖
function timeSuffix(d = new Date()) {
  const p = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}_${p(d.getHours())}${p(d.getMinutes())}`;
}

/// 生成带时间后缀的默认文件名，如 评测结果_20261105_1656.csv
function stampedPath(name, ext) {
  return `${name}_${timeSuffix()}.${ext}`;
}

/// 标题会进文件名，Windows 不允许的字符统一换掉
function safeTitle(title) {
  return String(title || "")
    .replace(/[\\/:*?"<>|\x00-\x1f]/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 60);
}

/// 有评测标题就用「标题_通用名_时间」，没有退回原来的「通用名_时间」
function stemWith(title, fallback) {
  const t = safeTitle(title);
  return t ? `${t}_${fallback}` : fallback;
}

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

export async function exportCsv(results, title = "") {
  try {
    const path = await save({
      defaultPath: stampedPath(stemWith(title, "评测结果"), "csv"),
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

export async function exportJson(results, columns, title = "") {
  try {
    const path = await save({
      defaultPath: stampedPath(stemWith(title, "评测结果"), "json"),
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

/// 完整报告：概览（通过率 / 关键指标）+ 失败分类 + 未计入明细 + 分组汇总 + 全部明细，一个 xlsx 多 sheet
export async function exportFullReport(payload) {
  try {
    const path = await save({
      defaultPath: stampedPath(stemWith(payload.title, "评测报告"), "xlsx"),
      filters: [{ name: "Excel 工作簿", extensions: ["xlsx"] }],
    });
    if (!path) return;
    await invoke("export_excel_report", { path, payload });
    toast(`完整报告已导出：${payload.rows.length} 条明细`, "ok");
  } catch (e) {
    toast("导出失败：" + e, "err");
  }
}
