/**
 * ThoughtSavePopover — 将 AI 深化回答保存为理解区块（thought）到指定笔记。
 * Phase 4E: 笔记选择器 + 临时复选框 + 保存/取消。
 */

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { insertThoughtToNote } from "../utils/thoughtFrontmatterEdit";
import "./ThoughtSavePopover.css";

type TreeNode = { name: string; rel_path: string; children?: TreeNode[] };

/** 迭代 DFS：顺序与原先递归版一致，避免极深目录树占用调用栈。 */
function flattenMarkdownPaths(nodes: TreeNode[]): string[] {
  const out: string[] = [];
  const stack: TreeNode[] = [];
  for (let i = nodes.length - 1; i >= 0; i--) {
    stack.push(nodes[i]);
  }
  while (stack.length > 0) {
    const n = stack.pop();
    if (n === undefined) continue;
    // 与递归版一致：`[]` 仍为真值，表示空目录，不写入 rel_path
    if (n.children) {
      for (let i = n.children.length - 1; i >= 0; i--) {
        stack.push(n.children[i]);
      }
    } else {
      out.push(n.rel_path);
    }
  }
  return out;
}

type Props = {
  content: string;
  defaultRelPath: string | null;
  isSelection?: boolean;
  onSaved: () => void;
  onCancel: () => void;
  /** 被动高亮入口：展示「不准确」反馈 */
  variant?: "default" | "passive";
  onMarkInaccurate?: () => void | Promise<void>;
};

export function ThoughtSavePopover({
  content,
  defaultRelPath,
  isSelection,
  onSaved,
  onCancel,
  variant = "default",
  onMarkInaccurate,
}: Props) {
  const { t } = useTranslation();
  const popoverRef = useRef<HTMLDivElement>(null);

  const [allFiles, setAllFiles] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);
  const [filterText, setFilterText] = useState(defaultRelPath ?? "");
  const [selectedFile, setSelectedFile] = useState(defaultRelPath ?? "");
  const [dropdownOpen, setDropdownOpen] = useState(false);
  const [temporary, setTemporary] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // 挂载时加载文件列表
  useEffect(() => {
    let cancelled = false;
    void invoke<TreeNode[]>("refresh_md_tree")
      .then((tree) => {
        if (cancelled) return;
        setAllFiles(flattenMarkdownPaths(tree));
        setLoading(false);
      })
      .catch(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // 点击外部 / Escape 关闭
  useEffect(() => {
    const onMouseDown = (e: MouseEvent) => {
      if (popoverRef.current && !popoverRef.current.contains(e.target as Node)) {
        onCancel();
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onCancel();
      }
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKeyDown, true);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKeyDown, true);
    };
  }, [onCancel]);

  const filteredFiles =
    filterText.trim().length === 0
      ? allFiles
      : allFiles.filter((f) => f.toLowerCase().includes(filterText.toLowerCase()));

  const handleInputChange = useCallback((value: string) => {
    setFilterText(value);
    setSelectedFile(value);
    setDropdownOpen(true);
    setError(null);
  }, []);

  const handleSelectFile = useCallback((path: string) => {
    setFilterText(path);
    setSelectedFile(path);
    setDropdownOpen(false);
    setError(null);
  }, []);

  const handleSave = useCallback(async () => {
    const target = selectedFile.trim();
    if (!target) {
      setError(t("thoughtSave.noFile"));
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await insertThoughtToNote({ relPath: target, content, temporary });
      onSaved();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setSaving(false);
    }
  }, [selectedFile, content, temporary, onSaved, t]);

  const canSave = selectedFile.trim().length > 0 && !saving;

  const popover = (
    <div className="thought-save-popover" ref={popoverRef} role="dialog" aria-label={t("thoughtSave.title")}>
      <div className="thought-save-popover__header">{t(isSelection ? "thoughtSave.titleSelection" : "thoughtSave.title")}</div>
      <hr className="thought-save-popover__divider" />

      <div className="thought-save-popover__section">
        <div className="thought-save-popover__label">{t("thoughtSave.targetNote")}</div>
        <div className="thought-save-popover__file-field">
          <svg className="thought-save-popover__search-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
            <circle cx="11" cy="11" r="8" />
            <path d="m21 21-4.3-4.3" />
          </svg>
          <input
            className="thought-save-popover__file-input"
            type="text"
            value={filterText}
            onChange={(e) => handleInputChange(e.target.value)}
            onFocus={() => setDropdownOpen(true)}
            onBlur={() => setDropdownOpen(false)}
            placeholder={t("thoughtSave.filePlaceholder")}
            disabled={saving}
          />
          {dropdownOpen && !loading && filteredFiles.length > 0 && (
            <div className="thought-save-popover__dropdown">
              {filteredFiles.map((f) => (
                <button
                  key={f}
                  type="button"
                  className={`thought-save-popover__dropdown-item${f === selectedFile ? " is-selected" : ""}`}
                  onMouseDown={(e) => {
                    e.preventDefault();
                    handleSelectFile(f);
                  }}
                >
                  {f}
                </button>
              ))}
            </div>
          )}
          {dropdownOpen && loading && (
            <div className="thought-save-popover__dropdown">
              <span className="thought-save-popover__dropdown-loading">...</span>
            </div>
          )}
        </div>
      </div>

      <div className="thought-save-popover__options-bar">
        <label className="thought-save-popover__options">
          <input
            type="checkbox"
            checked={temporary}
            onChange={(e) => setTemporary(e.target.checked)}
            disabled={saving}
          />
          <span>{t("thoughtSave.temporary")}</span>
        </label>
      </div>

      {error && <div className="thought-save-popover__error" role="alert">{error}</div>}

      <div className="thought-save-popover__actions">
        {variant === "passive" && onMarkInaccurate ? (
          <button
            type="button"
            className="thought-save-popover__btn thought-save-popover__btn--inaccurate"
            onClick={() => void onMarkInaccurate()}
            disabled={saving}
          >
            {t("thoughtSave.markInaccurate")}
          </button>
        ) : <div />}
        <div className="thought-save-popover__actions-end">
          <button
            type="button"
            className="thought-save-popover__btn thought-save-popover__btn--cancel"
            onClick={onCancel}
            disabled={saving}
          >
            {t("thoughtSave.cancel")}
          </button>
          <button
            type="button"
            className="thought-save-popover__btn thought-save-popover__btn--save"
            disabled={!canSave}
            onClick={() => void handleSave()}
          >
            {saving ? "..." : t("thoughtSave.save")}
          </button>
        </div>
      </div>
    </div>
  );

  return createPortal(popover, document.body);
}
