"""提示词自动迭代脚本。

两种用法：

  单次运行：跑一遍全量数据，产出完整结果文件。
  迭代运行：把评测提示词当代码管理，有轮次上限、版本存档、回归守卫。

目录约定（只是默认值，都能用参数覆盖）：

  <工作目录>/
  ├─ 评测提示词/   裁判提示词、被测提示词
  ├─ 评测数据/     数据集（csv / json / xlsx）
  ├─ 评测输出/     完整结果、缓存、审计日志
  └─ 提示词历史/   提示词版本存档

脚本和说明随应用安装，通常不需要放进工作目录。工作目录默认为当前目录，
也可以用 --workdir 指定；所有输出和默认路径都相对于它。

脚本唯一假定的，是模板里的变量关系：

  {{input}}  裁判看到的输入 = 被测提示词 + 本条输入
  {{output}} 被测模型对这一条的实际输出

除此之外一概不预设：数据集有哪些列、裁判用什么格式表达通过与否，脚本都不知道。
列名由你通过 {{列名}} 写进模板；判定方式由你读完裁判提示词后用参数指定。

被测提示词、用户消息模板、裁判提示词、数据集都可以直接用参数指向文件，
不需要先导入评测台；评测台只负责存 API 配置。

config.json 里密钥支持环境变量引用：
  "apiKey": "${TARGET_API_KEY}"

没有 config.json 时，退回读取「批量评测台」已保存的配置（只读）。

首次使用建议先在终端跑一次 --mode single，确认接口通、结果合理，
再交给 agent 用 --mode iterate 迭代。

操作手册（给 agent 读）：说明文档/自动迭代_说明.md

退出码：0 通过 / 1 有回归 / 2 超出轮次 / 3 拦截率下降 / 4 输入被改动 / 5 目标未达成 / 6 未判定需重跑
"""

import argparse
import csv
import hashlib
import json
import os
import re
import sys
import time
import urllib.request

# 脚本随应用安装，安装目录只读使用。工作目录取当前目录或 --workdir，
# 状态、输出、存档、默认的提示词/数据目录都相对于它。
def workdir_of(args):
    wd = getattr(args, "workdir", None)
    return os.path.abspath(wd) if wd else os.getcwd()


APP_ID = "com.batcheval.desktop"
STATE_NAME = "regression_state.json"
ARCHIVE_DIR_NAME = "提示词历史"
LOG_NAME = "regression_log.csv"
OUTPUT_DIR_NAME = "评测输出"
CACHE_NAME = "target_outputs.json"

ADVERSARIAL_FORMAT = '[{"name": "用例名", "content": "原文", "output": "模型输出（含原文没有的事实）"}]'


# ---------- 配置 ----------

def env_expand(value):
    """把 ${VAR} 展开成环境变量值，未设置时保留原样。"""
    if not isinstance(value, str):
        return value
    return re.sub(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}",
                  lambda m: os.environ.get(m.group(1), m.group(0)), value)


def expand_cfg(cfg):
    return {k: env_expand(v) for k, v in cfg.items()} if isinstance(cfg, dict) else cfg


def default_config_dir():
    if sys.platform == "win32":
        base = os.environ.get("APPDATA") or os.path.expanduser("~")
    elif sys.platform == "darwin":
        base = os.path.expanduser("~/Library/Application Support")
    else:
        base = os.environ.get("XDG_CONFIG_HOME") or os.path.expanduser("~/.config")
    return os.path.join(base, APP_ID)


