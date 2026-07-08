import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import sampleData from "../../resources/onboarding/sample_challenges.json";
import "./OnboardingOverlay.css";

type Step = 1 | 2 | 3 | 4;

type Props = {
  open: boolean;
  onClose: () => void;
  tauriRuntime: boolean;
};

const TOTAL_STEPS = 4;

export function OnboardingOverlay({ open, onClose, tauriRuntime }: Props) {
  const { t, i18n } = useTranslation();
  const isZh = i18n.language.startsWith("zh");
  const [step, setStep] = useState<Step>(1);
  const [answer, setAnswer] = useState("");
  const [showFeedback, setShowFeedback] = useState(false);
  const [providerUrl, setProviderUrl] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [aiSaved, setAiSaved] = useState(false);
  const [aiSaving, setAiSaving] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const challenge = sampleData.challenges[0];
  const thought = sampleData.thoughts[0];

  useEffect(() => {
    if (open) {
      setStep(1);
      setAnswer("");
      setShowFeedback(false);
      setProviderUrl("");
      setApiKey("");
      setAiSaved(false);
      setAiSaving(false);
    }
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        finish();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const finish = useCallback(() => {
    localStorage.setItem("knowforge:onboardingCompleted", "true");
    onClose();
  }, [onClose]);

  const goNext = useCallback(() => {
    setStep((s) => (s < TOTAL_STEPS ? ((s + 1) as Step) : s));
  }, []);

  const goPrev = useCallback(() => {
    setStep((s) => (s > 1 ? ((s - 1) as Step) : s));
  }, []);

  const handleSubmitAnswer = useCallback(() => {
    setShowFeedback(true);
  }, []);

  const handleSaveAi = useCallback(async () => {
    if (!tauriRuntime || !providerUrl.trim()) return;
    setAiSaving(true);
    try {
      const providerId = "onboarding-provider";
      await invoke("save_vault_config_patch", {
        patch: {
          ai: {
            activeProviderId: providerId,
            providers: [
              {
                id: providerId,
                label: "AI Provider",
                baseUrl: providerUrl.trim(),
                apiKey: apiKey.trim() || undefined,
                isRemote: true,
              },
            ],
          },
        },
      });
      setAiSaved(true);
    } catch {
      // silently fail — user can configure later
    } finally {
      setAiSaving(false);
    }
  }, [tauriRuntime, providerUrl, apiKey]);

  if (!open) return null;

  const stepIndicator = (
    <div className="onboarding__step-indicator">
      {t("onboarding.stepOf", { current: step, total: TOTAL_STEPS })}
    </div>
  );

  return (
    <div className="onboarding__scrim" role="presentation">
      <div
        className="onboarding__card"
        role="dialog"
        aria-modal="true"
        onClick={(e) => e.stopPropagation()}
      >
        {stepIndicator}

        {step === 1 && (
          <div className="onboarding__step">
            <h2 className="onboarding__title">{t("onboarding.step1Title")}</h2>
            <p className="onboarding__subtitle">{t("onboarding.step1Subtitle")}</p>
            <p className="onboarding__desc">{t("onboarding.step1Desc")}</p>
            <div className="onboarding__actions">
              <button
                type="button"
                className="onboarding__btn onboarding__btn--secondary"
                onClick={finish}
              >
                {t("onboarding.step1Skip")}
              </button>
              <button
                type="button"
                className="onboarding__btn onboarding__btn--primary"
                onClick={goNext}
              >
                {t("onboarding.step1Start")}
              </button>
            </div>
          </div>
        )}

        {step === 2 && (
          <div className="onboarding__step">
            <h2 className="onboarding__title">{t("onboarding.step2Title")}</h2>
            <p className="onboarding__desc">{t("onboarding.step2Desc")}</p>

            <div className="onboarding__thought-card">
              <span className="onboarding__thought-maturity">🌱</span>
              <p className="onboarding__thought-body">
                {isZh ? thought.body_zh : thought.body_en}
              </p>
            </div>

            <div className="onboarding__challenge">
              <p className="onboarding__question">
                {isZh ? challenge.question_zh : challenge.question_en}
              </p>

              {!showFeedback ? (
                <>
                  <textarea
                    ref={textareaRef}
                    className="onboarding__answer-input"
                    value={answer}
                    onChange={(e) => setAnswer(e.target.value)}
                    placeholder={t("onboarding.step2AnswerPlaceholder")}
                    rows={3}
                  />
                  <div className="onboarding__actions">
                    <button
                      type="button"
                      className="onboarding__btn onboarding__btn--secondary"
                      onClick={() => {
                        setShowFeedback(true);
                      }}
                    >
                      {t("onboarding.step2SkipQuestion")}
                    </button>
                    <button
                      type="button"
                      className="onboarding__btn onboarding__btn--primary"
                      onClick={handleSubmitAnswer}
                      disabled={!answer.trim()}
                    >
                      {t("onboarding.step2Submit")}
                    </button>
                  </div>
                </>
              ) : (
                <div className="onboarding__feedback">
                  <div className="onboarding__feedback-label">
                    {t("onboarding.step2FeedbackIntro")}
                  </div>
                  <p className="onboarding__feedback-text">
                    {isZh ? challenge.sampleFeedback_zh : challenge.sampleFeedback_en}
                  </p>
                  <div className="onboarding__maturity-animation">
                    <span className="onboarding__maturity-from">🌱</span>
                    <span className="onboarding__maturity-arrow">→</span>
                    <span className="onboarding__maturity-to">🌿</span>
                  </div>
                  <p className="onboarding__maturity-hint">
                    {t("onboarding.step2MaturityHint")}
                  </p>
                  <div className="onboarding__actions">
                    <button
                      type="button"
                      className="onboarding__btn onboarding__btn--secondary"
                      onClick={goPrev}
                    >
                      {t("onboarding.prev")}
                    </button>
                    <button
                      type="button"
                      className="onboarding__btn onboarding__btn--primary"
                      onClick={goNext}
                    >
                      {t("onboarding.step2Next")}
                    </button>
                  </div>
                </div>
              )}
            </div>
          </div>
        )}

        {step === 3 && (
          <div className="onboarding__step">
            <h2 className="onboarding__title">{t("onboarding.step3Title")}</h2>
            <p className="onboarding__desc">{t("onboarding.step3Desc")}</p>

            {!aiSaved ? (
              <div className="onboarding__ai-form">
                <label className="onboarding__field">
                  <span className="onboarding__field-label">API Base URL</span>
                  <input
                    type="url"
                    className="onboarding__field-input"
                    value={providerUrl}
                    onChange={(e) => setProviderUrl(e.target.value)}
                    placeholder="https://api.openai.com/v1"
                  />
                </label>
                <label className="onboarding__field">
                  <span className="onboarding__field-label">API Key</span>
                  <input
                    type="password"
                    className="onboarding__field-input"
                    value={apiKey}
                    onChange={(e) => setApiKey(e.target.value)}
                    placeholder="sk-..."
                  />
                </label>
                <div className="onboarding__actions">
                  <button
                    type="button"
                    className="onboarding__btn onboarding__btn--secondary"
                    onClick={goNext}
                  >
                    {t("onboarding.step3Skip")}
                  </button>
                  <button
                    type="button"
                    className="onboarding__btn onboarding__btn--primary"
                    onClick={handleSaveAi}
                    disabled={!providerUrl.trim() || aiSaving}
                  >
                    {aiSaving ? "…" : t("onboarding.step2Submit")}
                  </button>
                </div>
              </div>
            ) : (
              <div className="onboarding__ai-saved">
                <p className="onboarding__ai-saved-msg">✓ {t("onboarding.step3Saved")}</p>
                <div className="onboarding__actions">
                  <button
                    type="button"
                    className="onboarding__btn onboarding__btn--primary"
                    onClick={goNext}
                  >
                    {t("onboarding.step2Next")}
                  </button>
                </div>
              </div>
            )}
          </div>
        )}

        {step === 4 && (
          <div className="onboarding__step">
            <h2 className="onboarding__title">{t("onboarding.step4Title")}</h2>
            <p className="onboarding__desc">{t("onboarding.step4Desc")}</p>

            <div className="onboarding__tips">
              <div className="onboarding__tip-card">
                <div className="onboarding__tip-icon">📌</div>
                <div className="onboarding__tip-text">
                  <strong>{t("onboarding.step4Tip1Title")}</strong>
                  <span>{t("onboarding.step4Tip1Desc")}</span>
                </div>
              </div>
              <div className="onboarding__tip-card">
                <div className="onboarding__tip-icon">📖</div>
                <div className="onboarding__tip-text">
                  <strong>{t("onboarding.step4Tip2Title")}</strong>
                  <span>{t("onboarding.step4Tip2Desc")}</span>
                </div>
              </div>
              <div className="onboarding__tip-card">
                <div className="onboarding__tip-icon">⌨️</div>
                <div className="onboarding__tip-text">
                  <strong>{t("onboarding.step4Tip3Title")}</strong>
                  <span>{t("onboarding.step4Tip3Desc")}</span>
                </div>
              </div>
            </div>

            <div className="onboarding__actions">
              <button
                type="button"
                className="onboarding__btn onboarding__btn--primary"
                onClick={finish}
              >
                {t("onboarding.step4Done")}
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
