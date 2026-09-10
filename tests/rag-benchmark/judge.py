#!/usr/bin/env python3
"""RAG 模型评测（阶段 3/4）：LLM-as-judge 评审 + 跨次运行回归对比。

评审口径：
  - 硬信号先行（不花模型调用）：回答为空/报错 → ERROR；
  "没调 manual-rag"门禁默认关闭——AUTO-RAG 架构下（2026-09-08 起，服务端每轮自动注入
  检索，LITECLAW_AUTO_RAG=0 可关）SSE 流的 tool 事件是可选的，有无都不能说明是否检索过；
  只有评测 LITECLAW_AUTO_RAG=0 的部署时加 --agent-mode 才启用该门禁；
  - 其余每题把【标准参考答案 golden】与【被评回答】一起交给评审模型
    （默认 qwen3:30b-a3b @ 本地 ollama，temperature=0，无工具可调，
     与被评模型物理隔离，不存在"自己评自己"的通道）；
  - golden 支持 facts-v1 事实清单格式（【核心事实】+【补充事实】，由子代理按
    section_path 穷尽提取，见 manual_tool.py / factsheets/）：覆盖度按必答要点判定，
    补充事实被回答引用视为加分，清单外内容不自动判编造；
  - 评审输出：verdict(PASS/WEAK/FAIL) + score(0-10) + 逐要点覆盖 + 编造检测；
    编造(fabrication=true)强制 FAIL。评审模型与端点记录在 summary/baseline，
    换评审模型（或换端点）后的分数不可直接跨版本对比。

评审模型配置（环境变量，见 ~/judge_env.sh 模板）：
  JUDGE_MODEL=qwen38-local                        # 模型名（本地 ollama tag 或网关注册名）
  JUDGE_OPENAI_BASE=http://host:16019/v1          # 可选：OpenAI 兼容远程端点（不设则走本地 ollama）
  JUDGE_OPENAI_KEY=sk-xxx                         # 远程端点密钥
  JUDGE_CONCURRENCY=4                             # 并发评审路数（远程端点建议 3~4）

回归对比：
  python3 judge.py runs/A                 # 评审 A（已评过则断点续跑）
  python3 judge.py runs/B --vs runs/A     # 评审 B 并与 A 对比，有回归退出码 1
  python3 judge.py runs/B --save-baseline # 把 B 设为基线（baseline.json）
  python3 judge.py runs/C --vs baseline   # 与基线对比

结果写入 run 目录：judged.jsonl（逐题）、summary.md（人读）、summary.json（机读）。
"""
import json, os, re, sys, time
import requests

HERE = os.path.dirname(os.path.abspath(__file__))
GOLDEN_FILE = os.path.join(HERE, "golden.jsonl")
BASELINE_FILE = os.path.join(HERE, "baseline.json")
OLLAMA = os.environ.get("OLLAMA_NATIVE_URL", "http://localhost:11434")
JUDGE_MODEL = os.environ.get("JUDGE_MODEL", "qwen3:30b-a3b")
NUM_CTX = 16384
# OpenAI 兼容评审端点（可选）。设置后评审走 {JUDGE_OPENAI_BASE}/chat/completions，
# 例如部署在另一台机器 new-api 后面的模型：
#   JUDGE_OPENAI_BASE=http://host:16019/v1 JUDGE_OPENAI_KEY=sk-xxx JUDGE_MODEL=qwen38-local
JUDGE_OPENAI_BASE = os.environ.get("JUDGE_OPENAI_BASE", "")
JUDGE_OPENAI_KEY = os.environ.get("JUDGE_OPENAI_KEY", "")
JUDGE_CONCURRENCY = max(1, int(os.environ.get("JUDGE_CONCURRENCY", "1")))  # 并发评审路数
JUDGE_MODEL_WIRE = JUDGE_MODEL          # 发给网关的模型名，必须与网关注册名一致
if JUDGE_OPENAI_BASE:
    from urllib.parse import urlparse
    _p = urlparse(JUDGE_OPENAI_BASE)
    JUDGE_MODEL = f"{JUDGE_MODEL}@{_p.hostname}:{_p.port or 80}"   # 仅用于 summary/baseline 展示溯源
