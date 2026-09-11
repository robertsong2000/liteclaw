#!/usr/bin/env python3
"""golden v2 生成流水线（生成员 = 对话模型，经子代理执行）。

与 v1（本地 30b + top-8 检索）的区别：
  - 生成员换成对话模型（最强可用模型）；
  - 取材范围扩大：问题 top-20 宽检索 + v1 golden 已知页码的段落合并，
    最大限度避免"相关章节没进检索结果"的漏段事故（hit-jack/12v 两次教训）；
  - 答案格式强制分【核心事实】/【补充事实】两档，评审时核心算覆盖、补充算加分，
    修复"golden 不全冤枉好模型"的系统性偏差。

用法：
  python3 build_golden_v2.py --prep [--id X] [--topk 20]
      组装生成任务分片 -> golden_gen_tasks/<批次>/batch_NN.jsonl
  （子代理按 golden_gen_tasks/GEN_RUBRIC.md 撰写，写 out_NN.jsonl）
  python3 build_golden_v2.py --merge [--apply]
      合并 out_*.jsonl 校验后写入 golden.jsonl（--apply 才落盘，默认预览）
"""
import json, glob, os, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
GOLDEN_FILE = os.path.join(HERE, "golden.jsonl")
V1_BACKUP = os.path.join(HERE, "golden_v1_backup.jsonl")
GEN_DIR = os.path.join(HERE, "golden_gen_tasks")
TOP_K = 20


def load_goldens():
    src = V1_BACKUP if os.path.exists(V1_BACKUP) else GOLDEN_FILE
    return [json.loads(l) for l in open(src, encoding="utf-8") if l.strip()]


def passages_for(rag, g, topk):
    """问题宽检索 + v1 golden 已知页码的段落，按 chunk 去重合并，取材最全。"""
    import io, contextlib, re
    from build_golden import retrieve as _r
    hits = _r(rag, g["question"], topk)
    recs = {h.get("chunk_id"): h for h in hits}
    # 把 v1 golden 引用页码的段落也拉进来（这些页曾证明与题目相关）
    idx = os.path.join(os.environ.get("LITECLAW_MANUAL_DIR",
                        os.path.abspath(os.path.join(HERE, "..", "..", "manual"))), ".index", "index.json")
    try:
        records = json.load(open(idx, encoding="utf-8"))
        want = set()
        for p in g.get("pages", []):
            want.update(int(x) for x in re.findall(r"\d+", p))
        for r in records:
            ps, pe = r.get("page_start"), r.get("page_end")
            if ps and pe and any(ps <= p <= pe for p in want) and r.get("chunk_id") not in recs:
                recs[r["chunk_id"]] = {**r, "rank": 999}
    except FileNotFoundError:
        pass
    out = []
    for h in sorted(recs.values(), key=lambda x: x.get("rank", 999)):
        page = f"p.{h.get('page_start')}-{h.get('page_end')}" if h.get("page_start") else "?"
        out.append(f"[{h.get('rank','?')}] {h.get('section') or '?'} ({page})\n{h['text']}")
    return out


def cmd_prep(argv):
    only = argv[argv.index("--id") + 1] if "--id" in argv else None
    topk = int(argv[argv.index("--topk") + 1]) if "--topk" in argv else TOP_K
    size = int(argv[argv.index("--size") + 1]) if "--size" in argv else 5
    sys.path.insert(0, HERE)
    from build_golden import load_rag
    rag = load_rag()

    outdir = os.path.join(GEN_DIR, time.strftime("%Y-%m-%dT%H%M"))
    os.makedirs(outdir, exist_ok=True)
    batch, n, total = [], 0, 0
    for g in load_goldens():
        if only and only not in g["id"]:
            continue
        print(f"[prep] {g['id']} ({g['kind']}) 取材中...", flush=True)
        rec = {"id": g["id"], "kind": g["kind"], "question": g["question"],
               "must_points": g.get("must_points", []),
               "passages": passages_for(rag, g, topk)}
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
    print(f"\n{total} 条生成任务 -> {n} 个分片 -> {outdir}")
    print("下一步：派子代理按 GEN_RUBRIC.md 撰写，输出 out_NN.jsonl；然后 --merge --apply")


def cmd_merge(apply=False):
    goldens = {g["id"]: g for g in load_goldens()}
    gen = {}
    for p in sorted(glob.glob(os.path.join(GEN_DIR, "*", "out_*.jsonl"))):
        for l in open(p, encoding="utf-8"):
            if l.strip():
                j = json.loads(l)
                gen[j["id"]] = j
    n_ok = n_fail = 0
    updated = []
    for gid, g in gen.items():
        base = goldens.get(gid)
        if not base:
            continue
        found = bool(g.get("found"))
        entry = {**base,
                 "golden_answer": (g.get("golden_answer") or "").strip(),
                 "pages": g.get("pages", base.get("pages", [])),
                 "found": found,
                 "golden_model": "zcode-agent(对话模型, golden v2)",
                 "built_at": time.strftime("%Y-%m-%d %H:%M"),
                 "reviewed": found and bool((g.get("golden_answer") or "").strip())}
        updated.append(entry)
        if found:
            n_ok += 1
        else:
            n_fail += 1
            print(f"  [未找到] {gid}: {entry['golden_answer'][:80]}")
    print(f"生成 {len(updated)} 条：found {n_ok} / 不足 {n_fail}")
    if apply:
        with open(GOLDEN_FILE, "w", encoding="utf-8") as f:
            for x in updated:
                f.write(json.dumps(x, ensure_ascii=False) + "\n")
        print(f"已写入 {GOLDEN_FILE}（v1 备份在 golden_v1_backup.jsonl）")
    else:
        print("预览模式：加 --apply 落盘")


def main():
    argv = sys.argv[1:]
    if "--prep" in argv:
        cmd_prep(argv)
    elif "--merge" in argv:
        cmd_merge(apply="--apply" in argv)
    else:
        print(__doc__)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
