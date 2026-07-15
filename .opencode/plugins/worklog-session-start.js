import { mkdirSync, appendFileSync } from "node:fs";
import { execSync } from "node:child_process";
import { join } from "node:path";

export default async ({ directory }) => {
  // Write session header to worklog on plugin init (session start)
  const worklogDir = join(directory, "worklog");
  mkdirSync(worklogDir, { recursive: true });

  const now = new Date();
  const pad = (n) => String(n).padStart(2, "0");
  const date = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
  const time = `${pad(now.getHours())}:${pad(now.getMinutes())}:${pad(now.getSeconds())}`;
  const sessionId = Math.random().toString(36).slice(2, 10);

  let branch = "none";
  try {
    branch = execSync("git branch --show-current", { cwd: directory, encoding: "utf8" }).trim() || "none";
  } catch {}

  const header = `\n## ${date} ${time} · session:${sessionId} · branch:${branch}\n`;
  appendFileSync(join(worklogDir, `${date}.md`), header);

  return {};
};
