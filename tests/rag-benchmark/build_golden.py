#!/usr/bin/env python3
"""阶段 1：从 RAG 检索结果生成"标准参考答案"（golden），供 LLM-as-judge 评审用。

流程（每道 hit/paraphrase 题）：
  1. 用 skills/manual-rag/scripts/rag.py 对题目做混合检索（BM25+向量，与线上同源）；
  2. 把 top-N 段落喂给本地最强模型（默认 qwen3:30b-a3b-nothink，temperature=0），
     要求【只依据检索原文】写出标准答案 + 来源页码（JSON 输出）；
  3. 拒答题不调模型：golden = 固定的"应拒答"判据（用例里已写明原因）；
  4. 全部写入 golden.jsonl，并记录手册文件 sha256——手册重新 ingest 后必须重建。

golden.jsonl 里 reviewed 字段默认 false：生成结果需人工过目后手工改为 true，
judge.py 汇总时会统计未复核比例。改完某题后可 `--id <case_id>` 单独重建。

用法：
  python3 build_golden.py                    # 全量生成（约 20~40 分钟）
  python3 build_golden.py --id hit-fog-lights
  python3 build_golden.py --force            # 覆盖已有条目

结果写入脚本同目录 golden.jsonl。
"""
import json, os, sys, hashlib, time, contextlib, io
import requests

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
CASES_FILE = os.path.join(HERE, "cases.jsonl")
GOLDEN_FILE = os.path.join(HERE, "golden.jsonl")
MANUAL_FILE = os.path.join(REPO, "manual", "renault-5-e-tech-2025.jsonl")
RAG_SCRIPT = os.path.join(REPO, "skills", "manual-rag", "scripts", "rag.py")
MANUAL_DIR = os.environ.get("LITECLAW_MANUAL_DIR", os.path.join(REPO, "manual"))
OLLAMA = os.environ.get("OLLAMA_NATIVE_URL", "http://localhost:11434")
GOLDEN_MODEL = os.environ.get("GOLDEN_MODEL", "qwen3:30b-a3b")
TOP_K = 8
NUM_CTX = 16384

GOLDEN_PROMPT = """你是雷诺 5 E-Tech 2025 车主手册问答的"标准答案"撰写员。
下面是针对问题的检索段落（来自官方手册英文原文，含页码）。请【只依据这些段落】
写一份中文标准参考答案，供后续评审其他模型的回答使用。

要求：
- 只使用段落里出现的事实；数字、警告措辞照抄语义，不得用你自己的知识补充；
- 在答案中自然标注来源页码（如 p.181-182）；
- 若段落与问题完全无关、确实无法作答，found 填 false，answer 写"检索段落不足以回答本题"；
- 特别注意：很多题手册本应【不含直接数值】（如充电时长、胎压标准值、续航数字），
  这不是检索失败！此时标准答案必须写明"手册未给出该数值"，
  并转述段落里给出的规则/正确信息来源（如仪表盘显示、车门标签、保养文档），found 填 true。

问题：{question}

检索段落：
{passages}

输出严格 JSON：
{{"found": true, "answer": "标准参考答案（中文，含页码）", "pages": ["p.x-y"]}}"""


def load_cases():
    return [json.loads(l) for l in open(CASES_FILE, encoding="utf-8") if l.strip()]


def load_rag():
    """加载 rag.py（宿主机路径跑检索，改写 MANUAL_DIR/INDEX_FILE）。"""
    import importlib.util
    os.environ.setdefault("OLLAMA_URL", OLLAMA)
    spec = importlib.util.spec_from_file_location("rag", RAG_SCRIPT)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    mod.MANUAL_DIR = MANUAL_DIR
    mod.INDEX_FILE = os.path.join(MANUAL_DIR, ".index", "index.json")
    return mod


def retrieve(rag, query, top_k=TOP_K):
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        rag.cmd_search(query, top_k)
    return json.loads(buf.getvalue())


def ask_ollama(prompt, model=GOLDEN_MODEL):
    r = requests.post(f"{OLLAMA}/api/chat", json={
        "model": model, "messages": [{"role": "user", "content": prompt}],
        "format": "json", "stream": False, "think": False,
        "options": {"temperature": 0, "num_ctx": NUM_CTX},
    }, timeout=600)
    r.raise_for_status()
    return json.loads(r.json()["message"]["content"])


