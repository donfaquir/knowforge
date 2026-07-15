import { useCallback, useEffect, useRef, type RefObject } from "react";
import GithubSlugger from "github-slugger";

const PM_HEADING_SELECTOR =
  ".ProseMirror h1, .ProseMirror h2, .ProseMirror h3, .ProseMirror h4, .ProseMirror h5, .ProseMirror h6";

/** Wikilink `#` heading navigation: max wait for ProseMirror mount (ms) */
const WIKI_HEADING_NAV_RETRY_BUDGET_MS = 2500;

/** Find a ProseMirror heading DOM element by GitHub slug within a scroll container */
function findProseMirrorHeadingBySlug(
  scrollEl: HTMLElement | null,
  slug: string,
): HTMLElement | null {
  if (!scrollEl) {
    return null;
  }
  const slugger = new GithubSlugger();
  const headings = scrollEl.querySelectorAll(PM_HEADING_SELECTOR);
  for (const heading of headings) {
    if (!(heading instanceof HTMLElement)) {
      continue;
    }
    const text = heading.textContent?.trim() ?? "";
    if (slugger.slug(text) === slug) {
      return heading;
    }
  }
  return null;
}

function scrollMilkdownHeadingIntoView(scrollEl: HTMLElement, headingEl: HTMLElement) {
  const pad = 12;
  const cRect = scrollEl.getBoundingClientRect();
  const eRect = headingEl.getBoundingClientRect();
  const top = scrollEl.scrollTop + (eRect.top - cRect.top) - pad;
  scrollEl.scrollTo({ top: Math.max(0, top), behavior: "smooth" });
}

/**
 * Provides heading navigation within the Milkdown editor scroll container.
 * Supports both instant navigation (outline click) and retry-based navigation
 * (wikilink with `#fragment` — waits for ProseMirror to mount the target heading).
 */
export function useHeadingNavigation(
  editorScrollRef: RefObject<HTMLDivElement | null>,
): {
  navigateToHeading: (slug: string) => void;
  navigateToHeadingWithRetry: (slug: string) => void;
} {
  const retryGenerationRef = useRef(0);

  // Cancel any pending retry on unmount
  useEffect(() => {
    return () => {
      retryGenerationRef.current += 1;
    };
  }, []);

  const navigateToHeading = useCallback((slug: string) => {
    requestAnimationFrame(() => {
      const outer = editorScrollRef.current;
      const scrollEl = outer?.querySelector("[data-milkdown-root]") as HTMLElement | null;
      const el = findProseMirrorHeadingBySlug(scrollEl, slug);
      if (!(el instanceof HTMLElement) || !scrollEl) {
        return;
      }
      scrollMilkdownHeadingIntoView(scrollEl, el);
    });
  }, [editorScrollRef]);

  const navigateToHeadingWithRetry = useCallback((slug: string) => {
    const myGen = (retryGenerationRef.current += 1);
    const t0 = performance.now();

    const step = () => {
      if (retryGenerationRef.current !== myGen) {
        return;
      }
      if (performance.now() - t0 > WIKI_HEADING_NAV_RETRY_BUDGET_MS) {
        return;
      }
      const outer = editorScrollRef.current;
      const scrollEl = outer?.querySelector("[data-milkdown-root]") as HTMLElement | null;
      const el = findProseMirrorHeadingBySlug(scrollEl, slug);
      if (el && scrollEl) {
        scrollMilkdownHeadingIntoView(scrollEl, el);
        return;
      }
      requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  }, [editorScrollRef]);

  return { navigateToHeading, navigateToHeadingWithRetry };
}
