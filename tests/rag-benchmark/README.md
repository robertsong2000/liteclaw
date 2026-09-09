# RAG 评测基准（rag-benchmark）

车书助手 manual-rag 流程的自动化评测：**用例集 → 各模型答题 → RAG 标准答案比对 → LLM 评审打分 → 回归对比**。
用于回答"哪个模型能胜任"、以及"改动（换模型/改提示词/重建索引/改技能）之后有没有变差"。

## 评测流水线（4 个阶段）

```
cases.jsonl ──build_golden.py──> golden.jsonl ──────────────┐
     │                                          （标准参考答案）│
     └──compare_models.py──> runs/<时间戳>/answers.jsonl ──judge.py──> judged.jsonl
                                                    │        + summary.md/json
                                              （各模型回答全文）  └─(--vs 上次)→ 回归报告
```

| 文件 | 角色 |
|---|---|
| `cases.jsonl` | 用例集：`hit`（应命中）/ `paraphrase`（手册无数值应转述）/ `refuse`（应拒答）/ `chain`（多轮链式追问），每题带人工编写的必答要点 `must_points` 和 `bad_signals` |
| `build_golden.py` | 阶段1：对每题跑 `manual-rag` 检索 → 用最强本地模型**只依据检索原文**生成标准参考答案（含页码）；拒答题 golden 为固定判据；记录手册文件 sha256 |
| `golden.jsonl` | 标准参考答案。`reviewed:false` 表示尚未人工复核——judge 汇总会统计未复核比例 |
| `compare_models.py` | 阶段2：登录 → 携带前端同款 SYSTEM_PROMPT → `/api/chat` SSE 解析；单轮与链式都在此跑；回答**全文**写入 `runs/<时间戳>/answers.jsonl`（永不覆盖历史） |
| `judge.py` | 阶段3/4：LLM-as-judge 评审 + 跨次回归对比 |

## 快速上手

```bash
# 0. 前提：车书助手容器已启动（http://localhost:9999），ollama 可达
# 1. 生成/更新标准答案（新用例或手册 ingest 变更后跑；约 20~40 分钟）
python3 build_golden.py                 # 全量；--id <case_id> 单题重建；--force 覆盖重建
#    ↑ 生成后人工过目 golden.jsonl（重点核对页码与数值），把确认过的 reviewed 改为 true

# 2. 跑模型答题（默认 3 模型 × 35 题，约 1.5~3 小时）
python3 compare_models.py                          # 全量
python3 compare_models.py --model openbmb/minicpm5:latest --tag minicpm
python3 compare_models.py --kind refuse,paraphrase # 只跑指定类型（快速回归）
python3 compare_models.py --retry runs/<旧目录>    # 只补答断流/空回答的记录（链式断一轮整链重跑）
python3 compare_models.py --smoke                  # 单模型单题冒烟

# 3. LLM 评审打分（未调检索的题直接判 0，不耗评审调用）
python3 judge.py runs/2026-09-08T2315
#    -> runs/<ts>/summary.md（人读榜单）+ summary.json（机读）+ judged.jsonl（逐题）

# 4. 回归对比：改动前后各跑一次，然后
python3 judge.py runs/<新> --vs runs/<旧>          # 有回归则退出码 1（可当门禁）
python3 judge.py runs/<新> --save-baseline         # 把本次设为基线 baseline.json
python3 judge.py runs/<再新> --vs baseline         # 日常：只跟基线比
```

## 评审口径

- **架构自适应**：2026-09-08 起服务端启用 AUTO-RAG（`liteclaw-web/src/handlers.rs`，
  `LITECLAW_AUTO_RAG=0` 可关）——每轮由服务端自动跑 `manual-rag` 注入检索结果，
  SSE 流里不再有 tool 事件，"没看到工具事件"≠"没检索"。
  judge 自动识别：仅当本次运行其他题出现过 `skill_run` 事件（agent 模式）时，
  某题没调检索才判 FAIL 0 分；AUTO-RAG 模式下不以此扣分。
- **回答为空/报错 → ERROR**。
- **LLM 评审**（默认 `qwen3:30b-a3b-nothink`，temperature=0，直连 ollama、无工具可调，
  与被评模型物理隔离）：对照 golden 逐条核对 `must_points` 覆盖
  （covered/partial/missing）+ 编造检测（fabrication 强制 FAIL、分数压到 ≤3）+ 引用检查。
  引用页码与 golden 不同但内容一致不算编造（检索召回相邻页是正常的）。
- 分数锚点：9-10 全覆盖零编造 / 7-8 小遗漏 / 4-6 明显缺失 / 0-3 编造或答非所问。
- 回归判定：verdict 下滑（PASS→WEAK/FAIL）即回归；单题分数掉 ≥2 记警告。
- **换评审模型后分数不可直接跨版本对比**（评审模型记录在 summary.json）。
- golden 是唯一事实依据：评审不以评审模型自己的知识纠错；golden 基于**手册文件 sha256**，
  重新 ingest 手册后必须 `--force` 重建 golden 再评测，否则结论无效。

## 维护约定

- **扩用例**：直接往 `cases.jsonl` 追加（保持四类都有覆盖），然后 `build_golden.py --id <新id>`，
  人工复核 golden 后即可评测。新题务必先确认检索能召回依据（跑一下 `rag.py "<问题>"`）。
- **手册更新**：`rag.py --ingest` 后 → `build_golden.py --force`（golden 里的 sha256 会过期报警）。
- **改造检索/提示词后**：跑一次 `--kind refuse,paraphrase` 快速看纪律题有没有失守，
  再跑全量 + `--vs` 看整体回归。

## 历史结论（旧版 14 题时代，详见 REPORT.md / docs/rag-model-compare.md）

- `qwen3:30b-a3b`：综合最优（21s/题）；偶发跳过检索（1/14）。`-nothink` 版零质量损失、快 2.7 倍。
- `qwen3:8b`：合格的轻量备选（44s/题），纪律稳；偶发"幻景式工具声明"（无害）。
- `openbmb/minicpm5`、`qwen3:1.7b` 等 1~2B 模型：工具调用纪律差，不服从"必须先调检索"，
  评测中 FAIL 偏多属预期——这正是基准要量化的东西。（AUTO-RAG 落地后此短板被服务端兜底，
  1~2B 模型值得在新架构下重测。）
- 多轮链式（同对话连续追问、历史真实累积）：所有配置最终都会漂移，根治需服务端自动注入检索
  （详见 `multiturn_chained.py` 与 `rag_multiturn_report.json`）。
  **2026-09-08 起 AUTO-RAG + 历史压缩已在服务端落地，链式用例（chain-*）测的就是新架构的
  实际效果**；上面的漂移结论对应旧架构，可作为 `LITECLAW_AUTO_RAG=0` 时的参照。
