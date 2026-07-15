## Worklog 工作日志习惯

每个 session 必须维护 `worklog/YYYY-MM-DD.md`，让用户次日打开就能续接工作。

### Session 启动时（第一次回应用户之前）

必读最近一份 worklog 获取上下文：
1. 先读 `worklog/<今天>.md`；如不存在或为空，读 `ls -1t worklog/*.md | head -2` 找最近 1-2 天的文件
2. 重点看最近 session 段落里的「下一步」字段，作为接续点
3. worklog-session-start 插件已自动在当日 worklog 写入本 session 的段落头(`## 时间 · session:xxx · branch:xxx`)，无需手写

### 工作进行中（关键节点主动 append）

以下时机必须在当前 session 段落下 append 一段记录到当日 worklog：
- 一个任务被标记 completed 时
- 做出重要架构/技术决策时
- 遇到无法自行解决的阻塞（需要用户决策、外部依赖）时
- Session 即将结束、要交接给用户时

### Append 格式

```
### HH:MM <一行总结，10 字内>
- **做了什么**: 简述动作和涉及的文件
- **为什么**: 动机、约束、上下文（用户次日看时能复原决策）
- **下一步**: 明确的接续动作，让下个 session 知道从哪里开始
```

「下一步」是核心字段——次日新 session 启动后会优先读它来决定接什么。
