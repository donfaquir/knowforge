export type WikiSuggestOpenPayload = {
  anchor: number;
  head: number;
  filter: string;
};

export type WikiSuggestSnapshot =
  | { open: false }
  | ({ open: true } & WikiSuggestOpenPayload);

let snapshot: WikiSuggestSnapshot = { open: false };
const listeners = new Set<() => void>();

/** 用户 Esc / 点击外部关闭后，在同一未完成 `[[` 的 anchor 上抑制立即重开 */
let dismissedAnchor: number | null = null;

export function getWikiSuggestSnapshot(): WikiSuggestSnapshot {
  return snapshot;
}

export function subscribeWikiSuggest(onStoreChange: () => void): () => void {
  listeners.add(onStoreChange);
  return () => {
    listeners.delete(onStoreChange);
  };
}

function emit(): void {
  for (const fn of listeners) {
    fn();
  }
}

export function setWikiSuggestSnapshot(next: WikiSuggestSnapshot): void {
  if (next.open === snapshot.open) {
    if (!next.open && !snapshot.open) {
      return;
    }
    if (next.open && snapshot.open) {
      if (
        next.anchor === snapshot.anchor &&
        next.head === snapshot.head &&
        next.filter === snapshot.filter
      ) {
        return;
      }
    }
  }
  snapshot = next;
  emit();
}

export function dismissWikiSuggestAtAnchor(anchor: number): void {
  dismissedAnchor = anchor;
  setWikiSuggestSnapshot({ open: false });
}

export function clearWikiSuggestDismissIfStale(anchor: number | null): void {
  if (dismissedAnchor != null && dismissedAnchor !== anchor) {
    dismissedAnchor = null;
  }
  if (anchor == null) {
    dismissedAnchor = null;
  }
}

export function isWikiSuggestDismissedForAnchor(anchor: number): boolean {
  return dismissedAnchor === anchor;
}

/** 供插入后清理 */
export function clearWikiSuggestDismiss(): void {
  dismissedAnchor = null;
}
