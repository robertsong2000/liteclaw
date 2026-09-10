#!/usr/bin/env python3
"""golden.jsonl 自动质检：用云端模型逐条核对未复核的 golden 与手册原文。

背景：hit-jack 事故证明 golden 生成时的检索可能漏段，导致"答对的模型被冤枉"。
本脚本对每条 reviewed=false 的 golden：
  1. 用更宽的 top_k=12 重新检索手册（比 build_golden 的 top_k=8 覆盖更广）；
  2. 让质检模型（云端 qwen3.8-flash，与被评模型同源但只做文本比对）对照
     【问题】【标准答案】【检索段落】逐项核对：事实有无依据、页码是否一致、
     段落里有而 golden 漏掉的重要信息（漏点会冤枉答对的模型）；
  3. 拒答题反向核验：确认检索确实找不到能回答问题的段落。

结果写入 golden_review.jsonl（逐条），控制台输出分档汇总。
verdict: pass=自动核验通过 / warn=有遗漏或页码小问题 / fail=明确矛盾。
warn 与 fail 建议人工过目后修 golden 并把 reviewed 改 true。

用法：python3 verify_golden.py            # 全部未复核条目
      python3 verify_golden.py --id xxx   # 单条
"""
import json, os, sys, time
import requests

HERE = os.path.dirname(os.path.abspath(__file__))
GOLDEN_FILE = os.path.join(HERE, "golden.jsonl")
REVIEW_FILE = os.path.join(HERE, "golden_review.jsonl")
LITECLAW_CFG = os.path.expanduser("~/.liteclaw/config.json")
VERIFY_MODEL = os.environ.get("VERIFY_MODEL", "qwen3.8-flash")
TOP_K = 12

PROMPT = """你是车主手册问答"标准答案"的质检员。对照【检索段落】核对【标准答案】。

逐项核对：
1. 事实核对：标准答案中的每个事实/数值/步骤，段落里是否有依据？
   注意：段落可能不全（检索缺口）。只有当段落内容与标准答案【明确矛盾】时才标为错误；
   段落里找不到依据但也不矛盾的，标注为"待人工"即可，不要武断判编造。
2. 页码核对：标准答案标注的页码是否与段落来源页码一致？
3. 完整性核对（最重要）：段落中与问题直接相关、但标准答案【遗漏】的重要信息
   （数值、警告、步骤、例外条件）。漏点会导致答对的模型被冤枉扣分。

【问题】{question}

【标准答案】{golden}

【检索段落】
{passages}

输出严格 JSON：
{{"verdict":"pass|warn|fail","wrong_facts":["与段落明确矛盾的内容"],"missing_points":["段落有但golden漏掉的重要信息"],"page_issues":["页码问题"],"notes":"一句话总结"}}"""

REFUSE_PROMPT = """你是车主手册问答"标准答案"的质检员。本题是拒答题：手册（纯电车）应当没有
能回答下面问题的内容，标准答案的判据是"应明确拒答、不得编造"。
请检查检索段落：【是否存在任何段落能直接回答这个问题？】
若某段落看似相关但实际答不到点子上（如只是提到别的内容），不算能回答。

【问题】{question}

【检索段落】
{passages}

输出严格 JSON：
{{"verdict":"pass|warn|fail","wrong_facts":[],"missing_points":[],"page_issues":[],"notes":"说明是否有段落能直接回答"}}"""


