/**
 * 覆盖 remark-stringify 的 text handler：wikilink / 嵌入片段不经过 state.safe，避免 \[\[ 转义。
 * 与 mdast-util-to-markdown 默认 unsafe 规则对齐：仅识别连续 `[[...]]` 与 `![[...]]`。
 */
import type { Ctx } from "@milkdown/ctx";
import { remarkStringifyOptionsCtx } from "@milkdown/core";
import type { Text } from "mdast";
import type { Info, State } from "mdast-util-to-markdown";
import { WIKI_LINK_BODY } from "../utils/wikiLinkBodyRegex";

/**
 * 与 Milkdown core 默认 text handler 一致：纯空白行特殊处理；其余分段 safe。
 */
export function remarkSerializeTextPreserveWikilink(
  node: Text,
  _: unknown,
  state: State,
  info: Info,
): string {
  const value = node.value;
  if (/^[^*_\\]*\s+$/.test(value)) {
    return value;
  }
  const encodeOpts = { ...info, encode: [] } as Parameters<State["safe"]>[1];
  let out = "";
  let last = 0;
  const re = new RegExp(WIKI_LINK_BODY.source, "g");
  let m: RegExpExecArray | null;
  while ((m = re.exec(value)) !== null) {
    const isEmbed = m.index > 0 && value[m.index - 1] === "!";
    const segStart = isEmbed ? m.index - 1 : m.index;
    const segEnd = m.index + m[0].length;

    if (segStart > last) {
      out += state.safe(value.slice(last, segStart), encodeOpts);
    }
    out += value.slice(segStart, segEnd);
    last = segEnd;
  }
  if (last === 0) {
    return state.safe(value, encodeOpts);
  }
  if (last < value.length) {
    out += state.safe(value.slice(last), encodeOpts);
  }
  return out;
}

/** 在 Crepe `Editor` 上注册，须在 create 之前调用 */
export function configureRemarkStringifyPreserveWikilink(ctx: Ctx): void {
  ctx.update(remarkStringifyOptionsCtx, (prev) => ({
    ...prev,
    handlers: {
      ...prev.handlers,
      text: remarkSerializeTextPreserveWikilink,
    },
  }));
}
