# Knowforge

**Local-first desktop workspace for notes and knowledge — 本地优先的笔记与知识桌面工作台**

[简体中文](#简体中文) · [English](#english)

---

## 简体中文

### 简介

Knowforge 是一款基于 [Tauri 2](https://tauri.app/) 的跨平台桌面应用：前端为 **React + TypeScript + Vite**，核心业务逻辑在 **Rust**（`src-tauri/`）中运行。数据以本地为主，适合管理 Markdown 笔记、知识库目录与检索、写作辅助等场景（具体能力随版本迭代，以应用内体验为准）。

若你关注**从公司内部走向对外开源**的背景与必要性说明，参见 [OPEN_SOURCE.md](OPEN_SOURCE.md)。

### 功能概览

- 本地 Markdown / 知识库工作流，结合文件监听与索引能力  
- 全文与上下文检索、笔记元数据与隐私相关能力  
- 图表与可视化（如 Mermaid）、部分编辑与排版由 Milkdown / CodeMirror 等组件支撑  
- 界面支持国际化（i18next）  
- 可选的本地或端侧 AI 相关能力（以当前代码与配置为准）

### 技术栈

| 层级 | 技术 |
|------|------|
| 桌面壳 | Tauri 2 |
| 前端 | React 19、Vite 8、TypeScript |
| 后端 / 原生 | Rust（edition 2024）、SQLite（rusqlite）等 |

### 环境要求

- **Rust**：不低于 `src-tauri/Cargo.toml` 中的 `rust-version`  
- **Node.js**：建议使用当前 LTS  
- 操作系统与系统库：请遵循 [Tauri 官方前置条件](https://tauri.app/start/prerequisites/)

### 快速开始

```bash
git clone https://github.com/caichangqing/knowforge.git
cd knowforge
npm ci
```

仅启动前端开发服务器（Vite）：

```bash
npm run dev
```

启动完整桌面应用（Tauri + 前端）：

```bash
npm run tauri dev
```

### 常用命令

| 命令 | 说明 |
|------|------|
| `npm run dev` | 前端开发服务器 |
| `npm run build` | 类型检查 + 生产级前端构建 |
| `npm run preview` | 预览构建后的前端资源 |
| `npm run tauri dev` | Tauri 开发模式 |
| `npm run tauri build` | 打包桌面安装包 / 可分发产物 |
| `cd src-tauri && cargo test` | Rust 单元与集成测试 |

### 仓库结构（节选）

```
knowforge/
├── src/                 # 前端源码（React）
├── src-tauri/           # Tauri 与 Rust 后端
├── package.json
├── LICENSE              # Apache-2.0
├── NOTICE               # 版权与第三方说明（含随仓库分发的模型资源提示）
├── OPEN_SOURCE.md       # 开源背景与必要性（中英）
└── CONTRIBUTING.md      # 参与贡献说明
```

### 参与贡献与安全

- 贡献流程与约定见 [CONTRIBUTING.md](CONTRIBUTING.md)  
- 行为准则见 [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)  
- **请勿在公开 Issue 讨论未修复的安全问题**；报告方式见 [SECURITY.md](SECURITY.md)

### 许可证

本项目在 **Apache License 2.0** 下发布，详见 [LICENSE](LICENSE)。仓库中可能包含需单独遵守许可的第三方文件（例如 `NOTICE` 中列出的嵌入模型权重），分发或再发布时请一并保留 `NOTICE` 与相关说明。

---

## English

### Overview

Knowforge is a **local-first** desktop application built with [Tauri 2](https://tauri.app/). The UI is **React + TypeScript + Vite**, while core logic runs in **Rust** under `src-tauri/`. It targets Markdown notes, knowledge-vault workflows, search, and writing assistance (exact features evolve with releases; the in-app experience is authoritative).

For **background and rationale** on moving from a company-internal context to public open source, see [OPEN_SOURCE.md](OPEN_SOURCE.md).

### Highlights

- Local Markdown / vault-oriented workflows with filesystem watching and indexing  
- Full-text and contextual search, note metadata, and privacy-related controls  
- Diagrams and visualization (e.g. Mermaid); editing powered by Milkdown, CodeMirror, and related libraries  
- UI internationalization via i18next  
- Optional AI-related workflows, depending on build and configuration  

### Tech stack

| Layer | Technology |
|--------|------------|
| Shell | Tauri 2 |
| Frontend | React 19, Vite 8, TypeScript |
| Backend / native | Rust (edition 2024), SQLite (rusqlite), etc. |

### Prerequisites

- **Rust**: at least the `rust-version` declared in `src-tauri/Cargo.toml`  
- **Node.js**: current LTS recommended  
- **OS / system libraries**: follow [Tauri prerequisites](https://tauri.app/start/prerequisites/)

### Quick start

```bash
git clone https://github.com/caichangqing/knowforge.git
cd knowforge
npm ci
```

Frontend only (Vite dev server):

```bash
npm run dev
```

Full desktop app (Tauri + frontend):

```bash
npm run tauri dev
```

### Common commands

| Command | Description |
|---------|-------------|
| `npm run dev` | Start Vite dev server |
| `npm run build` | Typecheck + production frontend build |
| `npm run preview` | Preview built frontend assets |
| `npm run tauri dev` | Tauri development mode |
| `npm run tauri build` | Package the desktop app |
| `cd src-tauri && cargo test` | Rust tests |

### Repository layout (partial)

```
knowforge/
├── src/                 # Frontend (React)
├── src-tauri/           # Tauri + Rust backend
├── package.json
├── LICENSE              # Apache-2.0
├── NOTICE               # Attribution & third-party notes (e.g. bundled model weights)
├── OPEN_SOURCE.md       # Open-source background & rationale (bilingual)
└── CONTRIBUTING.md      # Contribution guide
```

### Contributing & security

- See [CONTRIBUTING.md](CONTRIBUTING.md) for workflow and expectations.  
- Community standards: [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).  
- **Do not file public issues for unfixed security vulnerabilities**; use [SECURITY.md](SECURITY.md).

### License

Licensed under the **Apache License 2.0** — see [LICENSE](LICENSE). Third-party materials may ship under their own terms (for example embedding weights referenced in [NOTICE](NOTICE)); retain `NOTICE` and upstream notices when redistributing.

---

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-Apache_2.0-blue.svg" alt="License: Apache-2.0" /></a>
</p>
