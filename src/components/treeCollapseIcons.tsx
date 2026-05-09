/** 左侧文件树、右侧大纲折叠三角、全局展开/折叠双箭头，以及右栏分段图标（大纲 / 回顾） */

/** 全局折叠：全部展开时双箭头朝外；任一分支收起时双箭头朝内 */
export function FolderBulkToggleIcon({ allExpanded }: { allExpanded: boolean }) {
  const stroke = 1.65;
  if (allExpanded) {
    return (
      <svg
        className="file-tree__bulk-toggle-svg"
        width="20"
        height="22"
        viewBox="0 0 24 28"
        fill="none"
        stroke="currentColor"
        strokeWidth={stroke}
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden={true}
      >
        {/* 上半：尖朝上；下半：尖朝下；中间留白 */}
        <path d="M7 11 12 5 17 11" />
        <path d="M7 17 12 23 17 17" />
      </svg>
    );
  }
  return (
    <svg
      className="file-tree__bulk-toggle-svg"
      width="20"
      height="22"
      viewBox="0 0 24 28"
      fill="none"
      stroke="currentColor"
      strokeWidth={stroke}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      {/* 上半尖朝下、下半尖朝上，相向但不共点 */}
      <path d="M7 8 12 12 17 8" />
      <path d="M7 20 12 16 17 20" />
    </svg>
  );
}

export function DirChevronIcon({ expanded }: { expanded: boolean }) {
  return (
    <svg
      className={`file-tree__chevron-svg${expanded ? " is-expanded" : ""}`}
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="m9 18 6-6-6-6" />
    </svg>
  );
}

/** 右栏分段：大纲（描边与大纲行 DirChevronIcon 一致） */
export function RightPanelOutlineIcon() {
  return (
    <svg
      className="right-panel-shell__segment-svg"
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="M5 8h14M5 12h10M5 16h12" />
    </svg>
  );
}

/** 右栏分段：链接推荐（火花，与面板内「获取推荐」语义一致） */
export function RightPanelLinkRecIcon() {
  return (
    <svg
      className="right-panel-shell__segment-svg"
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="m12 3-1.912 5.813a2 2 0 0 1-1.275 1.275L3 12l5.813 1.912a2 2 0 0 1 1.275 1.275L12 21l1.912-5.813a2 2 0 0 1 1.275-1.275L21 12l-5.813-1.912a2 2 0 0 1-1.275-1.275L12 3Z" />
      <path d="M5 3v3" />
      <path d="M19 18v3" />
      <path d="M3 5h3" />
      <path d="M18 19h3" />
    </svg>
  );
}

/** 右栏分段：理解网络（节点 + 连线示意） */
export function RightPanelGraphIcon() {
  return (
    <svg
      className="right-panel-shell__segment-svg"
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <circle cx="6" cy="8" r="2.5" fill="currentColor" stroke="none" />
      <circle cx="18" cy="6" r="2.5" fill="currentColor" stroke="none" />
      <circle cx="15" cy="17" r="2.5" fill="currentColor" stroke="none" />
      <path d="M8 9.5 13.5 6.5M16.5 8 14.5 15" />
    </svg>
  );
}

/** 右栏分段：挑战回顾（书本 + 行，与大纲图标同尺寸） */
export function RightPanelReviewIcon() {
  return (
    <svg
      className="right-panel-shell__segment-svg"
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20" />
      <path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z" />
      <path d="M8 7h8" />
      <path d="M8 11h8" />
    </svg>
  );
}
