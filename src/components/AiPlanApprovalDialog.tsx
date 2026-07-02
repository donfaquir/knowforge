import { useCallback } from "react";
import { useTranslation } from "react-i18next";

import type { PlanApprovalRequest } from "../types/toolTypes";
import { AiAssistantMarkdown } from "./AiAssistantMarkdown";
import "./AiPlanApprovalDialog.css";

interface AiPlanApprovalDialogProps {
  request: PlanApprovalRequest | null;
  onApprove: (approvalId: string) => void;
  onReject: (approvalId: string) => void;
}

export function AiPlanApprovalDialog({
  request,
  onApprove,
  onReject,
}: AiPlanApprovalDialogProps) {
  const { t } = useTranslation();

  const approve = useCallback(() => {
    if (request) onApprove(request.approvalId);
  }, [request, onApprove]);

  const reject = useCallback(() => {
    if (request) onReject(request.approvalId);
  }, [request, onReject]);

  if (!request) return null;

  return (
    <div
      className="plan-approval-overlay"
      role="alertdialog"
      aria-modal="true"
      aria-label={t("aiPanel.planApprovalTitle")}
    >
      <div className="plan-approval-overlay__card">
        <div className="plan-approval-overlay__header">
          <h3 className="plan-approval-overlay__title">
            {t("aiPanel.planApprovalTitle")}
          </h3>
        </div>

        <p className="plan-approval-overlay__description">
          {t("aiPanel.planApprovalDescription")}
        </p>

        <div className="plan-approval-overlay__plan">
          <AiAssistantMarkdown content={request.planText} />
        </div>

        <div className="plan-approval-overlay__actions">
          <button
            type="button"
            className="plan-approval-overlay__btn plan-approval-overlay__btn--reject"
            onClick={reject}
          >
            {t("aiPanel.planApprovalReject")}
          </button>
          <button
            type="button"
            className="plan-approval-overlay__btn plan-approval-overlay__btn--approve"
            onClick={approve}
            autoFocus
          >
            {t("aiPanel.planApprovalRun")}
          </button>
        </div>
      </div>
    </div>
  );
}
