#!/usr/bin/env python3
"""golden 质检流水线（质检员 = 对话模型，经子代理执行）。

用法：
  python3 verify_golden.py --prep [--all] [--id X] [--topk 12] [--size 6]
      为待质检 golden 组装任务分片（问题 + 标准答案 + top-12 宽检索手册段落）
      -> golden_qc_tasks/<批次名>/batch_NN.jsonl
      默认只取 reviewed=false 的条目；--all 连已复核的一起重检
  （子代理按 golden_qc_tasks/VERIFY_RUBRIC.md 逐条核对，写 out_NN.jsonl）
  python3 verify_golden.py --merge
      把 golden_qc_tasks/*/out_*.jsonl 合并进 golden_review.jsonl（按 id 覆盖），
      打印 pass/warn/fail 分档汇总

注意：质检员与被评模型必须相互独立——旧版直连云端 qwen3.8-flash 质检的路径
已移除（运动员兼裁判）。当前质检员为对话模型（子代理）。
"""
import json, glob, os, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
GOLDEN_FILE = os.path.join(HERE, "golden.jsonl")
REVIEW_FILE = os.path.join(HERE, "golden_review.jsonl")
QCDIR = os.path.join(HERE, "golden_qc_tasks")
TOP_K = 12


def load_goldens():
    return [json.loads(l) for l in open(GOLDEN_FILE, encoding="utf-8") if l.strip()]


def passages_block(hits):
    out = []
    for h in hits:
        page = f"p.{h.get('page_start')}-{h.get('page_end')}" if h.get('page_start') else (h.get('source') or '?')[-12:]
        out.append(f"[{h['rank']}] {h.get('section') or '?'} ({page})\n{h['text']}")
    return out


def cmd_prep(argv):
    all_flag = "--all" in argv
    only = argv[argv.index("--id") + 1] if "--id" in argv else None
    topk = int(argv[argv.index("--topk") + 1]) if "--topk" in argv else TOP_K
    size = int(argv[argv.index("--size") + 1]) if "--size" in argv else 6
    sys.path.insert(0, HERE)
    from build_golden import load_rag, retrieve
    rag = load_rag()

    tag = time.strftime("%Y-%m-%dT%H%M")
    if all_flag:
        tag += "-all"
    outdir = os.path.join(QCDIR, tag)
    os.makedirs(outdir, exist_ok=True)

    targets = [g for g in load_goldens() if all_flag or not g.get("reviewed")]
    if only:
        targets = [g for g in targets if only in g["id"]]
    batch, n, total = [], 0, 0
    for g in targets:
        print(f"[prep] {g['id']} ({g['kind']}) 检索中...", flush=True)
        rec = {"id": g["id"], "kind": g["kind"], "question": g["question"],
               "golden_answer": g["golden_answer"], "golden_pages": g.get("pages", []),
               "passages": passages_block(retrieve(rag, g["question"], topk))}
        batch.append(rec)
        total += 1
        if len(batch) >= size:
            n += 1
            with open(os.path.join(outdir, f"batch_{n:02d}.jsonl"), "w", encoding="utf-8") as f:
                for x in batch:
                    f.write(json.dumps(x, ensure_ascii=False) + "\n")
            batch = []
    if batch:
        n += 1
        with open(os.path.join(outdir, f"batch_{n:02d}.jsonl"), "w", encoding="utf-8") as f:
            for x in batch:
                f.write(json.dumps(x, ensure_ascii=False) + "\n")
    print(f"\n{total} 条待质检 -> {n} 个分片 -> {outdir}")
    print(f"下一步：派子代理按 {QCDIR}/VERIFY_RUBRIC.md 评审各 batch_NN.jsonl，"
          f"输出 out_NN.jsonl 到同目录；然后 python3 verify_golden.py --merge")


def cmd_merge():
    rows = {}
    for p in sorted(glob.glob(os.path.join(QCDIR, "*", "out_*.jsonl"))):
        for l in open(p, encoding="utf-8"):
            if l.strip():
                r = json.loads(l)
                rows[r["id"]] = r
    goldens = {g["id"]: g for g in load_goldens()}
    merged = {}
    if os.path.exists(REVIEW_FILE):
        for l in open(REVIEW_FILE, encoding="utf-8"):
            if l.strip():
                r = json.loads(l)
                merged[r["id"]] = r
    for rid, r in rows.items():
        r.update({"kind": goldens.get(rid, {}).get("kind", r.get("kind")),
                  "question": goldens.get(rid, {}).get("question", r.get("question")),
                  "merged_at": time.strftime("%Y-%m-%d %H:%M")})
        merged[rid] = r
    with open(REVIEW_FILE, "w", encoding="utf-8") as f:
        for x in merged.values():
            f.write(json.dumps(x, ensure_ascii=False) + "\n")

    vals = list(rows.values())
    ok = [r for r in vals if r.get("verdict") == "pass"]
    warn = [r for r in vals if r.get("verdict") == "warn"]
    fail = [r for r in vals if r.get("verdict") == "fail"]
    print(f"本次合并 {len(vals)} 条质检结果（golden_review.jsonl 现共 {len(merged)} 条）")
    print(f"pass {len(ok)} / warn {len(warn)} / fail {len(fail)}")
    for r in warn + fail:
        print(f"  [{r.get('verdict')}] {r['id']}")
        for m in r.get("missing_points", []):
            print(f"     漏点: {m[:100]}")
        for m in r.get("wrong_facts", []):
            print(f"     矛盾: {m[:100]}")
        for m in r.get("page_issues", []):
            print(f"     页码: {m[:100]}")


def main():
    argv = sys.argv[1:]
    if "--prep" in argv:
        cmd_prep(argv)
    elif "--merge" in argv:
        cmd_merge()
    else:
        print(__doc__)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
