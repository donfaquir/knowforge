import { Crepe, CrepeFeature } from "@milkdown/crepe";
import { crepeCodeMirrorLanguages } from "../config/crepeCodeMirrorLanguages";
import { Milkdown, MilkdownProvider, useEditor } from "@milkdown/react";
import { $remark, getMarkdown, replaceAll } from "@milkdown/utils";
import remarkBreaks from "remark-breaks";
import type { EditorView } from "@milkdown/prose/view";
import { TextSelection } from "@milkdown/prose/state";
import { editorViewCtx } from "@milkdown/kit/core";
import { useCallback, useEffect, useLayoutEffect, useRef } from "react";
import { mermaidPreviewPlugin } from "./mermaidPreviewPlugin";
import { configureRemarkStringifyPreserveWikilink } from "./remarkStringifyPreserveWikilink";
import { thoughtCalloutPlugin } from "./thoughtCalloutPlugin";
import { wikiLinkDecoratePlugin } from "./wikiLinkDecoratePlugin";
import { wikiLinkSuggestPlugin } from "./wikiLinkSuggestPlugin";
import { WikiLinkSuggestPopover } from "./WikiLinkSuggestPopover";
import { wikiLinkContextRef } from "./wikiLinkContext";
import {
  editorAutosaveBridgeRef,
  type EditorAutosaveBridgeContext,
} from "./editorDocAutosaveBridgeContext";
import { editorDocAutosaveBridgePlugin } from "./editorDocAutosaveBridgePlugin";
import {
  internalMarkdownLinkClickPlugin,
  internalMarkdownLinkOpenRef,
  type InternalMarkdownLinkOpenMeta,
} from "./internalMarkdownLinkClickPlugin";
import {
  dispatchFindMeta,
  editorFindHighlightPlugin,
  findScrollToCurrent,
  getFindMatchSummary,
} from "./editorFindHighlightPlugin";
import { appendRelatedNotesWikiLinkToMarkdown } from "../utils/appendRelatedNotesSection";
import { buildPreviewScrollNeedles, findFirstDocMatchForNeedles } from "../utils/previewScrollNeedle";
import { endPerfTrace, startPerfTrace } from "../utils/perfTrace";
import type { WikiSuggestFileRow } from "../utils/flattenMarkdownTreeForWikiSuggest";

// 段落内单次换行按硬换行解析（CommonMark 默认会合并为空格）
const remarkBreaks$ = $remark("remark-breaks", () => remarkBreaks);

// frame：主题变量；common：列表、代码块、ProseMirror 等组件样式（仅 frame 会导致列表/代码块无样式）
import "@milkdown/crepe/theme/frame.css";
import "@milkdown/crepe/theme/common/style.css";
import "./CrepeMarkdownEditor.css";

/**
 * 修复中文输入法 composition 期间页面异常滚动
 * 在 compositionstart 到 compositionend 期间禁用 scrollIntoView
 */
function setupCompositionScrollFix(rootElement: HTMLElement) {
  let isComposing = false;

  const handleCompositionStart = () => {
    isComposing = true;
  };

  const handleCompositionEnd = () => {
    isComposing = false;
  };

  const handleBeforeInput = (e: Event) => {
    if (isComposing) {
      // 在 composition 期间阻止可能导致滚动的默认行为
      const inputEvent = e as InputEvent;
      if (inputEvent.inputType?.includes("insertCompositionText")) {
        // 保存当前滚动位置
        /* 正文滚动在 data-milkdown-root 上，与 CrepeMarkdownEditor.css 一致 */
        const scrollContainer =
          (rootElement.closest("[data-milkdown-root]") as HTMLElement | null) ?? rootElement;
        if (scrollContainer) {
          const scrollTop = scrollContainer.scrollTop;
          // 使用 requestAnimationFrame 恢复滚动位置
          requestAnimationFrame(() => {
            if (scrollContainer.scrollTop !== scrollTop) {
              scrollContainer.scrollTop = scrollTop;
            }
          });
        }
      }
    }
  };

  rootElement.addEventListener("compositionstart", handleCompositionStart);
  rootElement.addEventListener("compositionend", handleCompositionEnd);
  rootElement.addEventListener("beforeinput", handleBeforeInput);

  return () => {
    rootElement.removeEventListener("compositionstart", handleCompositionStart);
    rootElement.removeEventListener("compositionend", handleCompositionEnd);
    rootElement.removeEventListener("beforeinput", handleBeforeInput);
  };
}

