// 生成压测数据集（JSONL）。用法：
//   node tests/gen-dataset.mjs [行数=10000] [输出路径=tests/dataset-10000.jsonl]
// 每行是一个对象，列名与导入逻辑一致：数据组状态 / question / note。
import fs from "node:fs";
import path from "node:path";

const total = Number(process.argv[2] || 10000);
const out = process.argv[3] || path.join("tests", `dataset-${total}.jsonl`);

const groups = ["正常场景", "边界输入", "长文本", "极短"];
const notes = ["请给出结论", "请分点说明", "请引用原文", "极短回答也可以"];

const lines = [];
for (let i = 0; i < total; i++) {
  lines.push(
    JSON.stringify({
      数据组状态: groups[i % groups.length],
      question: `第 ${i + 1} 条测试输入：请简要说明你对「主题 ${i + 1}」的理解。`,
      note: notes[i % notes.length],
    })
  );
}

fs.mkdirSync(path.dirname(out), { recursive: true });
fs.writeFileSync(out, lines.join("\n") + "\n", "utf8");
console.log(`已生成 ${total} 行 → ${out}（${(fs.statSync(out).size / 1024 / 1024).toFixed(1)} MB）`);
