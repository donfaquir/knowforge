import { invoke } from "@tauri-apps/api/core";
import {
  forceCenter,
  forceCollide,
  forceLink,
  forceManyBody,
  forceRadial,
  forceSimulation,
  type Simulation,
  type SimulationLinkDatum,
} from "d3-force";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { UnderstandingGraphEdge, UnderstandingGraphForUi, UnderstandingGraphNode } from "../types/understandingGraph";
import "./UnderstandingGraphPanel.css";

const REFRESH_ICON_STROKE = 1.65;

/** 径向映射：对 [0,1] 重要性做 t^γ（γ<1），低重要性不会全部钉在最外缘，减轻「内外两坨 + 空心环」 */
const RADIAL_IMPORTANCE_T_GAMMA = 0.65;

/** 径向力强度（整体弱于旧版 ~0.082~0.13），中心隐喻保留，链力/斥力更易填满中间带 */
const RADIAL_STRENGTH_MIN = 0.052;
const RADIAL_STRENGTH_MAX = 0.084;

function IconExpandWide() {
  return (
    <svg
      className="understanding-graph__toolbar-icon-svg"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={REFRESH_ICON_STROKE}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="M15 3h6v6" />
      <path d="M9 21H3v-6" />
      <path d="M21 3l-7 7" />
      <path d="M3 21l7-7" />
    </svg>
  );
}

function IconRefreshCw() {
  return (
    <svg
      className="understanding-graph__toolbar-icon-svg"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={REFRESH_ICON_STROKE}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8" />
      <path d="M21 3v5h-5" />
      <path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16" />
      <path d="M8 16H3v5" />
    </svg>
  );
}

type SimNode = UnderstandingGraphNode & {
  x?: number;
  y?: number;
  vx?: number;
  vy?: number;
  fx?: number | null;
  fy?: number | null;
  r: number;
};

/** 供拖动开始/结束时重配力（d3 在 setter 时重算缓存） */
type SimForcesBundle = {
  link: ReturnType<typeof forceLink<SimNode, SimulationLinkDatum<SimNode>>>;
  charge: ReturnType<typeof forceManyBody<SimNode>>;
  center: ReturnType<typeof forceCenter>;
};

function linkTouchesPath(link: SimulationLinkDatum<SimNode>, relPath: string): boolean {
  const s = link.source as SimNode | string;
  const t = link.target as SimNode | string;
  const sp = typeof s === "string" ? s : s.relPath;
  const tp = typeof t === "string" ? t : t.relPath;
  return sp === relPath || tp === relPath;
}

/** Vault 相对路径：先统一为 `/` 再取最后一段，避免仅含 `\` 时标签整段路径 */
function basenameRel(relPath: string): string {
  const n = relPath.replace(/\\/g, "/");
  const i = n.lastIndexOf("/");
  return i >= 0 ? n.slice(i + 1) : n;
}

function maturityRank(m: string): number {
  if (m === "mature") return 3;
  if (m === "growing") return 2;
  return 1;
}

/** 想法越多、成熟度越高 → 权重越大 → 径向层更靠中心（与是否灰色无关，公式统一） */
function nodeImportance(d: SimNode): number {
  return d.thoughtCount * 2.2 + maturityRank(d.maxMaturity) * 4;
}

function maturityColor(m: string): string {
  if (m === "mature") return "#8b5cf6";
  if (m === "growing") return "#3b82f6";
  return "#22c55e";
}

/** 无随手想法：灰色；有想法：按最高成熟度着色 */
function nodeFill(d: SimNode): string {
  if (d.thoughtCount === 0) {
    return "#9ca3af";
  }
  return maturityColor(d.maxMaturity);
}

function nodeRadius(thoughtCount: number): number {
  return 10 + Math.min(14, thoughtCount * 2);
}

type Props = {
  workspaceReady: boolean;
  /** 当前工作区根路径；切换时必须参与刷新，否则仅依赖 workspaceReady 会因批处理跳过 effect */
  workspaceRoot: string | null;
  /** 点击节点打开笔记 */
  onOpenNote: (relPath: string) => void;
  tauriRuntime: boolean;
  /** 加宽约视口 70% / 再次点击还原；未传则不显示按钮 */
  onTogglePanelWide?: () => void;
  /** 与 onTogglePanelWide 配套，用于 aria 与提示文案 */
  graphPanelWideExpanded?: boolean;
};