VERDICT_RANK = {"PASS": 2, "WEAK": 1, "FAIL": 0, "ERROR": -1, "NO_GOLDEN": -2}

KIND_RULES = {
    "hit": """本题类型：hit（手册含该内容，应命中作答）。
- 编造判定：被评回答出现与标准答案冲突或标准答案完全没有的具体数值/周期/型号/步骤 → fabrication=true。标准答案里已有的数字被复述不算编造。
- 页码判定：只检查 citation_ok=是否附了页码引用。引用的页码与标准答案不完全相同但内容一致，不算编造（检索召回的相邻页是正常的）；回答内容与标准答案矛盾才算。
- 逐条检查【必答要点】列表，每个要点一条 covered/partial/missing，不得增删要点。""",
    "paraphrase": """本题类型：paraphrase（手册不含直接数值，正确行为是说明无数值并指向正确来源）。
- 编造判定：被评回答给出标准答案之外的具体数值（bar/PSI/公里数/小时数等）作为"标准值" → fabrication=true。注意：复述标准答案里已有的规则数字（如"+0.2~0.3 bar"）是正确转述，不算编造；仪表盘图例中的元件编号被当成数值引用才算可疑。
- 正确行为：明确说手册无数值 + 指向标准答案指出的来源（如车门标签），并转述标准答案中的操作规则。直接拒绝回答（说查不到就完事）不算 PASS，只能算 WEAK。
- 逐条检查【必答要点】列表，每个要点一条 covered/partial/missing，不得增删要点。""",
    "refuse": """本题类型：refuse（手册必然没有该内容，正确行为是明确拒答）。
- 编造判定（从严）：只有当被评回答【对所问内容本身】给出具体的周期/部件/位置/步骤（如"机油每1万公里换一次"、"油箱盖在左后方"），或声称手册中存在该内容 → fabrication=true，必须 FAIL。
- 注意：被评回答在【明确说明手册没有/该车无此部件】之后，附加手册内的替代建议（如"可检查冷却液/制动液/玻璃水"、"对应的是充电口盖"、"周期见保养文档"）是合理的延伸服务，不算编造——即使引用的页码与所问问题间接相关。
- 逐条检查【必答要点】列表，每个要点一条 covered/partial/missing，不得增删要点。""",
}

JUDGE_PROMPT = """你是"雷诺 5 E-Tech 2025 车主手册 RAG 问答"的严格评审。对照【标准参考答案】评审【被评回答】。
只以标准参考答案为事实依据，不要用你自己的知识补充或纠正；两份回答冲突时，以标准答案为准。

背景：被评回答来自一个"必须先调用手册检索、只用检索内容作答并附页码引用"的车书助手。
{kind_rules}

【标准答案格式说明】若标准答案包含【核心事实】与【补充事实】清单：
- 覆盖度仍按【必答要点】判定；
- 被评回答引用了补充清单中的内容视为覆盖全面（在 reason 注明加分），绝不因"标准答案正文没有"而判编造；
- 被评回答出现两份清单都没有的内容：与清单矛盾 → 编造；无法从清单判断真伪 → 不判编造，在 reason 注明"清单外内容"，覆盖分按必答要点正常评。

评分锚点：9-10 必答要点全覆盖、引用规范、零编造；7-8 基本覆盖有小遗漏；4-6 要点明显缺失；
0-3 编造或答非所问。verdict 规则：fabrication=true 或要点大面积缺失 → FAIL；部分覆盖 → WEAK；否则 PASS。

【问题】{question}

【必答要点】{must_points}

【标准参考答案】{golden}

【被评回答】{answer}

输出严格 JSON：
{{"verdict":"PASS|WEAK|FAIL","score":0,"points":[{{"point":"要点","status":"covered|partial|missing"}}],"fabrication":false,"citation_ok":true,"reason":"一句话理由"}}"""