def read_text(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def resolve_template_arg(value):
    """模板参数可以直接是文本，也可以指向文件；文件存在就读文件。"""
    if not value:
        return ""
    if os.path.isfile(value):
        return read_text(value)
    # 看着像文件路径但文件不存在时，别把它当模板内容发出去
    if os.path.splitext(value)[1].lower() in (".md", ".txt", ".template") \
            or os.sep in value or "/" in value:
        raise SystemExit("模板文件不存在：%s" % value)
    return value


def load_json(path):
    with open(path, encoding="utf-8-sig") as f:
        return json.load(f)


def load_runtime(args):
    """返回 target_cfg, judge_cfg, system_tpl, user_tpl, rows, source。"""
    if args.config:
        cfg = load_json(args.config)
        target = expand_cfg(cfg.get("target") or {})
        judge = expand_cfg(cfg.get("judge") or {})
        system_tpl = read_text(args.prompt_target) if args.prompt_target else ""
        user_tpl = resolve_template_arg(args.user_template) or cfg.get("userTemplate")
        if user_tpl is None:
            raise SystemExit(
                "config.json 缺少 userTemplate。\n"
                "它是被测模型的 user 消息模板，用数据集的列名写，例如 \"{{content}}\"，"
                "也可以用 --user-template 指定。")
        rows = load_dataset(args.dataset or find_dataset(os.path.join(workdir_of(args), "评测数据")))
        return target, judge, system_tpl, user_tpl, rows, "config"

    cfg_dir = args.config_dir or default_config_dir()
    settings_path = args.settings or os.path.join(cfg_dir, "settings.json")
    if not os.path.isfile(settings_path):
        raise SystemExit(
            "找不到应用配置：%s\n"
            "独立使用时请用 --config config.json 指定接口配置（格式见说明文档）。" % settings_path)
    settings = load_json(settings_path)

    dataset_path = args.dataset
    if not dataset_path:
        app_dataset = os.path.join(cfg_dir, "dataset.json")
        dataset_path = (app_dataset if os.path.isfile(app_dataset)
                        else find_dataset(os.path.join(workdir_of(args), "评测数据")))
    rows = load_dataset(dataset_path)
    st = settings.get("settings") or {}
    target = expand_cfg(st.get("target") or {})
    judge = expand_cfg(st.get("judge") or {})
    prompts = settings.get("prompts") or {}
    system_tpl = read_text(args.prompt_target) if args.prompt_target else prompts.get("systemPrompt", "")
    user_tpl = resolve_template_arg(args.user_template) or prompts.get("userTemplate", "")
    return target, judge, system_tpl, user_tpl, rows, "app"


def col_index(ref):
    n = 0
    for ch in ref:
        n = n * 26 + (ord(ch) - 64)
    return n - 1


def load_xlsx(path):
    """读第一个工作表，返回 [{列名: 值}]。只用标准库，够读 Excel 导出的文本表。"""
    import zipfile
    import xml.etree.ElementTree as ET

    ns = "{http://schemas.openxmlformats.org/spreadsheetml/2006/main}"
    with zipfile.ZipFile(path) as z:
        names = z.namelist()
        shared = []
        if "xl/sharedStrings.xml" in names:
            root = ET.fromstring(z.read("xl/sharedStrings.xml"))
            for si in root.iter(ns + "si"):
                shared.append("".join(t.text or "" for t in si.iter(ns + "t")))
        sheets = sorted(n for n in names
                        if n.startswith("xl/worksheets/sheet") and n.endswith(".xml"))
        if not sheets:
            raise SystemExit("xlsx 里没有工作表：%s" % path)
        root = ET.fromstring(z.read(sheets[0]))

    table = []
    for row in root.iter(ns + "row"):
        cells = {}
        for c in row.findall(ns + "c"):
            m = re.match(r"[A-Z]+", c.get("r", ""))
            if not m:
                continue
            t = c.get("t")
            if t == "inlineStr":
                val = "".join(x.text or "" for x in c.iter(ns + "t"))
            else:
                v = c.find(ns + "v")
                val = v.text if v is not None and v.text is not None else ""
                if t == "s" and val != "":
                    val = shared[int(val)]
            cells[col_index(m.group(0))] = val
        if cells:
            table.append(cells)

    if not table:
        return []
    width = max(table[0]) + 1
    keys = [str(table[0].get(i, "")).strip() or ("列%d" % (i + 1)) for i in range(width)]
    rows = []
    for cells in table[1:]:
        r = {keys[i]: cells.get(i, "") for i in range(width)}
        if any(str(v).strip() for v in r.values()):
            rows.append(r)
    return rows


def load_dataset(path):
    if not path:
        raise SystemExit(
            "找不到数据集：评测数据/ 下没有可读文件，或有多份候选无法确定用哪一份。\n"
            "请用 --dataset 指定 CSV / JSON / XLSX。")
    if not os.path.isfile(path):
        raise SystemExit("数据集不存在：%s" % path)
    low = path.lower()
    if low.endswith(".json"):
        data = load_json(path)
        rows = data["rows"] if isinstance(data, dict) and "rows" in data else data
    elif low.endswith(".xlsx"):
        rows = load_xlsx(path)
    elif low.endswith(".csv"):
        with open(path, encoding="utf-8-sig") as f:
            rows = list(csv.DictReader(f))
    else:
        raise SystemExit("不支持的数据集格式：%s\n支持 CSV / JSON / XLSX。" % path)
    if not isinstance(rows, list) or not rows:
        raise SystemExit("数据集为空：%s" % path)
    if any(not isinstance(r, dict) for r in rows):
        raise SystemExit("数据集的每一行都必须是对象（列名: 值），当前格式不对：%s" % path)
    return rows


def find_dataset(folder):
    """在指定目录里找唯一的数据集文件，找不到或不唯一时返回 None。"""
    if not os.path.isdir(folder):
        return None
    hits = [os.path.join(folder, n) for n in sorted(os.listdir(folder))
            if os.path.splitext(n)[1].lower() in (".csv", ".json", ".xlsx")]
    return hits[0] if len(hits) == 1 else None


# ---------- 工具 ----------

def sha(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def sha_items(items):
    return sha(json.dumps(items, ensure_ascii=False, sort_keys=True))


def row_label(row):
    """只说这一行有什么，不猜哪一列是输入。"""
    vals = [str(v).strip() for v in row.values() if str(v).strip()]
    return " / ".join(vals)[:60].replace("\n", " ")


RESERVED_VARS = ("input", "output")


def render(template, row):
    out = template
    for k, v in row.items():
        out = out.replace("{{%s}}" % k, v if isinstance(v, str) else str(v))
    # 未提供的列替换为空，避免占位符泄漏到输出；
    # input / output 是裁判保留变量，留给 judge_one 填，不能在这里清掉。
    out = re.sub(r"\{\{\s*([A-Za-z_][A-Za-z0-9_]*)\s*\}\}",
                 lambda m: m.group(0) if m.group(1) in RESERVED_VARS else "", out)
    return out


def missing_vars(tpl, columns):
    """模板里引用了、但数据集里没有的变量。"""
    used = set(re.findall(r"\{\{\s*([^{}]+?)\s*\}\}", tpl or ""))
    return sorted(v for v in used if v not in columns and v not in RESERVED_VARS)


def judge_input_text(system_tpl, user_tpl, row):
    """裁判看到的 {{input}}，与批量评测台 runner::judge_input_text 保持一致。"""
    sys_text = render(system_tpl, row).strip()
    user_text = render(user_tpl, row)
    if sys_text:
        return "【被评估模型的提示词】\n%s\n\n【本条输入】\n%s" % (sys_text, user_text)
    return user_text


def api_url(cfg):
    """与评测台 api::endpoint 一致：baseUrl 可以只填到 /v1。"""
    b = str(cfg.get("baseUrl") or "").strip().rstrip("/")
    if b.endswith("/chat/completions") or b.endswith("/messages"):
        return b
    tail = "/messages" if cfg.get("protocol") == "anthropic" else "/chat/completions"
    return b + tail if b.endswith("/v1") else b + "/v1" + tail


def call_chat(cfg, messages, attempts=3):
    if cfg.get("protocol") == "anthropic":
        raise SystemExit(
            "暂不支持 anthropic 原生协议，请在评测台里改用 OpenAI 兼容的接口入口。")
    body = {
        "model": cfg["model"],
        "messages": messages,
        "max_tokens": cfg.get("maxTokens", 2048),
        "temperature": cfg.get("temperature", 0.2),
        "top_p": cfg.get("topP", 1),
    }
    if cfg.get("extraBody"):
        body.update(json.loads(cfg["extraBody"]))
    headers = {"Content-Type": "application/json", "Accept": "*/*", "User-Agent": "reqwest/0.12"}
    if cfg.get("apiKey"):
        headers["Authorization"] = "Bearer " + cfg["apiKey"]
    if cfg.get("extraHeaders"):
        headers.update(json.loads(cfg["extraHeaders"]))
    timeout = cfg.get("timeoutSecs", 120)
    last = None
    for _ in range(attempts):
        try:
            req = urllib.request.Request(
                api_url(cfg), data=json.dumps(body).encode("utf-8"),
                headers=headers, method="POST")
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                data = json.loads(resp.read().decode("utf-8"))
            return data["choices"][0]["message"]["content"]
        except Exception as e:  # noqa: BLE001
            last = e
    raise RuntimeError("接口调用失败：%s" % last)


def parse_verdict(text):
    t = text.strip()
    t = re.sub(r"^```(?:json)?\s*", "", t)
    t = re.sub(r"\s*```$", "", t)
    m = re.search(r"\{.*\}", t, re.S)
    if not m:
        return None
    try:
        return json.loads(m.group(0))
    except json.JSONDecodeError:
        return None


# ---------- 被测模型 ----------

def run_target(target_cfg, system_tpl, user_tpl, rows, only=None):
    """逐条调用被测模型，返回 {序号: 输出}。"""
    outs = {}
    for idx, row in enumerate(rows, start=1):
        if only and idx not in only:
            continue
        messages = []
        sys_text = render(system_tpl, row).strip()
        if sys_text:
            messages.append({"role": "system", "content": sys_text})
        messages.append({"role": "user", "content": render(user_tpl, row)})
        outs[idx] = call_chat(target_cfg, messages)
        print("  被测 %d/%d" % (idx, len(rows)))
        sys.stdout.flush()
    return outs


def load_cache(state_dir):
    p = os.path.join(state_dir, OUTPUT_DIR_NAME, CACHE_NAME)
    if not os.path.isfile(p):
        return None
    try:
        return load_json(p)
    except Exception:  # noqa: BLE001
        return None


def target_signature(cfg):
    """被测模型的调用参数指纹。换了模型或采样参数，缓存的输出就不能再用。"""
    keys = ("baseUrl", "model", "temperature", "topP", "maxTokens", "extraBody")
    return sha(json.dumps({k: cfg.get(k) for k in keys},
                          ensure_ascii=False, sort_keys=True))


def save_cache(state_dir, dataset_hash, outs, system_tpl="", user_tpl="", target_sig=""):
    d = os.path.join(state_dir, OUTPUT_DIR_NAME)
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, CACHE_NAME), "w", encoding="utf-8") as f:
        json.dump({"dataset_hash": dataset_hash,
                   "system_sha": sha(system_tpl or ""),
                   "user_sha": sha(user_tpl or ""),
                   "target_sig": target_sig,
                   "items": outs}, f, ensure_ascii=False, indent=2)


