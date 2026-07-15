import { invoke } from "@tauri-apps/api/core";
import { useCallback, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ReviewQueueItem } from "../../types/cognitiveTypes";
import { DiscoveryDetailView } from "./DiscoveryDetailView";
import { DiscoveryPane, type CandidateForUi } from "./DiscoveryPane";
import { PracticeReviewPane } from "./PracticeReviewPane";
import { PracticeSourcePreview } from "./PracticeSourcePreview";
import "./PracticeMode.css";

export type PracticeSubTab = "review" | "discovery";

export interface PracticeModeProps {
  workspaceReady: boolean;
  tauriRuntime: boolean;
  workspaceRoot: string | null;
}

export function PracticeMode({ workspaceReady, tauriRuntime }: PracticeModeProps) {
  const { t } = useTranslation();
  const [subTab, setSubTab] = useState<PracticeSubTab>("review");
  const [reviewCompleted, setReviewCompleted] = useState(false);
  const [focusedThought, setFocusedThought] = useState<ReviewQueueItem | null>(null);
  const [selectedCandidate, setSelectedCandidate] = useState<CandidateForUi | null>(null);
  const [discoveryRefreshKey, setDiscoveryRefreshKey] = useState(0);
  const selectedCandidateRef = useRef(selectedCandidate);
  selectedCandidateRef.current = selectedCandidate;

  const handleReviewCompleted = useCallback(() => {
    setReviewCompleted(true);
  }, []);

  const handleSelectCandidate = useCallback((candidate: CandidateForUi | null) => {
    setSelectedCandidate(candidate);
  }, []);

  /** Promote from detail view — calls backend and refreshes list */
  const handleDetailPromote = useCallback(async (candidateId: string) => {
    try {
      await invoke<string>("promote_candidate_to_thought", { candidateId });
      setSelectedCandidate(null);
      setDiscoveryRefreshKey((k) => k + 1);
    } catch (err) {
      console.error("promote_candidate_to_thought failed:", err);
    }
  }, []);

  /** Dismiss from detail view — calls backend and refreshes list */
  const handleDetailDismiss = useCallback(async (candidateId: string) => {
    try {
      await invoke<void>("dismiss_latent_candidate", { candidateId });
      setSelectedCandidate(null);
      setDiscoveryRefreshKey((k) => k + 1);
    } catch (err) {
      console.error("dismiss_latent_candidate failed:", err);
    }
  }, []);

  /** "View in source" — switch to files mode and open the note */
  const handleViewInSource = useCallback((relPath: string, _startLine: number) => {
    // Dispatch event to navigate back to files mode (App.tsx listens)
    window.dispatchEvent(
      new CustomEvent("knowforge:openNoteInEditor", { detail: { relPath } }),
    );
  }, []);

  return (
    <div className="practice-mode">
      <header className="practice-mode__header">
        <div className="practice-mode__tabs" role="tablist">
          <button
            type="button"
            role="tab"
            className={`practice-mode__tab${subTab === "review" ? " practice-mode__tab--active" : ""}`}
            aria-selected={subTab === "review"}
            onClick={() => { setSubTab("review"); setReviewCompleted(false); }}
          >
            {t("practice.tabReview", "Review")}
          </button>
          <button
            type="button"
            role="tab"
            className={`practice-mode__tab${subTab === "discovery" ? " practice-mode__tab--active" : ""}`}
            aria-selected={subTab === "discovery"}
            onClick={() => setSubTab("discovery")}
          >
            {t("practice.tabDiscovery", "Discovery")}
          </button>
        </div>
      </header>

      <div className="practice-mode__body">
        <div className="practice-mode__left">
          {subTab === "review" && !reviewCompleted && (
            <PracticeReviewPane
              workspaceReady={workspaceReady}
              tauriRuntime={tauriRuntime}
              onReviewCompleted={handleReviewCompleted}
              onFocusThought={setFocusedThought}
            />
          )}
          {subTab === "review" && reviewCompleted && (
            <div className="practice-mode__placeholder">
              <p>{t("practice.reviewCompleted", "Today's review completed!")}</p>
              <button
                type="button"
                className="practice-mode__tab"
                onClick={() => setSubTab("discovery")}
              >
                {t("practice.goToDiscovery", "Explore discoveries")}
              </button>
            </div>
          )}
          {subTab === "discovery" && (
            <DiscoveryPane
              workspaceReady={workspaceReady}
              tauriRuntime={tauriRuntime}
              onSelectCandidate={handleSelectCandidate}
              refreshKey={discoveryRefreshKey}
            />
          )}
        </div>
        <div className="practice-mode__right">
          {subTab === "review" ? (
            <PracticeSourcePreview
              relPath={focusedThought?.relPath ?? null}
              startLine={focusedThought?.startLine ?? undefined}
            />
          ) : (
            <DiscoveryDetailView
              candidate={selectedCandidate}
              onPromote={(id) => void handleDetailPromote(id)}
              onDismiss={(id) => void handleDetailDismiss(id)}
              onViewInSource={handleViewInSource}
            />
          )}
        </div>
      </div>
    </div>
  );
}
