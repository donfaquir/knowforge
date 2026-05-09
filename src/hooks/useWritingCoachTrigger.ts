import type { RefObject } from "react";
import { useEffect, useRef } from "react";
import type { EditorView } from "@milkdown/prose/view";
import { selectionInsideThoughtCallout } from "../utils/thoughtCalloutPm";

const POLL_MS = 650;

function countCodePoints(s: string): number {
  return [...s].length;
}

/** 取光标所在最近 textblock 的纯文本 */
function nearestTextblockText(state: EditorView["state"]): string {
  const { $from } = state.selection;
  // 光标多在 text 内：parent 常为 textblock（Milkdown 多层包裹时比自 depth 向下更稳）
  const parent = $from.parent;
  if (parent.isTextblock) {
    const start = $from.start($from.depth);
    const end = $from.end($from.depth);
    return state.doc.textBetween(start, end, "\n");
  }
  for (let d = $from.depth; d >= 1; d -= 1) {
    const n = $from.node(d);
    if (n.isTextblock) {
      const start = $from.start(d);
      const end = $from.end(d);
      return state.doc.textBetween(start, end, "\n");
    }
  }
  return "";
}

function depthWritingTrigger(text: string, depthMinChars: number): boolean {
  return countCodePoints(text.trim()) >= depthMinChars;
}

function termDensityTrigger(text: string, termMinChars: number): boolean {
  const t = text.trim();
  if (countCodePoints(t) < termMinChars) {
    return false;
  }
  const cues = /(?:即|也就是|比如|意味着|定义为|类比|如同|类似于|refers to|i\.e\.|e\.g\.)/i.test(t);
  const hanRuns = (t.match(/[\u4e00-\u9fff]{2,}/g) ?? []).length;
  const camelWords = (t.match(/\b[A-Za-z]*[a-z]+[A-Z][A-Za-z0-9]*\b/g) ?? []).length;
  const capsWords = (t.match(/\b[A-Z][a-z]{3,}\b/g) ?? []).length;
  const termish = hanRuns + camelWords + capsWords;
  const denom = Math.max(18, Math.floor(countCodePoints(t) / 2));
  const ratio = termish / denom;
  return ratio > 0.2 && !cues;
}

export type WritingCoachTriggerPayload = {
  paragraphText: string;
  /** 相对 host 容器内容区的垂直锚点（px） */
  anchorTopPx: number;
};

/**
 * 轮询 ProseMirror：停顿达到配置时长且（深度写作或术语密度）满足时触发一次，直到文档指纹变化重新武装。
 */
export function useWritingCoachTrigger(opts: {
  hostRef: RefObject<HTMLElement | null>;
  getEditorView: () => EditorView | null;
  docKey: string | null;
  disabled: boolean;
  /** 无编辑停顿毫秒数 */
  idleMs: number;
  /** 深度写作触发：当前块最少码点 */
  depthMinChars: number;
  /** 术语密度检测：段落最少码点 */
  termMinChars: number;
  onFire: (payload: WritingCoachTriggerPayload) => void;
}): void {
  const { hostRef, getEditorView, docKey, disabled, idleMs, depthMinChars, termMinChars, onFire } = opts;
  const onFireRef = useRef(onFire);
  onFireRef.current = onFire;
  const fpRef = useRef("");
  const lastChangeRef = useRef(0);
  const armedRef = useRef(true);

  useEffect(() => {
    fpRef.current = "";
    lastChangeRef.current = Date.now();
    armedRef.current = true;
  }, [docKey]);

  useEffect(() => {
    if (disabled || !docKey) {
      return;
    }
    // bubble/panel 展示时 disabled 会 teardown interval；恢复后若 armed 仍为 false 且 fp 未变，将永远不触发
    armedRef.current = true;
    fpRef.current = "";
    lastChangeRef.current = Date.now();
    const safeIdleMs = Number.isFinite(idleMs) && idleMs > 0 ? idleMs : 15_000;
    const id = window.setInterval(() => {
      const view = getEditorView();
      const host = hostRef.current;
      if (!view || !host) {
        return;
      }
      const st = view.state;
      if (selectionInsideThoughtCallout(st)) {
        return;
      }
      const fp = `${st.doc.content.size}:${st.selection.from}:${st.selection.to}`;
      const now = Date.now();
      if (fp !== fpRef.current) {
        fpRef.current = fp;
        lastChangeRef.current = now;
        armedRef.current = true;
        return;
      }
      if (!armedRef.current) {
        return;
      }
      if (now - lastChangeRef.current < safeIdleMs) {
        return;
      }
      const blockText = nearestTextblockText(st);
      const hit =
        depthWritingTrigger(blockText, depthMinChars) || termDensityTrigger(blockText, termMinChars);
      if (!hit) {
        return;
      }
      const $from = st.selection.$from;
      const sideCoords = view.coordsAtPos($from.pos, -1);
      const hostRect = host.getBoundingClientRect();
      const anchorTopPx = sideCoords.top - hostRect.top + host.scrollTop;
      onFireRef.current({ paragraphText: blockText, anchorTopPx });
      armedRef.current = false;
    }, POLL_MS);
    return () => {
      window.clearInterval(id);
    };
  }, [disabled, docKey, depthMinChars, getEditorView, hostRef, idleMs, termMinChars]);
}
