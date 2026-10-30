// 本地 mock OpenAI 兼容服务，用于端到端验证：被测模型返回一句带缺陷的回答，评测模型返回评测 JSON。
//
// 环境变量：
//   MOCK_PORT=18999      监听端口
//   MOCK_DELAY_MS=0      每个请求固定延迟，用来验证批量进度事件与取消
//   MOCK_FAIL_RATE=0     按概率返回 429（带 Retry-After）或 503（HTML 网关页），验证退避重试与错误可读化
//   MOCK_FAIL_KIND=429   失败时固定用哪种状态码；不设则两者交替
import http from "node:http";

const PORT = Number(process.env.MOCK_PORT || 18999);
const DELAY_MS = Number(process.env.MOCK_DELAY_MS || 0);
const FAIL_RATE = Number(process.env.MOCK_FAIL_RATE || 0);
const FAIL_KIND = process.env.MOCK_FAIL_KIND || "";

let failCount = 0;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const server = http.createServer(async (req, res) => {
  if (req.method !== "POST") {
    res.writeHead(404).end();
    return;
  }
  let body = "";
  for await (const chunk of req) body += chunk;
  let payload = {};
  try {
    payload = JSON.parse(body || "{}");
  } catch {}

  if (DELAY_MS > 0) await sleep(DELAY_MS);

  if (FAIL_RATE > 0 && Math.random() < FAIL_RATE) {
    failCount += 1;
    const kind = FAIL_KIND || (failCount % 2 === 1 ? "429" : "503");
    if (kind === "429") {
      res
        .writeHead(429, { "content-type": "application/json", "retry-after": "1" })
        .end(JSON.stringify({ error: { message: "Rate limit reached, retry shortly", type: "rate_limit_error" } }));
    } else {
      // 模拟网关直接吐 HTML，用来验证程序不会把整页 HTML 抛给用户
      res.writeHead(503, { "content-type": "text/html" }).end("<html><body><h1>503 Service Unavailable</h1></body></html>");
    }
    return;
  }

  const messages = payload.messages || [];
  const lastUser = messages.filter((m) => m.role === "user").map((m) => m.content).join("\n");
  const sysMsg = messages.filter((m) => m.role === "system").map((m) => m.content).join("\n");

  // 回显模式：把收到的 system / user 原样返回，用来验证变量替换与链路传递
  if (process.env.MOCK_ECHO === "1") {
    const echo = `[SYSTEM]\n${sysMsg}\n[/SYSTEM]\n[USER]\n${lastUser}\n[/USER]`;
    res.writeHead(200, { "content-type": "application/json" });
    res.end(
      JSON.stringify({
        id: "echo",
        object: "chat.completion",
        choices: [{ index: 0, message: { role: "assistant", content: echo }, finish_reason: "stop" }],
        usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
      })
    );
    return;
  }

  const isJudge = lastUser.includes("OUTPUT_START") || lastUser.includes("评测");

  let content;
  if (isJudge) {
    // 假裁判：长度过短判「内容单薄」，否则通过
    const outMatch = lastUser.match(/<<<OUTPUT_START>>>([\s\S]*?)<<<OUTPUT_END>>>/);
    const output = outMatch ? outMatch[1].trim() : "";
    if (output.length < 40) {
      content = JSON.stringify({
        violations: [
          { errorType: "内容单薄", evidence: output.slice(0, 20), reason: "回答缺少必要的细节" },
        ],
        pass: false,
        errorType: ["内容单薄"],
        details: "回答过短",
      });
    } else {
      content = "```json\n" + JSON.stringify({ violations: [], pass: true, errorType: [], details: "" }) + "\n```";
    }
  } else {
    const short = lastUser.includes("极短");
    content = short ? "好的。" : `针对该输入的完整回答：${lastUser.slice(0, 120)}。已覆盖主要要点。`;
  }

  res.writeHead(200, { "content-type": "application/json" });
  res.end(
    JSON.stringify({
      id: "mock",
      model: payload.model || "mock-model",
      choices: [{ index: 0, message: { role: "assistant", content }, finish_reason: "stop" }],
      usage: { prompt_tokens: 100, completion_tokens: 50 },
    })
  );
});

server.listen(PORT, () =>
  console.log(
    `mock api on http://127.0.0.1:${PORT}/v1  (delay=${DELAY_MS}ms, failRate=${FAIL_RATE})`
  )
);
