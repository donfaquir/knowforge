// Iter 5 #3 后续：把斜杠命令直接当 skill id 解析（无 `/skill ` 前缀）。
// 形态：
//   "/skills" 或 "/skill"   → list
//   "/<skill_id> <body>"    → 触发 skill；id 必须在 validSkillIds 内，否则返回 null（调用方按普通消息发送）
// 命名规则与后端 `is_valid_skill_id`（skills/registry.rs）一致：小写字母开头、含字母/数字/下划线、长度 1–64。

export type SlashCommand =
  | { kind: "skill"; skillId: string; body: string }
  | { kind: "skills-list" };

const SLASH_RE = /^\/([a-z][a-z0-9_]{0,63})(?:\s+([\s\S]+))?$/i;

export function parseSlashCommand(
  input: string,
  validSkillIds: readonly string[],
): SlashCommand | null {
  const t = input.trim();
  if (!t.startsWith("/")) {
    return null;
  }
  if (t === "/skills" || t === "/skill") {
    return { kind: "skills-list" };
  }
  const m = SLASH_RE.exec(t);
  if (!m) {
    return null;
  }
  const body = (m[2] ?? "").trim();
  if (!body) {
    return null;
  }
  const skillId = m[1].toLowerCase();
  if (!validSkillIds.includes(skillId)) {
    return null;
  }
  return { kind: "skill", skillId, body };
}
