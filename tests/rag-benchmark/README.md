# RAG 模型基准测试（rag-benchmark）

用车书助手边界测试的 14 道题（6 基础 + 1 转述 + 4 命中 + 3 拒答）对不同本地模型做
自动化对比，验证"哪个模型能胜任 manual-rag Agent 流程"。

## 目录内容

| 文件 | 说明 |
|---|---|
| `compare_models.py` | 测试脚本：登录 → 携带与前端一致的 SYSTEM_PROMPT → `/api/chat` → SSE 解析 → 自动评分 |
| `rag_report.json` | 最近一次跑分原始数据（逐题回答摘录、工具调用、耗时、判定） |
| `REPORT.md` | 结论报告（qwen3:8b vs qwen3:30b-a3b，含人工复核说明） |

## 运行方式

```bash
# 前提：车书助手容器已启动（默认 http://localhost:9999），ollama 可从容器内访问
python3 compare_models.py                     # 默认两个模型全部 14 题（约 30~45 分钟）
python3 compare_models.py --model qwen3:14b   # 测单个模型
python3 compare_models.py --smoke             # 单模型单题冒烟
```

环境变量：`LITECLAW_URL`（默认 `http://localhost:9999`）、`OLLAMA_URL`（默认
`http://172.21.0.1:11434/v1`，即容器内访问宿主机 ollama）、`LC_USER`/`LC_PASS`（登录凭证）。

结果实时写入 `rag_report.json`，跑完可直接 diff 看变化。

## 判定口径（经 2026-09-05 实测校准）

- `rag_called`：出现 `skill_run(manual-rag)` 工具事件才算数——**声称调用但无事件 = 幻景**；
- `refuse` 题以"明确说手册未找到"为通过，编造周期/部件为失败；
- `paraphrase` 题（胎压）以"指向车门标签 + 不给具体 bar 数"为通过；
- 自动评分对长答案有误报可能（例如把转述规则里的 "0.2–0.3 bar" 误判为给硬数值），
  发布结论前需人工复核 `rag_report.json` 中的 `answer` 原文，详见 `REPORT.md` 的案例。

## 已测结论速览（详见 REPORT.md）

- `qwen3:30b-a3b`：综合最优，平均 21s/题；偶发跳过检索凭记忆作答（1/14）。
- `qwen3:8b`：合格的轻量备选，44s/题，纪律执行稳定；偶发"幻景式工具声明"（1/14，无害）。
- `openbmb/minicpm5`、`qwen3:1.7b` 等 1~2B 模型：无法稳定完成工具调用，不适合本场景。
