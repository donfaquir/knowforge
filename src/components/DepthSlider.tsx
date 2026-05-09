/** 深度下拉选择器：浅 / 中 / 深 / 自动，持久化到 vault config。 */
import { invoke, isTauri } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { DepthMode } from "../types/cognitiveTypes";
import type { VaultConfigForUi } from "../types/vaultAiConfig";
import "./DepthSlider.css";

const MODES: DepthMode[] = ["shallow", "medium", "deep", "auto"];

/** 当日手动覆盖自动的最大次数，超过则锁定为手动模式 */
const OVERRIDE_THRESHOLD = 5;

type Props = {
  value: DepthMode;
  onChange: (mode: DepthMode) => void;
  autoResolved?: "shallow" | "medium" | "deep" | null;
  disabled?: boolean;
  /** 紧凑：仅「箭头 + 当前档位文案」触发，无说明行与决策详情 */
  compact?: boolean;
};

/** IPC 返回的决策日志条目，对齐 DepthDecisionEntry (camelCase) */
type DecisionEntry = {
  timestamp: string;
  autoResolved: DepthMode;
  reason: string;
  userOverride?: DepthMode;
};

/** 计算次日凌晨 00:00 的 ISO 时间戳 */
function nextMidnightIso(): string {
  const d = new Date();
  d.setDate(d.getDate() + 1);
  d.setHours(0, 0, 0, 0);
  return d.toISOString();
}

/* ---- inline SVG helpers ---- */