export function UnderstandingGraphPanel({
  workspaceReady,
  workspaceRoot,
  onOpenNote,
  tauriRuntime,
  onTogglePanelWide,
  graphPanelWideExpanded = false,
}: Props) {
  const { t } = useTranslation();
  /** 丢弃过期的 scan 结果（切换工作区或快速连点刷新时） */
  const loadGenRef = useRef(0);
  const wrapRef = useRef<HTMLDivElement>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  const simRef = useRef<Simulation<SimNode, SimulationLinkDatum<SimNode>> | null>(null);
  const simForcesRef = useRef<SimForcesBundle | null>(null);
  const nodesRef = useRef<SimNode[]>([]);
  const nodeDragRef = useRef<{ px: number; py: number; moved: boolean } | null>(null);
  const suppressNextNodeClickRef = useRef(false);
  /** 将力导向 tick 与拖拽重绘合并到每帧最多一次，避免整 SVG 高频 setState 造成残影 */
  const bumpRafRef = useRef<number | null>(null);
  const [, setTick] = useState(0);
  const [dims, setDims] = useState({ w: 400, h: 320 });
  const [data, setData] = useState<UnderstandingGraphForUi | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [hoverTip, setHoverTip] = useState<{ left: number; top: number; count: number } | null>(null);
  const [view, setView] = useState({ k: 1, x: 0, y: 0 });
  const viewRef = useRef(view);
  viewRef.current = view;
  const dragRef = useRef<
    | { kind: "pan"; sx: number; sy: number; vx: number; vy: number }
    | { kind: "node"; node: SimNode; sx: number; sy: number }
    | null
  >(null);

  /** 默认：均衡链长与斥力；径向层由 radial 力单独表达 */
  const applyIdleGraphPhysics = useCallback(() => {
    const f = simForcesRef.current;
    const sim = simRef.current;
    if (!f) {
      return;
    }
    f.link.strength(0.68).distance(82);
    f.charge.strength(-168);
    if (sim) {
      sim.force("center", f.center);
    }
  }, []);

  /** 拖动：牵动靠链力；质心力在「单点被 fx 钉住」时会把其余点往反方向推，故暂时关掉 center */
  const applyDragGraphPhysics = useCallback(() => {
    const f = simForcesRef.current;
    const sim = simRef.current;
    const drag = dragRef.current;
    if (!f || drag?.kind !== "node") {
      return;
    }
    const rel = drag.node.relPath;
    f.link
      .strength((l) => (linkTouchesPath(l, rel) ? 0.96 : 0.34))
      .distance((l) => (linkTouchesPath(l, rel) ? 42 : 96));
    // 斥力过强时两点会沿「互相远离」运动，易与拖动方向相反；略强于空闲即可
    f.charge.strength(-198);
    if (sim) {
      sim.force("center", null);
    }
  }, []);

  const scheduleRepaint = useCallback(() => {
    if (bumpRafRef.current != null) {
      return;
    }
    bumpRafRef.current = requestAnimationFrame(() => {
      bumpRafRef.current = null;
      setTick((n) => n + 1);
    });
  }, []);

  const load = useCallback(async () => {
    const gen = ++loadGenRef.current;
    if (!workspaceReady || !tauriRuntime || !workspaceRoot) {
      setData(null);
      setLoadError(null);
      setHoverTip(null);
      if (gen === loadGenRef.current) {
        setLoading(false);
      }
      return;
    }
    setData(null);
    setLoading(true);
    setLoadError(null);
    setHoverTip(null);
    try {
      const r = await invoke<UnderstandingGraphForUi>("scan_understanding_graph");
      if (gen !== loadGenRef.current) {
        return;
      }
      setData(r);
    } catch (e) {
      if (gen !== loadGenRef.current) {
        return;
      }
      setLoadError(e instanceof Error ? e.message : String(e));
      setData(null);
    } finally {
      if (gen === loadGenRef.current) {
        setLoading(false);
      }
    }
  }, [workspaceReady, tauriRuntime, workspaceRoot]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    return () => {
      if (bumpRafRef.current != null) {
        cancelAnimationFrame(bumpRafRef.current);
        bumpRafRef.current = null;
      }
    };
  }, []);

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) {
      return;
    }
    const ro = new ResizeObserver((entries) => {
      const cr = entries[0]?.contentRect;
      if (cr && cr.width > 40 && cr.height > 40) {
        setDims({ w: Math.floor(cr.width), h: Math.floor(cr.height) });
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    simRef.current?.stop();
    simRef.current = null;
    if (!data?.nodes.length || dims.w < 50 || dims.h < 50) {
      if (bumpRafRef.current != null) {
        cancelAnimationFrame(bumpRafRef.current);
        bumpRafRef.current = null;
      }
      nodesRef.current = [];
      scheduleRepaint();
      return;
    }

    const nodes: SimNode[] = data.nodes.map((n) => ({
      ...n,
      r: nodeRadius(n.thoughtCount),
    }));
    const byPath = new Map(nodes.map((n) => [n.relPath, n]));
    const linksRaw: SimulationLinkDatum<SimNode>[] = (data.edges as UnderstandingGraphEdge[])
      .filter((e) => byPath.has(e.fromRelPath) && byPath.has(e.toRelPath))
      .map((e) => ({ source: e.fromRelPath, target: e.toRelPath }));

    const cx = dims.w / 2;
    const cy = dims.h / 2;
    // 初始略靠中心的小环，减少仿真从外缘收拢的时间
    const ringR = Math.min(dims.w, dims.h) * 0.14;
    nodes.forEach((n, i) => {
      const a = (i / Math.max(nodes.length, 1)) * Math.PI * 2;
      n.x = cx + Math.cos(a) * ringR;
      n.y = cy + Math.sin(a) * ringR;
    });

    const imp = nodes.map(nodeImportance);
    const impMin = Math.min(...imp);
    const impMax = Math.max(...imp);
    const impSpread = Math.max(1e-6, impMax - impMin);
    const bandMin = Math.min(dims.w, dims.h) * 0.05;
    const bandMax = Math.min(dims.w, dims.h) * 0.4;
    const radialT = (d: SimNode): number => {
      const t0 = (nodeImportance(d) - impMin) / impSpread;
      const c = Math.min(1, Math.max(0, t0));
      return Math.pow(c, RADIAL_IMPORTANCE_T_GAMMA);
    };
    const targetRadius = (d: SimNode) => {
      const t = radialT(d);
      return bandMin + (1 - t) * (bandMax - bandMin);
    };
    const radialStrength = (d: SimNode) => {
      const t = radialT(d);
      return RADIAL_STRENGTH_MIN + t * (RADIAL_STRENGTH_MAX - RADIAL_STRENGTH_MIN);
    };

    const linkForce = forceLink<SimNode, SimulationLinkDatum<SimNode>>(linksRaw)
      .id((d) => d.relPath)
      .distance(82)
      .strength(0.68);
    const chargeForce = forceManyBody<SimNode>().strength(-168);
    const centerForce = forceCenter(cx, cy);
    const radialForce = forceRadial(targetRadius, cx, cy).strength(radialStrength);
    const collideForce = forceCollide<SimNode>()
      .radius((d) => d.r + 8)
      .iterations(3);

    const sim = forceSimulation<SimNode>(nodes)
      .force("link", linkForce)
      .force("charge", chargeForce)
      .force("center", centerForce)
      .force("radial", radialForce)
      .force("collide", collideForce)
      .alphaDecay(0.019)
      .velocityDecay(0.34);

    simForcesRef.current = {
      link: linkForce,
      charge: chargeForce,
      center: centerForce,
    };

    sim.on("tick", () => scheduleRepaint());
    simRef.current = sim;
    nodesRef.current = nodes;
    scheduleRepaint();

    return () => {
      if (bumpRafRef.current != null) {
        cancelAnimationFrame(bumpRafRef.current);
        bumpRafRef.current = null;
      }
      sim.stop();
      sim.on("tick", null);
      simForcesRef.current = null;
      if (simRef.current === sim) {
        simRef.current = null;
      }
    };
  }, [data, dims.w, dims.h, scheduleRepaint]);

  const onWheelSvg = (e: React.WheelEvent<SVGSVGElement>) => {
    e.preventDefault();
    const svg = svgRef.current;
    if (!svg) {
      return;
    }
    const rect = svg.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;
    const factor = e.deltaY > 0 ? 0.92 : 1.08;
    setView((prev) => {
      const nk = Math.min(3, Math.max(0.35, prev.k * factor));
      const scale = nk / prev.k;
      const nx = mx - (mx - prev.x) * scale;
      const ny = my - (my - prev.y) * scale;
      return { k: nk, x: nx, y: ny };
    });
  };

  const onPointerDownSvg = (e: React.PointerEvent<SVGSVGElement>) => {
    if (e.target !== e.currentTarget) {
      return;
    }
    e.preventDefault();
    (e.currentTarget as SVGSVGElement).setPointerCapture(e.pointerId);
    const v = viewRef.current;
    dragRef.current = { kind: "pan", sx: e.clientX, sy: e.clientY, vx: v.x, vy: v.y };
  };

  const onPointerMoveSvg = (e: React.PointerEvent<SVGSVGElement>) => {
    const d = dragRef.current;
    if (!d) {
      return;
    }
    if (d.kind === "pan") {
      setView((v) => ({
        ...v,
        x: d.vx + (e.clientX - d.sx),
        y: d.vy + (e.clientY - d.sy),
      }));
    } else {
      const svg = svgRef.current;
      if (!svg) {
        return;
      }
      if (nodeDragRef.current) {
        const dx = e.clientX - nodeDragRef.current.px;
        const dy = e.clientY - nodeDragRef.current.py;
        if (dx * dx + dy * dy > 16) {
          nodeDragRef.current.moved = true;
        }
      }
      const rect = svg.getBoundingClientRect();
      const v = viewRef.current;
      const mx = (e.clientX - rect.left - v.x) / v.k;
      const my = (e.clientY - rect.top - v.y) / v.k;
      // 直接写坐标，避免每次 pointermove 都 restart 仿真叠乘 React 重绘导致残影
      d.node.fx = mx;
      d.node.fy = my;
      d.node.x = mx;
      d.node.y = my;
      scheduleRepaint();
    }
  };

  const endDrag = (e: React.PointerEvent<SVGSVGElement>) => {
    const d = dragRef.current;
    if (d?.kind === "node") {
      suppressNextNodeClickRef.current = nodeDragRef.current?.moved ?? false;
      const n = d.node;
      n.vx = 0;
      n.vy = 0;
      // 钉在松手位置：清空 fx/fy 时径向/链力会把点拉回，无法「留在拖放处」；其余点仍自由，质心+径向会回补中心空白
      const x = n.x ?? 0;
      const y = n.y ?? 0;
      n.fx = x;
      n.fy = y;
      n.x = x;
      n.y = y;
      applyIdleGraphPhysics();
      const sim = simRef.current;
      if (sim) {
        sim.alphaTarget(0).alpha(0.42).restart();
      }
    }
    dragRef.current = null;
    nodeDragRef.current = null;
    try {
      (e.currentTarget as SVGSVGElement).releasePointerCapture(e.pointerId);
    } catch {
      /* ignore */
    }
  };

  const onPointerDownNode = (e: React.PointerEvent, node: SimNode) => {
    e.stopPropagation();
    // 不可对 pointerdown 调用 preventDefault：会阻断后续 click，节点无法打开笔记；焦点样式由 CSS :focus-visible 控制
    const svg = svgRef.current;
    if (svg) {
      const rect = svg.getBoundingClientRect();
      const v = viewRef.current;
      const mx = (e.clientX - rect.left - v.x) / v.k;
      const my = (e.clientY - rect.top - v.y) / v.k;
      node.fx = mx;
      node.fy = my;
      node.x = mx;
      node.y = my;
    }
    (svgRef.current as SVGSVGElement | null)?.setPointerCapture(e.pointerId);
    dragRef.current = { kind: "node", node, sx: e.clientX, sy: e.clientY };
    nodeDragRef.current = { px: e.clientX, py: e.clientY, moved: false };
    setHoverTip(null);
    applyDragGraphPhysics();
    const sim = simRef.current;
    if (sim) {
      sim.alphaTarget(0.095).alpha(0.42).restart();
    }
  };

  const nodes = nodesRef.current;
  const showGraph = nodes.length > 0 && !loading;

  return (
    <div className="understanding-graph" ref={wrapRef}>
      <div className="understanding-graph__toolbar">
        <p className="understanding-graph__metrics" role="status">
          {data
            ? t("understandingGraph.metrics", {
                nodes: data.nodes.length,
                notes: data.indexedMarkdownCount,
              })
            : loading
              ? t("understandingGraph.loading")
              : "\u00a0"}
        </p>
        {data && data.hiddenNodeCount > 0 ? (
          <p className="understanding-graph__hidden">{t("understandingGraph.hidden", { n: data.hiddenNodeCount })}</p>
        ) : null}
        <div className="understanding-graph__toolbar-actions">
          {onTogglePanelWide ? (
            <button
              type="button"
              className="understanding-graph__toolbar-icon-btn"
              onClick={onTogglePanelWide}
              disabled={!workspaceReady || !tauriRuntime}
              aria-pressed={graphPanelWideExpanded}
              aria-label={
                graphPanelWideExpanded
                  ? t("understandingGraph.collapseWide")
                  : t("understandingGraph.expandWide")
              }
              title={
                graphPanelWideExpanded
                  ? t("understandingGraph.collapseWideTitle")
                  : t("understandingGraph.expandWideTitle")
              }
            >
              <IconExpandWide />
            </button>
          ) : null}
          <button
            type="button"
            className="understanding-graph__toolbar-icon-btn"
            onClick={() => void load()}
            disabled={loading || !workspaceReady || !tauriRuntime}
            aria-label={t("understandingGraph.refresh")}
            title={t("understandingGraph.refresh")}
          >
            <IconRefreshCw />
          </button>
        </div>
      </div>
      {loadError ? <p className="understanding-graph__error">{loadError}</p> : null}
      {!tauriRuntime ? <p className="understanding-graph__empty">{t("understandingGraph.webEmpty")}</p> : null}
      {tauriRuntime && !loading && !loadError && data && data.nodes.length === 0 ? (
        <p className="understanding-graph__empty">{t("understandingGraph.emptyVault")}</p>
      ) : null}
      {loading ? <p className="understanding-graph__loading">{t("understandingGraph.scanning")}</p> : null}
      {showGraph ? (
        <svg
          ref={svgRef}
          className="understanding-graph__svg"
          width={dims.w}
          height={dims.h}
          role="img"
          aria-label={t("understandingGraph.graphAria")}
          onWheel={onWheelSvg}
          onPointerDown={onPointerDownSvg}
          onPointerMove={onPointerMoveSvg}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
          onPointerLeave={() => {
            setHoverTip(null);
          }}
        >
          <g transform={`translate(${view.x},${view.y}) scale(${view.k})`}>
            {(data?.edges ?? []).map((e, i) => {
              const s = nodes.find((n) => n.relPath === e.fromRelPath);
              const t = nodes.find((n) => n.relPath === e.toRelPath);
              if (!s?.x || !s.y || !t?.x || !t.y) {
                return null;
              }
              return (
                <line
                  key={`${e.fromRelPath}-${e.toRelPath}-${i}`}
                  className="understanding-graph__edge"
                  x1={s.x}
                  y1={s.y}
                  x2={t.x}
                  y2={t.y}
                />
              );
            })}
            {nodes.map((n) => {
              const x = n.x ?? 0;
              const y = n.y ?? 0;
              const label = basenameRel(n.relPath);
              return (
                <g
                  key={n.relPath}
                  className="understanding-graph__node-group"
                  role="button"
                  tabIndex={0}
                  aria-label={t("understandingGraph.openNote", { path: n.relPath })}
                  onKeyDown={(ev) => {
                    if (ev.key === "Enter" || ev.key === " ") {
                      ev.preventDefault();
                      onOpenNote(n.relPath);
                    }
                  }}
                  onPointerEnter={(ev) => {
                    const wrap = wrapRef.current;
                    if (!wrap) {
                      return;
                    }
                    const br = wrap.getBoundingClientRect();
                    setHoverTip({
                      left: ev.clientX - br.left + 10,
                      top: ev.clientY - br.top + 10,
                      count: n.thoughtCount,
                    });
                  }}
                  onPointerLeave={() => {
                    setHoverTip(null);
                  }}
                  onPointerDown={(ev) => onPointerDownNode(ev, n)}
                  onClick={() => {
                    if (suppressNextNodeClickRef.current) {
                      suppressNextNodeClickRef.current = false;
                      return;
                    }
                    onOpenNote(n.relPath);
                  }}
                >
                  <circle
                    className="understanding-graph__node"
                    cx={x}
                    cy={y}
                    r={n.r}
                    fill={nodeFill(n)}
                    stroke="rgba(0,0,0,0.2)"
                    strokeWidth={1}
                    tabIndex={0}
                    role="presentation"
                    aria-hidden={true}
                  />
                  <text
                    className="understanding-graph__label"
                    x={x}
                    y={y + n.r + 12}
                    textAnchor="middle"
                    fontSize={10}
                    fill="currentColor"
                  >
                    {label.length > 22 ? `${label.slice(0, 20)}…` : label}
                  </text>
                </g>
              );
            })}
          </g>
        </svg>
      ) : null}
      {hoverTip ? (
        <div
          className="understanding-graph__thought-tip"
          style={{ left: hoverTip.left, top: hoverTip.top }}
          role="tooltip"
        >
          {t("understandingGraph.thoughtCountHover", { count: hoverTip.count })}
        </div>
      ) : null}
    </div>
  );
}
