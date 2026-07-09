import type React from "react";
import { useTranslation } from "react-i18next";

export type LeftPanelView = "files" | "thoughts";

type Props = {
  activeView: LeftPanelView;
  onViewChange: (view: LeftPanelView) => void;
  onOpenCognitiveReport: () => void;
  onOpenSettings: () => void;
};

function FilesIcon() {
  return (
    <svg
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.93a2 2 0 0 1-1.66-.9l-.82-1.2A2 2 0 0 0 7.93 2H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2Z" />
    </svg>
  );
}

function ThoughtsIcon() {
  return (
    <svg
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M8 6h13" />
      <path d="M8 12h13" />
      <path d="M8 18h13" />
      <path d="M3 6h.01" />
      <path d="M3 12h.01" />
      <path d="M3 18h.01" />
    </svg>
  );
}

function ReportIcon() {
  return (
    <svg
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M3 3v18h18" />
      <path d="M7 16V9" />
      <path d="M11 16V5" />
      <path d="M15 16v-4" />
      <path d="M19 16v-7" />
    </svg>
  );
}

function SettingsIcon() {
  return (
    <svg
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.6a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82 1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z" />
      <circle cx="12" cy="12" r="3" />
    </svg>
  );
}

const VIEW_ICONS: Record<LeftPanelView, () => React.JSX.Element> = {
  files: FilesIcon,
  thoughts: ThoughtsIcon,
};

const VIEW_I18N_KEYS: Record<LeftPanelView, string> = {
  files: "activityBar.files",
  thoughts: "activityBar.thoughts",
};

export function ActivityBar({
  activeView,
  onViewChange,
  onOpenCognitiveReport,
  onOpenSettings,
}: Props) {
  const { t } = useTranslation();

  return (
    <nav className="activity-bar" aria-label={t("activityBar.label")} data-tauri-drag-region-exclude>
      <div className="activity-bar__top">
        {(Object.keys(VIEW_ICONS) as LeftPanelView[]).map((view) => {
          const Icon = VIEW_ICONS[view];
          const active = activeView === view;
          return (
            <button
              key={view}
              type="button"
              className={`activity-bar__btn${active ? " activity-bar__btn--active" : ""}`}
              aria-label={t(VIEW_I18N_KEYS[view])}
              data-tooltip={t(VIEW_I18N_KEYS[view])}
              aria-pressed={active}
              onClick={() => onViewChange(view)}
            >
              <Icon />
            </button>
          );
        })}
      </div>

      <div className="activity-bar__bottom">
        <button
          type="button"
          className="activity-bar__btn"
          aria-label={t("activityBar.report")}
          data-tooltip={t("activityBar.report")}
          onClick={onOpenCognitiveReport}
        >
          <ReportIcon />
        </button>
        <button
          type="button"
          className="activity-bar__btn"
          aria-label={t("activityBar.settings")}
          data-tooltip={t("activityBar.settings")}
          onClick={onOpenSettings}
        >
          <SettingsIcon />
        </button>
      </div>
    </nav>
  );
}
