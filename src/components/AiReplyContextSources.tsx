import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ReplyContextSources } from "../types/replyContextSources";
import "./AiReplyContextSources.css";

const MAX_PATH_ROWS = 10;

type Props = {
  sources: ReplyContextSources;
  onOpenMarkdown?: (relPath: string) => void;
};

function shortThoughtId(id: string): string {
  const s = typeof id === "string" ? id : String(id ?? "");
  if (s.length <= 24) return s;
  return `${s.slice(0, 10)}…${s.slice(-8)}`;
}

/** 折叠条左侧图标：仅装饰，无语义 */
function ContextSourcesGlyph() {
  return (
    <svg className="ai-reply-sources__glyph" viewBox="0 0 16 16" fill="none" aria-hidden={true}>
      <path
        d="M3.25 3.25h6.5a1 1 0 011 1v8.5a1 1 0 01-1 1h-6.5a1 1 0 01-1-1v-8.5a1 1 0 011-1z"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinejoin="round"
        opacity={0.45}
      />
      <path
        d="M5.25 5.75h7.5a1 1 0 011 1v6a1 1 0 01-1 1h-7.5a1 1 0 01-1-1v-6a1 1 0 011-1z"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** 折叠箭头：展开时旋转 */
function ContextSourcesChevron({ expanded }: { expanded: boolean }) {
  return (
    <svg
      className={`ai-reply-sources__chevron-svg${expanded ? " ai-reply-sources__chevron-svg--open" : ""}`}
      viewBox="0 0 12 12"
      fill="none"
      aria-hidden={true}
    >
      <path d="M2.5 4.25L6 7.75l3.5-3.5" stroke="currentColor" strokeWidth="1.35" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

export function AiReplyContextSources({ sources, onOpenMarkdown }: Props) {
  const { t } = useTranslation();
  const uid = useId().replace(/:/g, "");
  const headingId = `ai-reply-sources-h-${uid}`;
  const panelId = `ai-reply-sources-p-${uid}`;
  // 默认折叠，减少助手气泡末尾信息密度
  const [expanded, setExpanded] = useState(false);
  const rawSem = sources.semantic;
  const sem = {
    injected: rawSem?.injected === true,
    documentPaths: Array.isArray(rawSem?.documentPaths) ? rawSem!.documentPaths : [],
    thoughtIds: Array.isArray(rawSem?.thoughtIds) ? rawSem!.thoughtIds : [],
  };
  const vaultEntries = Array.isArray(sources.vaultKeyword?.entries) ? sources.vaultKeyword!.entries : [];

  return (
    <div className="ai-reply-sources">
      <button
        type="button"
        id={headingId}
        className="ai-reply-sources__toggle"
        aria-expanded={expanded}
        aria-controls={panelId}
        title={expanded ? t("aiPanel.contextSourcesToggleCollapseTitle") : t("aiPanel.contextSourcesToggleExpandTitle")}
        onClick={() => setExpanded((v) => !v)}
      >
        <span className="ai-reply-sources__toggle-leading" aria-hidden={true}>
          <ContextSourcesGlyph />
        </span>
        <span className="ai-reply-sources__toggle-main">
          <span className="ai-reply-sources__toggle-label">{t("aiPanel.contextSourcesTitle")}</span>
          {!expanded ? (
            <>
              <span className="ai-reply-sources__toggle-sep" aria-hidden={true}>
                ·
              </span>
              <span className="ai-reply-sources__toggle-hint">{t("aiPanel.contextSourcesToggleHintVisible")}</span>
            </>
          ) : null}
        </span>
        <span className="ai-reply-sources__toggle-trail" aria-hidden={true}>
          <ContextSourcesChevron expanded={expanded} />
        </span>
      </button>

      <div
        id={panelId}
        className="ai-reply-sources__panel"
        role="region"
        aria-labelledby={headingId}
        hidden={!expanded}
      >
        {sources.currentNote?.relPath ? (
          <div className="ai-reply-sources__block">
            <div className="ai-reply-sources__label">{t("aiPanel.contextSourceCurrentNote")}</div>
            <div className="ai-reply-sources__row">
              {sources.currentNote.mode === "full" && onOpenMarkdown ? (
                <button
                  type="button"
                  className="ai-reply-sources__link"
                  onClick={() => onOpenMarkdown(sources.currentNote!.relPath)}
                >
                  {sources.currentNote.relPath}
                </button>
              ) : (
                <span className="ai-reply-sources__path">{sources.currentNote.relPath}</span>
              )}
              {sources.currentNote.mode === "redacted" ? (
                <span className="ai-reply-sources__badge">{t("aiPanel.contextSourceRedacted")}</span>
              ) : null}
            </div>
          </div>
        ) : null}

        {vaultEntries.length > 0 ? (
          <div className="ai-reply-sources__block">
            <div className="ai-reply-sources__label">
              {t("aiPanel.contextSourceVaultKeyword")}
              {sources.vaultKeyword?.truncated ? (
                <span className="ai-reply-sources__badge ai-reply-sources__badge--muted">
                  {t("aiPanel.contextSourceTruncated")}
                </span>
              ) : null}
            </div>
            <ul className="ai-reply-sources__list">
              {vaultEntries.slice(0, MAX_PATH_ROWS).map((e, i) => (
                <li key={`${e?.relPath ?? i}:${e?.kind ?? ""}:${i}`} className="ai-reply-sources__li">
                  {e?.kind === "excerpt" && onOpenMarkdown && e.relPath ? (
                    <button type="button" className="ai-reply-sources__link" onClick={() => onOpenMarkdown(e.relPath)}>
                      {e.relPath}
                    </button>
                  ) : (
                    <span className="ai-reply-sources__path">{e?.relPath ?? ""}</span>
                  )}
                  {e?.kind === "privateOmitted" ? (
                    <span className="ai-reply-sources__badge">{t("aiPanel.contextSourcePrivateSlot")}</span>
                  ) : null}
                </li>
              ))}
            </ul>
            {vaultEntries.length > MAX_PATH_ROWS ? (
              <div className="ai-reply-sources__more">
                {t("aiPanel.contextSourceAndMore", { count: vaultEntries.length - MAX_PATH_ROWS })}
              </div>
            ) : null}
          </div>
        ) : null}

        {sem.injected ? (
          <div className="ai-reply-sources__block">
            <div className="ai-reply-sources__label">{t("aiPanel.contextSourceSemantic")}</div>
            {sem.documentPaths.length > 0 ? (
              <ul className="ai-reply-sources__list">
                {sem.documentPaths.slice(0, MAX_PATH_ROWS).map((p) => (
                  <li key={p} className="ai-reply-sources__li">
                    {onOpenMarkdown ? (
                      <button type="button" className="ai-reply-sources__link" onClick={() => onOpenMarkdown(p)}>
                        {p}
                      </button>
                    ) : (
                      <span className="ai-reply-sources__path">{p}</span>
                    )}
                  </li>
                ))}
              </ul>
            ) : null}
            {sem.thoughtIds.length > 0 ? (
              <ul className="ai-reply-sources__list ai-reply-sources__list--thoughts">
                {sem.thoughtIds.map((id) => (
                  <li key={id} className="ai-reply-sources__li">
                    <code className="ai-reply-sources__thought-id" title={id}>
                      {shortThoughtId(id)}
                    </code>
                    <span className="ai-reply-sources__thought-hint">{t("aiPanel.contextSourceThoughtEntry")}</span>
                  </li>
                ))}
              </ul>
            ) : null}
            {sem.documentPaths.length === 0 && sem.thoughtIds.length === 0 ? (
              <span className="ai-reply-sources__muted">{t("aiPanel.contextSourceSemanticEmpty")}</span>
            ) : null}
          </div>
        ) : null}

        {sources.thoughtFocus?.thoughtId ? (
          <div className="ai-reply-sources__block">
            <div className="ai-reply-sources__label">{t("aiPanel.contextSourceThoughtFocus")}</div>
            <code className="ai-reply-sources__thought-id" title={sources.thoughtFocus.thoughtId}>
              {sources.thoughtFocus.thoughtId}
            </code>
          </div>
        ) : null}
      </div>
    </div>
  );
}
