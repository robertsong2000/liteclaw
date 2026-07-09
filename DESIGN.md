# liteclaw — 设计文档

> 小龙虾:claw 生态的轻量级 Rust 执行末端 + 可组合瑞士军刀。

---

## 1. 定位

`liteclaw` 不是「又一个小型 coding agent」,而是 **claw 生态的 Rust 执行末端 +
可组合轻工具集**。它既能被 openclaw 当 worker spawn,又能当独立瑞士军刀;
每个子命令是一只「钳子」(claw),可单用、可拼装。

**一句话**:一个 1.4MB 的静态二进制,内含一组继承内置安全内核的文件/搜索/
审计工具,默认只读、按需放写。

---

## 2. 调研:现有方案对标(2025–2026)

| 方案 | 语言 | 体积/内存 | 特点 | liteclaw 的差异 |
|---|---|---|---|---|
| Zerostack | Rust | 8.9MB / 10MB idle | MCP+ACP+沙箱+本地模型,单 agent loop | liteclaw 走「子命令即工具」正交组合,而非单一 loop |
| NCA CLI | Rust | <20MB 启动 | TUI/REPL/one-shot,类 Claude Code | liteclaw 无 TUI,更小,且内置安全前置 |
| r/rust minimal agent | Rust | ~16MB | 功能极少 | liteclaw 功能更全且每工具自带 Defender |
| Codex CLI | Rust | 中 | AGENTS.md/MCP/三档权限 | 锁 OpenAI;liteclaw 生态原生(clawdefender/openclaw) |
| Aider | Python | 重 | 100+ 模型/git | Python 运行时重;liteclaw 零运行时依赖 |

**行业共识基线**(Zylos 研究 + arXiv 论文):Agent Loop、目录级文件访问、
Shell+权限模型、Hooks 生命周期、会话管理、MCP 互操作、AGENTS.md、安全三件套。
liteclaw 的 MVP 落地了其中与「独立瑞士军刀」最相关的子集。

---

## 3. 五个独有特点

### ① 钳子架构(Claw-as-Tools)
单个静态二进制,`lc <claw>` 即一个独立轻功能。借鉴 Unix "do one thing well",
把工具能力做成正交子命令,可单用、可经 stdin/stdout/`--json` 拼装。

### ② 原生 clawdefender 安全内核(Defender)
内置 clawdefender 的 Rust 规则引擎,**每次 tool call 前自动 sanitize**
(prompt injection / SSRF / path traversal / 凭据外泄)。这是 claw 生态独有,
别的轻量 agent 都没有「安全前置」。规则忠实移植 + 修复 3 个安全 bug(见 §6)。

### ③ 双模运行(规划中)
- `standalone`:直接当 CLI 用(MVP 已实现)
- `--acp`:作为 openclaw spawn 的子进程 worker(后续里程碑)

### ④ 零依赖部署 / 资源硬约束
- 纯 Rust + 静态链接 → 单文件
- **实测 1.4MB release 二进制**(opt-level=z + lto + strip + panic=abort)
- 目标:idle <8MB RAM,0 CPU 空闲;可跑 Alpine/CI runner/树莓派

### ⑤ Streaming-first & Deeply Pipeable
所有 claw 支持 `--json` / stdin / stdout,能和 jq/git/make 串起来。

---

## 4. 架构

```
┌─────────────────────────────────────────────────────────────┐
│                        lc 二进制                             │
│                                                              │
│  ┌──────────┐   ┌──────────────────────────────────────┐   │
│  │ CLI 分发  │──▶│            Claw Registry              │   │
│  │ (clap)   │   │  read │ grep │ edit │ audit │ ...     │   │
│  └────┬─────┘   └───────────────┬──────────────────────┘   │
│       │                          │                           │
│       │         ┌────────────────▼───────────────┐          │
│       │         │       Agent Loop (规划中)        │          │
│       │         └────────────────┬───────────────┘          │
│       │                          │                           │
│  ┌────▼──────────────────────────▼──────────────┐           │
│  │           共享内核 (Shared Core)              │           │
│  │  ┌─────────┐ ┌────────┐ ┌───────┐ ┌────────┐ │           │
│  │  │Defender │ │ FS 工具 │ │Sandbox│ │ Model  │ │           │
│  │  │ 安全前置 │ │ 引擎   │ │ 隔离  │ │ Client │ │           │
│  │  └─────────┘ └────────┘ └───────┘ └────────┘ │           │
│  └─────────────────────────────────────────────┘           │
└─────────────────────────────────────────────────────────────┘
```

### 模块职责

| 模块 | 职责 | 依赖 |
|---|---|---|
| **clap CLI** | 子命令分发、`--json`/`--no-defender`/`--allow-write` | `clap` |
| **Claw Registry** | 注册各 claw,统一 trait `Claw` | 自研 |
| **Agent Loop** | 多轮工具调用循环(规划中) | `tokio` |
| **Defender 内核** | 输入/输出 sanitize、URL 校验、路径校验 | `regex` |
| **FS 引擎** | read/grep/edit,ripgrep 式搜索 | `ignore`,`regex` |
| **Sandbox** | 写白名单、`.liteclawignore` | 自研 |
| **Model Client** | OpenAI 兼容 / Ollama(骨架) | `reqwest`+rustls(规划) |

---

## 5. `Claw` trait(钳子的统一契约)

