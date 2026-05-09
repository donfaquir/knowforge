import { useTranslation } from "react-i18next";
import "./PrivacyChangeOverlay.css";

interface PrivacyChangeOverlayProps {
  onNewChat: () => void;
  onContinue: () => void;
}

export function PrivacyChangeOverlay({ onNewChat, onContinue }: PrivacyChangeOverlayProps) {
  const { t } = useTranslation();

  return (
    <div className="privacy-overlay" role="alertdialog" aria-modal="true" aria-label={t("privacy.sharedDocWarning")}>
      <div className="privacy-overlay__card">
        <div className="privacy-overlay__icon" aria-hidden="true">&#x1f512;</div>
        <p className="privacy-overlay__text">{t("privacy.sharedDocWarning")}</p>
        <div className="privacy-overlay__actions">
          <button type="button" className="privacy-overlay__btn privacy-overlay__btn--primary" onClick={onNewChat}>
            {t("privacy.newConversation")}
          </button>
          <button type="button" className="privacy-overlay__btn privacy-overlay__btn--secondary" onClick={onContinue}>
            {t("privacy.continueConversation")}
          </button>
        </div>
      </div>
    </div>
  );
}
