import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ChatMessageTiming } from "../hooks/useWorkspaceAiConversations";
import "./StreamingTimer.css";

function formatSeconds(ms: number): string {
  return (ms / 1000).toFixed(1);
}

/** 流式生成中显示实时计时，完成后显示首字延迟和总耗时。 */
export function StreamingTimer({
  timing,
  streaming,
  modelName,
}: {
  timing: ChatMessageTiming;
  streaming: boolean;
  modelName?: string;
}) {
  const { t } = useTranslation();
  const [now, setNow] = useState(Date.now);

  useEffect(() => {
    if (!streaming) return;
    const id = window.setInterval(() => setNow(Date.now()), 200);
    return () => window.clearInterval(id);
  }, [streaming]);

  const modelTag = modelName ? (
    <span className="streaming-timer__model">{modelName}</span>
  ) : null;

  if (streaming) {
    const elapsed = now - timing.startMs;
    return (
      <span className="streaming-timer streaming-timer--active" aria-live="off">
        {modelTag}
        {t("aiPanel.timerElapsed", { sec: formatSeconds(elapsed) })}
      </span>
    );
  }

  if (!timing.endMs) return null;

  const total = timing.endMs - timing.startMs;
  const ttft = timing.firstTokenMs
    ? timing.firstTokenMs - timing.startMs
    : null;

  return (
    <span className="streaming-timer" aria-label={t("aiPanel.timerLabel")}>
      {modelTag}
      {ttft !== null
        ? t("aiPanel.timerDone", {
            ttft: formatSeconds(ttft),
            total: formatSeconds(total),
          })
        : t("aiPanel.timerTotal", { total: formatSeconds(total) })}
    </span>
  );
}
