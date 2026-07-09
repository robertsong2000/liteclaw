# liteclaw 脚手架 + DESIGN.md 实施计划

## 目标与范围
在 `/Users/robertsong/Downloads/code/liteclaw`(当前空目录)搭建 Rust Cargo workspace 骨架,并落盘详细设计文档。本步交付:**可编译、可运行的 MVP** —— `lc read` / `lc grep` / `lc edit` / `lc audit` 四个独立 claw + Defender 安全内核 + 沙箱,完整验证「独立瑞士军刀 + 安全前置 + 低资源」三个卖点。Model client 和 `lc chat` 搭骨架但标注 TODO(下个里程碑接 OpenAI 兼容 API)。

不在本步范围:ACP worker(`--acp`)、完整 Agent Loop、`lc serve` MCP server —— crate 留位但不实现。

---

## 仓库结构(交付文件树)

```
liteclaw/
├── Cargo.toml                  # workspace 根 [workspace]
├── rust-toolchain.toml         # 固定 stable toolchain
├── .gitignore                  # 标准 Rust + IDE
├── AGENTS.md                   # 仓库级 agent 配置(遵循开放标准)
├── DESIGN.md                   # 完整设计文档(本步核心交付物之一)
├── README.md                   # 简短介绍 + 快速开始
├── crates/
│   ├── liteclaw-core/          # Claw trait, Ctx, Defender, Sandbox
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── claw.rs         # Claw trait + ClawArgs + ExitCode
│   │       ├── ctx.rs          # Ctx: 贯穿所有 claw 的共享上下文
│   │       ├── defender/
│   │       │   ├── mod.rs
│   │       │   ├── rules.rs    # 规则表(编译期常量,移植自 clawdefender)
│   │       │   └── engine.rs   # 扫描引擎: max-score 评分 + JSON/文本输出
│   │       └── sandbox.rs      # 写白名单 + .liteclawignore + 网络域名白名单
│   ├── liteclaw-fs/            # read / grep / edit / patch
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── read.rs         # lc read
│   │       ├── grep.rs         # lc grep (ignore crate, ripgrep 同源)
│   │       ├── edit.rs         # lc edit (强制唯一匹配替换)
│   │       └── patch.rs        # lc patch (TODO 占位)
│   ├── liteclaw-model/         # OpenAI 兼容 model client
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs          # ModelClient trait
│   │       └── openai.rs       # TODO: reqwest rustls 流式
│   └── liteclaw-cli/           # main + clap + registry
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs         # clap 分发 + 全局 flags (--json/--no-defender)
│           └── registry.rs     # Claw registry
└── tests/
    └── smoke.rs                # 集成测试: lc read/grep/edit/audit 基本路径
```

---

## 关键设计(将写入 DESIGN.md)

### 1. `Claw` trait(liteclaw-core/src/claw.rs)
每个"钳子"是独立可用的子命令。`Ctx` 注入 Defender/Sandbox/I/O,让所有 claw 天然继承安全前置与流式。

```rust
#[async_trait::async_trait]
pub trait Claw: Send + Sync {
    fn name(&self) -> &'static str;
    fn desc(&self) -> &'static str;
    /// Every claw inherits defender + sandbox + streaming via ctx.
    async fn run(&self, args: &ClawArgs, ctx: &Ctx) -> Result<ExitCode>;
}
```

### 2. Defender 内核(liteclaw-core/src/defender/)
基于提取的 clawdefender 规则,**忠实移植 6 个规则数组 + max-score 评分**,但修复 3 个安全 bug:
- 移植:PROMPT_INJECTION_CRITICAL(90)、PROMPT_INJECTION_WARNING(40)、COMMAND_INJECTION(90)、CREDENTIAL_EXFIL(90)、SSRF_PATTERNS(90)、PATH_TRAVERSAL(70)
- 评分:取所有命中最大分;阈值 ≥90 block/≥70 block/≥40 warn/<40 allow(与 bash 版一致)
- 修复:① 允许域名检查改为锚定匹配(原版子串旁路)② 转义 `<|endoftext|>` 等正则 ③ 接入原本是死代码的 SENSITIVE_FILES(作为 INFO 提示)
- 用 `regex` crate 编译期编译所有 pattern(case_insensitive),替代 bash 每条 pattern 起子进程
- 提供 `scan_text()` / `scan_url()` 两个入口;`--json` 输出聚合 `{clean, severity, score, action}`(与 bash 版对齐),并新增 `findings` 数组(增强)

### 3. Sandbox(liteclaw-core/src/sandbox.rs)
- 写白名单:`--allow-write <path>` 显式授权;默认只读
- `.liteclawignore`:复用 `ignore` crate(与 ripgrep 同源)解析
- 网络白名单:通过 Defender 的 URL 校验;model client 请求走单独显式放行

### 4. `lc read` 完整实现示意(本步唯一全功能 claw 之外的范本)
带行号、超长截断、目录列表、Defender 前置(扫描文件内容,命中即 warn)、`--json` 输出。作为其他 claw 的实现样板。

### 5. Model client(liteclaw-model,仅骨架)
`ModelClient` trait + `OpenAiClient` 占位(reqwest + rustls,无 openssl),标注 TODO。

---

## 依赖选型(全轻量、纯 Rust 优先)
- `clap`(derive)— CLI
- `tokio`(只开 rt + macros + io-std + fs feature,最小化)— 异步
- `regex` — Defender 规则
- `ignore` — ripgrep 同源的 .gitignore/.liteclawignore
- `reqwest` + rustls(无 openssl)— model client
- `serde` / `serde_json` — JSON
- `anyhow` / `thiserror` — 错误
- `async-trait` — Claw trait

目标:`cargo build --release` 单二进制 < 10MB,启动 RSS < 8MB(写进 DESIGN.md 作为承诺,后续里程碑加 CI 断言)。

---

## 本步交付清单
1. **DESIGN.md** — 完整设计:定位、调研对标、五个特点、架构图、各 crate 职责、Defender 规则表与 bug 修复、沙箱模型、Claw trait、路线图、资源承诺
2. **AGENTS.md** — 仓库级 agent 配置
3. **完整 Cargo workspace** — 4 个 crate 全部可 `cargo build` 通过
4. **4 个能跑的 claw** — `lc read` / `lc grep` / `lc edit` / `lc audit`
5. **smoke 集成测试** — 基本路径通过
6. **README.md** — 快速开始

## 验证方式
- `cargo build --release` 成功
- `cargo test` 通过(含 smoke)
- `./target/release/lc read Cargo.toml` 能正常输出
- `./target/release/lc audit <dir>` 能复现 clawdefender 的 prompt injection 检测
- `./target/release/lc --help` 列出所有子命令

## 决策说明(非阻塞,可在批准时推翻)
- **Defender bug 策略**:默认「忠实移植规则 + 修复可安全修复的 bug」。如需与 bash 版 100% 严格一致(含 bug)请告知。
- **`lc chat`/model**:本步只搭骨架不实现,因为 MVP 聚焦验证「轻量+安全前置」;OpenAI 兼容 client 是紧随其后的里程碑。
- **git init**:目录当前非 git 仓库,我会 `git init` 并做首个提交(遵循惯例,除非你不要)。