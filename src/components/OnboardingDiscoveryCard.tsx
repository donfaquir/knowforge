import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import "./OnboardingDiscoveryCard.css";

interface CandidateForUi {
  id: string;
  relPath: string;
  excerpt: string;
  markingReason: string;
  similarityScore: number | null;
  pairedRelPath: string | null;
  startLine: number;
  endLine: number;
}

type Phase = "loading" | "found" | "building";

interface Props {
  tauriRuntime: boolean;
  onStartChallenge: () => void;
  onFinish: () => void;
}

const PREVIEW_COUNT = 3;
const SCAN_TIMEOUT_MS = 8000;

export default function OnboardingDiscoveryCard({
  tauriRuntime,
  onStartChallenge,
  onFinish,
}: Props) {
  const { t } = useTranslation();
  const [phase, setPhase] = useState<Phase>("loading");
  const [candidates, setCandidates] = useState<CandidateForUi[]>([]);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    if (!tauriRuntime) {
      setPhase("building");
      return;
    }

    void (async () => {
      // 1. Check existing candidates (instant)
      try {
        const existing = await invoke<CandidateForUi[]>("list_latent_candidates");
        if (!mountedRef.current) return;
        if (existing.length > 0) {
          setCandidates(existing);
          setPhase("found");
          return;
        }
      } catch {
        // DB may not exist
      }
      if (!mountedRef.current) return;

      // 2. Try scan with timeout — only succeeds if index already built
      try {
        const scanned = await Promise.race([
          invoke<CandidateForUi[]>("trigger_latent_scan"),
          new Promise<null>((_, reject) =>
            setTimeout(() => reject(new Error("timeout")), SCAN_TIMEOUT_MS),
          ),
        ]);
        if (!mountedRef.current) return;
        if (scanned && scanned.length > 0) {
          setCandidates(scanned);
          setPhase("found");
          return;
        }
      } catch {
        // scan failed or timed out
      }
      if (!mountedRef.current) return;

      // 3. Index not ready — kick off rebuild in background, don't wait
      setPhase("building");
      invoke("rebuild_embeddings", { resume: false }).catch(() => {});
    })();

    return () => {
      mountedRef.current = false;
    };
  }, [tauriRuntime]);

  if (phase === "loading") {
    return (
      <div className="discovery-card">
        <div className="discovery-card__loading">
          <span className="discovery-card__dot" />
          <span className="discovery-card__dot" />
          <span className="discovery-card__dot" />
        </div>
        <p className="discovery-card__loading-text">
          {t("onboarding.discovery.loading")}
        </p>
      </div>
    );
  }

  if (phase === "building") {
    return (
      <div className="discovery-card">
        <h3 className="discovery-card__empty-title">
          {t("onboarding.discovery.buildingTitle")}
        </h3>
        <p className="discovery-card__empty-desc">
          {t("onboarding.discovery.buildingDesc")}
        </p>
        <div className="onboarding__actions">
          <button className="onboarding__btn onboarding__btn--primary" onClick={onFinish}>
            {t("onboarding.discovery.finishGuide")}
          </button>
        </div>
      </div>
    );
  }

  const previews = candidates.slice(0, PREVIEW_COUNT);
  const uniqueDocs = new Set(candidates.map((c) => c.relPath)).size;

  return (
    <div className="discovery-card">
      <span className="discovery-card__count">{candidates.length}</span>
      <h3 className="discovery-card__found-title">
        {t("onboarding.discovery.foundTitle", { count: candidates.length })}
      </h3>
      <p className="discovery-card__found-subtitle">
        {t("onboarding.discovery.foundSubtitle", { docCount: uniqueDocs })}
      </p>

      <div className="discovery-card__previews">
        {previews.map((c) => (
          <div key={c.id} className="discovery-card__preview-item">
            <span className="discovery-card__preview-excerpt">{c.excerpt}</span>
            <span className="discovery-card__preview-source">{c.relPath}</span>
          </div>
        ))}
      </div>

      <div className="onboarding__actions">
        <button className="onboarding__btn onboarding__btn--primary" onClick={onStartChallenge}>
          {t("onboarding.discovery.startChallenge")}
        </button>
        <button className="onboarding__btn onboarding__btn--secondary" onClick={onFinish}>
          {t("onboarding.discovery.finishGuide")}
        </button>
      </div>
    </div>
  );
}
