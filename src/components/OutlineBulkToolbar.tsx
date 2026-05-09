import { useTranslation } from "react-i18next";
import { FolderBulkToggleIcon } from "./treeCollapseIcons";

type Props = {
  outlineHasBranches: boolean;
  allOutlineBranchesExpanded: boolean;
  onToggleBulk: () => void;
};

/** 与右栏顶栏同一行右侧：全部展开/折叠大纲 */
export function OutlineBulkToolbar({
  outlineHasBranches,
  allOutlineBranchesExpanded,
  onToggleBulk,
}: Props) {
  const { t } = useTranslation();
  if (!outlineHasBranches) {
    return null;
  }

  return (
    <div className="outline-bulk-toolbar">
      <button
        type="button"
        className="file-tree__bulk-toggle"
        onClick={onToggleBulk}
        aria-label={
          allOutlineBranchesExpanded ? t("outline.collapseAll") : t("outline.expandAll")
        }
        title={
          allOutlineBranchesExpanded
            ? t("outline.collapseAllTitle")
            : t("outline.expandAllTitle")
        }
      >
        <FolderBulkToggleIcon allExpanded={allOutlineBranchesExpanded} />
      </button>
    </div>
  );
}
