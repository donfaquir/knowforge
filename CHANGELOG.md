# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/), and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.7.6] - 2026-07-15

### Added
- Practice Mode: new activity bar entry with routing, DiscoveryPane, PracticeSourcePreview (dual-pane), and PracticeReviewPane
- Review completion transition with daily picks in Practice Mode
- Discovery batch operations: multi-select, batch dismiss/promote
- Discovery detail views by candidate type
- Backend commands: `list_discovery_candidates` and `batch_dismiss_candidates`
- Latent paragraphs: embedding-based paragraph candidate engine
- Challenge review: support latent paragraph candidates in review queue
- Challenge: question quality feedback loop
- Review: show related documents for latent paragraph candidates
- Review: abandon button in QA phase to return to pick
- Review push notifications and thought growth story export
- AI degraded guidance and export enhancements (spec-1c, spec-3b)
- AI guide: show setup guidance when LLM is not configured
- Onboarding: replace step 4 tips with dynamic discovery card
- SM-2 algorithm upgrade for challenge scheduling
- i18n: en/zh translations for practice, discovery, reviewReminder

### Changed
- Right panel: remove review tab, redirect to Practice Mode
- App.tsx refactored: extract ContentArea, EditorView, AppTopToolbar and hooks
- Cognitive report panel split into card-based subcomponents
- Review: replace inline challenge Q&A with lightweight reminder
- Graph, topic-network and skills modules frozen (chore)

### Fixed
- Agent loop: prevent watchdog timeout during long async operations
- Latent: trigger scan on workspace open when candidates table is empty
- Latent: add filter versioning to invalidate stale candidates
- Review: deduplicate prefetch and on-demand question generation
- Review: pre-generate challenge questions to eliminate wait time
- Graph/topic-network entry restored in ActivityBar
- Suppress compiler warnings (unused imports, variables, dead code)

## [0.7.5] - 2026-07-08

### Added
- Activity Bar for global navigation (files, graph, thoughts) with instant CSS tooltips
- Save-as-thought button in editor selection toolbar
- 4-step onboarding wizard for new users
- Writing coach: manual trigger, performance optimizations, and streaming support
- Active model label displayed below the AI chat input box

### Changed
- Link recommendations moved from full-screen view to right sidebar tab (4th tab, disabled when no document is open)
- Thought management rendered as Activity Bar view mode instead of a separate panel
- Right panel simplified to 3 core tabs (outline, AI, review) plus link-rec
- Open-folder action moved from top toolbar to sidebar footer
- ThoughtSavePopover redesigned: section dividers, labels, search icon, options-bar grouping
- Depth selector removed from thought-save popover; default depth changed to "deep"
- New-file and new-folder icons redesigned for better visual distinction
- Redundant "workspace keyword search" toggle removed from AI panel (semantic search covers it)

### Fixed
- Editor scroll position now saves/restores correctly when switching document tabs
- macOS traffic light controls vertically centered in the toolbar
- Sidebar toggle icon no longer clipped when sidebar is collapsed on macOS
- Search icon stays visible when sidebar is collapsed
- File tree auto-expands when clicking the files icon in Activity Bar
- Activity Bar remains visible when sidebar is collapsed
- Thought-save popover dropdown now closes on blur

## [0.7.4] - 2026-07-06

### Added
- 后端心跳事件（`llm:heartbeat`），agent loop 迭代及工具执行期间每 10 秒发送
- 前端 watchdog 定时器，30 秒无事件自动恢复 streaming 状态
- 事件缓冲队列：前置事件未到时缓冲后续事件，前置满足后自动冲刷
- 记忆提案卡片展示完整 content 字段（之前仅显示推荐原因）

### Changed
- Planning 模式从 Phase A/B 两阶段简化为单次 agent loop 执行
- AiConversationPanel 拆分：提取 ToolCallItem、MessageBubble 组件和 useAgentEventHandlers hook（-31%）

### Fixed
- 事件处理器幂等化：tool-call-start / skill-spawn / tool-call-done 防重复
- context-guard 使用消息计数替代索引，避免快照漂移
- 工具审批等待时间不再计入工具超时

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