def get_target_outputs(args, target_cfg, system_tpl, user_tpl, rows, state_dir, only=None):
    """优先复用缓存；数据集、被测提示词、用户消息模板任一变化或 --rerun-target 时重跑。"""
    dataset_hash = sha_items(rows)
    target_sig = target_signature(target_cfg)
    cache = load_cache(state_dir)
    cache_ok = (cache is not None
                and cache.get("dataset_hash") == dataset_hash
                and cache.get("system_sha") == sha(system_tpl or "")
                and cache.get("user_sha") == sha(user_tpl or "")
                and cache.get("target_sig") == target_sig
                and not args.rerun_target)
    if cache_ok:
        cached = {int(k): v for k, v in cache["items"].items()}
        missing = [i for i in range(1, len(rows) + 1) if i not in cached]
        if not missing:
            print("复用已缓存的被测输出（%d 条）。" % len(cached))
            return cached
        if not only:
            print("缓存缺 %d 条，重跑被测模型。" % len(missing))
    print("调用被测模型…")
    outs = run_target(target_cfg, system_tpl, user_tpl, rows, only)
    if only:
        return outs
    save_cache(state_dir, dataset_hash, outs, system_tpl, user_tpl, target_sig)
    return outs


# ---------- 裁判 ----------

def as_bool(val):
    if isinstance(val, bool):
        return val
    if isinstance(val, str):
        return val.strip().lower() in ("true", "yes", "1", "pass", "通过", "合格")
    return bool(val)


