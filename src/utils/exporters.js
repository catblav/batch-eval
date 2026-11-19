import { save } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { state, toast } from "../store";

const HEADERS = [
  "序号",
  "数据组状态",
  "结果",
  "分类",
  "分类原因",
  "细节",
  "被测模型输出",
  "system",
  "user",
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

/// 导出落点：设了固定导出文件夹就直接用里面的文件名，不再弹保存框；
/// 没设、或目录临时不可用（盘没插、没权限）时退回手动选择。
/// 返回 { path, fixed }，path 为空表示用户在对话框里取消了。
async function resolvePath(fileName, filterName, ext) {
  if (state.exportDir?.set) {
    try {
      const p = await invoke("export_file_path", { fileName });
      if (p) return { path: p, fixed: true };
    } catch (e) {
      toast("导出文件夹不可用，改为手动选择位置：" + e, "err", 6000);
    }
  }
  const path = await save({
    defaultPath: fileName,
    filters: [{ name: filterName, extensions: [ext] }],
  });
  return { path, fixed: false };
}

/// toast 里只放文件名，完整路径太长会撑破提示条
function fileNameOf(path) {
  return String(path).split(/[\\/]/).pop();
}

function esc(v) {
  const s = v === null || v === undefined ? "" : String(v);
  if (/[",\n\r]/.test(s)) return '"' + s.replace(/"/g, '""') + '"';
  return s;
}

/// 槽位数据模板里声明的列名（{{列名}}）。input/output 是虚拟变量，不算数据列。
function slotColumns(slot) {
  if (!slot?.enabled || !String(slot.text || "").trim()) return [];
  const cols = [];
  for (const m of String(slot.text).matchAll(/\{\{([\s\S]*?)\}\}/g)) {
    const k = m[1].trim();
    if (!k || k === "input" || k === "output" || cols.includes(k)) continue;
    cols.push(k);
  }
  return cols;
}

/// 某槽位收到的原始数据：槽位声明的列按「列名：值」逐段列出，值保持数据集原文。
export function slotDataText(meta, row, slotKey) {
  if (!row) return "";
  const cols = slotColumns(meta?.prompts?.injections?.[slotKey]);
  return cols
    .filter((k) => Object.prototype.hasOwnProperty.call(row, k))
    .map((k) => `${k}：${row[k] == null ? "" : String(row[k])}`)
    .join("\n\n");
}

/// 导出取值器：每条结果按 index 从任务的数据集快照里取该行原始数据。
/// 老任务没有数据集快照时退回结果里存的渲染后消息，避免整列空掉。
export function slotDataGetter(meta, rows) {
  return (r) => {
    const row = rows?.[r.index];
    if (!row) return { system: r.system || "", user: r.input || "" };
    return {
      system: slotDataText(meta, row, "targetSystem"),
      user: slotDataText(meta, row, "targetUser"),
    };
  };
}

/// 一行结果 → CSV 字段数组。列名不再参与，数据列本就来自每条结果自身的字段。
function rowOf(r, data) {
  return [
    r.index + 1,
    r.statusLabel,
    r.pass ? "通过" : "不通过",
    (r.errorTypes || []).join(" / "),
    (r.reasons || []).join("；"),
    r.details || "",
    r.output || "",
    data?.system || "",
    data?.user || "",
    r.targetLatencyMs,
    r.judgeLatencyMs,
    r.promptTokens ?? "",
    r.completionTokens ?? "",
    r.judgeRaw || "",
    r.targetError || "",
  ].map(esc);
}

export async function exportCsv(results, title = "", dataOf = null) {
  try {
    const fileName = stampedPath(stemWith(title, "评测结果"), "csv");
    const { path, fixed } = await resolvePath(fileName, "CSV", "csv");
    if (!path) return;

    // BOM 让 Excel 正确识别 UTF-8 中文；首块带表头，顺带截断旧文件
    await invoke("save_text_file", {
      path,
      content: "﻿" + HEADERS.map(esc).join(",") + "\r\n",
    });
    for (let i = 0; i < results.length; i += CSV_CHUNK) {
      const lines = results
        .slice(i, i + CSV_CHUNK)
        .map((r) => rowOf(r, dataOf ? dataOf(r) : null).join(","));
      await invoke("append_text_file", { path, content: lines.join("\r\n") + "\r\n" });
    }
    const where = fixed ? `：${fileNameOf(path)}` : "，可直接用 Excel 打开";
    toast(`已导出 ${results.length} 条 CSV${where}`, "ok");
  } catch (e) {
    toast("导出失败：" + e, "err");
  }
}

export async function exportJson(results, columns, title = "", dataOf = null) {
  try {
    const fileName = stampedPath(stemWith(title, "评测结果"), "json");
    const { path, fixed } = await resolvePath(fileName, "JSON", "json");
    if (!path) return;

    // 手写流式结构，避免 1 万条时一次性 stringify 出几十 MB 的大字符串
    const head =
      "{\n" +
      `  "exportedAt": ${JSON.stringify(new Date().toISOString())},\n` +
      `  "columns": ${JSON.stringify(columns || [])},\n` +
      `  "results": [`;
    await invoke("save_text_file", { path, content: head });

    for (let i = 0; i < results.length; i += JSON_CHUNK) {
      const items = results.slice(i, i + JSON_CHUNK).map((r) => {
        const data = dataOf ? dataOf(r) : null;
        // system / input 换成该行注入到两个槽位的原始数据，字段名保持兼容
        const item = data ? { ...r, system: data.system, input: data.user } : r;
        return JSON.stringify(item, null, 2);
      });
      const body = (i > 0 ? ",\n" : "\n") + items.join(",\n");
      await invoke("append_text_file", { path, content: body });
    }

    await invoke("append_text_file", { path, content: "\n  ]\n}\n" });
    const where = fixed ? `：${fileNameOf(path)}` : "";
    toast(`已导出 ${results.length} 条 JSON${where}`, "ok");
  } catch (e) {
    toast("导出失败：" + e, "err");
  }
}

/// 完整报告：概览（通过率 / 关键指标）+ 失败分类 + 未计入明细 + 分组汇总 + 全部明细，一个 xlsx 多 sheet
export async function exportFullReport(payload) {
  try {
    const fileName = stampedPath(stemWith(payload.title, "评测报告"), "xlsx");
    const { path, fixed } = await resolvePath(fileName, "Excel 工作簿", "xlsx");
    if (!path) return;
    await invoke("export_excel_report", { path, payload });
    const where = fixed ? `：${fileNameOf(path)}` : "";
    toast(`完整报告已导出${where}（${payload.rows.length} 条明细）`, "ok");
  } catch (e) {
    toast("导出失败：" + e, "err");
  }
}
