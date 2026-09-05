#!/usr/bin/env python3
"""RAG 模型对比基准：用车书助手的 14 道边界测试题对多个本地模型打分。

模拟前端完整行为：登录 → 携带 SYSTEM_PROMPT(自动从 index.html 提取) →
POST /api/chat → 字节级解析 SSE 事件流（text_delta / tool_start / tool_result）。

自动评分维度：
  - rag_called   是否真的调用了 skill_run("manual-rag", ...)（Agent 纪律核心）
  - cited        回答是否附"参考来源"/页码引用
  - refuse 类题  是否明确拒答而不是编造（发动机机油/汽油滤芯/油箱盖）
  - paraphrase 类题  是否编造具体胎压 bar 数（应指向车门标签）
  - latency_s    端到端耗时（含检索 + 生成）

用法：
  python3 compare_models.py                     # 默认 MODELS 列表全部 14 题
  python3 compare_models.py --model qwen3:8b    # 只测一个模型
  python3 compare_models.py --smoke             # 单模型单题冒烟

环境变量：
  LITECLAW_URL  车书助手地址（默认 http://localhost:9999）
  OLLAMA_URL    容器内可达的 OpenAI 兼容端点（默认 http://172.21.0.1:11434/v1）

结果写入脚本同目录：rag_report.json（逐题原始数据，含回答全文摘录）。
注意：qwen3 系列带 <think> 思考块，评分前已剥离；服务端流结束时 hyper
关闭连接可能触发 requests InvalidChunkLength，属正常现象，已保留已收数据。
"""
import json, os, re, sys, time
import requests

BASE = os.environ.get("LITECLAW_URL", "http://localhost:9999")
OLLAMA_URL = os.environ.get("OLLAMA_URL", "http://172.21.0.1:11434/v1")
MODELS = ["qwen3:8b", "qwen3:30b-a3b"]
HERE = os.path.dirname(os.path.abspath(__file__))
REPORT = os.path.join(HERE, "rag_report.json")

QUESTIONS = [
    # (类型, 问题)  hit=手册应命中; paraphrase=手册无数值须转述; refuse=纯电手册应拒答
    ("hit",        "儿童安全座椅怎么安装？"),
    ("hit",        "胎压警告灯亮了怎么办？"),
    ("hit",        "充电需要多长时间？"),
    ("hit",        "冬天续航里程为什么会下降？"),
    ("hit",        "保养周期是多久？"),
    ("hit",        "雾灯怎么开？"),
    ("paraphrase", "轮胎的标准胎压是多少？"),
    ("hit",        "千斤顶应该支撑在车底什么位置？"),
    ("hit",        "车钥匙电池没电了怎么更换？"),
    ("hit",        "12伏蓄电池亏电了怎么办？"),
    ("hit",        "长途出行前应该检查哪些项目？"),
    ("refuse",     "发动机机油多久换一次？"),
    ("refuse",     "汽油滤芯多久换一次？"),
    ("refuse",     "油箱盖开关在哪里？"),
]
REFUSE_MARKS = ["未找到", "没有找到", "手册中没", "手册中未", "无此内容",
                "没有相关", "无法找到", "不包含", "没有查到", "查不到", "没有提及"]
PAGE_RE = re.compile(r"[pP]\.?\s*\d+|第\s*\d+\s*页")


def load_system_prompt():
    """从前端 index.html 提取 SYSTEM_PROMPT，保证与线上行为一致。"""
    path = os.path.join(HERE, "..", "..", "crates", "liteclaw-web", "src", "static", "index.html")
    html = open(path).read()
    m = re.search(r"const SYSTEM_PROMPT =\s*((?:\s*'[^']*'\s*\+?)+);", html)
    parts = re.findall(r"'((?:[^'\\]|\\.)*)'", m.group(1))
    return "".join(p.replace("\\n", "\n") for p in parts)


