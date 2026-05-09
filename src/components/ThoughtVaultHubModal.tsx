import { invoke, isTauri } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { VaultThoughtListPage, VaultThoughtListRow } from "../types/cognitiveTypes";
import "./ThoughtVaultHubModal.css";

type Props = {
  open: boolean;
  onClose: () => void;
  workspaceReady: boolean;
  tauriRuntime: boolean;
  onOpenNote: (relPath: string) => void;
};

export function ThoughtVaultHubModal({
  open,
  onClose,
  workspaceReady,
  tauriRuntime,
  onOpenNote,
}: Props) {
  const { t } = useTranslation();
  const [q, setQ] = useState("");
  const [rows, setRows] = useState<VaultThoughtListRow[]>([]);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const qRef = useRef(q);
  qRef.current = q;

  const refresh = useCallback(async () => {
    if (!open || !workspaceReady || !tauriRuntime || !isTauri()) {
      return;
    }
    setLoading(true);
    setErr(null);
    try {
      const page = await invoke<VaultThoughtListPage>("list_vault_thoughts", {
        args: { query: qRef.current.trim() || null, limit: 500, offset: 0 },
      });
      setRows(page.rows);
    } catch (e) {
      setRows([]);
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [open, workspaceReady, tauriRuntime]);

  useEffect(() => {
    if (open) {
      setQ("");
    }
  }, [open]);

  useEffect(() => {
    if (!open) {
      return;
    }
    const delay = q.trim() ? 280 : 0;
    const id = window.setTimeout(() => {
      void refresh();
    }, delay);
    return () => window.clearTimeout(id);
  }, [open, q, refresh]);

  if (!open) {
    return null;
  }

  return (
    <div
      className="thought-vault-hub-backdrop"
      role="presentation"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="thought-vault-hub" role="dialog" aria-label={t("thoughtVaultHub.title")}>
        <header className="thought-vault-hub__head">
          <h2 className="thought-vault-hub__title">{t("thoughtVaultHub.title")}</h2>
          <button type="button" className="thought-vault-hub__close" onClick={onClose}>
            {t("thoughtVaultHub.close")}
          </button>
        </header>
        <input
          className="thought-vault-hub__search"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder={t("thoughtVaultHub.placeholder")}
          aria-label={t("thoughtVaultHub.placeholder")}
        />
        {loading ? <p className="thought-vault-hub__muted">{t("thoughtVaultHub.loading")}</p> : null}
        {err ? (
          <p className="thought-vault-hub__err" role="alert">
            {err}
          </p>
        ) : null}
        {!loading && rows.length === 0 ? (
          <p className="thought-vault-hub__muted">{t("thoughtVaultHub.empty")}</p>
        ) : null}
        <ul className="thought-vault-hub__list">
          {rows.map((r) => (
            <li key={`${r.relPath}-${r.thoughtId}`} className="thought-vault-hub__li">
              <button
                type="button"
                className="thought-vault-hub__row"
                onClick={() => {
                  onOpenNote(r.relPath);
                  onClose();
                }}
              >
                <span className="thought-vault-hub__path">{r.relPath}</span>
                <code className="thought-vault-hub__tid">{r.thoughtId}</code>
                <span className="thought-vault-hub__excerpt">{r.excerpt}</span>
                <span className="thought-vault-hub__meta">
                  {r.maturity}
                  {r.temporary ? ` · ${t("thoughtPanel.temporary")}` : ""}
                </span>
              </button>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
