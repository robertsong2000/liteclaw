#!/usr/bin/env python3
"""手册 JSONL 挖掘工具（供事实提取子代理使用）。

用法：
  python3 manual_tool.py sections                 # 列出全部 section_path 及 chunk 数
  python3 manual_tool.py grep 关键词 [上限=8]      # 全文搜 chunk（id/章节/页码/全文）
  python3 manual_tool.py show "driving > Gear control"   # 打印某章节全部 chunk
"""
import json, os, sys

MANUAL = os.environ.get("MANUAL_FILE",
                        os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                     "..", "..", "manual", "renault-5-e-tech-2025.jsonl"))


def load():
    for line in open(MANUAL, encoding="utf-8"):
        if line.strip():
            yield json.loads(line)


def content(r):
    return r.get("content") or r.get("text") or ""


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "sections"
    if cmd == "sections":
        import collections
        secs = collections.Counter()
        for r in load():
            secs[r.get("section_path") or "?"] += 1
        for s, c in sorted(secs.items()):
            print(f"{c:4d}  {s}")
    elif cmd == "grep":
        kw = sys.argv[2]
        limit = int(sys.argv[3]) if len(sys.argv) > 3 else 8
        n = 0
        for r in load():
            t = content(r)
            if kw.lower() in t.lower() or kw.lower() in (r.get("section_path") or "").lower():
                n += 1
                print(f"=== {r.get('chunk_id')} | {r.get('section_path')} | "
                      f"p.{r.get('page_start')}-{r.get('page_end')} | safety={r.get('safety_level')}")
                print(t[:1200])
                print()
                if n >= limit:
                    print(f"(已达上限 {limit}，还有更多可用更高 limit 重试)")
                    break
        if n == 0:
            print("(无匹配——试试英文关键词或换拼法)")
    elif cmd == "show":
        sec = sys.argv[2]
        for r in load():
            if (r.get("section_path") or "") == sec:
                print(f"=== {r.get('chunk_id')} | p.{r.get('page_start')}-{r.get('page_end')} | "
                      f"safety={r.get('safety_level')}")
                print(content(r))
                print()
    else:
        print(__doc__)


if __name__ == "__main__":
    main()