def strip_think(text):
    """qwen3/deepseek-r1 会输出 <think>...</think>，评分前剥离。"""
    return re.sub(r"<think>.*?</think>", "", text, flags=re.S).strip()


def run_question(s, token, model, q, system_prompt):
    payload = {
        "messages": [{"role": "system", "content": system_prompt},
                     {"role": "user", "content": q}],
        "model": {"base_url": OLLAMA_URL, "api_key": "", "model": model},
        "auto_mode": True,
    }
    t0 = time.time()
    tools, answer, err, broken = [], [], None, None
    try:
        with s.post(f"{BASE}/api/chat", json=payload, stream=True,
                    timeout=(15, 900), headers={"Authorization": f"Bearer {token}"}) as r:
            if r.status_code != 200:
                return {"error": f"HTTP {r.status_code}: {r.text[:200]}"}
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
                        tools.append({"tool": ev.get("tool"), "id": str(args.get("id", ""))[:40]})
                    elif t == "error":
                        err = str(ev)[:200]
    except requests.RequestException as e:
        broken = str(e)[:120]
    full = strip_think("".join(answer))
    skill_run = [x for x in tools if x["tool"] == "skill_run"]
    return {
        "latency_s": round(time.time() - t0, 1),
        "tool_calls": [f"{x['tool']}({x['id']})" for x in tools],
        "rag_called": any("manual" in x["id"].lower() for x in skill_run),
        "cited": bool(PAGE_RE.search(full)) or "参考来源" in full,
        "answer": full[:1200],
        "error": err,
        "stream_broken": broken,
    }


def score(kind, res):
    if res.get("error") and not res.get("answer"):
        return "ERROR"
    a = res.get("answer", "")
    refused = any(m in a for m in REFUSE_MARKS)
    if kind == "refuse":
        if refused and not res["cited"]:
            return "PASS(拒答)"
        if refused:
            return "PASS(拒答+引用)"
        return "FAIL(编造)"
    if kind == "paraphrase":
        hard_num = bool(re.search(r"\d+(\.\d+)?\s*(bar|巴)", a))
        if refused:
            return "PASS(指出无数值)"
        if hard_num:
            return "FAIL(给硬数值)"
        return "PASS(转述)" if res["cited"] else "WEAK(无引用)"
    return "PASS" if (res["rag_called"] and res["cited"]) else (
        "WEAK(有引用没RAG)" if res["cited"] else "FAIL")


def main():
    system_prompt = load_system_prompt()
    print(f"SYSTEM_PROMPT: {len(system_prompt)} 字符", flush=True)
    if "--smoke" in sys.argv:
        MODELS[:] = [MODELS[0]]
        QUESTIONS[:] = QUESTIONS[:1]
    if "--model" in sys.argv:
        MODELS[:] = [sys.argv[sys.argv.index("--model") + 1]]

    s = requests.Session()
    r = s.post(f"{BASE}/api/login",
               json={"username": os.environ.get("LC_USER", "renault"),
                     "password": os.environ.get("LC_PASS", "renault123")}, timeout=15)
    r.raise_for_status()
    token = r.json()["token"]
    print("登录 OK", flush=True)

    report = {}
    if os.path.exists(REPORT):
        report = json.load(open(REPORT))
    for model in MODELS:
        report[model] = []
        for i, (kind, q) in enumerate(QUESTIONS, 1):
            print(f"[{model}] {i}/{len(QUESTIONS)} {q}", flush=True)
            try:
                res = run_question(s, token, model, q, system_prompt)
            except requests.RequestException as e:
                res = {"error": f"request failed: {e}"}
            res.update({"kind": kind, "question": q, "verdict": score(kind, res)})
            report[model].append(res)
            with open(REPORT, "w") as f:
                json.dump(report, f, ensure_ascii=False, indent=1)
            print(f"  -> {res.get('verdict')}  {res.get('latency_s', '?')}s  tools={res.get('tool_calls')}",
                  flush=True)
    print("ALL DONE ->", REPORT, flush=True)


if __name__ == "__main__":
    main()
