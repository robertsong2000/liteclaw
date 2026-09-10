#!/usr/bin/env python3
"""合并子代理评审输出 -> judged.jsonl，并重生成 summary。

用法：python3 merge_agent_judge.py <run_dir>
读取 agent_judge_tasks/<run名>/out_*.jsonl，写 judged.jsonl（judge 标记为 zcode-agent），
并用 judge.py 的 summarize/write_summary 重出 summary.md/json。
"""
import json, glob, os, sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import judge

JUDGE_LABEL = "zcode-agent(对话模型)"


def main():
    run_dir = sys.argv[1].rstrip("/")
    name = os.path.basename(run_dir)
    out = {}
    for p in sorted(glob.glob(os.path.join(HERE, "agent_judge_tasks", name, "out_*.jsonl"))):
        for l in open(p, encoding="utf-8"):
            if l.strip():
                j = json.loads(l)
                out[(j["model"], j["id"])] = j
    answers = [json.loads(l) for l in open(os.path.join(run_dir, "answers.jsonl"), encoding="utf-8") if l.strip()]
    records = []
    for a in answers:
        key = (a["model"], a["id"])
        if key not in out:
            print(f"警告: 缺少评审 {key}")
            continue
        j = out[key]
        j.update({"model": a["model"], "id": a["id"], "kind": a["kind"],
                  "question": a["question"], "latency_s": a.get("latency_s"),
                  "rag_called": a.get("rag_called"), "cited": a.get("cited")})
        records.append(judge.normalize(j))
    with open(os.path.join(run_dir, "judged.jsonl"), "w", encoding="utf-8") as f:
        for x in records:
            f.write(json.dumps(x, ensure_ascii=False) + "\n")
    judge.JUDGE_MODEL = JUDGE_LABEL
    summary = judge.summarize(records, n_unreviewed=0)
    judge.write_summary(run_dir, summary)
    print(f"{name}: {len(records)} 条评审合并完成, judge={JUDGE_LABEL}")
    for m, d in sorted(summary["models"].items(), key=lambda kv: -kv[1]["avg_score"]):
        print(f"  {m}: 均分 {d['avg_score']} "
              f"(PASS {d['verdicts'].get('PASS',0)}/WEAK {d['verdicts'].get('WEAK',0)}/FAIL {d['verdicts'].get('FAIL',0)})")


if __name__ == "__main__":
    main()