def cloud_chat(prompt):
    """质检端点解析优先级：JUDGE_OPENAI_BASE/KEY 环境变量（同 judge.py，
    模板见 judge_env.example.sh）> ~/.liteclaw/config.json 的 model_endpoints。"""
    base = os.environ.get("JUDGE_OPENAI_BASE")
    key = os.environ.get("JUDGE_OPENAI_KEY")
    model = os.environ.get("VERIFY_MODEL") or os.environ.get("JUDGE_MODEL") or VERIFY_MODEL
    if not (base and key):
        ep = json.load(open(LITECLAW_CFG)).get("model_endpoints", {}).get(model)
        if not ep:
            raise SystemExit(f"未找到质检端点：请 source judge_env（设 JUDGE_OPENAI_BASE/"
                             f"JUDGE_OPENAI_KEY），或在 {LITECLAW_CFG} 的 model_endpoints "
                             f"里配置 \"{model}\"")
        base, key = ep["base_url"], ep["api_key"]
    r = requests.post(f"{base.rstrip('/')}/chat/completions",
                      headers={"Authorization": f"Bearer {key}"},
                      json={"model": model, "temperature": 0,
                            "messages": [{"role": "user", "content": prompt}]},
                      timeout=180)
    r.raise_for_status()
    return r.json()["choices"][0]["message"]["content"]


def parse_json(text):
    text = text.strip()
    if text.startswith("```"):
        text = text.split("```")[1].removeprefix("json").strip()
    start = text.find("{")
    end = text.rfind("}")
    return json.loads(text[start:end + 1])


def passages_block(hits):
    out = []
    for h in hits:
        page = (h.get("source") or "?").split(".pdf")[-1]
        out.append(f"[{h['rank']}] {h.get('section') or '?'} ({page})\n{h['text']}")
    return "\n\n".join(out)


def main():
    only = None
    if "--id" in sys.argv:
        only = sys.argv[sys.argv.index("--id") + 1]
    sys.path.insert(0, HERE)
    from build_golden import load_rag
    rag = load_rag()

    goldens = [json.loads(l) for l in open(GOLDEN_FILE, encoding="utf-8") if l.strip()]
    prev = {}
    if os.path.exists(REVIEW_FILE):
        for l in open(REVIEW_FILE, encoding="utf-8"):
            if l.strip():
                r = json.loads(l)
                prev[r["id"]] = r

    results = []
    for g in goldens:
        if g.get("reviewed"):
            continue
        if only and only not in g["id"]:
            continue
        if g["id"] in prev:   # 断点续跑：已质检过的跳过
            results.append(prev[g["id"]])
            continue
        from build_golden import retrieve as _r
        hits = _r(rag, g["question"], TOP_K)
        if g["kind"] == "refuse":
            res = parse_json(cloud_chat(REFUSE_PROMPT.format(
                question=g["question"], passages=passages_block(hits))))
        else:
            res = parse_json(cloud_chat(PROMPT.format(
                question=g["question"], golden=g["golden_answer"],
                passages=passages_block(hits))))
        res.update({"id": g["id"], "kind": g["kind"], "question": g["question"],
                    "checked_at": time.strftime("%Y-%m-%d %H:%M"),
                    "verifier": VERIFY_MODEL})
        results.append(res)
        with open(REVIEW_FILE, "w", encoding="utf-8") as f:
            for x in results:
                f.write(json.dumps(x, ensure_ascii=False) + "\n")
        print(f"[{res.get('verdict','?').upper():4s}] {g['id']}  "
              f"漏:{len(res.get('missing_points',[]))} 错:{len(res.get('wrong_facts',[]))}  "
              f"{res.get('notes','')[:70]}", flush=True)

    ok = [r for r in results if r.get("verdict") == "pass"]
    warn = [r for r in results if r.get("verdict") == "warn"]
    fail = [r for r in results if r.get("verdict") == "fail"]
    print(f"\n===== 质检汇总：{len(results)} 条 —— pass {len(ok)} / warn {len(warn)} / fail {len(fail)} =====")
    for title, rows in (("⚠️ 建议人工修正（有遗漏/矛盾）", warn + fail),):
        print(title)
        for r in rows:
            print(f"  [{r.get('verdict')}] {r['id']} ({r['kind']})")
            for m in r.get("missing_points", []):
                print(f"     漏点: {m[:100]}")
            for m in r.get("wrong_facts", []):
                print(f"     矛盾: {m[:100]}")
            for m in r.get("page_issues", []):
                print(f"     页码: {m[:100]}")
    print(f"\n明细 -> {REVIEW_FILE}")


if __name__ == "__main__":
    main()
