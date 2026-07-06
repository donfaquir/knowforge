# Plan Index

> 总控表：所有待办 spec 的状态、优先级一览。
> 最后更新：2026-07-06

---

## Active

（暂无进行中的 spec）

---

## Backlog

（暂无 backlog — 当前架构已稳定，不再主动优化已知内部问题）

---

## Done

| Spec | 优先级 | 完成日期 | 说明 |
|------|--------|----------|------|
| graph 私密节点过滤 | P0 | 2026-06-30 | 图谱泄漏私密笔记信息 |
| note.read symlink 修复 | P0 | 2026-06-30 | 符号链接逃逸漏洞 |
| link 隐私覆盖确认 | P1 | 2026-06-30 | link.suggest_related 隐私过滤 |
| [thought.read](_archive/202506-tool-gap/spec-thought-read.md) | P0 | 2026-07-01 | 想法全文读取工具 |
| [网络瞬态重试](_archive/202506-agent-loop-enhance/spec-error-recovery.md) | P1 | 2026-07-01 | 指数退避 + cancel 感知重试 |
| [目标稳定性](_archive/202507-backlog-cleanup/goal-stability/README.md) | P1 | 2026-07-01 | 目标提取钉住 + 漂移警告 |
| [计划审批](_archive/202507-backlog-cleanup/spec-plan-approval.md) | P1 | 2026-07-02 | Phase A→B 审批门（后已合并为单阶段） |
| [主动压缩](_archive/202507-backlog-cleanup/spec-proactive-compaction.md) | P2 | 2026-07-03 | 修复快照索引漂移 |
| Planning 模式简化 | P1 | 2026-07-03 | Phase A/B 合并为单次执行 |
| God Component 拆分 | P1 | 2026-07-03 | AiConversationPanel 2570→1758 行 |
| [事件可靠性](_archive/fix-event-reliability/) | P1 | 2026-07-06 | 心跳 + watchdog + 幂等 + 缓冲队列 |
| 动态预算 | P1 | 已落地 | max_tool_calls 25 + budget 耗尽优雅降级 |
| Agent Memory v2 | P1 | 已落地 | MemoryManager + agent_memory.json + LLM 提取 |

---

## Closed (Won't Fix)

以下问题经评估后决定不修复，原因见括号：

- D-2 隐式状态机（watchdog + 幂等 + 缓冲已从事件层面解决状态不一致）
- D-4 会话重连（桌面应用刷新概率极低，审批超时已兜底）
- D-6 压缩层边界不清（功能正确，重构风险 > 收益）
- D-7 Skill 隔离（vault 非敌对数据，资源消耗已有界）
- D-8 参数过多（纯美观，不影响功能）
- 消息优先级（128K context 下几乎不触发）
- 搜索重排序（LLM 本身是更强的 reranker）
- 深度研究 / 工作记忆 / 结构化追踪 / 会话回放 / 并行 Agent / 零配置搜索 / MCP / 子 Agent / 思考可视化（关闭理由见 2026-07-03 评审）

---

## Archive

历史计划文档已移至 `_archive/`，保留完整 git 历史。包含：

- `iter1/` — 初始 AI 能力建设（P0-P5 工具 + agent loop）
- `iter1-ai-upgrade/` — ContextGuard + 计划审查等 spec
- `unified-provider/` — 统一 Provider 重构
- `llm-latency/` — LLM 响应延迟优化
- `agent-memory/` — Agent 记忆系统（20 个 spec）
- `context-management-redesign/` — 上下文管理重设计（Phase 1 已完成）
- `search-agent/` — 搜索 Agent 升级分析
- `tool-gap-analysis/` — 工具缺口分析
- `iter2-agent-evolution/` — Agent 架构进化路线（Phase 1-4）
- `web-search-evolution/` — Web 搜索竞品分析与路线
- `202506-security-privacy-fix/` — 安全与隐私修复（3 spec，全部完成）
- `202506-tool-gap/` — 工具缺口（thought.read 完成，其余 wontfix）
- `202506-agent-loop-enhance/` — Agent Loop 增强（重试已完成，反思循环 P3 延后）
- `202507-backlog-cleanup/` — 2026-07 Backlog 清理
- `fix-event-reliability/` — 事件可靠性三层修复（2026-07-06 完成）
- `refactor-god-component/` — God Component 拆分方案
- `review-agent-loop-architecture.md` — Agent Loop 架构评审
- `session-context-management-analysis.md` — 会话上下文管理分析
