#!/usr/bin/env python3
"""按 golden_review.jsonl 的质检结果修订 golden.jsonl：

  - fail/warn 条目：把质检发现的【事实矛盾】改正、【漏点】全部融合进标准答案
    （由云端模型重写，保留原页码与漏点页码，不新增段落之外的内容）；
  - pass 条目：自动核验通过，直接标 reviewed=true（含 5 条拒答题）。

全部完成后 golden.jsonl 35 条均为 reviewed=true，可重新评审历史 run。
用法：python3 apply_golden_fixes.py [--dry]     # --dry 只看第一条的修订效果
"""
import json, os, sys, time
import requests

HERE = os.path.dirname(os.path.abspath(__file__))
GOLDEN_FILE = os.path.join(HERE, "golden.jsonl")
REVIEW_FILE = os.path.join(HERE, "golden_review.jsonl")
LITECLAW_CFG = os.path.expanduser("~/.liteclaw/config.json")
FIX_MODEL = os.environ.get("VERIFY_MODEL", "qwen3.8-flash")

REWRITE_PROMPT = """你是车主手册问答"标准答案"的修订员。下面的标准答案经质检发现问题，请修订。

【问题】{question}

【原标准答案】
{golden}

【质检发现的事实矛盾（必须按矛盾注中的手册语义改正）】
{wrong}

【质检发现的漏点（全部融合进修订后的答案，保留各漏点自带的页码）】
{missing}

【页码问题（按备注修正）】
{pages}

要求：
- 中文，结构清晰可分点；保留原答案中正确的内容；融合全部漏点；
- 每条信息标注来源页码；只使用原答案与漏点注中的信息，不要自行新增；
- 若矛盾注指出原答案与手册相反，以矛盾注为准改正。

直接输出修订后的标准答案全文，不要 JSON、不要解释。"""


def cloud_chat(prompt):
    cfg = json.load(open(LITECLAW_CFG))["model_endpoints"][FIX_MODEL]
    r = requests.post(f"{cfg['base_url'].rstrip('/')}/chat/completions",
                      headers={"Authorization": f"Bearer {cfg['api_key']}"},
                      json={"model": FIX_MODEL, "temperature": 0,
                            "messages": [{"role": "user", "content": prompt}]},
                      timeout=240)
    r.raise_for_status()
    return r.json()["choices"][0]["message"]["content"]


def bullets(items):
    return "\n".join(f"- {x}" for x in items) or "（无）"


def main():
    dry = "--dry" in sys.argv
    goldens = [json.loads(l) for l in open(GOLDEN_FILE, encoding="utf-8") if l.strip()]
    reviews = {r["id"]: r for r in
               (json.loads(l) for l in open(REVIEW_FILE, encoding="utf-8") if l.strip())}

    n_fix = n_pass = 0
    for g in goldens:
        rev = reviews.get(g["id"])
        if g.get("reviewed") or "修订" in g.get("golden_model", "") or not rev:
            continue   # 已复核 / 已修订 / 无质检记录（如 hit-jack 手工条）跳过
        if rev["verdict"] == "pass":
            g["reviewed"] = True
            n_pass += 1
            continue
        if dry and n_fix >= 1:
            continue
        new_answer = cloud_chat(REWRITE_PROMPT.format(
            question=g["question"], golden=g["golden_answer"],
            wrong=bullets(rev.get("wrong_facts", [])),
            missing=bullets(rev.get("missing_points", [])),
            pages=bullets(rev.get("page_issues", []))))
        g["golden_answer"] = new_answer.strip()
        g["golden_model"] = f"{FIX_MODEL} 修订(依 golden_review 漏点/矛盾)"
        g["reviewed"] = True
        n_fix += 1
        print(f"[修订] {g['id']}  原 {len(reviews[g['id']].get('missing_points', []))} 漏 "
              f"{len(reviews[g['id']].get('wrong_facts', []))} 错 -> 新答案 {len(g['golden_answer'])} 字", flush=True)
        if dry:
            print(new_answer[:600])
            break
        with open(GOLDEN_FILE, "w", encoding="utf-8") as f:
            for x in goldens:
                f.write(json.dumps(x, ensure_ascii=False) + "\n")

    with open(GOLDEN_FILE, "w", encoding="utf-8") as f:
        for x in goldens:
            f.write(json.dumps(x, ensure_ascii=False) + "\n")
    n_rev = sum(1 for x in goldens if x.get("reviewed"))
    print(f"\n完成：修订 {n_fix} 条，pass 标记 {n_pass} 条，golden 已复核 {n_rev}/{len(goldens)}")


if __name__ == "__main__":
    main()
