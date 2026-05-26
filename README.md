# Knowforge

**Local-first desktop workspace for notes and knowledge — 本地优先的笔记与知识桌面工作台**

[简体中文](#简体中文) · [English](#english)

---

## 简体中文

### 简介

Knowforge 是一款基于 [Tauri 2](https://tauri.app/) 的跨平台桌面应用：前端为 **React + TypeScript + Vite**，核心业务逻辑在 **Rust**（`src-tauri/`）中运行。数据以本地为主，适合管理 Markdown 笔记、知识库目录与检索、写作辅助等场景（具体能力随版本迭代，以应用内体验为准）。

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

### 编译前：语义嵌入模型（BGE）

为控制仓库体积，**BAAI bge-small-zh-v1.5** 权重文件默认不随 Git 提交（见仓库根目录 `.gitignore` 与 [NOTICE](NOTICE)）。在运行 **`npm run tauri dev`** 或 **`npm run tauri build`** 之前，请在本机补齐模型，否则依赖本地向量索引/语义能力的特性可能不可用。

在仓库根目录下，将以下三个文件放到 **`src-tauri/resources/models/bge-small-zh-v1.5/`**（与其中 `.gitkeep` 同级）：

| 文件 | 说明 |
|------|------|
| `config.json` | 模型配置 |
| `tokenizer.json` | 分词器 |
| `model.safetensors` | 权重（较大） |

**下载方式（任选其一）**

1. **Hugging Face Hub CLI**（推荐；需 Python 3）  
   在克隆后的项目根目录执行：

   ```bash
   pip install -U "huggingface_hub[cli]"
   huggingface-cli download BAAI/bge-small-zh-v1.5 --local-dir src-tauri/resources/models/bge-small-zh-v1.5
   ```

   若已安装新版 CLI，也可使用：`hf download BAAI/bge-small-zh-v1.5 --local-dir src-tauri/resources/models/bge-small-zh-v1.5`

2. **网页手动下载**  
   打开模型页 [huggingface.co/BAAI/bge-small-zh-v1.5](https://huggingface.co/BAAI/bge-small-zh-v1.5)，在 **Files and versions** 中下载上述三个文件，保存到 `src-tauri/resources/models/bge-small-zh-v1.5/`。

首次成功加载后，应用会把完整三件套复制到用户缓存目录 **`~/.cache/knowforge/models/bge-small-zh-v1.5/`**（一般无需手动创建）。若仅将文件放在用户缓存而不放 `src-tauri/resources/...`，需自行保证路径与文件名与上表一致。

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

### macOS 安全提示

如果从 GitHub 下载安装后，macOS 提示应用"已损坏，无法打开"，这是因为应用尚未进行 Apple 公证（notarization）。请在终端执行以下命令解除限制：

```bash
xattr -cr /Applications/Knowforge.app
```

然后重新打开应用即可。

### 仓库结构（节选）

```
knowforge/
├── src/                 # 前端源码（React）
├── src-tauri/           # Tauri 与 Rust 后端
├── package.json
├── LICENSE              # Apache-2.0
├── NOTICE               # 版权与第三方说明（含随仓库分发的模型资源提示）
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

### Before you build: embedding weights (BGE)

To keep the Git repository small, **BAAI bge-small-zh-v1.5** weight files are **not** committed by default (see `.gitignore` at the repo root and [NOTICE](NOTICE)). Download them locally **before** running **`npm run tauri dev`** or **`npm run tauri build`**, or features that rely on the bundled embedding model may not work.

From the repository root, place these three files under **`src-tauri/resources/models/bge-small-zh-v1.5/`** (alongside the existing `.gitkeep`):

| File | Role |
|------|------|
| `config.json` | Model config |
| `tokenizer.json` | Tokenizer |
| `model.safetensors` | Weights (large) |

**How to obtain the files (pick one)**

1. **Hugging Face Hub CLI** (recommended; Python 3 required)  
   Run from the cloned repository root:

   ```bash
   pip install -U "huggingface_hub[cli]"
   huggingface-cli download BAAI/bge-small-zh-v1.5 --local-dir src-tauri/resources/models/bge-small-zh-v1.5
   ```

   If you use the newer CLI entrypoint: `hf download BAAI/bge-small-zh-v1.5 --local-dir src-tauri/resources/models/bge-small-zh-v1.5`

2. **Browser download**  
   Open [huggingface.co/BAAI/bge-small-zh-v1.5](https://huggingface.co/BAAI/bge-small-zh-v1.5), use **Files and versions**, and download the three files into `src-tauri/resources/models/bge-small-zh-v1.5/`.

On first successful load, the app copies the complete set into **`~/.cache/knowforge/models/bge-small-zh-v1.5/`** (you normally do not need to create this manually). If you only populate the user cache and skip `src-tauri/resources/...`, you must keep the same three filenames under that cache path.

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

### macOS Security Notice

If macOS shows "app is damaged and can't be opened" after downloading from GitHub, this is because the app has not been notarized by Apple yet. Run the following command in Terminal to bypass:

```bash
xattr -cr /Applications/Knowforge.app
```

Then reopen the app.

### Repository layout (partial)

```
knowforge/
├── src/                 # Frontend (React)
├── src-tauri/           # Tauri + Rust backend
├── package.json
├── LICENSE              # Apache-2.0
├── NOTICE               # Attribution & third-party notes (e.g. bundled model weights)
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