def passages_block(hits):
    out = []
    for h in hits:
        src = (h.get("source") or "?").split("/")[-1]
        out.append(f"[{h['rank']}] {src} {h.get('source','')[-12:]} | {h.get('section') or '?'}\n{h['text']}")
    return "\n\n".join(out)


def build_one(rag, case):
    kind = case["kind"]
    base = {"id": case["id"], "kind": kind, "question": case.get("question") or case["id"],
            "must_points": case.get("must_points", []),
            "golden_model": GOLDEN_MODEL, "built_at": time.strftime("%Y-%m-%d %H:%M"),
            "reviewed": False}
    if kind == "refuse":
        base.update({"found": True, "pages": [],
                     "golden_answer": (f"手册中未找到相关内容（{case['reason']}）。正确行为：明确告知"
                                       "手册未包含该内容，可建议查询保养文档或联系经销商；不得编造"
                                       "周期、部件或位置，不得引用无关检索段落硬凑答案。")})
        return base
    hits = retrieve(rag, case["question"])
    if not hits:
        base.update({"found": False, "pages": [], "golden_answer": "检索无结果——请先确认索引已 ingest。"})
        return base
    res = ask_ollama(GOLDEN_PROMPT.format(question=case["question"], passages=passages_block(hits)))
    base.update({"found": bool(res.get("found")),
                 "pages": res.get("pages", []),
                 "golden_answer": (res.get("answer") or "").strip(),
                 "retrieved_chunks": [h.get("chunk_id") for h in hits[:4]]})
    return base


def main():
    only = None
    if "--id" in sys.argv:
        only = sys.argv[sys.argv.index("--id") + 1]
    force = "--force" in sys.argv
    cases = load_cases()
    golden = {}
    if os.path.exists(GOLDEN_FILE):
        for l in open(GOLDEN_FILE, encoding="utf-8"):
            if l.strip():
                g = json.loads(l)
                golden[g["id"]] = g
    manual_sha = hashlib.sha256(open(MANUAL_FILE, "rb").read()).hexdigest()[:12]
    rag = load_rag()
    # 先确定本次要重建的条目，其余既有条目原样保留（--id 单题重建不得冲掉全文件）
    plan = []
    for case in cases:
        targets = [{"id": case["id"], "kind": case["kind"], "question": case.get("question"),
                    "must_points": case.get("must_points", [])}] if case["kind"] != "chain" else [
            {"id": f"{case['id']}#{i}", "kind": t["kind"], "question": t["question"],
             "must_points": t.get("must_points", [])}
            for i, t in enumerate(case["turns"], 1)]
        for tgt in targets:
            if only and only not in tgt["id"]:
                continue
            if golden.get(tgt["id"]) and not force:
                continue
            plan.append({**case, **tgt})
    out = {g["id"]: g for g in golden.values() if g["id"] not in {t["id"] for t in plan}}
    n_new = 0
    for tgt in plan:
        print(f"[golden] {tgt['id']} ({tgt['kind']}) {tgt['question']}", flush=True)
        g = build_one(rag, {**tgt})
        g["manual_sha"] = manual_sha
        out[g["id"]] = g
        n_new += 1
        with open(GOLDEN_FILE, "w", encoding="utf-8") as f:
            for x in out.values():
                f.write(json.dumps(x, ensure_ascii=False) + "\n")
        if g["kind"] != "refuse":
            print(f"  found={g['found']} pages={g.get('pages')} {g['golden_answer'][:80]}...", flush=True)
    out = list(out.values())
    n_stale = sum(1 for g in out if g.get("manual_sha") != manual_sha)
    print(f"\nDONE: {len(out)} 条 golden（本次新建 {n_new}），手册版本 {manual_sha}。")
    if n_stale:
        print(f"警告：{n_stale} 条 golden 基于旧版手册，请用 --force 重建后再跑评审。")
    print("请人工复核 golden.jsonl（重点：页码与数值是否与段落一致），复核后把 reviewed 改为 true。")


if __name__ == "__main__":
    main()
