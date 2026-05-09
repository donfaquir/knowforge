import { useCallback, useEffect, useRef, useState } from "react";

type ResizableOptions = {
  /** 拖拽方向：left = 面板在左侧（拖拽条在右边缘），right = 面板在右侧（拖拽条在左边缘） */
  side: "left" | "right";
  defaultWidth: number;
  minWidth: number;
  maxWidth: number;
};

type ResizableResult = {
  width: number;
  isDragging: boolean;
  handleMouseDown: (e: React.MouseEvent) => void;
  /** 按像素设宽（仍受 min/max 钳制），供工具栏「一键加宽」等 */
  setProgrammaticWidth: (px: number) => void;
};

export function useResizable({
  side,
  defaultWidth,
  minWidth,
  maxWidth,
}: ResizableOptions): ResizableResult {
  const [width, setWidth] = useState(defaultWidth);
  const dragging = useRef(false);
  const startX = useRef(0);
  const startWidth = useRef(0);
  const [isDragging, setIsDragging] = useState(false);

  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      dragging.current = true;
      startX.current = e.clientX;
      startWidth.current = width;
      setIsDragging(true);
    },
    [width],
  );

  /** maxWidth / minWidth 随窗口变化时钳制当前宽度，避免右栏在缩小窗口后仍宽于上限 */
  useEffect(() => {
    setWidth((w) => Math.min(maxWidth, Math.max(minWidth, w)));
  }, [maxWidth, minWidth]);

  useEffect(() => {
    const onMouseMove = (e: MouseEvent) => {
      if (!dragging.current) return;
      const delta = e.clientX - startX.current;
      const newWidth =
        side === "left"
          ? startWidth.current + delta
          : startWidth.current - delta;
      setWidth(Math.min(maxWidth, Math.max(minWidth, newWidth)));
    };

    const onMouseUp = () => {
      if (!dragging.current) return;
      dragging.current = false;
      setIsDragging(false);
    };

    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
    return () => {
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
    };
  }, [side, minWidth, maxWidth]);

  const setProgrammaticWidth = useCallback(
    (px: number) => {
      const next = Math.floor(px);
      setWidth(Math.min(maxWidth, Math.max(minWidth, next)));
    },
    [maxWidth, minWidth],
  );

  return { width, isDragging, handleMouseDown, setProgrammaticWidth };
}
