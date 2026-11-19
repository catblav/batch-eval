/// 详情抽屉里「被测输出 / 评测输出」的显示用格式化：JSON 缩进展开，其余文字原样留着。
/// 模型经常把结果压成一行 JSON，不展开根本读不了；但评测模型有时会在 JSON 外
/// 补一句说明，那句话也是信息，不能为了好读把它吃掉。

function tryPretty(s) {
  try {
    return JSON.stringify(JSON.parse(s), null, 2);
  } catch {
    return null;
  }
}

/// 第一段括号配平的 JSON 区间 [start, end)，字符串内的括号不算。
function jsonSpan(s) {
  const start = s.search(/[{\[]/);
  if (start < 0) return null;
  const open = s[start];
  const close = open === "{" ? "}" : "]";
  let depth = 0;
  let inStr = false;
  let esc = false;
  for (let i = start; i < s.length; i++) {
    const c = s[i];
    if (inStr) {
      if (esc) esc = false;
      else if (c === "\\") esc = true;
      else if (c === '"') inStr = false;
      continue;
    }
    if (c === '"') inStr = true;
    else if (c === open) depth++;
    else if (c === close && --depth === 0) return [start, i + 1];
  }
  return null;
}

/// 整体是 JSON（含被 ``` 围栏包住）就直接格式化；
/// 混在说明文字里的只格式化那一段，前后文字保持原样；
/// 解析不出来就原样返回——不是 JSON 的输出不该被改动。
export function prettyJson(text) {
  const raw = String(text ?? "");
  const t = raw.trim();
  if (!t || !/[{\[]/.test(t)) return raw;

  const fenced = t.match(/^```[a-zA-Z]*\s*\n?([\s\S]*?)\n?```$/);
  const whole = tryPretty(fenced ? fenced[1].trim() : t);
  if (whole) return whole;

  const span = jsonSpan(t);
  if (!span) return raw;
  const inner = tryPretty(t.slice(span[0], span[1]));
  if (!inner) return raw;
  return t.slice(0, span[0]) + inner + t.slice(span[1]);
}
