<p align="center">
  <img src="src-tauri/icons/128x128.png" width="88" alt="批量评测台">
</p>

<h1 align="center">批量评测台</h1>

<p align="center">
  用自己的 LLM API 批量跑评测数据，跑完按评测提示词定义的分类聚合，一眼看出问题集中在哪类数据、哪类问题上。
</p>

<p align="center">
  <img alt="Windows" src="https://img.shields.io/badge/Windows-10%2B-0078D4?logo=windows&logoColor=white">
  <img alt="macOS" src="https://img.shields.io/badge/macOS-universal-000000?logo=apple&logoColor=white">
  <img alt="Tauri" src="https://img.shields.io/badge/Tauri-2-orange?logo=tauri&logoColor=white">
</p>

![分析页](docs/screenshots/05-analysis.png)

## 这个工具解决什么

在浏览器里手工评测提示词，瓶颈不在模型，在流程：数据要一条条粘，几百条下来手会断；判定结果散在聊天记录里，跑完不知道「哪类测试数据最容易出哪类问题」。

批量评测台把这件事流水线化。你提供三样东西——测试数据、被测提示词、评测（裁判）提示词，它并发调用两个 API：被测模型逐条生成输出，裁判模型按评测提示词逐条判定，最后把失败归到你定义的分类里，按分类和测试分组聚合。

全程在本地跑。API Key 存在本机配置文件，不经过任何第三方服务器。

## 五个页面

工具按评测流程拆成五页，顺序就是使用顺序。

### ① API 配置

两个模型分开配：被测模型负责生成，评测模型负责判定。

两者可以是同一个 API，也可以分开——判定要稳定时，裁判建议换更强的模型并把 temperature 设 0。配置项包括协议（OpenAI 兼容 / Anthropic）、Base URL、模型名、上下文长度、max_tokens、temperature、top_p、超时、自定义请求体与请求头。填完可以点「测试连接」先验证通不通。

![API 配置](docs/screenshots/01-api.png)

### ② 提示词

三块内容：主体提示词（被测模型的 system prompt）、用户消息模板（用 `{{列名}}` 注入数据列）、评测提示词（裁判逐条调用的提示词，必须含 `{{input}}` 与 `{{output}}`）。

页面下方实时显示实际发送给 API 的完整内容，变量替换是对是错一眼可见。

![提示词](docs/screenshots/02-prompts.png)

### ③ 数据集

支持 CSV / TSV / Excel / JSON / JSONL / 纯文本，也可以直接粘贴。中文编码自动识别，UTF-8 失败回退 GB18030。

勾选要注入的列即可生成用户消息模板。列名包含「数据组状态」时会被识别为分组列，运行页和分析页按它分组展示。

![数据集](docs/screenshots/03-dataset.png)

### ④ 运行

设并发数和失败重试次数，开始跑。运行中实时看到每条的通过状态、分类和原因，随时可以取消。

中断了不用从头来——结果增量写入磁盘，重启后点「断点续跑」只补没跑的。修好某条之后也可以只重跑失败项。

![运行](docs/screenshots/04-run.png)

### ⑤ 分析

通过率环图、错误分类分布、明细分页表。分类完全来自评测提示词输出的 `errorType` 字面，工具不内置任何分类体系。

点分类条形图可以筛选，点明细任意一行打开详情抽屉：原始数据、实际发送的消息、被测输出、裁判原始输出、判定原因并排看。

![分析明细](docs/screenshots/06-detail.png)

## 几个真正省事的地方

**解析容错**。裁判模型不一定听话，可能把 JSON 包在代码块里、字段名写成 `error_type`、前后带解释文字。解析做了三层：剥代码块 → 括号配平提取 JSON → 字段名大小写与同义词兼容。实在解析不出来的归入「评测输出解析失败」单独计数，不会污染你自己的分类。

**失败不混进分类**。被测 API 挂了、裁判 API 挂了，分别归入「被测模型调用失败」「评测模型调用失败」，和你业务上的分类区分开。

**限流不会一起冲**。遇到 429 / 503 按 `Retry-After` 全局限速，其余并发一起等待，带抖动退避重试。连续 5 条调不通会自动叫停整批，不会白烧额度。

**万级数据不卡**。配置、数据集、结果分文件存储；进度事件按 10 条 / 250ms 批量推送；分析页明细分页，筛选和搜索只作用于当前结果；导出 CSV / JSON 分块写盘。

## 下载安装

**Windows**：下载 `批量评测台_x.y.z_x64-setup.exe` 安装，或者直接用免安装的 `batch-eval.exe`。

**macOS**：下载 `BatchEval_x.y.z_macOS_universal.dmg`，双击后把应用拖进 Applications。包是 universal（Apple Silicon 与 Intel 都能跑），未做 Apple 公证，首次打开需要在图标上右键 →「打开」。如果系统仍然拦截：

```bash
xattr -cr "/Applications/批量评测台.app"
```

## 数据存在哪

全部在本机用户目录，按用途分文件：

```
Windows   %APPDATA%\com.batcheval.app\
macOS     ~/Library/Application Support/com.batcheval.app/

settings.json               配置、提示词、列选择、主题
dataset.json                数据集（单独存，不拖慢配置保存）
runs/current/meta.json      当前运行的元信息
runs/current/results.jsonl  每跑完一条追加一行，崩溃也不丢
```

只保留当前一次运行，新跑会清空上一轮。API Key 明文存在本地，不会上传到任何地方。

## 开发

需要 Node 18+、Rust stable，以及各平台 Tauri 的系统依赖。

```bash
npm install
npm run tauri dev        # 开发模式
npx tauri build          # 打包（Windows 出 exe + NSIS，macOS 出 dmg）
```

前端 Vue 3 + Vite + Tailwind，后端 Rust。跨平台构建走 GitHub Actions，见 `.github/workflows/build-macos.yml`。

### 测试

```bash
cd src-tauri
cargo test --lib                                 # 单元测试

# 端到端（先起 mock 服务）
node ../tests/mock-api.mjs
cargo test --lib -- --ignored --nocapture

# 压测：生成一万行数据集 + 慢速易失败的 mock
node tests/gen-dataset.mjs 10000
MOCK_DELAY_MS=300 MOCK_FAIL_RATE=0.05 node tests/mock-api.mjs
```
