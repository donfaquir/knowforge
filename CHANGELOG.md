# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/), and this project adheres to [Semantic Versioning](https://semver.org/).

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
