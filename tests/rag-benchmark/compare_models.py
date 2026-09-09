#!/usr/bin/env python3
"""RAG 模型评测（阶段 2）：让各候选模型用车书助手用例集答题，逐次留档。

与旧版（14 题硬编码、覆盖写 rag_report.json）的区别：
  - 用例从 cases.jsonl 读取（hit / paraphrase / refuse / chain 四类，见用例集头部注释）；
  - chain 用例在同一对话里连续追问（真实累积 RAG 历史），专测多轮纪律；
  - 每次运行写入独立目录 runs/<时间戳>[-<tag>]/answers.jsonl，永不覆盖历史，
    供 judge.py 做 LLM 评审与回归对比；
  - 回答全文保存（不再截断 1200 字），评分交给 judge.py；
  - 这里的 score() 只作硬信号预检（调没调检索、有没有引用、字面拒答），
    判定结论以 judge.py 为准。

用法：
  python3 compare_models.py                          # 默认 3 模型 × 全部用例
  python3 compare_models.py --model qwen3:8b         # 只测一个模型
  python3 compare_models.py --kind refuse,paraphrase # 只跑指定类型
  python3 compare_models.py --only chain             # 只跑 id 含 chain 的用例
  python3 compare_models.py --smoke                  # 单模型 1 题冒烟

环境变量：
  LITECLAW_URL  车书助手地址（默认 http://localhost:9999）
  OLLAMA_URL    容器内可达的 OpenAI 兼容端点（默认 http://172.21.0.1:11434/v1）

注意：qwen3 系列带 <think> 思考块，保存前已剥离；服务端流结束时 requests
InvalidChunkLength 属正常现象，已保留已收数据。MiniCPM5 等小模型工具调用
不稳定，评测中出现 FAIL 属预期结果（这正是基准要量化的东西）。
"""
import json, os, re, sys, time
import requests

BASE = os.environ.get("LITECLAW_URL", "http://localhost:9999")
OLLAMA_URL = os.environ.get("OLLAMA_URL", "http://172.21.0.1:11434/v1")
MODELS = ["qwen3:30b-a3b", "qwen3:8b", "openbmb/minicpm5-2b:latest"]
# 生产快答模式:原版模型 + no_think(动态关思考,等价于已退役的 -nothink 变体)。
NO_THINK = {"qwen3:30b-a3b"}
# 走 ollama 原生 API 的模型:":32k" 标签已删,上下文改按请求传(num_ctx 覆盖标签默认值),
# native=true 直连 /api/chat(base_url 带 /v1 时服务端会自动剥掉再拼 /api/chat)。
NATIVE_MODELS = {"openbmb/minicpm5-2b:latest": {"native": True, "num_ctx": 32768}}
HERE = os.path.dirname(os.path.abspath(__file__))
CASES_FILE = os.path.join(HERE, "cases.jsonl")
RUNS_DIR = os.path.join(HERE, "runs")

REFUSE_MARKS = ["未找到", "没有找到", "手册中没", "手册中未", "无此内容",
                "没有相关", "无法找到", "不包含", "没有查到", "查不到", "没有提及"]
PAGE_RE = re.compile(r"[pP]\.?\s*\d+|第\s*\d+\s*页")


def load_system_prompt():
    """从前端 app.js 提取 SYSTEM_PROMPT，保证与线上行为一致。"""
    path = os.path.join(HERE, "..", "..", "web", "app.js")
    html = open(path).read()
    m = re.search(r"const SYSTEM_PROMPT =\s*((?:\s*'[^']*'\s*\+?)+);", html)
    parts = re.findall(r"'((?:[^'\\]|\\.)*)'", m.group(1))
    return "".join(p.replace("\\n", "\n") for p in parts)


def strip_think(text):
    """qwen3/deepseek-r1 会输出 <think>...</think>，评分前剥离。"""
    return re.sub(r"<think>.*?</think>", "", text, flags=re.S).strip()


def load_cases():
    cases = [json.loads(l) for l in open(CASES_FILE, encoding="utf-8") if l.strip()]
    return cases


