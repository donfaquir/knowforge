/**
 * 在 language=mermaid 的 code_block 下方渲染 Mermaid 预览（Decoration.widget）。
 */
import { $prose } from "@milkdown/utils";
import type { Node as PmNode } from "@milkdown/prose/model";
import { Plugin, PluginKey } from "@milkdown/prose/state";
import { Decoration, DecorationSet } from "@milkdown/prose/view";

let mermaidReady: Promise<typeof import("mermaid").default> | null = null;

/** 连续编辑时合并重建，避免每个 transaction 都跑 buildDecorations + mermaid.run */
const MERMAID_PREVIEW_DEBOUNCE_MS = 320;
const MERMAID_FLUSH_META = "kf-mermaid-flush";

function loadMermaid(): Promise<typeof import("mermaid").default> {
  if (!mermaidReady) {
    mermaidReady = import("mermaid").then(({ default: mermaid }) => {
      const dark =
        typeof document !== "undefined" &&
        document.documentElement.getAttribute("data-theme") === "dark";
      mermaid.initialize({
        startOnLoad: false,
        securityLevel: "strict",
        theme: dark ? "dark" : "neutral",
      });
      return mermaid;
    });
  }
  return mermaidReady;
}

function isMermaidBlock(node: PmNode): boolean {
  return node.type.name === "code_block" && String(node.attrs.language || "").toLowerCase() === "mermaid";
}

/** 与 Decoration.widget destroy 配合，撤销尚未完成的 mermaid.run */
type MermaidPreviewWrap = HTMLElement & { __kfMermaidCancel?: () => void };

function buildDecorations(doc: PmNode): DecorationSet {
  const decos: Decoration[] = [];
  doc.descendants((node, pos) => {
    if (!isMermaidBlock(node)) {
      return;
    }
    const source = node.textContent;
    const widgetPos = pos + node.nodeSize;
    decos.push(
      Decoration.widget(
        widgetPos,
        () => {
          const wrap = document.createElement("div") as MermaidPreviewWrap;
          wrap.className = "kf-mermaid-preview";
          const inner = document.createElement("div");
          inner.className = "mermaid";
          inner.textContent = source;
          wrap.appendChild(inner);

          let cancelled = false;
          wrap.__kfMermaidCancel = () => {
            cancelled = true;
          };

          void loadMermaid().then(async (mermaid) => {
            if (cancelled || !inner.isConnected) {
              return;
            }
            try {
              await mermaid.run({ nodes: [inner], suppressErrors: true });
            } catch {
              if (!cancelled && inner.isConnected) {
                inner.textContent = "Mermaid render failed.";
              }
            }
          });

          return wrap;
        },
        {
          side: 1,
          key: `kf-mmd-${pos}`,
          // 文档变更卸载 widget 时调用，避免旧异步仍操作已摘除的 DOM
          destroy(dom: Node) {
            (dom as MermaidPreviewWrap).__kfMermaidCancel?.();
          },
        },
      ),
    );
  });
  return DecorationSet.create(doc, decos);
}

export const mermaidPreviewPlugin = $prose(() => {
  const key = new PluginKey("kf-mermaid-preview");
  return new Plugin({
    key,
    state: {
      init(_, { doc }) {
        return buildDecorations(doc);
      },
      apply(tr, oldSet, _oldState, newState) {
        // 防抖到期：整表重建，使 code_block 内新正文反映到预览
        if (tr.getMeta(key) === MERMAID_FLUSH_META) {
          return buildDecorations(newState.doc);
        }
        // 其余 transaction：仅映射装饰位置，避免每次按键都销毁/新建全部 widget
        return oldSet.map(tr.mapping, newState.doc);
      },
    },
    props: {
      decorations(state) {
        return this.getState(state);
      },
    },
    view(editorView) {
      let debounceTimer: ReturnType<typeof setTimeout> | null = null;
      return {
        update(_view, prevState) {
          if (editorView.state.doc.eq(prevState.doc)) {
            return;
          }
          if (debounceTimer !== null) {
            clearTimeout(debounceTimer);
          }
          debounceTimer = window.setTimeout(() => {
            debounceTimer = null;
            try {
              editorView.dispatch(editorView.state.tr.setMeta(key, MERMAID_FLUSH_META));
            } catch {
              // 视图已销毁等：忽略
            }
          }, MERMAID_PREVIEW_DEBOUNCE_MS);
        },
        destroy() {
          if (debounceTimer !== null) {
            clearTimeout(debounceTimer);
            debounceTimer = null;
          }
        },
      };
    },
  });
});
