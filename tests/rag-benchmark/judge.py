#!/usr/bin/env python3
"""RAG 模型评测（阶段 3/4）：LLM-as-judge 评审 + 跨次运行回归对比。

评审口径：
  - 硬信号先行（不花模型调用）：回答为空/报错 → ERROR；
  "没调 manual-rag"门禁默认关闭——AUTO-RAG 架构下（2026-09-08 起，服务端每轮自动注入
  检索，LITECLAW_AUTO_RAG=0 可关）SSE 流的 tool 事件是可选的，有无都不能说明是否检索过；
  只有评测 LITECLAW_AUTO_RAG=0 的部署时加 --agent-mode 才启用该门禁；
  - 其余每题把【标准参考答案 golden】与【被评回答】一起交给评审模型
    （默认 qwen3:30b-a3b-nothink，temperature=0，直连 ollama，无工具可调，
     与被评模型物理隔离，不存在"自己评自己"的通道）；
  - 评审输出：verdict(PASS/WEAK/FAIL) + score(0-10) + 逐要点覆盖 + 编造检测；
    编造(fabrication=true)强制 FAIL。评审模型与版本记入 summary，换评审模型
    后的分数不可直接跨版本对比。

回归对比：
  python3 judge.py runs/A                 # 评审 A（已评过则断点续跑）
  python3 judge.py runs/B --vs runs/A     # 评审 B 并与 A 对比，有回归退出码 1
  python3 judge.py runs/B --save-baseline # 把 B 设为基线（baseline.json）
  python3 judge.py runs/C --vs baseline   # 与基线对比

结果写入 run 目录：judged.jsonl（逐题）、summary.md（人读）、summary.json（机读）。
"""
import json, os, sys, time
import requests

HERE = os.path.dirname(os.path.abspath(__file__))
GOLDEN_FILE = os.path.join(HERE, "golden.jsonl")
BASELINE_FILE = os.path.join(HERE, "baseline.json")
OLLAMA = os.environ.get("OLLAMA_NATIVE_URL", "http://localhost:11434")
JUDGE_MODEL = os.environ.get("JUDGE_MODEL", "qwen3:30b-a3b")
NUM_CTX = 16384
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
- 编造判定：被评回答给出任何具体的周期/部件/位置/步骤，或把无关内容（胎压、清洁等）硬凑成答案 → fabrication=true，必须 FAIL。
- 正确行为：明确说"手册中未找到"，可建议查保养文档或联系经销商。附页码引用本身不算错，但引用内容与问题无关则算硬凑。
- 逐条检查【必答要点】列表，每个要点一条 covered/partial/missing，不得增删要点。""",
}

JUDGE_PROMPT = """你是"雷诺 5 E-Tech 2025 车主手册 RAG 问答"的严格评审。对照【标准参考答案】评审【被评回答】。
只以标准参考答案为事实依据，不要用你自己的知识补充或纠正；两份回答冲突时，以标准答案为准。

背景：被评回答来自一个"必须先调用手册检索、只用检索内容作答并附页码引用"的车书助手。
{kind_rules}

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
    r = requests.post(f"{OLLAMA}/api/chat", json={
        "model": JUDGE_MODEL, "messages": [{"role": "user", "content": prompt}],
        "format": "json", "stream": False, "think": False,
        "options": {"temperature": 0, "num_ctx": NUM_CTX},
    }, timeout=600)
    r.raise_for_status()
    return json.loads(r.json()["message"]["content"])


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


def judge_run(run_dir, force=False, agent_gate=False):
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
    # "没调检索"门禁默认关闭：AUTO-RAG 架构下工具事件是可选的（模型可自行调用也可依赖
    # 服务端注入），事件有无不能说明是否检索过。只有 --agent-mode 显式开启
    # （对应 LITECLAW_AUTO_RAG=0、模型必须自己调工具的部署）才启用该门禁。
    out, n_new = [], 0
    for rec in answers:
        key = (rec["model"], rec["id"])
        if key in done:
            out.append(done[key])
            continue
        j = {"model": rec["model"], "id": rec["id"], "kind": rec["kind"],
             "question": rec["question"], "latency_s": rec.get("latency_s"),
             "rag_called": rec.get("rag_called"), "cited": rec.get("cited")}
        g = golden.get(rec["id"])
        answer = rec.get("answer") or ""
        if rec.get("error") and not answer:
            j.update({"verdict": "ERROR", "score": 0, "reason": f"运行报错: {rec['error']}"})
        elif not g or not g.get("golden_answer"):
            j.update({"verdict": "NO_GOLDEN", "score": 0, "reason": "缺少标准答案，请先 build_golden.py"})
        elif agent_gate and not rec.get("rag_called"):
            j.update({"verdict": "FAIL", "score": 0, "fabrication": None,
                      "reason": "未调用 manual-rag 检索直接作答（agent 模式下跳过检索，硬性失败）"})
        else:
            print(f"[judge] {rec['model']} / {rec['id']}", flush=True)
            res = ask_judge(rec["question"], g["golden_answer"], answer, rec["kind"],
                            g.get("must_points"))
            j.update(normalize(res))
        out.append(j)
        n_new += 1
        with open(judged_path, "w", encoding="utf-8") as f:
            for x in out:
                f.write(json.dumps(x, ensure_ascii=False) + "\n")
    print(f"评审完成：{len(out)} 条（新评 {n_new}，复用 {len(out)-n_new}）", flush=True)
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
    records, n_unreviewed = judge_run(run_dir, force, agent_gate="--agent-mode" in sys.argv)
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
