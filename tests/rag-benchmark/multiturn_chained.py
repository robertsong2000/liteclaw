#!/usr/bin/env python3
"""多轮链式 RAG 漂移测试: 14 道边界测试题在【同一个对话】里连续提问,
历史(含每轮完整 RAG 检索原文)真实累积, 用于复现和量化"多轮对话后模型
跳过检索、凭常识作答"的退化现象。

对照组:
  1. nothink + 全量历史        —— 复现漂移(第4题起, 输入1.6万字后开始跳检索)
  2. nothink + 瘦身历史        —— 缓解一半(异常 9 -> 6, 但尾部拒答题仍失守)
  3. 深度思考 + 全量历史       —— 抗性最强(漂移推迟到第10题/4.7万字, 引用纪律14/14)

关键发现(2026-09-07):
  - 漂移会自我强化: 一条编造回答进入历史后, 后续轮次被带坏
  - 瘦身将输入量削减约 3 倍, PASS 5/14 -> 8/14, 但对 nothink 不能根治
  - 深度思考在 5 万字极端深度下同样沦陷 => 根治需服务端自动注入检索

用法:
  python3 multiturn_chained.py            # 三组全跑(约15分钟)
  python3 multiturn_chained.py --smoke    # 单组前2题冒烟

结果写入 rag_multiturn_report.json。
"""

import json, re, sys, time
import requests

sys.path.insert(0, "/home/ae/liteclaw/tests/rag-benchmark")
from compare_models import load_system_prompt, score, BASE

MODEL_NOTHINK = "qwen3:30b-a3b-nothink"
MODEL_DEEP = "qwen3:30b-a3b"
RUNS = [
    ("nothink-全量历史", MODEL_NOTHINK, False),
    ("nothink-瘦身历史", MODEL_NOTHINK, True),
    ("深度思考-全量历史", MODEL_DEEP, False),
]
REPORT = "/tmp/rag_multiturn_report.json"
SLIM_KEEP = 8
REFUSE_MARKS = ["未找到","没有找到","手册中没","手册中未","无此内容","没有相关","无法找到","不包含","没有查到","查不到","没有提及"]

QUESTIONS = [
    ("hit","儿童安全座椅怎么安装？"), ("hit","胎压警告灯亮了怎么办？"),
    ("hit","充电需要多长时间？"), ("hit","冬天续航里程为什么会下降？"),
    ("hit","保养周期是多久？"), ("hit","雾灯怎么开？"),
    ("paraphrase","轮胎的标准胎压是多少？"), ("hit","千斤顶应该支撑在车底什么位置？"),
    ("hit","车钥匙电池没电了怎么更换？"), ("hit","12伏蓄电池亏电了怎么办？"),
    ("hit","长途出行前应该检查哪些项目？"), ("refuse","发动机机油多久换一次？"),
    ("refuse","汽油滤芯多久换一次？"), ("refuse","油箱盖开关在哪里？"),
]
PAGE_RE = re.compile(r"[pP]\.?\s*\d+|第\s*\d+\s*页")

def slim(msgs):
    cut = max(0, len(msgs) - SLIM_KEEP)
    return [{**m, "content": "[已检索: manual-rag 结果(略)]"} if (i < cut and m.get("role") == "tool") else m
            for i, m in enumerate(msgs)]

def ask(s, token, model, msgs):
    """发送完整历史, 返回 (检索词列表, tool结果摘要列表, 最终回答, 输入字符量)"""
    payload = {"messages": msgs, "model": {"base_url": "http://172.21.0.1:11434/v1", "api_key": "", "model": model}, "auto_mode": True}
    tools, results, answer, broken = [], [], [], None
    try:
        with s.post(f"{BASE}/api/chat", json=payload, stream=True, timeout=(15, 900),
                    headers={"Authorization": f"Bearer {token}"}) as r:
            if r.status_code != 200:
                return tools, results, f"HTTP{r.status_code}", sum(len(str(m.get('content',''))) for m in msgs)
            buf = b""
            for chunk in r.iter_content(chunk_size=2048):
                buf += chunk
                while b"\n" in buf:
                    raw, buf = buf.split(b"\n", 1)
                    line = raw.decode("utf-8", "replace").strip()
                    if not line.startswith("data: "): continue
                    try: ev = json.loads(line[6:])
                    except: continue
                    t = ev.get("type")
                    if t == "text_delta": answer.append(ev.get("text",""))
                    elif t == "tool_start":
                        a = ev.get("arguments") or {}
                        tools.append(f"{a.get('id','')}|{a.get('args','')}")
                    elif t == "tool_result": results.append(ev.get("summary",""))
    except requests.RequestException as e:
        broken = str(e)[:80]
    text = re.sub(r"<think>.*?</think>", "", "".join(answer), flags=re.S).strip()
    size = sum(len(str(m.get("content","") or "")) for m in msgs)
    return tools, results, text, size

def main():
    sp = load_system_prompt()
    if "--smoke" in sys.argv:
        RUNS[:] = [RUNS[0]]
        QUESTIONS[:] = QUESTIONS[:2]
    s = requests.Session()
    token = s.post(f"{BASE}/api/login", json={"username":"renault","password":"renault123"}, timeout=15).json()["token"]
    print("登录 OK", flush=True)
    report = {}
    for label, model, use_slim in RUNS:
        print(f"\n===== {label} =====", flush=True)
        msgs = [{"role":"system","content":sp}]
        items = []
        cid = 0
        for i, (kind, q) in enumerate(QUESTIONS, 1):
            msgs.append({"role":"user","content":q})   # 先入列再发送
            to_send = slim(msgs) if use_slim else msgs
            size = sum(len(str(m.get("content","") or "")) for m in to_send)
            t0 = time.time()
            tools, results, text, _ = ask(s, token, model, to_send)
            lat = round(time.time()-t0, 1)
            rag_called = any("manual" in t for t in tools) or "manual" in str(tools)
            cited = bool(PAGE_RE.search(text)) or "参考来源" in text
            v = score(kind, {"rag_called": rag_called, "cited": cited, "answer": text})
            generic = any(k in text for k in ["CR2032","螺丝刀","视车型而定，一般"])
            items.append({"q": q, "verdict": v, "rag": rag_called, "cited": cited,
                          "latency": lat, "answer_len": len(text), "input_chars": size,
                          "drift_generic": generic, "answer_head": text[:100]})
            with open(REPORT, "w") as f: json.dump(report, f, ensure_ascii=False, indent=1)
            print(f"  {i}/14 [{v}] {q}  {lat}s 输入{size}字  检索词={tools[0][:20] if tools else '-'}", flush=True)
            # 重建历史: user → assistant(tool_calls) → tool(完整结果) → assistant(最终回答)
            cid += 1
            msgs.append({"role":"user","content":q})
            msgs.append({"role":"assistant","content":"","tool_calls":[{"id":f"c{cid}","type":"function",
                          "function":{"name":"skill_run","arguments":json.dumps({"id":"manual-rag","args":q}, ensure_ascii=False)}}]})
            msgs.append({"role":"tool","tool_call_id":f"c{cid}","content": results[0] if results else "(无结果)"})
            msgs.append({"role":"assistant","content": text})
        report[label] = items
        with open(REPORT, "w") as f: json.dump(report, f, ensure_ascii=False, indent=1)
    print("\nALL DONE", flush=True)

if __name__ == "__main__":
    main()
