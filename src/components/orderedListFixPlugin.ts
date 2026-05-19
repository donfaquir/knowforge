import { $prose } from '@milkdown/utils'
import { Plugin, PluginKey } from '@milkdown/prose/state'

/**
 * 有序列表 "N." 自动插入 bug 防御插件。
 *
 * 根因：ProseMirror 的 readDOMChange（isGeneric: true 事务）在 Enter 换行后
 * 通过 MutationObserver 将 label-wrapper 区域的序号文字误读为段落内容并插入文档。
 *
 * 本插件通过 filterTransaction 拦截满足以下全部条件的事务并拒绝：
 *   1. isGeneric: true（DOM mutation 驱动，非用户命令）
 *   2. 含 ReplaceStep 纯插入（from === to）
 *   3. 插入内容匹配 /^\d+\.$/ （如 "1." "2." "3."）
 *   4. 插入位置在空 paragraph（content.size === 0）内
 *   5. 该 paragraph 的父节点是 list_item
 */

const ORDERED_LABEL_RE = /^\d+\.$/

export const orderedListFixPlugin = $prose(() =>
  new Plugin({
    key: new PluginKey('ordered-list-fix'),
    filterTransaction(tr, state) {
      // 仅拦截 DOM-mutation 驱动的事务（readDOMChange 标志）
      if (!(tr as any).isGeneric || !tr.docChanged) return true

      for (const step of tr.steps) {
        const s = step as any
        if (s.constructor?.name !== 'ReplaceStep') continue
        if (s.from !== s.to) continue // 仅拦截纯插入（非替换）

        const text: string =
          s.slice?.content?.textBetween?.(0, s.slice?.content?.size ?? 0) ?? ''
        if (!ORDERED_LABEL_RE.test(text)) continue // 仅拦截 "N." 格式

        try {
          const $from = state.doc.resolve(s.from)
          if (
            $from.parent.type.name === 'paragraph' &&
            $from.parent.content.size === 0 && // 段落为空（Enter 后新行）
            $from.node(-1)?.type.name === 'list_item'
          ) {
            return false // 拒绝本事务
          }
        } catch {
          // 位置解析失败时不拦截，保守处理
        }
      }
      return true
    },
  })
)
