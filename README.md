# liteclaw 🦞

> 小龙虾 — 一个轻量级的本地 AI 编程助手。单二进制 ~3MB，内含 Web 对话界面 +
> Agent Loop（模型可自动读写文件、执行命令、调用 78+ skill）+ Defender 安全内核。

## 它能做什么

- 💬 **Web 对话界面** — 浏览器打开，流式输出，Markdown 渲染，TPS 吞吐率显示
- 🤖 **Agent Loop** — 模型自主调用工具：读文件、搜索、创建文件、编译、执行命令
- 🔧 **Skill 系统** — 自动发现并调用已安装的 skill（飞书、搜索、安全扫描等 78+）
- 🛡️ **Defender 安全内核** — 自动拦截 prompt injection / 危险命令 / 路径穿越
- 🐳 **容器部署** — Dockerfile + docker-compose 一键部署

## 快速开始

### 方式一：直接运行（本地编译）

```bash
# 编译（首次较慢）
cargo build --release

# 启动 Web 界面
./target/release/lc serve 9999
```

浏览器打开 `http://localhost:9999`，登录后即可对话。

### 方式二：Docker 部署

```bash
docker compose up -d
```

浏览器打开 `http://localhost:8080`。

> Docker 内访问宿主机模型服务，base_url 填 `http://172.21.0.1:11434/v1`（Ollama）
> 或 `http://host.docker.internal:11434/v1`。

### 方式三：CLI 工具（无需模型）

```bash
lc read README.md           # 读文件（带行号）
lc grep "async" crates/     # 搜索内容（尊重 .gitignore）
lc edit file.txt old new    # 精确替换（需 --allow-write）
lc audit .                  # 安全扫描
lc skills                   # 列出所有 skill
lc skill-run clawdefender --version  # 执行 skill
```

## 配置模型

### 浏览器配置

打开 Web 界面后，顶部配置栏：

| 字段 | 说明 | 示例 |
|---|---|---|
| **base_url** | OpenAI 兼容 API 地址 | `http://localhost:11434/v1` |
| **model** | 模型 id | `qwen3:30b-a3b` |
| **api_key** | API key（本地可填任意） | `sk-xxx` 或 `lm-studio` |

点「保存」→ 存到 `~/.liteclaw/config.json`，下次自动加载。

### 支持的模型后端

任何 OpenAI 兼容的 `/v1/chat/completions` 端点：

| 后端 | base_url | 说明 |
|---|---|---|
| **Ollama** | `http://localhost:11434/v1` | 本地，免费，隐私好 |
| **LM Studio** | `http://localhost:1234/v1` | 本地，GUI 管理 |
| **NewAPI / OneAPI** | `http://your-gateway:port/v1` | 自建网关，多模型聚合 |
| **OpenAI / GLM / DeepSeek** | 云端 `/v1` | 官方 API |

### Function Calling 支持

模型必须支持 function calling 才能自动调工具：

| 模型 | function calling | 推荐 |
|---|---|---|
| MiniMax-M3 | ✅ 稳定 | ⭐ 最佳 |
| Qwen3-30B-A3B | ✅ 支持（有 think） | 推荐 |
| Qwen3-8B | ✅ 支持 | 可用 |
| Gemma4-E2B | ❌ 不支持 | 仅纯对话 |

> 不支持 function calling 的模型只能纯聊天，不能自动调工具。

## 登录

| 用户名 | 密码 |
|---|---|
| `renault` | `renault123` |

Token 24 小时有效。修改密码编辑 `crates/liteclaw-web/src/auth.rs`。

## Agent 工具

模型在对话中可自动调用：

| 工具 | 功能 | 权限 |
|---|---|---|
| `read(path)` | 读文件 / 列目录 | 自动 |
| `grep(pattern, path)` | 搜索内容 | 自动 |
| `audit(path)` | 安全扫描 | 自动 |
| `skill_list()` | 列出所有 skill | 自动 |
| `skill_run(id, args)` | 执行 skill | 自动 |
| `write(path, content)` | 创建 / 覆盖文件 | 自动模式 / 确认 |
| `edit(path, old, new)` | 修改文件 | 自动模式 / 确认 |
| `bash(command)` | 执行 shell 命令 | 自动模式 / 确认 |

**自动模式**（勾选后所有工具免确认）：
- ☑ 勾选 → write/edit/bash 自动执行
- ☐ 不勾 → write/edit/bash 弹「允许/拒绝」按钮

**安全机制**：
- Defender 拦截危险命令（`rm -rf /`、`mkfs`、reverse shell 等）
- bash 执行 60 秒超时
- 沙箱限制写操作在允许目录内
- 输出截断防爆上下文

## CLI 命令

```bash
lc serve [--host 0.0.0.0] [PORT]   # 启动 Web 界面
lc read <path>                      # 读文件
lc grep <pattern> [path]            # 搜索
lc edit <path> <old> <new>          # 替换（需 --allow-write）
lc audit <path>                     # 安全扫描
lc skills                           # 列出 skill
lc skill <id>                       # 查看 skill 详情
lc skill-run <id> [args]            # 执行 skill
lc claws                            # 列出所有命令
```

**全局选项**：
- `--json` — JSON 输出（便于管道拼装）
- `--no-defender` — 关闭安全前置（仅用于可信输入）
- `--allow-write <DIR>` — 授予写权限（可重复）

## 项目结构

```
liteclaw/
├── crates/
│   ├── liteclaw-core/      # Claw trait, Ctx, Defender 安全内核, Sandbox
│   ├── liteclaw-fs/        # read / grep / edit
│   ├── liteclaw-model/     # 流式 OpenAI 兼容 client (reqwest + rustls)
│   ├── liteclaw-skills/    # SKILL.md 解析 + 发现 + 执行
│   ├── liteclaw-agent/     # Agent Loop (reason → tool → observe)
│   ├── liteclaw-web/       # axum HTTP server + 嵌入式前端
│   └── liteclaw-cli/       # main, clap 分发, registry
├── Dockerfile              # 多阶段构建
├── docker-compose.yml      # 容器编排
└── DESIGN.md               # 完整设计文档
```

## 开发

```bash
cargo build                  # debug 编译
cargo build --release        # release 编译（~3MB，LTO 优化）
cargo test                   # 全量测试
cargo clippy --all-targets   # lint
```

依赖全是纯 Rust（rustls 而非 openssl，ignore 而非完整 ripgrep），release 二进制约 3MB。

## 架构特点

1. **Claw trait** — 每个子命令是独立工具，统一 trait，可单用可拼装
2. **Defender 前置** — 安全扫描是 pre-check，不是事后补救
3. **Agent Loop** — reason → tool_call → observe → decide 的多轮循环
4. **零运行时依赖** — 单静态二进制，拷贝即用

详见 [DESIGN.md](DESIGN.md)。

## License

MIT