def ask_judge(question, golden, answer, kind, must_points=None):
    prompt = JUDGE_PROMPT.format(kind_rules=KIND_RULES[kind], question=question,
                                 must_points=json.dumps(must_points or [], ensure_ascii=False),
                                 golden=golden, answer=answer)
    if JUDGE_OPENAI_BASE:
        # OpenAI 兼容端点（如另一台机器 new-api 后的模型）。思考型模型会输出
        # <think> 块，解析前剥离；温度 0，不强制 response_format（网关兼容性参差）。
        r = requests.post(f"{JUDGE_OPENAI_BASE.rstrip('/')}/chat/completions",
                          headers={"Authorization": f"Bearer {JUDGE_OPENAI_KEY}"},
                          json={"model": JUDGE_MODEL_WIRE, "temperature": 0,
                                "messages": [{"role": "user", "content": prompt}]},
                          timeout=600)
        r.raise_for_status()
        content = r.json()["choices"][0]["message"]["content"]
    else:
        r = requests.post(f"{OLLAMA}/api/chat", json={
            "model": JUDGE_MODEL, "messages": [{"role": "user", "content": prompt}],
            "format": "json", "stream": False, "think": False,
            "options": {"temperature": 0, "num_ctx": NUM_CTX},
        }, timeout=600)
        r.raise_for_status()
        content = r.json()["message"]["content"]
    text = re.sub(r"<think>.*?</think>", "", content, flags=re.S).strip()
    start, end = text.find("{"), text.rfind("}")
    return json.loads(text[start:end + 1])


def normalize(judged):
    """兜底修正评审输出：编造强制 FAIL、分数钳位、字段缺省。"""
    judged["score"] = max(0, min(10, int(judged.get("score", 0))))
    if judged.get("fabrication"):
        judged["verdict"] = "FAIL"
        judged["score"] = min(judged["score"], 3)
    if judged.get("verdict") not in VERDICT_RANK:
        judged["verdict"] = "FAIL"
    judged.setdefault("points", [])
    judged.setdefault("citation_ok", None)
    judged["reason"] = (judged.get("reason") or "").strip()[:300]
    return judged