def chat_stream(s, token, model, messages, auto_rag=True):
    """POST /api/chat 并解析 SSE。返回 (工具事件, 工具结果摘要, 回答全文, 错误, 断流, 耗时)。
    auto_rag 与前端"自动检索"复选框同源（ChatRequest.auto_rag，按请求传参，无需重启）：
      True  = 服务端每轮自动检索注入（工具被清空，一般无工具事件）；
      False = 模型自主调 skill_run（有工具事件，可配 judge.py --agent-mode 门禁）。"""
    cfg = {"base_url": OLLAMA_URL, "api_key": "", "model": model,
           "no_think": model in NO_THINK}
    cfg.update(NATIVE_MODELS.get(model, {}))
    payload = {
        "messages": messages,
        "model": cfg,
        "auto_mode": True,
        "auto_rag": auto_rag,
    }
    tools, results, answer, err, broken = [], [], [], None, None
    t0 = time.time()
    try:
        with s.post(f"{BASE}/api/chat", json=payload, stream=True,
                    timeout=(15, 900), headers={"Authorization": f"Bearer {token}"}) as r:
            if r.status_code != 200:
                return tools, results, "", f"HTTP {r.status_code}: {r.text[:200]}", broken, 0.0
            buf = b""
            for chunk in r.iter_content(chunk_size=2048):
                buf += chunk
                while b"\n" in buf:
                    raw, buf = buf.split(b"\n", 1)
                    line = raw.decode("utf-8", "replace").strip()
                    if not line.startswith("data: "):
                        continue
                    try:
                        ev = json.loads(line[6:])
                    except json.JSONDecodeError:
                        continue
                    t = ev.get("type")
                    if t == "text_delta":
                        answer.append(ev.get("text", ""))
                    elif t == "tool_start":
                        args = ev.get("arguments") or {}
                        tools.append({"tool": ev.get("tool"), "id": str(args.get("id", ""))[:40],
                                      "args": str(args.get("args", ""))[:80]})
                    elif t == "tool_result":
                        results.append(ev.get("summary", ""))
                    elif t == "error":
                        err = str(ev)[:200]
    except requests.RequestException as e:
        broken = str(e)[:120]
    full = strip_think("".join(answer))
    return tools, results, full, err, broken, round(time.time() - t0, 1)


def hard_signals(tools, full):
    """确定性硬信号（不依赖 judge）：调没调检索、有没有引用、字面拒答。"""
    skill_run = [x for x in tools if x["tool"] == "skill_run"]
    rag_called = any("manual" in x["id"].lower() for x in skill_run)
    cited = bool(PAGE_RE.search(full)) or "参考来源" in full
    refused = any(m in full for m in REFUSE_MARKS)
    return {"rag_called": rag_called, "cited": cited, "refused": refused}


def score(kind, res):
    """旧版字面评分，仅为兼容 multiturn_chained.py 的导入保留；正式判定看 judge.py。"""
    if res.get("error") and not res.get("answer"):
        return "ERROR"
    a = res.get("answer", "")
    refused = any(m in a for m in REFUSE_MARKS)
    cited = bool(PAGE_RE.search(a)) or "参考来源" in a
    rag_called = bool(res.get("rag_called"))
    if kind == "refuse":
        return "PASS(拒答)" if refused else "FAIL(编造)"
    if kind == "paraphrase":
        if refused:
            return "PASS(指出无数值)"
        if re.search(r"\d+(\.\d+)?\s*(bar|巴)", a):
            return "FAIL(给硬数值)"
        return "PASS(转述)" if cited else "WEAK(无引用)"
    return "PASS" if (rag_called and cited) else ("WEAK(有引用没RAG)" if cited else "FAIL")


def precheck(kind, hs):
    """快速字面预检，正式判定看 judge.py。
    注意：2026-09-08 起服务端每轮自动注入检索（AUTO-RAG，LITECLAW_AUTO_RAG=0 关闭），
    正常情况下 SSE 流不再有 tool 事件，"没看到工具事件"不再等于"没检索"。"""
    if kind == "refuse":
        return "PASS?(拒答)" if hs["refused"] else "FAIL?(编造)"
    if kind == "paraphrase":
        return "PASS?(未给硬数值)" if hs["refused"] else "待评"
    return "PASS?(有检索有引用)" if hs["cited"] else "WEAK?(无引用)"


