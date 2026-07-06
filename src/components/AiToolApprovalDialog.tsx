import { useCallback, useEffect, useMemo } from "react";
import { useTranslation } from "react-i18next";

import type { ApprovalRequest } from "../types/toolTypes";
import "./AiToolApprovalDialog.css";

interface AiToolApprovalDialogProps {
  request: ApprovalRequest | null;
  onResolve: (approvalId: string, decision: boolean) => void;
}

export function AiToolApprovalDialog({ request, onResolve }: AiToolApprovalDialogProps) {
  const { t } = useTranslation();

  const allow = useCallback(() => {
    if (request) onResolve(request.approvalId, true);
  }, [request, onResolve]);

  const deny = useCallback(() => {
    if (request) onResolve(request.approvalId, false);
  }, [request, onResolve]);

  // 后端有 30 分钟兜底超时（到期自动 deny）；前端不响应 Escape 以避免误关行为不明
  useEffect(() => {
    if (!request) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        allow();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [request, allow]);

  const prettyInput = useMemo(() => {
    if (!request) return "";
    return request.inputSummary;
  }, [request]);

  if (!request) return null;

  const policyLabel =
    request.policy === "confirm_once_per_session"
      ? t("aiPanel.toolApprovalPolicyOnce")
      : t("aiPanel.toolApprovalPolicyEach");

  return (
    <div
      className="tool-approval-overlay"
      role="alertdialog"
      aria-modal="true"
      aria-label={t("aiPanel.toolApprovalTitle")}
    >
      <div className="tool-approval-overlay__card">
        <div className="tool-approval-overlay__header">
          <span
            className={`tool-approval-overlay__risk tool-approval-overlay__risk--${request.risk}`}
            aria-label={t("aiPanel.toolApprovalRiskLabel")}
          >
            {request.risk}
          </span>
          <h3 className="tool-approval-overlay__title">{t("aiPanel.toolApprovalTitle")}</h3>
        </div>

        <p className="tool-approval-overlay__description">
          {t("aiPanel.toolApprovalDescription", { tool: request.toolName })}
        </p>

        <dl className="tool-approval-overlay__meta">
          <dt>{t("aiPanel.toolApprovalEffectsLabel")}</dt>
          <dd>
            {request.effects.length === 0
              ? "—"
              : request.effects.map((eff) => (
                  <span key={eff} className="tool-approval-overlay__effect">
                    {eff}
                  </span>
                ))}
          </dd>
          <dt>{t("aiPanel.toolApprovalPolicyLabel")}</dt>
          <dd>{policyLabel}</dd>
        </dl>

        <pre className="tool-approval-overlay__input" aria-label="tool input summary">
          {prettyInput}
        </pre>

        <div className="tool-approval-overlay__actions">
          <button
            type="button"
            className="tool-approval-overlay__btn tool-approval-overlay__btn--deny"
            onClick={deny}
          >
            {t("aiPanel.toolApprovalDeny")}
          </button>
          <button
            type="button"
            className="tool-approval-overlay__btn tool-approval-overlay__btn--allow"
            onClick={allow}
            autoFocus
          >
            {t("aiPanel.toolApprovalAllow")}
          </button>
        </div>
      </div>
    </div>
  );
}
