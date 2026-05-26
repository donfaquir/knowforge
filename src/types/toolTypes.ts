// src/types/toolTypes.ts
// Layer 2 Tools 协议 v1.0 — 前端类型定义
// 对应 src-tauri/src/tools/types.rs 的 JSON 序列化形态

export type Effect = "read" | "write" | "network" | "llm" | "shell" | "system";

export type Risk = "safe" | "caution" | "dangerous";

export type ApprovalPolicy =
  | "auto"
  | "confirm_once_per_session"
  | "confirm_each"
  | "forbidden";

export type ToolErrorCode =
  | "INVALID_INPUT"
  | "NOT_FOUND"
  | "PERMISSION_DENIED"
  | "WORKSPACE_NOT_OPEN"
  | "PRIVACY_BLOCKED"
  | "NETWORK_DENIED"
  | "TIMEOUT"
  | "RATE_LIMITED"
  | "BUDGET_EXCEEDED"
  | "INTERNAL";

export interface ToolError {
  code: ToolErrorCode;
  message: string;
  retryable: boolean;
  cause?: string;
}

export interface ToolWarning {
  code: string;
  message: string;
}

export interface ToolMetrics {
  duration_ms: number;
  bytes_in: number;
  bytes_out: number;
  llm_tokens_in: number;
  llm_tokens_out: number;
  network_bytes: number;
}

export interface ToolExample {
  description: string;
  input: unknown;
  output: unknown;
}

export interface DeprecationInfo {
  since: string;
  replacement?: string;
}

export interface ToolManifestJson {
  name: string;
  version: string;
  protocol_version: string;
  description: string;
  input_schema: Record<string, unknown>;
  output_schema: Record<string, unknown>;
  effects: Effect[];
  risk: Risk;
  privacy_aware: boolean;
  requires_workspace: boolean;
  default_approval: ApprovalPolicy;
  examples: ToolExample[];
  tags: string[];
  deprecated?: DeprecationInfo;
}

// ToolResult 三态（对应 Rust #[serde(tag = "status")] 枚举）
export interface ToolResultOk {
  status: "ok";
  data: unknown;
  redacted_count: number;
  warnings: ToolWarning[];
  metrics: ToolMetrics;
}

export interface ToolResultPartialOk {
  status: "partial_ok";
  data: unknown;
  redacted_count: number;
  warnings: ToolWarning[];
  metrics: ToolMetrics;
  errors: ToolError[];
}

export interface ToolResultErr {
  status: "error";
  error: ToolError;
}

export type ToolResultJson = ToolResultOk | ToolResultPartialOk | ToolResultErr;

// ToolScope（用于 list_tools 的 scope 参数）
export type ToolScope = "global" | `conv:${string}`;

// 后端在执行非 Auto 策略的工具前 emit `llm:tool-approval-request`，
// 前端弹窗展示后通过 `respond_tool_approval` 回送 Allow/Deny。
export interface ApprovalRequest {
  sessionId: string;
  conversationId: string;
  approvalId: string;
  toolCallId: string;
  toolName: string;
  policy: ApprovalPolicy;
  inputSummary: string;
  risk: Risk;
  effects: Effect[];
}