def judge_run(run_dir, force=False, agent_gate=None):
    """agent_gate=None 时自动识别：本次运行全部记录都标了 auto_rag=false
    （模型自主调工具模式）才启用"没调检索"门禁；AUTO-RAG 运行（true 或无此字段的
    历史数据）一律不启用。--agent-mode 可强制开启。"""
    answers = [json.loads(l) for l in open(os.path.join(run_dir, "answers.jsonl"), encoding="utf-8") if l.strip()]
    golden = {}
    for l in open(GOLDEN_FILE, encoding="utf-8"):
        if l.strip():
            g = json.loads(l)
            golden[g["id"]] = g
    judged_path = os.path.join(run_dir, "judged.jsonl")
    done = {}
    if os.path.exists(judged_path) and not force:
        for l in open(judged_path, encoding="utf-8"):
            if l.strip():
                j = json.loads(l)
                done[(j["model"], j["id"])] = j

    n_unreviewed = sum(1 for g in golden.values() if not g.get("reviewed"))
    # "没调检索"门禁：AUTO-RAG 下工具事件是可选的（模型可自行调用也可依赖服务端注入），
    # 事件有无不能说明是否检索过。仅当全部记录 auto_rag=false（自主调工具模式）时
    # 自动启用；--agent-mode 可强制开启。
    if agent_gate is None:
        agent_gate = bool(answers) and all(r.get("auto_rag") is False for r in answers)

    def flush(out_map):
        with open(judged_path, "w", encoding="utf-8") as f:
            for x in out_map.values():
                f.write(json.dumps(x, ensure_ascii=False) + "\n")

    def judge_one(rec, g):
        """单条评审，网络抖动重试 3 次；最终失败记 ERROR（可用 --force 重评）。"""
        last = None
        for attempt in range(3):
            try:
                return normalize(ask_judge(rec["question"], g["golden_answer"],
                                           rec.get("answer") or "", rec["kind"],
                                           g.get("must_points")))
            except Exception as e:
                last = e
                time.sleep(5)
        return {"verdict": "ERROR", "score": 0,
                "reason": f"评审调用失败(重试3次): {str(last)[:120]}"}

    out_map, todo = {}, []
    for rec in answers:
        key = (rec["model"], rec["id"])
        j = {"model": rec["model"], "id": rec["id"], "kind": rec["kind"],
             "question": rec["question"], "latency_s": rec.get("latency_s"),
             "rag_called": rec.get("rag_called"), "cited": rec.get("cited")}
        if key in done and not force:
            out_map[key] = done[key]
            continue
        g = golden.get(rec["id"])
        answer = rec.get("answer") or ""
        if rec.get("error") and not answer:
            out_map[key] = {**j, "verdict": "ERROR", "score": 0,
                            "reason": f"运行报错: {rec['error']}"}
        elif not g or not g.get("golden_answer"):
            out_map[key] = {**j, "verdict": "NO_GOLDEN", "score": 0,
                            "reason": "缺少标准答案，请先 build_golden.py"}
        elif agent_gate and not rec.get("rag_called"):
            out_map[key] = {**j, "verdict": "FAIL", "score": 0, "fabrication": None,
                            "reason": "未调用 manual-rag 检索直接作答（agent 模式下跳过检索，硬性失败）"}
        else:
            todo.append((key, rec, j, g))
    flush(out_map)

    n_done = 0
    if todo:
        from concurrent.futures import ThreadPoolExecutor, as_completed
        print(f"评审 {len(todo)} 条（并发 {JUDGE_CONCURRENCY}）...", flush=True)
        with ThreadPoolExecutor(max_workers=JUDGE_CONCURRENCY) as ex:
            futs = {ex.submit(judge_one, rec, g): key for key, rec, j, g in todo}
            for fut in as_completed(futs):
                key = futs[fut]
                base = next(j for k, _, j, _ in todo if k == key)
                out_map[key] = {**base, **fut.result()}
                n_done += 1
                flush(out_map)
                print(f"[judge {n_done}/{len(todo)}] {key[0]} / {key[1]}", flush=True)
    out = [out_map[(r["model"], r["id"])] for r in answers]
    with open(judged_path, "w", encoding="utf-8") as f:
        for x in out:
            f.write(json.dumps(x, ensure_ascii=False) + "\n")
    print(f"评审完成：{len(out)} 条（新评 {n_done}，复用 {len(out)-n_done}）", flush=True)
    return out, n_unreviewed


def summarize(records, n_unreviewed):
    models = {}
    for j in records:
        m = models.setdefault(j["model"], {"n": 0, "score_sum": 0, "latency_sum": 0,
                                           "verdicts": {}, "by_kind": {}, "fails": []})
        m["n"] += 1
        m["score_sum"] += j["score"]
        m["latency_sum"] += j.get("latency_s") or 0
        m["verdicts"][j["verdict"]] = m["verdicts"].get(j["verdict"], 0) + 1
        k = m["by_kind"].setdefault(j["kind"], {"n": 0, "score_sum": 0})
        k["n"] += 1
        k["score_sum"] += j["score"]
        if j["verdict"] in ("FAIL", "ERROR"):
            m["fails"].append({"id": j["id"], "verdict": j["verdict"], "reason": j["reason"][:120]})
    for m in models.values():
        m["avg_score"] = round(m["score_sum"] / m["n"], 1)
        m["avg_latency_s"] = round(m["latency_sum"] / m["n"], 1)
    return {"judge_model": JUDGE_MODEL, "generated_at": time.strftime("%Y-%m-%d %H:%M"),
            "golden_unreviewed": n_unreviewed, "models": models, "records": records}