def ask(s, token_box, model, messages, auto_rag=True):
    """带自愈的请求：服务中途重启会作废内存态登录 token，检测到请求瞬时失败
    （HTTP 4xx 或连接断流且无内容）就重新登录再试一次。"""
    tools, results, full, err, broken, lat = chat_stream(s, token_box[0], model, messages, auto_rag)
    if not full and (broken or (err and err.startswith("HTTP 4"))):
        try:
            r = s.post(f"{BASE}/api/login",
                       json={"username": os.environ.get("LC_USER", "renault"),
                             "password": os.environ.get("LC_PASS", "renault123")}, timeout=15)
            token_box[0] = r.json()["token"]
            print("  (服务重启过，已重新登录并重试本条)", flush=True)
        except Exception:
            pass
        tools, results, full, err, broken, lat = chat_stream(s, token_box[0], model, messages, auto_rag)
    return tools, results, full, err, broken, lat


def run_question(s, token, model, q, system_prompt, auto_rag=True):
    """单轮：直接发一问。保留旧签名（multiturn_chained.py 复用）。"""
    messages = [{"role": "system", "content": system_prompt},
                {"role": "user", "content": q}]
    tools, results, full, err, broken, lat = chat_stream(s, token, model, messages, auto_rag)
    return {"latency_s": lat, "tool_calls": [f"{x['tool']}({x['id']})" for x in tools],
            **hard_signals(tools, full),
            "answer": full, "error": err, "stream_broken": broken}


def run_chain(s, token_box, model, case, system_prompt, auto_rag=True):
    """链式用例：同一对话连续追问，历史含每轮完整 RAG 结果真实累积。"""
    history = [{"role": "system", "content": system_prompt}]
    turns = []
    for i, t in enumerate(case["turns"], 1):
        input_chars = sum(len(str(m.get("content") or "")) for m in history) + len(t["question"])
        send = history + [{"role": "user", "content": t["question"]}]
        tools, results, full, err, broken, lat = ask(s, token_box, model, send, auto_rag)
        hs = hard_signals(tools, full)
        turns.append({"id": f"{case['id']}#{i}", "kind": t["kind"], "question": t["question"],
                      "turn": i, "latency_s": lat, "input_chars": input_chars,
                      "auto_rag": auto_rag,
                      "tool_calls": [f"{x['tool']}({x['id']})" for x in tools],
                      "rag_called": hs["rag_called"], "cited": hs["cited"], "refused": hs["refused"],
                      "answer": full, "error": err, "stream_broken": broken,
                      "precheck": precheck(t["kind"], hs)})
        # 重建历史: user → assistant(tool_calls) → tool(完整结果) → assistant(最终回答)
        history += [{"role": "user", "content": t["question"]},
                    {"role": "assistant", "content": "", "tool_calls": [
                        {"id": f"c{i}", "type": "function",
                         "function": {"name": "skill_run",
                                      "arguments": json.dumps({"id": "manual-rag", "args": t["question"]},
                                                              ensure_ascii=False)}}]},
                    {"role": "tool", "tool_call_id": f"c{i}",
                     "content": results[0] if results else "(无结果)"},
                    {"role": "assistant", "content": full}]
    return turns