/** Milkdown 就绪后供写作教练等挂载 ProseMirror 视图访问 */
export type CrepeMarkdownEditorApi = {
  getEditorView: () => EditorView | null;
  /** 当前缓冲区序列化 Markdown（可能与磁盘未保存差异）；编辑器不可用时 null */
  getCurrentMarkdown: () => string | null;
  /** 在当前选区插入文本（替换选区）；视图不可用时返回 false */
  insertTextAtCursor: (text: string) => boolean;
  /**
   * 在文末「## 相关笔记」小节追加 `- [[wikiInner]]`（无则创建小节）；已含同链接则 false。
   * 通过 replaceAll 注入；须走普通 onMarkdownChange（勿用 baseline），否则不会触发自动保存、重开仍见旧文。
   */
  appendRelatedNotesWikiLinkLine: (wikiInner: string) => boolean;
  findSetQuery: (query: string, caseSensitive: boolean) => void;
  findNext: () => void;
  findPrev: () => void;
  findClear: () => void;
  /** 按缓冲区全文行号在预览中定位（与全文搜索行号一致；命中 frontmatter 时落到正文首段） */
  scrollToSourceLineFromFullMarkdown: (fullMarkdown: string, line1Based: number) => boolean;
  /** 篇内查找：总命中数与当前序号（1-based），无查找词时 null */
  getFindMatchSummary: () => { total: number; current: number; truncated?: boolean } | null;
};

type Props = {
  /** 当前打开的文件路径；切换时仅替换文档，不销毁编辑器 */
  docKey: string;
  /** 与 docKey 对应的 Markdown 正文（切换标签时用 replaceAll 注入） */
  initialMarkdown: string;
  /** 磁盘重载等场景递增，docKey 不变时仍强制注入 initialMarkdown */
  contentSyncKey?: number;
  /**
   * baseline：replaceAll 后立即用注入正文对齐 saved，避免误报未保存。
   * 换文使用 replaceAll(..., false)：事务替换 doc，避免 flush 时整份 EditorState 重建带来的主线程开销。
   */
  onMarkdownChange: (
    relPath: string,
    markdown: string,
    meta?: { baseline?: boolean; fullDocument?: boolean },
  ) => void;
  onEditorReady?: (api: CrepeMarkdownEditorApi) => void;
  /** 编辑器销毁或换文导致视图不可用时通知宿主清理 */
  onEditorDispose?: () => void;
  /** 正文内 Vault 相对 `.md` 链接左键打开（wikilink 与普通 Markdown 链接共用） */
  onOpenInternalMarkdownLink?: (
    relPath: string,
    meta?: InternalMarkdownLinkOpenMeta,
  ) => void | Promise<void>;
  /** 输入 `[[` 时 wikilink 候选列表（工作区 Markdown 叶子）；默认空 */
  wikiSuggestFiles?: WikiSuggestFileRow[];
};