```rust
#[async_trait::async_trait]
pub trait Claw: Send + Sync {
    fn name(&self) -> &'static str;
    fn desc(&self) -> &'static str;
    /// Every claw inherits defender + sandbox + streaming via ctx.
    async fn run(&self, args: &ClawArgs, ctx: &Ctx) -> Result<ExitCode>;
}
```

`Ctx` 贯穿注入 Defender + Sandbox + I/O,让每只钳子天然继承安全前置与流式。
新增工具只需:① 实现 `Claw`;② 在 `registry::all_claws()` 注册;③ 在 clap
`Command` 加一个变体。

---

## 6. Defender 安全内核

### 规则表(移植自 clawdefender v1.0.0)
6 个规则数组,`grep -qiE`(大小写不敏感)语义,编译期编译为 regex:

| 模块 | 严重度 | 规则数 | 示例 |
|---|---|---|---|
| PromptInjection (Critical) | 90 | 60+ | `ignore previous instructions` |
| PromptInjection (Warning) | 40 | 30 | `pretend to be`,`<\|endoftext\|>` |
| CommandInjection | 90 | 19 | `rm -rf /`,`fork bomb` |
| CredentialExfil | 90 | 13 | `webhook.site`,`curl -d .env` |
| Ssrf | 90 | 9 | `169.254.169.254`,`metadata.google` |
| PathTraversal | 70 | 25 | `../../../`,`/etc/passwd` |
| SensitiveFiles | 40 | 7 | `.env`,`id_rsa`,`api.key` |

### 评分(与 bash 版一致)
取所有命中**最大分**(非累加):≥90 block / ≥70 block / ≥40 warn / <40 allow。

### 三处安全 bug 修复(忠实移植 + 修正)
1. **允许域名锚定**:原版子串未锚定,`evil.com/?x=github.com` 旁路 SSRF。
   改为主机精确匹配或子域匹配。
2. **正则元字符转义**:原版 `<|endoftext|>` 的 `|` 被 ERE 当成交替。已转义。
3. **接通死代码**:原版 `SENSITIVE_FILES` 定义了却从不调用。已接通(WARNING)。

### API
- `scan_text(&str) -> ScanReport`:全文威胁扫描(写操作前置)
- `scan_url(&str) -> ScanReport`:URL SSRF 检查
- `ScanReport::to_compact_json()`:与 bash 版 `--json` 形状对齐
  `{clean, severity, score, action}`,并额外含 `findings` 数组(增强)

---

## 7. Sandbox 模型

- **默认只读**:无 `--allow-write` 时所有写操作被拒
- **写白名单**:`--allow-write <dir>` 显式授权;路径规范化后做前缀匹配,
  拒绝 `..` 逃逸
- **`.liteclawignore`**:复用 `ignore` crate(ripgrep 同源)解析
- **网络**:经 Defender URL 校验;model client 请求走单独显式放行

---

## 8. 目录结构

```
liteclaw/
├── Cargo.toml                  # workspace
├── crates/
│   ├── liteclaw-core/          # Claw trait, Ctx, Defender, Sandbox
│   │   └── src/{lib,claw,ctx,sandbox,defender/}.rs
│   ├── liteclaw-fs/            # read / grep / edit
│   ├── liteclaw-model/         # ModelClient trait (骨架)
│   └── liteclaw-cli/           # main + clap + registry + audit + tests/smoke
├── DESIGN.md  AGENTS.md  README.md
```

workspace 拆分的好处:只想要 FS 工具的人可只编 `liteclaw-fs`,体积更小。

---

## 9. 依赖选型(全轻量、纯 Rust 优先)

| 依赖 | 用途 | 轻量理由 |
|---|---|---|
| `clap`(derive) | CLI | 标杆级轻量 |
| `tokio`(精简 feature) | 异步 | 仅开 rt/macros/io/fs |
| `regex` | Defender 规则 | 纯 Rust,无 PCRE |
| `ignore` | .gitignore/.liteclawignore | ripgrep 同源 |
| `reqwest`+rustls | model client(规划) | **无 openssl** |
| `serde`/`serde_json` | JSON | 标准 |
| `anyhow`/`thiserror` | 错误 | 零开销 |
| `async-trait` | Claw trait | 必需 |

release profile:`opt-level=z` + `lto=true` + `codegen-units=1` + `strip` +
`panic=abort`。**实测 1.4MB**。

---

## 10. 路线图

| 阶段 | 内容 | 状态 |
|---|---|---|
| 0 骨架 | read/grep/edit + Defender + 沙箱 | ✅ 完成 |
| 1 Model + chat | OpenAI 兼容 client,流式 `lc chat` | 🔨 骨架就绪 |
| 2 ACP Worker | `--acp` 被 openclaw spawn | ⏳ |
| 3 Agent Loop | 多轮工具调用 | ⏳ |
| 4 serve(MCP) | 把所有 claw 暴露成 MCP server | ⏳ |
| 5 资源断言 | CI 测内存峰值/二进制体积 | ⏳ |

---

## 11. 资源承诺(SLO,后续里程碑加 CI 断言)

| 指标 | 目标 | 实测(MVP) |
|---|---|---|
| release 二进制 | < 10MB | **1.4MB** ✅ |
| 启动 RSS | < 8MB | 待测 |
| idle CPU | 0% | 待测(无后台线程,天然 0) |

---

## 12. 状态

MVP 已实现并验证:`lc read | grep | edit | audit` + Defender 内核 + 沙箱。
9 个单测 + 7 个集成测试全过。`lc chat`/ACP/MCP 为后续里程碑保留 crate 位。
