// src/utils/toolInvoke.ts
// Tauri invoke 封装：list_tools / invoke_tool

import { invoke } from "@tauri-apps/api/core";
import type {
  ToolManifestJson,
  ToolResultErr,
  ToolResultJson,
  ToolResultOk,
  ToolScope,
} from "../types/toolTypes";

/**
 * 获取已注册工具的清单列表
 * @param scope 可选，默认 'global'
 */
export async function listTools(
  scope: ToolScope = "global",
): Promise<ToolManifestJson[]> {
  return invoke<ToolManifestJson[]>("list_tools", { scope });
}

/**
 * 调用指定工具
 * @param name 工具名，如 'time.now'、'vault.semantic_search'
 * @param input 工具输入，需符合工具的 input_schema
 * @param conversationId 关联的对话 ID（可选）
 */
export async function invokeTool(
  name: string,
  input: unknown = {},
  conversationId?: string,
): Promise<ToolResultJson> {
  return invoke<ToolResultJson>("invoke_tool", {
    name,
    input,
    conversationId: conversationId ?? null,
  });
}

/**
 * 类型守卫：判断 ToolResult 是否成功
 */
export function isToolOk(result: ToolResultJson): result is ToolResultOk {
  return result.status === "ok";
}

/**
 * 类型守卫：判断 ToolResult 是否为错误
 */
export function isToolErr(result: ToolResultJson): result is ToolResultErr {
  return result.status === "error";
}

/**
 * 回送审批结果（Allow / Deny）。
 */
export async function respondToolApproval(
  approvalId: string,
  decision: boolean,
): Promise<void> {
  return invoke<void>("respond_tool_approval", { args: { approvalId, decision } });
}

/**
 * 切换/删除会话时清理 ConfirmOncePerSession 会话级缓存。
 */
export async function clearConversationApprovals(
  conversationId: string,
): Promise<void> {
  return invoke<void>("clear_conversation_approvals", {
    args: { conversationId },
  });
}
