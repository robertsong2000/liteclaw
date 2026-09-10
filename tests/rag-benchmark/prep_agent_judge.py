#!/usr/bin/env python3
"""把各 run 的 (golden, answer) 组装成子代理评审任务分片。

用法：python3 prep_agent_judge.py <run_dir> [每片条数=20]
输出：agent_judge_tasks/<run名>/batch_NN.jsonl
"""
import json, os, sys

HERE = os.path.dirname(os.path.abspath(__file__))
GOLDEN_FILE = os.path.join(HERE, "golden.jsonl")
TASKS = os.path.join(HERE, "agent_judge_tasks")


def main():
    run_dir = sys.argv[1].rstrip("/")
    size = int(sys.argv[2]) if len(sys.argv) > 2 else 20
    goldens = {g["id"]: g for g in
               (json.loads(l) for l in open(GOLDEN_FILE, encoding="utf-8") if l.strip())}
    answers = [json.loads(l) for l in open(os.path.join(run_dir, "answers.jsonl"), encoding="utf-8") if l.strip()]
    name = os.path.basename(run_dir)
    outdir = os.path.join(TASKS, name)
    os.makedirs(outdir, exist_ok=True)
    batch, n = [], 0
    for a in answers:
        g = goldens.get(a["id"])
        if not g or not g.get("golden_answer"):
            continue
        batch.append({"model": a["model"], "id": a["id"], "kind": a["kind"],
                      "question": a["question"], "must_points": g.get("must_points", []),
                      "golden": g["golden_answer"], "answer": a.get("answer", "")})
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
    print(f"{name}: {n} 个分片, 共 {sum(1 for a in answers if a['id'] in goldens)} 条任务 -> {outdir}")


if __name__ == "__main__":
    main()
