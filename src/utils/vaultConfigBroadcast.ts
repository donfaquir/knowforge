/** 工作区 vault 配置（含独立回顾等）保存成功后派发，供侧栏回顾等立即重读 */
export const VAULT_CONFIG_UPDATED_EVENT = "knowforge:vaultConfigUpdated";

/** 由侧栏回顾等触发，请求打开「AI 与大模型」设置弹窗 */
export const OPEN_AI_SETTINGS_EVENT = "knowforge:openAiSettings";

export function dispatchVaultConfigUpdated(): void {
  window.dispatchEvent(new CustomEvent(VAULT_CONFIG_UPDATED_EVENT));
}

export function dispatchOpenAiSettings(): void {
  window.dispatchEvent(new CustomEvent(OPEN_AI_SETTINGS_EVENT));
}
