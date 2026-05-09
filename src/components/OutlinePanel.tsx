import { useTranslation } from "react-i18next";
import type { OutlineFoldModel } from "../hooks/useOutlineFoldModel";
import { DirChevronIcon } from "./treeCollapseIcons";

type Props = {
  fold: OutlineFoldModel;
  onNavigate: (slug: string) => void;
  characterCount: number;
  tauriDragRegion?: boolean;
};

/**
 * 大纲列表与字数条；折叠工具条由 RightPanelShell 顶栏与视图切换同一行展示。
 */
export function OutlinePanel({ fold, onNavigate, characterCount, tauriDragRegion }: Props) {
  const { t, i18n } = useTranslation();
  const {
    items,
    hasChildren,
    collapsedHeadings,
    visibleRows,
    indentBaseLevel,
    toggleHeadingCollapse,
  } = fold;

  const dragProps = tauriDragRegion
    ? ({ "data-tauri-drag-region": true } as const)
    : {};

  return (
    <aside className="outline-panel" aria-label={t("rightPanel.outline")} {...dragProps}>
      <nav className="outline-panel__nav outline-panel__nav--flush-top">
        {items.length === 0 ? (
          <p className="outline-panel__empty">{t("outline.empty")}</p>
        ) : (
          <ul className="outline-panel__list">
            {visibleRows.map(({ index, item }) => {
              const showTwisty = hasChildren[index];
              const branchExpanded = !collapsedHeadings.has(index);
              const rowKind = showTwisty
                ? "outline-panel__row--branch"
                : "outline-panel__row--leaf";

              return (
                <li key={`${index}-${item.slug}`} className="outline-panel__item">
                  <div
                    className={`outline-panel__row ${rowKind}`}
                    style={{ paddingLeft: `${6 + (item.level - indentBaseLevel) * 14}px` }}
                  >
                    {showTwisty ? (
                      <button
                        type="button"
                        className="file-tree__twisty"
                        onClick={(e) => {
                          e.preventDefault();
                          e.stopPropagation();
                          toggleHeadingCollapse(index);
                        }}
                        aria-expanded={branchExpanded}
                        aria-label={
                          branchExpanded ? t("outline.collapseSubsection") : t("outline.expandSubsection")
                        }
                        title={branchExpanded ? t("fileTree.collapse") : t("fileTree.expand")}
                      >
                        <DirChevronIcon expanded={branchExpanded} />
                      </button>
                    ) : (
                      <span className="file-tree__twisty-spacer" aria-hidden />
                    )}
                    <button
                      type="button"
                      className="outline-panel__link"
                      onClick={() => onNavigate(item.slug)}
                      title={item.text}
                    >
                      {item.text}
                    </button>
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </nav>
      <footer
        className="outline-panel__stats-bar"
        aria-label={t("outline.stats")}
        aria-live="polite"
        aria-atomic="true"
      >
        <span className="outline-panel__stats-label">{t("outline.characters")}</span>
        <span className="outline-panel__stats-value" title={`${characterCount}`}>
          {characterCount.toLocaleString(i18n.language.startsWith("zh") ? "zh-CN" : "en-US")}
        </span>
      </footer>
    </aside>
  );
}