function CrepeInner({
  docKey,
  initialMarkdown,
  contentSyncKey = 0,
  onMarkdownChange,
  onEditorReady,
  onEditorDispose,
  onOpenInternalMarkdownLink,
  wikiSuggestFiles = [],
}: Props) {
  const docKeyRef = useRef(docKey);
  docKeyRef.current = docKey;
  const markdownRef = useRef(initialMarkdown);
  markdownRef.current = initialMarkdown;
  const onChangeRef = useRef(onMarkdownChange);
  onChangeRef.current = onMarkdownChange;
  const onEditorReadyRef = useRef(onEditorReady);
  onEditorReadyRef.current = onEditorReady;
  const onEditorDisposeRef = useRef(onEditorDispose);
  onEditorDisposeRef.current = onEditorDispose;
  /** layout 阶段 get() 仍为空时，交给 effect + rAF 重试注入（关标签再开时易触发） */
  const needInjectRetryRef = useRef(false);
  /** replaceAll 与 baseline 完成前丢弃 markdownUpdated，避免先改 content 再 baseline 时 clean=false 永久脏标记 */
  const suppressMarkdownUntilBaselineRef = useRef(true);
  /** 与 queueMicrotask 配对，防止快速连续换文时误解除抑制 */
  const injectGenRef = useRef(0);
  /** composition 滚动修复清理函数 */
  const compositionCleanupRef = useRef<(() => void) | null>(null);

  useLayoutEffect(() => {
    internalMarkdownLinkOpenRef.current = onOpenInternalMarkdownLink ?? null;
    return () => {
      internalMarkdownLinkOpenRef.current = null;
    };
  }, [onOpenInternalMarkdownLink]);

  const { get, loading } = useEditor(
    (root) => {
      const crepe = new Crepe({
        root,
        defaultValue: "",
        features: {
          [CrepeFeature.Latex]: false,
        },
        featureConfigs: {
          [CrepeFeature.CodeMirror]: {
            languages: crepeCodeMirrorLanguages,
          },
        },
      });
      crepe.editor.config(configureRemarkStringifyPreserveWikilink);
      crepe.editor.use(remarkBreaks$);
      crepe.editor.use(editorDocAutosaveBridgePlugin);
      crepe.editor.use(thoughtCalloutPlugin);
      crepe.editor.use(mermaidPreviewPlugin);
      crepe.editor.use(wikiLinkDecoratePlugin);
      crepe.editor.use(wikiLinkSuggestPlugin);
      crepe.editor.use(internalMarkdownLinkClickPlugin);
      crepe.editor.use(editorFindHighlightPlugin);
      crepe.on((listen) => {
        // markdownUpdated 在插件内 debounce 200ms，且部分程序化事务可能不触发；
        // 基线必须在 replaceAll 之后同步对齐，勿用「下一次 markdownUpdated 回调」推断。
        listen.markdownUpdated((_ctx, markdown) => {
          if (suppressMarkdownUntilBaselineRef.current) {
            return;
          }
          onChangeRef.current(docKeyRef.current, markdown);
        });
      });
      // Milkdown 可能在同一 root 上 destroy 后再 create；先清理上一轮监听，避免重复注册
      compositionCleanupRef.current?.();
      compositionCleanupRef.current = setupCompositionScrollFix(root);
      return crepe;
    },
    [],
  );

  // 清理 composition 事件监听
  useEffect(() => {
    return () => {
      if (compositionCleanupRef.current) {
        compositionCleanupRef.current();
        compositionCleanupRef.current = null;
      }
    };
  }, []);

  const getRef = useRef(get);
  getRef.current = get;

  useLayoutEffect(() => {
    const bridge: EditorAutosaveBridgeContext = {
      getMarkdownBody: () => {
        const ed = getRef.current?.();
        if (!ed) {
          return "";
        }
        try {
          return ed.action(getMarkdown());
        } catch {
          return "";
        }
      },
      getDocKey: () => docKeyRef.current,
      isSuppressBaseline: () => suppressMarkdownUntilBaselineRef.current,
      onDocBodyChange: (relPath: string, markdown: string) => {
        onChangeRef.current(relPath, markdown);
      },
    };
    editorAutosaveBridgeRef.current = bridge;
    return () => {
      if (editorAutosaveBridgeRef.current === bridge) {
        editorAutosaveBridgeRef.current = null;
      }
    };
  }, [docKey]);

  const getEditorViewForSuggest = useCallback(() => {
    const ed = getRef.current?.();
    if (!ed) {
      return null;
    }
    try {
      return ed.ctx.get(editorViewCtx);
    } catch {
      return null;
    }
  }, []);

  // 绘制前同步换文，避免 rAF 多等一帧且仍显示旧文档
  useLayoutEffect(() => {
    if (loading) {
      suppressMarkdownUntilBaselineRef.current = true;
      needInjectRetryRef.current = false;
      return;
    }
    const ed = get();
    if (!ed) {
      suppressMarkdownUntilBaselineRef.current = true;
      needInjectRetryRef.current = true;
      return;
    }
    needInjectRetryRef.current = false;
    const gen = ++injectGenRef.current;
    suppressMarkdownUntilBaselineRef.current = true;
    wikiLinkContextRef.currentRelPath = docKeyRef.current;
    const injectTrace = startPerfTrace("markdown.crepe.replace_all", {
      relPath: docKeyRef.current,
      chars: markdownRef.current.length,
      contentSyncKey: contentSyncKey ?? 0,
      replaceAllFlush: false,
    });
    ed.action(replaceAll(markdownRef.current, false));
    endPerfTrace(injectTrace);
    // 换文首帧避免同步序列化整篇 Markdown，先用注入正文对齐 baseline，减少主线程阻塞
    onChangeRef.current(docKeyRef.current, markdownRef.current, { baseline: true });
    queueMicrotask(() => {
      if (injectGenRef.current === gen) {
        suppressMarkdownUntilBaselineRef.current = false;
      }
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps -- get() 引用不稳；docKey/loading/contentSyncKey 驱动换文
  }, [docKey, loading, contentSyncKey]);

  // 通知宿主：Crepe 视图就绪（写作教练等依赖 ProseMirror）；get() 偶晚一帧故用 rAF 重试
  useEffect(() => {
    if (loading) {
      onEditorDisposeRef.current?.();
      return;
    }
    let cancelled = false;
    let frames = 0;
    const tryReady = () => {
      if (cancelled) {
        return;
      }
      const ed = getRef.current?.();
      if (ed && onEditorReadyRef.current) {
        const getEditorView = (): EditorView | null => {
          const current = getRef.current?.();
          if (!current) {
            return null;
          }
          try {
            return current.ctx.get(editorViewCtx);
          } catch {
            return null;
          }
        };
        const api: CrepeMarkdownEditorApi = {
          getEditorView,
          getCurrentMarkdown: (): string | null => {
            const ed = getRef.current?.();
            if (!ed) {
              return null;
            }
            try {
              return ed.action(getMarkdown());
            } catch {
              return null;
            }
          },
          insertTextAtCursor: (text: string) => {
            const v = getEditorView();
            if (!v || !text) {
              return false;
            }
            const { state } = v;
            const tr = state.tr.insertText(text);
            v.dispatch(tr.scrollIntoView());
            return true;
          },
          appendRelatedNotesWikiLinkLine: (wikiInner: string) => {
            const ed = getRef.current?.();
            const inner = wikiInner.trim();
            if (!ed || !inner) {
              return false;
            }
            let md = "";
            try {
              md = ed.action(getMarkdown());
            } catch {
              return false;
            }
            const next = appendRelatedNotesWikiLinkToMarkdown(md, inner);
            if (!next) {
              return false;
            }
            const gen = ++injectGenRef.current;
            suppressMarkdownUntilBaselineRef.current = true;
            ed.action(replaceAll(next, false));
            // 非 baseline：与键盘输入一致，更新 content 且触发 scheduleAutoSave；baseline 会跳过持久化并可能误抬 savedContent
            onChangeRef.current(docKeyRef.current, next);
            queueMicrotask(() => {
              if (injectGenRef.current === gen) {
                suppressMarkdownUntilBaselineRef.current = false;
              }
            });
            return true;
          },
          findSetQuery: (query, caseSensitive) => {
            const v = getEditorView();
            if (!v) {
              return;
            }
            if (!query.trim()) {
              dispatchFindMeta(v, { type: "clear" });
              return;
            }
            dispatchFindMeta(v, { type: "set", query, caseSensitive });
            findScrollToCurrent(v);
          },
          findNext: () => {
            const v = getEditorView();
            if (!v) {
              return;
            }
            dispatchFindMeta(v, { type: "next" });
            findScrollToCurrent(v);
          },
          findPrev: () => {
            const v = getEditorView();
            if (!v) {
              return;
            }
            dispatchFindMeta(v, { type: "prev" });
            findScrollToCurrent(v);
          },
          findClear: () => {
            const v = getEditorView();
            if (!v) {
              return;
            }
            dispatchFindMeta(v, { type: "clear" });
          },
          scrollToSourceLineFromFullMarkdown: (fullMarkdown: string, line1Based: number) => {
            const v = getEditorView();
            if (!v) {
              return false;
            }
            const needles = buildPreviewScrollNeedles(fullMarkdown, line1Based);
            if (needles.length === 0) {
              return false;
            }
            const pos = findFirstDocMatchForNeedles(v.state.doc, needles);
            if (pos == null) {
              return false;
            }
            const tr = v.state.tr.setSelection(TextSelection.create(v.state.doc, pos)).scrollIntoView();
            v.dispatch(tr);
            return true;
          },
          getFindMatchSummary: () => {
            const v = getEditorView();
            if (!v) {
              return null;
            }
            return getFindMatchSummary(v);
          },
        };
        onEditorReadyRef.current(api);
        return;
      }
      frames += 1;
      if (frames < 90) {
        requestAnimationFrame(tryReady);
      }
    };
    const id = requestAnimationFrame(tryReady);
    return () => {
      cancelled = true;
      cancelAnimationFrame(id);
      onEditorDisposeRef.current?.();
    };
  }, [loading, docKey]);

  // Milkdown 在子组件 useEffect 里异步 create，可能出现 loading 已 false 但 get() 仍为空一帧；layout 已漏注入时在此补齐
  useEffect(() => {
    if (loading || !needInjectRetryRef.current) {
      return;
    }
    const keyWhenScheduled = docKey;
    let cancelled = false;
    let frames = 0;
    const maxFrames = 90;

    const tick = () => {
      if (cancelled || docKeyRef.current !== keyWhenScheduled) {
        return;
      }
      const ed = getRef.current?.();
      if (ed) {
        needInjectRetryRef.current = false;
        const gen = ++injectGenRef.current;
        suppressMarkdownUntilBaselineRef.current = true;
        wikiLinkContextRef.currentRelPath = docKeyRef.current;
        ed.action(replaceAll(markdownRef.current, false));
        // rAF 补注入路径与主路径保持一致，避免重复触发整篇序列化卡顿
        onChangeRef.current(docKeyRef.current, markdownRef.current, { baseline: true });
        queueMicrotask(() => {
          if (injectGenRef.current === gen) {
            suppressMarkdownUntilBaselineRef.current = false;
          }
        });
        return;
      }
      frames += 1;
      if (frames >= maxFrames) {
        return;
      }
      requestAnimationFrame(tick);
    };

    const id = requestAnimationFrame(tick);
    return () => {
      cancelled = true;
      cancelAnimationFrame(id);
    };
  }, [docKey, loading, contentSyncKey]);

  return (
    <>
      <Milkdown />
      <WikiLinkSuggestPopover getEditorView={getEditorViewForSuggest} wikiSuggestFiles={wikiSuggestFiles} />
    </>
  );
}

/**
 * Obsidian 风格：单页所见即所得（Milkdown Crepe），在渲染结果上直接编辑。
 * 仅单实例：多 Milkdown/Crepe 并行会触发 CodeMirror foldService 等 Facet 异常（如 service is not a function）。
 */
export function CrepeMarkdownEditor(props: Props) {
  return (
    <MilkdownProvider>
      <CrepeInner {...props} />
    </MilkdownProvider>
  );
}

export default CrepeMarkdownEditor;