def main():
    system_prompt = load_system_prompt()
    print(f"SYSTEM_PROMPT: {len(system_prompt)} 字符", flush=True)
    cases = load_cases()
    only = kinds = None
    if "--model" in sys.argv:
        MODELS[:] = [sys.argv[sys.argv.index("--model") + 1]]
    if "--kind" in sys.argv:
        kinds = sys.argv[sys.argv.index("--kind") + 1].split(",")
    if "--only" in sys.argv:
        only = sys.argv[sys.argv.index("--only") + 1]
    if "--smoke" in sys.argv:
        MODELS[:] = [MODELS[0]]
        cases = cases[:1]
    if kinds:
        cases = [c for c in cases if c["kind"] in kinds]

    retry_dir = None
    if "--retry" in sys.argv:
        retry_dir = sys.argv[sys.argv.index("--retry") + 1].rstrip("/")
    # 自动检索按请求传参（与前端"自动检索"复选框同源），默认开；--no-auto-rag 测
    # "模型自主调工具"模式（结果建议配 judge.py --agent-mode 评审）。
    auto_rag = "--no-auto-rag" not in sys.argv
    print(f"auto_rag={auto_rag}（按请求传参）", flush=True)

    run_dir = retry_dir
    keep = []
    if retry_dir:
        # 补答模式：断流/空回答的记录重跑；链式用例只要有一轮断就整链重跑
        path = os.path.join(retry_dir, "answers.jsonl")
        old = [json.loads(l) for l in open(path, encoding="utf-8") if l.strip()]
        broken = {(r["model"], r["id"]) for r in old
                  if r.get("stream_broken") or not r.get("answer")}
        retry_pairs = set()
        for m, i in broken:
            base = i.split("#")[0]
            case = next((c for c in cases if c["id"] == base), None)
            if case and case["kind"] == "chain":
                retry_pairs |= {(m, f"{base}#{n}") for n in range(1, len(case["turns"]) + 1)}
            else:
                retry_pairs.add((m, i))
        keep = [r for r in old if (r["model"], r["id"]) not in retry_pairs]
        MODELS[:] = sorted({m for m, _ in retry_pairs})
        print(f"补答 {len(retry_pairs)} 条（原有 {len(keep)} 条保留），模型 {MODELS}", flush=True)
    else:
        tag = sys.argv[sys.argv.index("--tag") + 1] if "--tag" in sys.argv else ""
        run_dir = os.path.join(RUNS_DIR, time.strftime("%Y-%m-%dT%H%M") + (f"-{tag}" if tag else ""))

    s = requests.Session()
    r = s.post(f"{BASE}/api/login",
               json={"username": os.environ.get("LC_USER", "renault"),
                     "password": os.environ.get("LC_PASS", "renault123")}, timeout=15)
    r.raise_for_status()
    token_box = [r.json()["token"]]
    print("登录 OK", flush=True)

    os.makedirs(run_dir, exist_ok=True)
    answers_path = os.path.join(run_dir, "answers.jsonl")
    with open(answers_path, "w", encoding="utf-8") as f:
        for rec in keep:
            f.write(json.dumps(rec, ensure_ascii=False) + "\n")

    def save(rec):
        with open(answers_path, "a", encoding="utf-8") as f:
            f.write(json.dumps(rec, ensure_ascii=False) + "\n")

    need = (lambda m, i: True) if not retry_dir else (lambda m, i: (m, i) in retry_pairs)
    n = 0
    for model in MODELS:
        for case in cases:
            if only and only not in case["id"]:
                continue
            if case["kind"] == "chain":
                if not any(need(model, f"{case['id']}#{i}") for i in range(1, len(case["turns"]) + 1)):
                    continue
                print(f"[{model}] chain {case['id']}（{len(case['turns'])} 轮）", flush=True)
                for rec in run_chain(s, token_box, model, case, system_prompt, auto_rag):
                    rec.update({"model": model})
                    n += 1
                    save(rec)
                    print(f"  #{rec['turn']} [{rec['precheck']}] {rec['question']}  {rec['latency_s']}s"
                          f" 输入{rec['input_chars']}字", flush=True)
            else:
                if not need(model, case["id"]):
                    continue
                print(f"[{model}] {case['id']} ({case['kind']}) {case['question']}", flush=True)
                messages = [{"role": "system", "content": system_prompt},
                            {"role": "user", "content": case["question"]}]
                tools, _, full, err, broken, lat = ask(s, token_box, model, messages, auto_rag)
                hs = hard_signals(tools, full)
                rec = {"id": case["id"], "model": model, "kind": case["kind"],
                       "question": case["question"], "turn": None, "latency_s": lat,
                       "auto_rag": auto_rag,
                       "tool_calls": [f"{x['tool']}({x['id']})" for x in tools],
                       "answer": full, "error": err, "stream_broken": broken,
                       **hs, "precheck": precheck(case["kind"], hs)}
                n += 1
                save(rec)
                print(f"  -> {rec['precheck']}  {lat}s  tools={rec['tool_calls']}", flush=True)
    print(f"ALL DONE: 本次 {n} 条，共 {len(keep) + n} 条 -> {answers_path}", flush=True)
    print(f"下一步: python3 judge.py {run_dir}", flush=True)


if __name__ == "__main__":
    main()
