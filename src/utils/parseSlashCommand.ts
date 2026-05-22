// Iter 5 #3: parse "/skill <id> <body>" and "/skills" / "/skill" out of the
// AI panel composer. Returns null for any non-slash or non-matching input so
// the caller can fall through to the normal chat send path.

export type SlashCommand =
  | { kind: "skill"; skillId: string; body: string }
  | { kind: "skills-list" };

// Skill id grammar mirrors backend `is_valid_skill_id` (skills/registry.rs):
// lowercase ascii letter/digit/underscore, 1-64 chars, must start with a letter.
const SKILL_RE = /^\/skill\s+([a-z][a-z0-9_]{0,63})\s+([\s\S]+)$/i;

export function parseSlashCommand(input: string): SlashCommand | null {
  const t = input.trim();
  if (!t.startsWith("/")) {
    return null;
  }
  if (t === "/skills" || t === "/skill") {
    return { kind: "skills-list" };
  }
  const m = SKILL_RE.exec(t);
  if (!m) {
    return null;
  }
  const body = m[2].trim();
  if (!body) {
    return null;
  }
  return { kind: "skill", skillId: m[1].toLowerCase(), body };
}
