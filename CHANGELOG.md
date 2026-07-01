# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/), and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.7.3] - 2026-07-01

### Added
- 工具结果外置到磁盘，配合 `tool.recall` 按需回取原始内容，降低长对话上下文占用
- `thought.read` 工具，读取想法正文与元数据，并带隐私过滤
- web_research skill 增强为方案调研（solution research）能力
- Agent loop 提取并固定任务上下文为 system message，循环告警时重锚定到已固定目标
- 单条工具结果截断阈值 + 模型上下文窗口自动推断
- 流式工具结果前置摘要，并增强 SSE 流健壮性

### Changed
- RESEARCH 提示词改为保存完整调研结果（含来源与分析），而非精简摘要；并优先使用 skill 而非裸工具
- 上下文摘要触发更早，改为累积合并
- ContextGuard 对工具结果降级而非直接删除；以 `summarized_up_to` 作为预摘要缓存键
- 提高 web_research 超时以匹配更高的工具预算
- 澄清 `note.create` 与 `thought.create` 的工具描述差异

### Fixed
- 聊天窗口仅在吸附底部时自动滚动，向上翻阅历史时不再被流式刷新打断
- Agent loop 采用取消感知的指数退避重试
- 每个工具独立超时，避免 skill 子轮次被提前终止
- 在 OpenAI API 边界映射工具名，规避函数名不允许点号的限制

### Security
- 阻止 `note.read` 通过软链接逃逸出工作区
- 链接推荐与主题网络图谱均过滤 kf-private 私有笔记

### Removed
- 清理死代码：provider 层的 `is_remote` 与 `create_provider_by_id`

## [0.6.0] - 2026-05-25

### Added
- 用户自定义 Skills 支持：通过 Markdown + YAML frontmatter 文件定义自定义 Skill，支持 UI 管理面板创建/编辑/删除
- Skill 管理面板集成到应用设置（与"通用"、"AI与大模型"并列的第三个 Tab）
- Skill 编辑器大尺寸 Modal（~90vw×85vh），双列布局优化编辑体验
- `note.append` 工具：在已有笔记文件末尾原子性追加内容
- Tool 自主审批机制：Tool trait 新增 `requires_approval()` 方法，与 manifest 策略取并集

### Changed
- 自定义 Skill 加载时机从 setup() 移至 open_workspace，确保重启后正确加载

### Removed
- 清理废弃的审批回调基础设施（ApprovalCallback trait、AutoApprovalCallback、user_approval_callback）
- 清理未使用的 ListFilter、ToolScope::Conversation、approval_id() 方法
- 清理 PrivacyFilter::filter_note_content() 预留方法（隐私最小单元为文档级）

## [0.5.1] - 2025-05-18

### Fixed

- Fixed ordered list bug: pressing Enter after "1. test" no longer inserts spurious "3." text into the new line
- Fixed production build issue where minification broke the filterTransaction class-name check (switched from `constructor.name` to `instanceof ReplaceStep`)
- Patched `@milkdown/preset-commonmark` splitListItemCommand to pass correct `itemAttrs` in ordered list context, eliminating the root-cause race condition between syncListOrderPlugin and Vue NodeView re-render
