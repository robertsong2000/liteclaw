# liteclaw 🦞

> 小龙虾 — claw 生态的轻量级 Rust 执行末端。一个 1.4MB 的二进制,内含一组
> 继承内置安全内核的文件/搜索/审计工具。

每个子命令是一只「钳子」(claw),可单用、可经 `--json` / 管道拼装。安全扫描
(prompt injection / SSRF / path traversal)是**内置前置**,不是外挂。

## 快速开始

```bash
cargo build --release          # → target/release/lc (1.4MB)
./target/release/lc --help

# 文件工具
./target/release/lc read Cargo.toml
./target/release/lc grep "async" crates/
./target/release/lc --allow-write . edit README.md liteclaw LiteClaw

# 安全审计(= clawdefender 的 Rust 版)
./target/release/lc audit .

# Skill 系统:发现 / 查看 / 执行
./target/release/lc skills                       # 列出全部 skill
./target/release/lc skill clawdefender           # 查看 SKILL.md
./target/release/lc skill-run clawdefender --audit .  # 执行 scripts 型 skill

# JSON 输出,便于拼装
./target/release/lc --json read Cargo.toml | jq .lines
```

## 内置 claw

| 命令 | 功能 |
|---|---|
| `lc read <path>` | 读文件(带行号/截断)或列目录 |
| `lc grep <pattern> [path]` | 内容搜索(尊重 .gitignore) |
| `lc edit <path> <old> <new>` | 精确唯一匹配替换(默认只读,需 `--allow-write`) |
| `lc audit <path>` | 扫描目录安全风险(prompt injection/SSRF/路径穿越) |
| `lc skills` | 列出发现的 skill(全局 `~/.agents/skills` + 项目 `.liteclaw/skills`) |
| `lc skill <id>` | 查看某 skill 的完整 SKILL.md |
| `lc skill-run <id> [args]` | 执行 scripts 型 skill(args 透传,Defender 前置) |
| `lc claws` | 列出所有内置 claw |

## 全局 flags

- `--json`:输出 JSON 而非人类可读文本
- `--no-defender`:关闭安全前置(仅用于可信输入)
- `--allow-write <DIR>`:授予目录写权限(可重复);默认只读

## 卖点

- **钳子架构**:子命令即正交工具,单用 + 拼装
- **Skill 原生支持**:发现/查看/执行 claw 生态的 SKILL.md,项目覆盖全局
- **安全前置**:clawdefender 规则引擎内嵌,每次写操作 + skill 执行自动 sanitize
- **资源极省**:纯 Rust 静态二进制 1.6MB,默认只读沙箱
- **可拼装**:`--json` + stdin/stdout,与 jq/git/make 串联

## 生态

liteclaw 是 claw 生态的执行末端:
- [openclaw](../openclaw) — 多平台消息编排(后续 `--acp` 接入)
- [clawdefender](../clawdefender) — 安全规则源头(liteclaw Defender 的 Rust 移植)

详见 [DESIGN.md](DESIGN.md)。

## 状态

MVP+:`read | grep | edit | audit | skills | skill | skill-run` + Defender 内核 +
沙箱 + skill 系统。`chat` / ACP worker / MCP server 为后续里程碑。

## License

MIT
