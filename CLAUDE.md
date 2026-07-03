# KnowForge Project Guidelines

## Build & Test

```bash
# Frontend type check
npx tsc --noEmit

# Backend tests
cargo test --manifest-path src-tauri/Cargo.toml

# Dev server
npm run dev
```

## Architecture: File Size Limits

Single-file components and modules must not grow unboundedly. Hard limits (lines):

| Threshold | Action |
|-----------|--------|
| > 800     | Consider extracting hooks or sub-components |
| > 1200    | Must extract before adding new logic |
| > 1800    | Treat as a bug — refactor before any other work |

Current large files that must NOT grow further without extraction:

- `src/App.tsx` (1921) — route/layout only; new features go in dedicated components
- `src/components/AiConversationPanel.tsx` (1758) — recently refactored; new event handlers → `useAgentEventHandlers`, new rendering → `MessageBubble` or new sub-component, new UI state → dedicated hook
- `src/components/AiLlmSettingsModal.tsx` (1680) — extract section components for new settings
- `src/hooks/useOpenDocs.ts` (1140) — extract sub-hooks for new doc operations
- `src-tauri/src/lib.rs` (1828) — Tauri command hub; new commands register here but implementation goes in domain modules
- `src-tauri/src/llm/agent_loop.rs` (1581) — core loop; new tool handling → tool modules, new context logic → context_guard

### Where to put new code

- New Tauri event listener → `src/hooks/useAgentEventHandlers.ts`
- New message rendering logic → `src/components/MessageBubble.tsx` or a new `src/components/<Name>.tsx`
- New tool call display → `src/components/ToolCallItem.tsx`
- New React hook → `src/hooks/use<Name>.ts`
- New Rust tool → `src-tauri/src/tools/built_in/<name>.rs`
- New LLM feature → `src-tauri/src/llm/<name>.rs`, not inline in `agent_loop.rs`