def as_list(val):
    if val is None or val == "":
        return []
    if isinstance(val, list):
        return [x for x in val if x not in (None, "")]
    if isinstance(val, str):
        return [val]
    return [val]


def make_verdict(raw, args):
    """按 agent 指定的方式，从裁判回复里取出「通过 / 不通过」。

    脚本不认识裁判的输出格式，只实现三种提取方式，必须显式选一种：
      --pass-field   JSON 字段，值为真即通过
      --pass-regex   文本命中即通过
      --fail-regex   文本命中即不通过，没命中一律不通过（需配合 pass-regex）
    取不出来返回 None，由调用方重试或记为未判定。
    """
    if args.pass_field:
        obj = parse_verdict(raw)
        if not isinstance(obj, dict) or args.pass_field not in obj:
            return None
        val = obj.get(args.pass_field)
        if val is None:
            return None
        return {
            "pass": as_bool(val),
            "types": ([str(x) for x in as_list(obj.get(args.type_field))]
                      if args.type_field else []),
            "reason": str(obj.get(args.reason_field) or "") if args.reason_field else raw,
            "raw": raw,
        }

    if args.fail_regex:
        if re.search(args.fail_regex, raw):
            return {"pass": False, "types": [], "reason": raw, "raw": raw}
        if not args.pass_regex:
            return {"pass": True, "types": [], "reason": raw, "raw": raw}
    if args.pass_regex:
        if re.search(args.pass_regex, raw):
            return {"pass": True, "types": [], "reason": raw, "raw": raw}
        if not args.fail_regex:
            return {"pass": False, "types": [], "reason": raw, "raw": raw}
    return None


def judge_one(judge_cfg, judge_tpl, row, input_text, output_text, args, attempts=3):
    prompt = render(judge_tpl, row)
    # 用函数做替换，避免输出内容里的反斜杠被当成转义序列
    prompt = re.sub(r"\{\{\s*input\s*\}\}", lambda _m: input_text, prompt)
    prompt = re.sub(r"\{\{\s*output\s*\}\}", lambda _m: output_text, prompt)
    raw = ""
    for _ in range(attempts):
        raw = call_chat(judge_cfg, [{"role": "user", "content": prompt}])
        v = make_verdict(raw, args)
        if v is not None:
            return v
    return {"pass": None, "types": [], "reason": raw, "raw": raw}


def verdict_reason(v):
    """判定理由就是裁判自己说的话，脚本不加工、不改写。"""
    text = (v.get("reason") or v.get("raw") or "").strip()
    return text[:2000]


def write_result(state_dir, rows, outputs, verdicts, stamp, user_tpl=""):
    d = os.path.join(state_dir, OUTPUT_DIR_NAME)
    os.makedirs(d, exist_ok=True)
    path = os.path.join(d, "result_%s.csv" % stamp)
    with open(path, "w", encoding="utf-8-sig", newline="") as f:
        w = csv.writer(f)
        w.writerow(["序号", "输入", "被测输出", "结果", "错误类型", "判定理由"])
        for idx, row in enumerate(rows, start=1):
            if idx not in verdicts:
                continue
            v = verdicts[idx]
            status = "通过" if v["pass"] else ("未判定" if v["pass"] is None else "不通过")
            w.writerow([idx, render(user_tpl, row), outputs.get(idx, ""),
                        status, "、".join(v["types"]), verdict_reason(v)])
    return path


def prune_results(state_dir, keep=5):
    """只保留最近 keep 份完整结果，避免 评测输出/ 无限累积。"""
    d = os.path.join(state_dir, OUTPUT_DIR_NAME)
    if not os.path.isdir(d):
        return 0
    files = sorted((n for n in os.listdir(d)
                    if n.startswith("result_") and n.endswith(".csv")), reverse=True)
    removed = 0
    for n in files[keep:]:
        try:
            os.remove(os.path.join(d, n))
            removed += 1
        except OSError:
            pass
    return removed


# ---------- 状态 ----------

def state_path(state_dir):
    return os.path.join(state_dir, STATE_NAME)


def scan_archive(state_dir):
    """存档目录是唯一事实来源：状态文件丢了（比如手动整理了目录），历史版本也不能丢。"""
    d = os.path.join(state_dir, ARCHIVE_DIR_NAME)
    if not os.path.isdir(d):
        return []
    out = []
    for n in sorted(os.listdir(d)):
        if not n.lower().endswith(".md"):
            continue
        stem = n[:-3]
        out.append({"id": stem, "hash": stem.rsplit("_", 1)[-1],
                    "time": stem[:15], "file": n})
    return out