function ChevronDown({ open }: { open: boolean }) {
  return (
    <svg
      className={`depth-slider__trigger-chevron${open ? " depth-slider__trigger-chevron--open" : ""}`}
      width="10"
      height="6"
      viewBox="0 0 10 6"
      fill="none"
      aria-hidden="true"
    >
      <path d="M1 1l4 4 4-4" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function CheckIcon() {
  return (
    <svg className="depth-slider__option-check" width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true">
      <path d="M3 7.5l2.5 2.5 5.5-6" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

export function DepthSlider({ value, onChange, autoResolved, disabled, compact }: Props) {
  const { t } = useTranslation();
  const [dropdownOpen, setDropdownOpen] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [decisions, setDecisions] = useState<DecisionEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [healingBanner, setHealingBanner] = useState<string | null>(null);

  const containerRef = useRef<HTMLDivElement>(null);

  /** 当前会话内手动覆盖自动的次数 */
  const overrideCountRef = useRef(0);

  // 挂载时仅跑一次异步自愈，但须调用最新的 onChange（父组件若换回调 ref 始终同步）
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  // ---- 自愈：启动时检查 autoManualOverrideUntil 是否已过期 ----
  useEffect(() => {
    if (!isTauri()) return;
    let cancelled = false;
    void (async () => {
      try {
        const cfg = await invoke<VaultConfigForUi>("get_vault_config_for_ui");
        if (cancelled) return;
        const until = cfg.cognitive?.autoManualOverrideUntil;
        if (until && new Date(until).getTime() <= Date.now()) {
          onChangeRef.current("auto");
          await invoke("save_vault_config_patch", {
            patch: { cognitive: { depthMode: "auto", autoManualOverrideUntil: null } },
          }).catch(() => {});
        }
      } catch {
        // 忽略
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const handleClick = useCallback(
    (mode: DepthMode) => {
      if (disabled) return;

      // 自愈计数：从 auto 切到手动档位
      if (value === "auto" && mode !== "auto") {
        overrideCountRef.current += 1;
        if (overrideCountRef.current > OVERRIDE_THRESHOLD) {
          const until = nextMidnightIso();
          onChange(mode);
          if (isTauri()) {
            invoke("save_vault_config_patch", {
              patch: { cognitive: { depthMode: mode, autoManualOverrideUntil: until } },
            }).catch(() => {});
          }
          setHealingBanner(t("depth.healingLocked"));
          window.setTimeout(() => setHealingBanner(null), 5000);
          return;
        }
      }

      if (mode === "auto") {
        overrideCountRef.current = 0;
      }

      onChange(mode);
      if (isTauri()) {
        invoke("save_vault_config_patch", {
          patch: { cognitive: { depthMode: mode } },
        }).catch(() => {});
      }
    },
    [onChange, disabled, value, t],
  );

  // ---- click-outside + Escape ----
  useEffect(() => {
    if (!dropdownOpen) return;
    const onMouseDown = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setDropdownOpen(false);
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        setDropdownOpen(false);
      }
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKeyDown, true);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKeyDown, true);
    };
  }, [dropdownOpen]);

  // 展开详情时加载决策日志
  useEffect(() => {
    if (!detailsOpen || !isTauri()) return;
    let cancelled = false;
    setLoading(true);
    invoke<DecisionEntry[]>("list_depth_decisions")
      .then((entries) => {
        if (!cancelled) setDecisions(entries);
      })
      .catch(() => {
        if (!cancelled) setDecisions([]);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [detailsOpen]);

  // ---- 计算 trigger 显示文本 ----
  const currentModeText =
    value === "auto" && autoResolved
      ? t("depth.autoResolved", { resolved: t(`depth.${autoResolved}`) })
      : t(`depth.${value}`);

  const description =
    value === "auto" && autoResolved
      ? t("depth.autoResolved", { resolved: t(`depth.${autoResolved}`) })
      : t(`depth.${value}Desc`);

  const fmtTime = (iso: string) => {
    try {
      const d = new Date(iso);
      return d.toLocaleString(undefined, {
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      });
    } catch {
      return iso;
    }
  };

  // ---- dropdown keyboard navigation ----
  const onDropdownKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    const focused = document.activeElement as HTMLElement | null;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      const next = focused?.nextElementSibling as HTMLElement | null;
      next?.focus();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      const prev = focused?.previousElementSibling as HTMLElement | null;
      prev?.focus();
    } else if (e.key === "Tab") {
      setDropdownOpen(false);
    } else if (e.key === "Home") {
      e.preventDefault();
      (el.firstElementChild as HTMLElement | null)?.focus();
    } else if (e.key === "End") {
      e.preventDefault();
      (el.lastElementChild as HTMLElement | null)?.focus();
    }
  };

  // auto-focus active option on dropdown open
  const dropdownRef = useCallback(
    (node: HTMLDivElement | null) => {
      if (!node) return;
      const activeBtn = node.querySelector("[aria-selected='true']") as HTMLElement | null;
      activeBtn?.focus();
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [dropdownOpen],
  );

  return (
    <div
      className={compact ? "depth-slider depth-slider--compact" : "depth-slider"}
      ref={containerRef}
      data-disabled={disabled || undefined}
    >
      {/* ---- Trigger ---- */}
      <button
        type="button"
        className="depth-slider__trigger"
        onClick={() =>
          setDropdownOpen((o) => {
            const next = !o;
            // 展开深度下拉时收起详情，避免与右栏布局叠压并减少「记忆」展开态
            if (next) setDetailsOpen(false);
            return next;
          })
        }
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={dropdownOpen}
        aria-label={compact ? `${t("depth.label")}: ${currentModeText}` : undefined}
      >
        {compact ? (
          <>
            <ChevronDown open={dropdownOpen} />
            <span className="depth-slider__trigger-value depth-slider__trigger-value--compact">{currentModeText}</span>
          </>
        ) : (
          <>
            <span className="depth-slider__trigger-label">{t("depth.label")}</span>
            <span className="depth-slider__trigger-value">{currentModeText}</span>
            <ChevronDown open={dropdownOpen} />
          </>
        )}
      </button>

      {/* ---- Dropdown ---- */}
      {dropdownOpen && !disabled && (
        <div
          className="depth-slider__dropdown"
          role="listbox"
          aria-label={t("depth.label")}
          ref={dropdownRef}
          onKeyDown={onDropdownKeyDown}
        >
          {MODES.map((mode) => {
            const isActive = mode === value;
            return (
              <button
                key={mode}
                type="button"
                role="option"
                aria-selected={isActive}
                className={`depth-slider__option${isActive ? " depth-slider__option--active" : ""}`}
                onClick={() => {
                  handleClick(mode);
                  setDropdownOpen(false);
                }}
              >
                <span className="depth-slider__option-name">{t(`depth.${mode}`)}</span>
                <span className="depth-slider__option-desc">{t(`depth.${mode}Desc`)}</span>
                {isActive && <CheckIcon />}
              </button>
            );
          })}
        </div>
      )}

      {!compact ? (
        <>
          {/* ---- Info row ---- */}
          <div className="depth-slider__info">
            <span className="depth-slider__desc">{description}</span>
            <button
              type="button"
              className="depth-slider__details-btn"
              onClick={() => setDetailsOpen((o) => !o)}
              aria-expanded={detailsOpen}
            >
              {t("depth.details")}
            </button>
          </div>

          {/* ---- Decision log ---- */}
          {detailsOpen && (
            <div className="depth-slider__details-panel">
              {loading ? (
                <span className="depth-slider__details-placeholder">
                  {t("depth.detailsLoading")}
                </span>
              ) : decisions.length === 0 ? (
                <span className="depth-slider__details-placeholder">
                  {t("depth.detailsPlaceholder")}
                </span>
              ) : (
                <ul className="depth-slider__decision-list">
                  {decisions.map((d, i) => (
                    <li key={i} className="depth-slider__decision-item">
                      <span className="depth-slider__decision-time">{fmtTime(d.timestamp)}</span>
                      <span className="depth-slider__decision-resolved">
                        {t(`depth.${d.autoResolved}`)}
                      </span>
                      {d.userOverride && (
                        <span className="depth-slider__decision-override">
                          {" "}
                          &rarr; {t(`depth.${d.userOverride}`)}
                        </span>
                      )}
                      <span className="depth-slider__decision-reason">{d.reason}</span>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}
        </>
      ) : null}

      {/* ---- Healing banner ---- */}
      {healingBanner && (
        <div className="depth-slider__healing-banner" role="status" aria-live="polite">
          {healingBanner}
        </div>
      )}
    </div>
  );
}
