/** kf-private 锁形图标：顶栏标签、侧栏树、主区「当前文档」条复用 */

type Props = {
  className?: string;
  /** 与 file-tree / editor-tab 内图标对齐的视口尺寸 */
  size?: number;
};

/** 私密笔记（kf-private）锁标，Feather 风格描边 */
export function KfPrivateLockIcon({ className, size = 14 }: Props) {
  const s = size;
  const sw = 1.65 * (size / 14);
  return (
    <svg
      className={className ? `kf-private-lock-icon ${className}` : "kf-private-lock-icon"}
      width={s}
      height={s}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={sw}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <rect x="5" y="11" width="14" height="10" rx="2" />
      <path d="M7 11V8a5 5 0 0 1 10 0v3" />
    </svg>
  );
}