def load_state(state_dir):
    p = state_path(state_dir)
    if not os.path.isfile(p):
        return {"round_limit": 5, "rounds_used": 0, "baseline": None,
                "last": None, "archive": scan_archive(state_dir)}
    return load_json(p)


def save_state(state_dir, state):
    os.makedirs(state_dir, exist_ok=True)
    tmp = state_path(state_dir) + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(state, f, ensure_ascii=False, indent=2)
    os.replace(tmp, state_path(state_dir))


def archive_prompt(state_dir, state, prompt_text, prompt_sha):
    last_id = state["archive"][-1]["hash"] if state["archive"] else None
    if last_id == prompt_sha:
        return None
    d = os.path.join(state_dir, ARCHIVE_DIR_NAME)
    os.makedirs(d, exist_ok=True)
    stamp = time.strftime("%Y%m%d_%H%M%S")
    archive_id = "%s_%s" % (stamp, prompt_sha[:8])
    with open(os.path.join(d, archive_id + ".md"), "w", encoding="utf-8") as f:
        f.write(prompt_text)
    state["archive"].append({"id": archive_id, "hash": prompt_sha, "time": stamp,
                             "file": archive_id + ".md"})
    return archive_id


def append_log(state_dir, entry):
    d = os.path.join(state_dir, OUTPUT_DIR_NAME)
    os.makedirs(d, exist_ok=True)
    path = os.path.join(d, LOG_NAME)
    fields = ["round", "time", "prompt_sha", "passed", "total", "adv_blocked", "adv_total",
              "regressions", "adv_dropped", "targets_ok", "verdict", "archive_id"]
    new_file = not os.path.isfile(path)
    # 只有新建文件时写 BOM；追加时再写会在每行前插一个 BOM
    with open(path, "a", encoding="utf-8-sig" if new_file else "utf-8", newline="") as f:
        w = csv.DictWriter(f, fieldnames=fields)
        if new_file:
            w.writeheader()
        w.writerow({k: entry.get(k, "") for k in fields})


def do_history(state_dir, state):
    if not state["archive"]:
        state["archive"] = scan_archive(state_dir)
    if not state["archive"]:
        print("还没有存档。")
        return 0
    print("轮次上限：%d，已用：%d" % (state["round_limit"], state["rounds_used"]))
    for a in state["archive"]:
        print("  %-26s %s" % (a["id"], a["file"]))
    print("回滚：python %s --restore <存档 ID>" % os.path.basename(sys.argv[0]))
    return 0


def do_restore(state_dir, state, archive_id):
    entries = state["archive"] or scan_archive(state_dir)
    hit = next((a for a in entries if a["id"] == archive_id), None)
    if not hit:
        raise SystemExit("找不到存档：%s，用 --history 查看。" % archive_id)
    src = os.path.join(state_dir, ARCHIVE_DIR_NAME, hit["file"])
    print("回滚不自动执行，请确认后运行：")
    print("  cp \"%s\" <你的裁判提示词文件>" % src)
    print("Windows:  copy \"%s\" <你的裁判提示词文件>" % src.replace("/", "\\"))
    return 0


# ---------- 交互确认 ----------

def ask(prompt, default=None):
    hint = " [%s]" % default if default else ""
    try:
        ans = input("%s%s： " % (prompt, hint)).strip()
    except EOFError:
        # Windows 下 stdin 重定向时 isatty() 仍可能为 True，
        # 读不到输入时必须中止，绝不能拿默认值当作「用户确认」。
        raise SystemExit(
            "未检测到交互输入，已中止。\n"
            "非交互环境（如由 agent 调用）请显式指定 --mode，并加 --yes 跳过确认。")
    return ans or default


def resolve_plan(args):
    """确定 mode / 轮次 / 是否输出完整结果。

    --yes 表示非交互（agent 调用）：全部走默认值，不问任何问题。
    不加 --yes 表示人在终端：依次询问，读不到输入直接中止。
    """
    interactive = not args.yes

    if not args.mode:
        if not interactive:
            raise SystemExit("--yes 模式下必须显式指定 --mode single 或 --mode iterate。")
        print("请选择运行方式：")
        print("  1) 单次 single   —— 跑一遍全量，输出完整结果文件")
        print("  2) 迭代 iterate  —— 带轮次上限、版本存档、回归守卫")
        choice = ask("输入 1 或 2", "1")
        args.mode = "iterate" if choice.strip() == "2" else "single"

    if args.mode == "iterate" and args.round_limit is None:
        if interactive:
            v = ask("最多允许迭代几轮", "5")
            try:
                args.round_limit = int(v)
            except ValueError:
                args.round_limit = 5
        else:
            args.round_limit = 5

    if args.full_output is None:
        if interactive:
            ans = ask("是否输出完整结果文件（含每条输入与模型输出）？y/n", "y")
            args.full_output = ans.strip().lower() in ("y", "yes", "1", "是")
        else:
            args.full_output = True

    if interactive:
        print("-" * 60)
        print("即将执行：%s" % ("单次运行" if args.mode == "single" else "迭代运行"))
        if args.mode == "iterate":
            print("轮次上限：%d" % args.round_limit)
        print("完整结果文件：%s" % ("输出" if args.full_output else "不输出"))
        if ask("确认开始？y/n", "y").strip().lower() not in ("y", "yes", "1", "是"):
            raise SystemExit("已取消。")
    return args