def write_summary(run_dir, summary):
    with open(os.path.join(run_dir, "summary.json"), "w", encoding="utf-8") as f:
        json.dump(summary, f, ensure_ascii=False, indent=1)
    lines = [f"# 评审摘要 — {os.path.basename(run_dir)}",
             f"- 评审模型：{summary['judge_model']} · 生成于 {summary['generated_at']}"
             f" · golden 未复核 {summary['golden_unreviewed']} 条", "",
             "| 模型 | 平均分 | PASS | WEAK | FAIL | ERROR | 平均耗时 |", "|---|---|---|---|---|---|---|"]
    order = sorted(summary["models"].items(), key=lambda kv: -kv[1]["avg_score"])
    for name, m in order:
        v = m["verdicts"]
        lines.append(f"| {name} | **{m['avg_score']}** | {v.get('PASS',0)} | {v.get('WEAK',0)} |"
                     f" {v.get('FAIL',0)} | {v.get('ERROR',0)} | {m['avg_latency_s']}s |")
    lines.append("")
    for name, m in order:
        lines.append(f"## {name}")
        bk = " · ".join(f"{k} 均分 {v['score_sum']/v['n']:.1f} ({v['n']}题)" for k, v in m["by_kind"].items())
        lines.append(f"- 分题型：{bk}")
        for f_ in m["fails"]:
            lines.append(f"- ❌ `{f_['id']}` {f_['verdict']}：{f_['reason']}")
        lines.append("")
    with open(os.path.join(run_dir, "summary.md"), "w", encoding="utf-8") as f:
        f.write("\n".join(lines))


def compare(new_dir, old_dir):
    def load(d):
        p = os.path.join(d, "judged.jsonl")
        return {(j["model"], j["id"]): j
                for j in (json.loads(l) for l in open(p, encoding="utf-8") if l.strip())}
    old, new = load(old_dir), load(new_dir)
    regs, imps, warns = [], [], []
    for key, j in sorted(new.items()):
        o = old.get(key)
        if not o or o["verdict"] in ("ERROR", "NO_GOLDEN") or j["verdict"] in ("ERROR", "NO_GOLDEN"):
            continue
        delta = j["score"] - o["score"]
        item = (key, f"{o['verdict']}({o['score']}) → {j['verdict']}({j['score']})", j["reason"] or o["reason"])
        if VERDICT_RANK[j["verdict"]] < VERDICT_RANK[o["verdict"]]:
            regs.append(item)
        elif delta <= -2:
            warns.append(item)
        elif VERDICT_RANK[j["verdict"]] > VERDICT_RANK[o["verdict"]] or delta >= 2:
            imps.append(item)
    lines = [f"# 回归对比 — {os.path.basename(new_dir)} vs {os.path.basename(old_dir)}",
             f"生成于 {time.strftime('%Y-%m-%d %H:%M')} · 评审模型 {JUDGE_MODEL}", ""]
    for title, rows, mark in (("❌ 回归（必须处理）", regs, "-"), ("⚠️ 分数明显下滑", warns, "-"),
                              ("✅ 改进", imps, "-")):
        lines.append(f"## {title}（{len(rows)}）")
        lines += [f"{mark} `{m}/{cid}` {chg} — {why}" for (m, cid), chg, why in rows] or ["（无）"]
        lines.append("")
    report = "\n".join(lines)
    with open(os.path.join(new_dir, "regressions.md"), "w", encoding="utf-8") as f:
        f.write(report)
    print(report)
    return len(regs)


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    run_dir = sys.argv[1].rstrip("/")
    force = "--force" in sys.argv
    records, n_unreviewed = judge_run(run_dir, force,
                                      agent_gate=True if "--agent-mode" in sys.argv else None)
    summary = summarize(records, n_unreviewed)
    write_summary(run_dir, summary)
    print(f"摘要 -> {os.path.join(run_dir, 'summary.md')}")
    if n_unreviewed:
        print(f"注意：{n_unreviewed} 条 golden 未人工复核，结论仅供参考。")

    exit_code = 0
    if "--vs" in sys.argv:
        vs = sys.argv[sys.argv.index("--vs") + 1]
        old_dir = json.load(open(BASELINE_FILE))["run_dir"] if vs == "baseline" else vs.rstrip("/")
        n_regs = compare(run_dir, old_dir)
        exit_code = 1 if n_regs else 0
    if "--save-baseline" in sys.argv:
        with open(BASELINE_FILE, "w") as f:
            json.dump({"run_dir": os.path.abspath(run_dir),
                       "saved_at": time.strftime("%Y-%m-%d %H:%M"),
                       "judge_model": JUDGE_MODEL}, f, ensure_ascii=False, indent=1)
        print(f"基线已保存 -> {BASELINE_FILE}")
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
