/**
 * 轻量命令面板：模糊匹配命令项（迭代 4 认知报告入口）。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import "./CommandPalette.css";

type CommandItem = {
  id: string;
  label: string;
  keywords: string;
  onSelect: () => void;
};

type Props = {
  open: boolean;
  onClose: () => void;
  onOpenCognitiveReport: () => void;
  onOpenThoughtVaultHub?: () => void;
  onOpenWorkspaceSearch?: () => void;
  onTriggerWritingCoach?: () => void;
  onStartOnboarding?: () => void;
};

export function CommandPalette({
  open,
  onClose,
  onOpenCognitiveReport,
  onOpenThoughtVaultHub,
  onOpenWorkspaceSearch,
  onTriggerWritingCoach,
  onStartOnboarding,
}: Props) {
  const { t } = useTranslation();
  const [q, setQ] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  // 父组件常传入内联函数；用 ref 保持最新实现，避免 items 的 useMemo 每帧失效
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  const onOpenCognitiveReportRef = useRef(onOpenCognitiveReport);
  onOpenCognitiveReportRef.current = onOpenCognitiveReport;
  const onOpenThoughtVaultHubRef = useRef(onOpenThoughtVaultHub);
  onOpenThoughtVaultHubRef.current = onOpenThoughtVaultHub;
  const onOpenWorkspaceSearchRef = useRef(onOpenWorkspaceSearch);
  onOpenWorkspaceSearchRef.current = onOpenWorkspaceSearch;
  const onTriggerWritingCoachRef = useRef(onTriggerWritingCoach);
  onTriggerWritingCoachRef.current = onTriggerWritingCoach;
  const onStartOnboardingRef = useRef(onStartOnboarding);
  onStartOnboardingRef.current = onStartOnboarding;

  const hasThoughtVaultHub = onOpenThoughtVaultHub != null;
  const hasWorkspaceSearch = onOpenWorkspaceSearch != null;
  const hasWritingCoach = onTriggerWritingCoach != null;
  const hasOnboarding = onStartOnboarding != null;

  const items: CommandItem[] = useMemo(() => {
    const base: CommandItem[] = [
      {
        id: "cognitive-report",
        label: t("commandPalette.cognitiveReport"),
        keywords: "认知成长报告 cognitive growth report stats",
        onSelect: () => {
          onOpenCognitiveReportRef.current();
          onCloseRef.current();
        },
      },
    ];
    if (hasThoughtVaultHub) {
      base.push({
        id: "thought-vault-hub",
        label: t("commandPalette.thoughtVaultHub"),
        keywords: "想法 随手 thoughts vault sqlite 全库",
        onSelect: () => {
          onOpenThoughtVaultHubRef.current?.();
          onCloseRef.current();
        },
      });
    }
    if (hasWorkspaceSearch) {
      base.push({
        id: "workspace-search",
        label: t("commandPalette.workspaceSearch"),
        keywords: "全文搜索 vault search grep 关键词 查找",
        onSelect: () => {
          onOpenWorkspaceSearchRef.current?.();
          onCloseRef.current();
        },
      });
    }
    if (hasWritingCoach) {
      base.push({
        id: "writing-coach",
        label: t("commandPalette.writingCoach"),
        keywords: "写作教练 writing coach 逻辑追问 reasoning",
        onSelect: () => {
          onTriggerWritingCoachRef.current?.();
          onCloseRef.current();
        },
      });
    }
    if (hasOnboarding) {
      base.push({
        id: "onboarding",
        label: t("commandPalette.onboarding"),
        keywords: "引导 新手 onboarding guide tutorial 教程",
        onSelect: () => {
          onStartOnboardingRef.current?.();
          onCloseRef.current();
        },
      });
    }
    return base;
  }, [t, hasThoughtVaultHub, hasWorkspaceSearch, hasWritingCoach, hasOnboarding]);

  const filtered = useMemo(() => {
    const s = q.trim().toLowerCase();
    if (!s) return items;
    return items.filter(
      (it) =>
        it.label.toLowerCase().includes(s) ||
        it.keywords.toLowerCase().includes(s) ||
        it.id.includes(s),
    );
  }, [items, q]);

  useEffect(() => {
    if (open) {
      setQ("");
      queueMicrotask(() => inputRef.current?.focus());
    }
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onCloseRef.current();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  const onBackdropMouseDown = useCallback((e: React.MouseEvent) => {
    if (e.target === e.currentTarget) onCloseRef.current();
  }, []);

  if (!open) {
    return null;
  }

  return (
    <div
      className="command-palette-backdrop"
      role="presentation"
      onMouseDown={onBackdropMouseDown}
    >
      <div className="command-palette" role="dialog" aria-label={t("commandPalette.title")}>
        <input
          ref={inputRef}
          className="command-palette__input"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder={t("commandPalette.placeholder")}
          aria-label={t("commandPalette.placeholder")}
        />
        <ul className="command-palette__list">
          {filtered.map((it) => (
            <li key={it.id}>
              <button type="button" className="command-palette__item" onClick={() => it.onSelect()}>
                {it.label}
              </button>
            </li>
          ))}
        </ul>
        <div className="command-palette__hint">{t("commandPalette.footerHint")}</div>
      </div>
    </div>
  );
}