# ---------- 主流程 ----------

def run(args):
    state_dir = args.state_dir or workdir_of(args)
    state = load_state(state_dir)
    # 旧状态文件或缺字段时不崩，缺什么补什么
    state.setdefault("round_limit", 5)
    state.setdefault("rounds_used", 0)
    state.setdefault("baseline", None)
    state.setdefault("last", None)
    state.setdefault("archive", scan_archive(state_dir))
    if args.round_limit is not None:
        state["round_limit"] = args.round_limit

    if args.history:
        return do_history(state_dir, state)
    if args.restore:
        return do_restore(state_dir, state, args.restore)
    if args.reset:
        if os.path.isfile(state_path(state_dir)):
            os.remove(state_path(state_dir))
        print("状态已清空（存档目录保留）。")
        return 0

    target_cfg, judge_cfg, system_tpl, user_tpl, rows, source = load_runtime(args)

    if args.list:
        print("列：%s" % " / ".join(rows[0].keys()))
        print("共 %d 条" % len(rows))
        for idx, row in enumerate(rows, start=1):
            print("%2d  %s" % (idx, row_label(row)))
        return 0

    if not (user_tpl or "").strip():
        cols = list(rows[0].keys())
        raise SystemExit(
            "没有用户消息模板。数据集的列是：%s\n"
            "用 --user-template 指定，例如：\n  --user-template \"%s\""
            % (" / ".join(cols), "\\n\\n".join("{{%s}}" % c for c in cols)))

    for label, cfg in (("被测模型", target_cfg), ("评测模型", judge_cfg)):
        if not str(cfg.get("baseUrl") or "").strip():
            raise SystemExit("%s没有 baseUrl，请先在评测台 ① API 配置 里填好并保存。" % label)
        if not str(cfg.get("model") or "").strip():
            raise SystemExit("%s没有模型名，请先在评测台 ① API 配置 里填好并保存。" % label)

    # 裁判提示词：显式指定 > 同目录 prompt_judge.md > 评测提示词/ 下唯一一份
    judge_path = args.prompt_judge
    if not judge_path:
        cand = os.path.join(workdir_of(args), "prompt_judge.md")
        judge_dir = os.path.join(workdir_of(args), "评测提示词")
        hits = ([os.path.join(judge_dir, n) for n in sorted(os.listdir(judge_dir))
                 if n.lower().endswith((".md", ".txt"))] if os.path.isdir(judge_dir) else [])
        judge_path = cand if os.path.isfile(cand) else (hits[0] if len(hits) == 1 else "")
    if not judge_path or not os.path.isfile(judge_path):
        raise SystemExit(
            "找不到裁判提示词。\n"
            "请用 --prompt-judge 指定，或把唯一一份裁判提示词放进 评测提示词/ 目录。")
    judge_tpl = read_text(judge_path)
    judge_sha = sha(judge_tpl)

    columns = set(rows[0].keys())
    for label, tpl in (("主体提示词", system_tpl), ("用户消息模板", user_tpl),
                       ("评测提示词", judge_tpl)):
        miss = missing_vars(tpl, columns)
        if miss:
            print("提示：%s引用了数据集里没有的变量：%s（这些位置会留空）"
                  % (label, "、".join(miss)))

    # 脚本不知道裁判怎么输出，必须由调用方看完裁判提示词后指定提取方式
    if args.pass_field and (args.pass_regex or args.fail_regex):
        raise SystemExit("--pass-field 与 --pass-regex / --fail-regex 只能二选一。")
    if not (args.pass_field or args.pass_regex or args.fail_regex):
        raise SystemExit(
            "不知道该从裁判回复里怎么取「通过 / 不通过」。\n"
            "先看裁判提示词的输出格式，然后指定一种：\n"
            "  --pass-field pass            JSON 字段，值为真即通过\n"
            "  --pass-regex \"判定：通过\"     文本命中即通过，未命中即不通过\n"
            "  --fail-regex \"不通过\"         文本命中即不通过，未命中即通过")

    args = resolve_plan(args)

    # 输入锁定：基线建立后，数据集 / 被测提示词 / 用户消息模板不允许悄悄变化。
    # 否则回归结论分不清是评测提示词改坏了，还是输入本身换了。
    if (args.mode == "iterate" and state.get("baseline")
            and not args.accept_input_change):
        base = state["baseline"].get("inputs") or {}
        now = {"dataset": sha_items(rows),
               "target": sha(system_tpl or ""),
               "userTemplate": sha(user_tpl or "")}
        names = {"dataset": "数据集", "target": "被测提示词", "userTemplate": "用户消息模板"}
        changed = [k for k in names if k in base and base[k] != now[k]]
        if changed:
            print("输入已变化：%s" % "、".join(names[k] for k in changed))
            print("基线是按旧输入建立的，直接比会得出错误结论。")
            print("确认是有意更换：用 --reset 清空状态重新建立基线（推荐）；")
            print("或加 --accept-input-change 保留历史继续（只放行这一次，不改本轮验收判定）。")
            append_log(state_dir, {"round": state["rounds_used"],
                                   "time": time.strftime("%Y-%m-%d %H:%M:%S"),
                                   "prompt_sha": judge_sha[:8], "verdict": "输入已变化"})
            return 4

    is_full = not args.only
    if args.mode == "iterate" and is_full:
        if state["rounds_used"] >= state["round_limit"] and not args.accept:
            print("已达轮次上限（%d 轮）。要强制继续请加 --accept 或 --round-limit 调整。"
                  % state["round_limit"])
            append_log(state_dir, {"round": state["rounds_used"],
                                   "time": time.strftime("%Y-%m-%d %H:%M:%S"),
                                   "prompt_sha": judge_sha[:8], "verdict": "超轮次拒绝"})
            return 2

    stamp = time.strftime("%Y%m%d_%H%M%S")

    # 被测输出
    outputs = get_target_outputs(args, target_cfg, system_tpl, user_tpl, rows, state_dir, args.only)

    # 裁判
    print("调用裁判模型…")
    verdicts = {}
    for idx, row in enumerate(rows, start=1):
        if args.only and idx not in args.only:
            continue
        v = judge_one(judge_cfg, judge_tpl, row,
                      judge_input_text(system_tpl, user_tpl, row), outputs.get(idx, ""), args)
        verdicts[idx] = v
        status = "通过" if v["pass"] else ("未判定" if v["pass"] is None else "不通过")
        print("  %2d  %s｜%s" % (idx, status, "、".join(v["types"]) or "无"))
        sys.stdout.flush()

    passed = sorted(i for i, v in verdicts.items() if v["pass"])
    undetermined = sorted(i for i, v in verdicts.items() if v["pass"] is None)
    print("-" * 60)
    print("通过 %d / %d，未判定 %d" % (len(passed), len(verdicts), len(undetermined)))

    # 对抗样本（迭代模式）
    adv_blocked = adv_total = None
    if args.mode == "iterate" and args.adversarial_file:
        cases = load_json(args.adversarial_file)
        if not isinstance(cases, list) or not cases:
            raise SystemExit("对抗样本文件应为非空 JSON 数组，格式：\n%s" % ADVERSARIAL_FORMAT)
        print("对抗样本…")
        adv_blocked, adv_total = 0, len(cases)
        for i, case in enumerate(cases, start=1):
            v = judge_one(judge_cfg, judge_tpl, case, case["content"], case["output"], args)
            if not v["pass"]:
                adv_blocked += 1
            print("  [%d] %s：%s" % (i, case.get("name", "样本 %d" % i),
                                     "✓ 拦下" if not v["pass"] else "❌ 漏放"))
        print("拦截 %d/%d" % (adv_blocked, adv_total))

    # 完整结果
    if args.full_output:
        path = write_result(state_dir, rows, outputs, verdicts, stamp, user_tpl)
        print("完整结果：%s" % path)
        pruned = prune_results(state_dir)
        if pruned:
            print("已清理旧结果 %d 份（只保留最近 5 份）。" % pruned)

    # 单次模式到此结束
    if args.mode == "single":
        if undetermined:
            return 6
        return 0

    # 单条定位（--only）：只看这几条，不计轮次、不比较基线、不更新基线
    if not is_full:
        archive_id = archive_prompt(state_dir, state, judge_tpl, judge_sha)
        state["last"] = {"prompt_sha": judge_sha, "passed": passed,
                         "adv_blocked": adv_blocked, "adv_total": adv_total,
                         "time": time.strftime("%Y-%m-%d %H:%M:%S")}
        save_state(state_dir, state)
        append_log(state_dir, {
            "round": state["rounds_used"], "time": time.strftime("%Y-%m-%d %H:%M:%S"),
            "prompt_sha": judge_sha[:8], "passed": len(passed), "total": len(verdicts),
            "adv_blocked": adv_blocked if adv_blocked is not None else "",
            "adv_total": adv_total if adv_total is not None else "",
            "verdict": "单条定位", "archive_id": archive_id or "",
        })
        print("单条定位运行：不计轮次、不比较基线、不更新基线。")
        return 6 if undetermined else 0

    # 迭代模式：对比基线
    baseline = state.get("baseline")
    regressions, adv_dropped, targets_ok = [], False, True
    if baseline:
        regressions = [i for i in baseline.get("passed", [])
                       if i in verdicts and verdicts[i]["pass"] is False]
        if adv_blocked is not None and baseline.get("adv_blocked") is not None:
            adv_dropped = adv_blocked < baseline["adv_blocked"]
    if args.target:
        targets_ok = all(verdicts.get(i, {}).get("pass") for i in args.target)

    verdicts_msg = []
    if regressions:
        verdicts_msg.append("有回归：%s" % regressions)
    if adv_dropped:
        verdicts_msg.append("拦截率下降：%d→%d" % (baseline["adv_blocked"], adv_blocked))
    if not targets_ok:
        verdicts_msg.append("目标未达成：%s" % [i for i in args.target if not verdicts.get(i, {}).get("pass")])
    if undetermined:
        verdicts_msg.append("未判定（建议重跑）：%s" % undetermined)

    accepted = not verdicts_msg or args.accept
    print("=" * 60)
    print("验收结论：%s" % ("采纳" if accepted else "拒绝采纳"))
    for m in verdicts_msg:
        print("  - %s" % m)

    archive_id = archive_prompt(state_dir, state, judge_tpl, judge_sha)
    if is_full:
        state["rounds_used"] += 1
    state["last"] = {"prompt_sha": judge_sha, "passed": passed,
                     "adv_blocked": adv_blocked, "adv_total": adv_total,
                     "time": time.strftime("%Y-%m-%d %H:%M:%S")}
    if accepted and is_full:
        state["baseline"] = {"prompt_sha": judge_sha, "passed": passed,
                             "adv_blocked": adv_blocked, "adv_total": adv_total,
                             "inputs": {"dataset": sha_items(rows),
                                        "target": sha(system_tpl or ""),
                                        "userTemplate": sha(user_tpl or "")},
                             "time": time.strftime("%Y-%m-%d %H:%M:%S")}
    save_state(state_dir, state)
    append_log(state_dir, {
        "round": state["rounds_used"], "time": time.strftime("%Y-%m-%d %H:%M:%S"),
        "prompt_sha": judge_sha[:8], "passed": len(passed), "total": len(verdicts),
        "adv_blocked": adv_blocked if adv_blocked is not None else "",
        "adv_total": adv_total if adv_total is not None else "",
        "regressions": ",".join(map(str, regressions)),
        "adv_dropped": "是" if adv_dropped else "",
        "targets_ok": "" if targets_ok else "否",
        "verdict": "采纳" if accepted else "拒绝采纳", "archive_id": archive_id or "",
    })

    if regressions:
        return 1
    if adv_dropped:
        return 3
    if not targets_ok:
        return 5
    if undetermined:
        return 6
    return 0


