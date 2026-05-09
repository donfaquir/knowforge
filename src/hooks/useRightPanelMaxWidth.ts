import { useEffect, useMemo, useState } from "react";

const HALF_CAP = 0.5;
const MIN_RIGHT_W = 280;
/** AI / 思考网络「铺满」时为中栏保留的最小宽度，避免编辑区被挤到 0 */
const MIN_CENTER_WHEN_FILL = 120;

type Params = {
  /** true：右栏上限为视口减左侧栏与中栏最小保留（类全屏）；false：上限为视口 50% */
  fillRemainder: boolean;
  /** 左侧文件树等已占用宽度（侧栏折叠时为 0） */
  leftSidebarPx: number;
};

function readInnerWidth(): number {
  if (typeof window === "undefined") {
    return 1200;
  }
  return window.innerWidth;
}

/**
 * 右栏可拖拽最大宽度：大纲 / 回顾为视口一半；AI 与思考网络为剩余空间（全屏式）。
 */
export function useRightPanelMaxWidthPx({ fillRemainder, leftSidebarPx }: Params): number {
  const [viewportW, setViewportW] = useState(readInnerWidth);

  useEffect(() => {
    const onResize = () => setViewportW(readInnerWidth());
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  return useMemo(() => {
    if (!fillRemainder) {
      return Math.max(MIN_RIGHT_W, Math.floor(viewportW * HALF_CAP));
    }
    return Math.max(MIN_RIGHT_W, viewportW - leftSidebarPx - MIN_CENTER_WHEN_FILL);
  }, [fillRemainder, viewportW, leftSidebarPx]);
}
