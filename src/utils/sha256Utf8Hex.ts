/** 与后端 `write_markdown_file` 一致：对 UTF-8 字节序列做 SHA-256，输出 64 位小写十六进制 */
export async function sha256Utf8Hex(text: string): Promise<string> {
  const data = new TextEncoder().encode(text);
  const hash = await crypto.subtle.digest("SHA-256", data);
  return Array.from(new Uint8Array(hash))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}