def main():
    ap = argparse.ArgumentParser(description="提示词自动迭代脚本")
    ap.add_argument("--mode", choices=["single", "iterate"], default=None,
                    help="single 单次输出完整结果 / iterate 迭代守护（非交互环境必填）")
    ap.add_argument("--config", default=None, help="独立配置文件 config.json")
    ap.add_argument("--prompt-target", default=None, help="被评测的提示词（被测模型 system）")
    ap.add_argument("--user-template", default="", metavar="TEXT|FILE",
                    help="被测模型的 user 消息模板，可直接写或指向文件")
    ap.add_argument("--prompt-judge", default=None, help="裁判提示词")
    ap.add_argument("--dataset", default=None, help="数据集 CSV / JSON / XLSX")
    ap.add_argument("--adversarial-file", default=None, help="对抗样本 JSON")
    # 裁判的输出格式脚本不认识，由调用方看完裁判提示词后指定
    ap.add_argument("--pass-field", default="", metavar="KEY",
                    help="裁判 JSON 里表示通过与否的字段名")
    ap.add_argument("--type-field", default="", metavar="KEY",
                    help="裁判 JSON 里错误类型字段名（可选）")
    ap.add_argument("--reason-field", default="", metavar="KEY",
                    help="裁判 JSON 里判定理由字段名（可选，不填则存原始回复）")
    ap.add_argument("--pass-regex", default="", metavar="RE",
                    help="文本命中即通过，未命中即不通过")
    ap.add_argument("--fail-regex", default="", metavar="RE",
                    help="文本命中即不通过，未命中即通过")
    ap.add_argument("--round-limit", type=int, default=None, help="轮次上限，默认 5")
    ap.add_argument("--only", default="", help="只跑指定序号，逗号分隔（不计轮次、不比较基线、不更新基线）")
    ap.add_argument("--target", default="", help="目标序号，逗号分隔")
    ap.add_argument("--rerun-target", action="store_true", help="强制重跑被测模型")
    ap.add_argument("--full-output", dest="full_output", action="store_true", default=None,
                    help="输出完整结果文件")
    ap.add_argument("--no-full-output", dest="full_output", action="store_false",
                    help="不输出完整结果文件")
    ap.add_argument("--accept", action="store_true", help="人工强制采纳本轮结果 / 超轮次继续")
    ap.add_argument("--accept-input-change", dest="accept_input_change", action="store_true",
                    help="数据集/被测提示词/用户模板变化后放行，不改本轮验收判定")
    ap.add_argument("--yes", action="store_true", help="跳过确认")
    ap.add_argument("--list", action="store_true", help="列出数据集条目")
    ap.add_argument("--history", action="store_true", help="列出提示词存档")
    ap.add_argument("--restore", default=None, metavar="ID", help="打印回滚命令")
    ap.add_argument("--reset", action="store_true", help="清空迭代状态")
    ap.add_argument("--workdir", default=None, help="工作目录，默认当前目录；输出、存档和默认提示词/数据目录都相对它")
    ap.add_argument("--state-dir", default=None, help="状态/存档/日志/输出目录")
    ap.add_argument("--config-dir", default=None, help="批量评测台配置目录")
    ap.add_argument("--settings", default=None, help="settings.json")
    args = ap.parse_args()
    args.target = [int(x) for x in args.target.split(",") if x.strip()]
    args.only = {int(x) for x in args.only.split(",") if x.strip()}
    return run(args)


if __name__ == "__main__":
    sys.exit(main())
