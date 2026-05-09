import { useTranslation } from "react-i18next";
import { useSemanticIndex } from "../hooks/useSemanticIndex";
import { canResumeRebuildCheckpoint, rebuildCheckpointPercent } from "../utils/rebuildCheckpointPercent";
import "./SemanticIndexStatus.css";

export type SemanticIndexStatusProps = {
  workspaceReady: boolean;
  tauriRuntime: boolean;
};

export function SemanticIndexStatus({ workspaceReady, tauriRuntime }: SemanticIndexStatusProps) {
  const { t } = useTranslation();
  const { status, busy, error, progressMessage, lastBuildResult, rebuildProgress, rebuild } = useSemanticIndex({
    workspaceReady,
    tauriRuntime,
  });

  const showSavedProgress = rebuildProgress != null;
  const savedPct = rebuildProgress ? rebuildCheckpointPercent(rebuildProgress) : 0;
  const canResume = canResumeRebuildCheckpoint(rebuildProgress);

  const phaseLabel =
    rebuildProgress == null
      ? ""
      : (() => {
          const key =
            (
              {
                scanning: "settings.semanticRebuildPhaseScanning",
                documents: "settings.semanticRebuildPhaseDocuments",
                thoughts: "settings.semanticRebuildPhaseThoughts",
                completed: "settings.semanticRebuildPhaseCompleted",
                failed: "settings.semanticRebuildPhaseFailed",
              } as Record<string, string>
            )[rebuildProgress.phase] ?? null;
          return key
            ? t(key)
            : t("settings.semanticRebuildPhaseUnknown", { phase: rebuildProgress.phase });
        })();

  if (!tauriRuntime || !workspaceReady) {
    return null;
  }

  return (
    <div className="semantic-index-status">
      <h4 className="semantic-index-status__title">{t("settings.semanticIndexTitle")}</h4>
      {status ? (
        <>
          <p className="semantic-index-status__row">
            {t("settings.semanticModelReady")}:{" "}
            <strong>{status.modelReady ? t("settings.yes") : t("settings.no")}</strong>
            {" · "}
            <code>{status.modelId}</code>
          </p>
          <p className="semantic-index-status__row">
            {t("settings.semanticDocChunks")}: {status.docChunkCount} · {t("settings.semanticThoughtVecs")}:{" "}
            {status.thoughtEmbeddingCount}
          </p>
          <p className="semantic-index-status__row">
            {t("settings.semanticTrackedFiles")}: {status.trackedFileCount} · {t("settings.semanticStaleFiles")}:{" "}
            {status.staleFileCount}
          </p>
        </>
      ) : (
        <p className="semantic-index-status__row">{t("settings.semanticStatusLoading")}</p>
      )}
      {progressMessage ? (
        <p className="semantic-index-status__progress" role="status">
          {progressMessage}
        </p>
      ) : null}
      {error ? (
        <p className="semantic-index-status__error" role="alert">
          {error}
        </p>
      ) : null}
      {lastBuildResult ? (
        <p className="semantic-index-status__row semantic-index-status__summary" role="status">
          {t("settings.semanticLastBuild", {
            chunks: lastBuildResult.indexedChunks,
            thoughts: lastBuildResult.indexedThoughts,
            ms: lastBuildResult.elapsedMs,
          })}
        </p>
      ) : null}
      {showSavedProgress ? (
        <div className="semantic-index-status__persisted" aria-label={t("settings.semanticRebuildPersistedLabel")}>
          <div className="semantic-index-status__persisted-head">
            <span className="semantic-index-status__persisted-title">
              {t("settings.semanticRebuildPersistedTitle")}
            </span>
            <span className="semantic-index-status__persisted-pct">{savedPct}%</span>
          </div>
          <progress
            className="semantic-index-status__progressbar"
            max={100}
            value={savedPct}
            aria-valuenow={savedPct}
            aria-valuemin={0}
            aria-valuemax={100}
          />
          <p className="semantic-index-status__persisted-meta">
            {t("settings.semanticRebuildPersistedMeta", {
              phase: phaseLabel,
              docsDone: rebuildProgress!.docsCompleted,
              docsTotal: rebuildProgress!.docsTotal,
              thDone: rebuildProgress!.thoughtsNextIndex,
              thTotal: rebuildProgress!.thoughtsTotal,
            })}
          </p>
          {rebuildProgress?.lastError ? (
            <p className="semantic-index-status__persisted-err" role="status">
              {rebuildProgress.lastError}
            </p>
          ) : null}
        </div>
      ) : null}
      {busy ? (
        <p className="semantic-index-status__rebuilding" role="status">
          {t("settings.semanticRebuilding")}
        </p>
      ) : null}
      <div className="semantic-index-status__actions">
        {canResume ? (
          <button
            type="button"
            className="app-modal__btn semantic-index-status__btn-continue"
            disabled={busy || !status?.modelReady}
            onClick={() => void rebuild(true)}
          >
            {t("settings.semanticRebuildContinue")}
          </button>
        ) : null}
        <button
          type="button"
          className="app-modal__btn"
          disabled={busy || !status?.modelReady}
          onClick={() => void rebuild(false)}
        >
          {t("settings.semanticRebuild")}
        </button>
      </div>
      {!status?.modelReady ? (
        <p className="semantic-index-status__hint">{t("settings.semanticModelMissingHint")}</p>
      ) : null}
    </div>
  );
}
