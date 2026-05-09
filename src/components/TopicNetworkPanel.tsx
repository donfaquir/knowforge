import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { message } from "@tauri-apps/plugin-dialog";
import {
  forceCenter,
  forceCollide,
  forceLink,
  forceManyBody,
  forceSimulation,
  type Simulation,
  type SimulationLinkDatum,
} from "d3-force";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type {
  AddManualTopicResult,
  DocNode,
  TopicDocEdge,
  TopicNetworkForUi,
  TopicNode,
  TopicTopicEdge,
} from "../types/topicNetworkTypes";
import "./TopicNetworkPanel.css";

const REFRESH_ICON_STROKE = 1.65;

function IconRefreshCw() {
  return (
    <svg
      className="topic-network__toolbar-icon-svg"
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

/** 圆内加号：新增主题 */
function IconAddTopic() {
  return (
    <svg
      className="topic-network__toolbar-icon-svg"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={REFRESH_ICON_STROKE}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <circle cx="12" cy="12" r="9" />
      <path d="M12 8.5v7M8.5 12h7" />
    </svg>
  );
}

/** 下载线：导出索引 */
function IconExportMarkdown() {
  return (
    <svg
      className="topic-network__toolbar-icon-svg"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={REFRESH_ICON_STROKE}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="M12 4v11" />
      <path d="m8 11 4 4 4-4" />
      <path d="M5 20h14" />
    </svg>
  );
}

function basenameRel(relPath: string): string {
  const n = relPath.replace(/\\/g, "/");
  const i = n.lastIndexOf("/");
  return i >= 0 ? n.slice(i + 1) : n;
}

function simTopicId(id: string): string {
  return `t:${id}`;
}

function simDocId(relPath: string): string {
  return `d:${relPath}`;
}

function maturityColor(m: string): string {
  if (m === "mature") return "#8b5cf6";
  if (m === "growing") return "#3b82f6";
  return "#22c55e";
}

function docFill(d: DocNode): string {
  if (d.thoughtCount === 0) {
    return "#9ca3af";
  }
  return maturityColor(d.maxMaturity);
}

function topicFill(docCount: number, maxCount: number): string {
  if (maxCount <= 0) {
    return "#93c5fd";
  }
  const t = Math.min(1, docCount / maxCount);
  const r = Math.round(147 + (37 - 147) * t);
  const g = Math.round(197 + (99 - 197) * t);
  const b = Math.round(253 + (235 - 253) * t);
  return `rgb(${r},${g},${b})`;
}

type SimTopic = TopicNode & {
  simId: string;
  kind: "topic";
  x?: number;
  y?: number;
  vx?: number;
  vy?: number;
  fx?: number | null;
  fy?: number | null;
  r: number;
};

type SimDoc = DocNode & {
  simId: string;
  kind: "doc";
  x?: number;
  y?: number;
  vx?: number;
  vy?: number;
  fx?: number | null;
  fy?: number | null;
  r: number;
};

type SimNode = SimTopic | SimDoc;

type SimForcesBundle = {
  link: ReturnType<typeof forceLink<SimNode, SimulationLinkDatum<SimNode>>>;
  charge: ReturnType<typeof forceManyBody<SimNode>>;
  center: ReturnType<typeof forceCenter>;
};

function linkTouchesSimId(link: SimulationLinkDatum<SimNode>, simId: string): boolean {
  const s = link.source as SimNode | string;
  const t = link.target as SimNode | string;
  const sid = typeof s === "string" ? s : s.simId;
  const tid = typeof t === "string" ? t : t.simId;
  return sid === simId || tid === simId;
}

type Props = {
  workspaceReady: boolean;
  /** 当前工作区根；切换时需触发重建，避免 workspaceReady 批处理导致仍显示旧库图 */
  workspaceRoot: string | null;
  tauriRuntime: boolean;
  onOpenNote: (relPath: string) => void;
  onTogglePanelWide?: () => void;
  graphPanelWideExpanded?: boolean;
};

type TopicExtractProgressPayload = {
  current: number;
  total: number;
};

export function TopicNetworkPanel({
  workspaceReady,
  workspaceRoot,
  tauriRuntime,
  onOpenNote,
  onTogglePanelWide,
  graphPanelWideExpanded = false,
}: Props) {
  const { t } = useTranslation();
  /** 丢弃过期的 build 结果 */
  const loadGenRef = useRef(0);
  const wrapRef = useRef<HTMLDivElement>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  const simRef = useRef<Simulation<SimNode, SimulationLinkDatum<SimNode>> | null>(null);
  const simForcesRef = useRef<SimForcesBundle | null>(null);
  const nodesRef = useRef<SimNode[]>([]);
  const nodeDragRef = useRef<{ px: number; py: number; moved: boolean } | null>(null);
  const suppressNextNodeClickRef = useRef(false);
  const bumpRafRef = useRef<number | null>(null);
  const [, setTick] = useState(0);
  const [dims, setDims] = useState({ w: 400, h: 320 });
  const [data, setData] = useState<TopicNetworkForUi | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [view, setView] = useState({ k: 1, x: 0, y: 0 });
  const viewRef = useRef(view);
  viewRef.current = view;
  const [extractProgress, setExtractProgress] = useState<{ current: number; total: number } | null>(null);
  const [selectedTopicId, setSelectedTopicId] = useState<string | null>(null);
  const [addTopicOpen, setAddTopicOpen] = useState(false);
  const [addTopicInput, setAddTopicInput] = useState("");
  const [addTopicBusy, setAddTopicBusy] = useState(false);
  const dragRef = useRef<
    | { kind: "pan"; sx: number; sy: number; vx: number; vy: number }
    | { kind: "node"; node: SimNode; sx: number; sy: number }
    | null
  >(null);

  const applyIdleGraphPhysics = useCallback(() => {
    const f = simForcesRef.current;
    const sim = simRef.current;
    if (!f) {
      return;
    }
    f.link.strength(0.55).distance(72);
    f.charge.strength(-220);
    if (sim) {
      sim.force("center", f.center);
    }
  }, []);

  const applyDragGraphPhysics = useCallback(() => {
    const f = simForcesRef.current;
    const sim = simRef.current;
    const drag = dragRef.current;
    if (!f || drag?.kind !== "node") {
      return;
    }
    const sid = drag.node.simId;
    f.link
      .strength((l) => (linkTouchesSimId(l, sid) ? 0.9 : 0.28))
      .distance((l) => (linkTouchesSimId(l, sid) ? 38 : 88));
    f.charge.strength(-260);
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
      setExtractProgress(null);
      setSelectedTopicId(null);
      setAddTopicOpen(false);
      if (gen === loadGenRef.current) {
        setLoading(false);
      }
      return;
    }
    setData(null);
    setSelectedTopicId(null);
    setAddTopicOpen(false);
    setLoading(true);
    setLoadError(null);
    setExtractProgress(null);
    try {
      const r = await invoke<TopicNetworkForUi>("build_topic_network");
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
        setExtractProgress(null);
      }
    }
  }, [workspaceReady, tauriRuntime, workspaceRoot]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (!tauriRuntime || !workspaceReady) {
      return;
    }
    const unsubs: UnlistenFn[] = [];
    let cancelled = false;
    void (async () => {
      const u = await listen<TopicExtractProgressPayload>("topic:extract-progress", (ev) => {
        if (!cancelled) {
          setExtractProgress({ current: ev.payload.current, total: ev.payload.total });
        }
      });
      if (!cancelled) {
        unsubs.push(u);
      }
    })();
    return () => {
      cancelled = true;
      for (const u of unsubs) {
        u();
      }
    };
  }, [tauriRuntime, workspaceReady, workspaceRoot]);

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
    if (!data?.topicNodes.length && !data?.docNodes.length) {
      if (bumpRafRef.current != null) {
        cancelAnimationFrame(bumpRafRef.current);
        bumpRafRef.current = null;
      }
      nodesRef.current = [];
      scheduleRepaint();
      return;
    }

    const topicSims: SimTopic[] = data.topicNodes.map((n) => ({
      ...n,
      simId: simTopicId(n.id),
      kind: "topic",
      r: 11 + Math.min(9, n.docCount * 1.5),
    }));
    const docSims: SimDoc[] = data.docNodes.map((d) => ({
      ...d,
      simId: simDocId(d.relPath),
      kind: "doc",
      r: 9,
    }));
    const nodes: SimNode[] = [...topicSims, ...docSims];
    const bySim = new Map(nodes.map((n) => [n.simId, n]));

    const linksRaw: SimulationLinkDatum<SimNode>[] = [];
    for (const e of data.topicDocEdges) {
      const a = simTopicId(e.topicId);
      const b = simDocId(e.docRelPath);
      if (bySim.has(a) && bySim.has(b)) {
        linksRaw.push({ source: a, target: b });
      }
    }
    for (const e of data.topicTopicEdges) {
      const a = simTopicId(e.sourceTopicId);
      const b = simTopicId(e.targetTopicId);
      if (bySim.has(a) && bySim.has(b)) {
        linksRaw.push({ source: a, target: b });
      }
    }

    const cx = dims.w / 2;
    const cy = dims.h / 2;
    const ringR = Math.min(dims.w, dims.h) * 0.12;
    nodes.forEach((n, i) => {
      const a = (i / Math.max(nodes.length, 1)) * Math.PI * 2;
      n.x = cx + Math.cos(a) * ringR;
      n.y = cy + Math.sin(a) * ringR;
    });

    const linkForce = forceLink<SimNode, SimulationLinkDatum<SimNode>>(linksRaw)
      .id((d) => d.simId)
      .distance(72)
      .strength(0.55);
    const chargeForce = forceManyBody<SimNode>().strength(-220);
    const centerForce = forceCenter(cx, cy);
    const collideForce = forceCollide<SimNode>()
      .radius((d) => d.r + 6)
      .iterations(2);

    const sim = forceSimulation<SimNode>(nodes)
      .force("link", linkForce)
      .force("charge", chargeForce)
      .force("center", centerForce)
      .force("collide", collideForce)
      .alphaDecay(0.022)
      .velocityDecay(0.36);

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

  const highlightedDocPaths = useMemo(() => {
    if (!data || !selectedTopicId) {
      return new Set<string>();
    }
    const set = new Set<string>();
    for (const e of data.topicDocEdges) {
      if (e.topicId === selectedTopicId) {
        set.add(e.docRelPath);
      }
    }
    return set;
  }, [data, selectedTopicId]);

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
    setSelectedTopicId(null);
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
      const x = n.x ?? 0;
      const y = n.y ?? 0;
      n.fx = x;
      n.fy = y;
      n.x = x;
      n.y = y;
      applyIdleGraphPhysics();
      const sim = simRef.current;
      if (sim) {
        sim.alphaTarget(0).alpha(0.38).restart();
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
    applyDragGraphPhysics();
    const sim = simRef.current;
    if (sim) {
      sim.alphaTarget(0.09).alpha(0.38).restart();
    }
  };

  const onAddTopicSubmit = useCallback(async () => {
    const name = addTopicInput.trim();
    if (!name || !workspaceReady || !tauriRuntime) {
      return;
    }
    setAddTopicBusy(true);
    try {
      const res = await invoke<AddManualTopicResult>("add_manual_topic_semantic", { displayName: name });
      setData(res.graph);
      setSelectedTopicId(null);
      setAddTopicOpen(false);
      setAddTopicInput("");
      const msg =
        res.associatedDocCount > 0
          ? t("topicNetwork.addTopicDone", {
              count: res.associatedDocCount,
              canonical: res.canonical,
            })
          : t("topicNetwork.addTopicDoneZero", { canonical: res.canonical });
      await message(msg, { title: t("topicNetwork.addTopicTitle"), kind: "info" });
    } catch (err) {
      await message(err instanceof Error ? err.message : String(err), {
        title: t("topicNetwork.addTopicTitle"),
        kind: "error",
      });
    } finally {
      setAddTopicBusy(false);
    }
  }, [addTopicInput, workspaceReady, tauriRuntime, t]);

  const onExportMd = useCallback(async () => {
    if (!workspaceReady || !tauriRuntime) {
      return;
    }
    try {
      const summary = await invoke<{ topicsWritten: number; exportDirRel: string }>("export_topic_index_markdown");
      await message(
        t("topicNetwork.exportDone", {
          count: summary.topicsWritten,
          path: summary.exportDirRel,
        }),
        { title: t("topicNetwork.exportTitle"), kind: "info" },
      );
    } catch (err) {
      await message(err instanceof Error ? err.message : String(err), {
        title: t("topicNetwork.exportTitle"),
        kind: "error",
      });
    }
  }, [workspaceReady, tauriRuntime, t]);

  const nodes = nodesRef.current;
  const showGraph = nodes.length > 0 && !loading;
  const maxTopicDocForFill = data ? Math.max(1, ...data.topicNodes.map((n) => n.docCount)) : 1;

  const dimNode = (n: SimNode): boolean => {
    if (!selectedTopicId) {
      return false;
    }
    if (n.kind === "topic") {
      return n.id !== selectedTopicId;
    }
    return !highlightedDocPaths.has(n.relPath);
  };

  const dimEdge = (e: TopicDocEdge | TopicTopicEdge, kind: "doc" | "topic"): boolean => {
    if (!selectedTopicId) {
      return false;
    }
    if (kind === "doc") {
      const ed = e as TopicDocEdge;
      return ed.topicId !== selectedTopicId;
    }
    return true;
  };

  return (
    <div className="topic-network" ref={wrapRef}>
      {addTopicOpen ? (
        <div
          className="topic-network__modal-backdrop"
          role="presentation"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget && !addTopicBusy) {
              setAddTopicOpen(false);
            }
          }}
        >
          <div
            className="topic-network__modal"
            role="dialog"
            aria-modal={true}
            aria-labelledby="topic-network-add-topic-heading"
            onMouseDown={(e) => e.stopPropagation()}
          >
            <h3 id="topic-network-add-topic-heading">{t("topicNetwork.addTopicTitle")}</h3>
            <p>{t("topicNetwork.addTopicHint")}</p>
            <input
              type="text"
              value={addTopicInput}
              onChange={(e) => setAddTopicInput(e.target.value)}
              placeholder={t("topicNetwork.addTopicPlaceholder")}
              disabled={addTopicBusy}
              autoFocus
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  void onAddTopicSubmit();
                }
                if (e.key === "Escape" && !addTopicBusy) {
                  setAddTopicOpen(false);
                }
              }}
            />
            <div className="topic-network__modal-actions">
              <button
                type="button"
                className="topic-network__text-btn"
                disabled={addTopicBusy}
                onClick={() => {
                  if (!addTopicBusy) {
                    setAddTopicOpen(false);
                  }
                }}
              >
                {t("topicNetwork.addTopicCancel")}
              </button>
              <button
                type="button"
                className="topic-network__text-btn"
                disabled={addTopicBusy || !addTopicInput.trim()}
                onClick={() => void onAddTopicSubmit()}
              >
                {addTopicBusy ? t("topicNetwork.addTopicRunning") : t("topicNetwork.addTopicConfirm")}
              </button>
            </div>
          </div>
        </div>
      ) : null}
      <div className="topic-network__toolbar">
        <p className="topic-network__metrics" role="status">
          {extractProgress
            ? t("topicNetwork.extractProgress", {
                current: extractProgress.current,
                total: extractProgress.total,
              })
            : data
              ? t("topicNetwork.metrics", {
                  topics: data.topicNodes.length,
                  docs: data.docNodes.length,
                })
              : loading
                ? t("topicNetwork.loading")
                : "\u00a0"}
        </p>
        <div className="topic-network__toolbar-actions">
          <button
            type="button"
            className="topic-network__toolbar-icon-btn"
            disabled={loading || addTopicBusy || !workspaceReady || !tauriRuntime}
            aria-label={t("topicNetwork.addTopic")}
            title={t("topicNetwork.addTopicIconTitle")}
            onClick={() => {
              setAddTopicInput("");
              setAddTopicOpen(true);
            }}
          >
            <IconAddTopic />
          </button>
          <button
            type="button"
            className="topic-network__toolbar-icon-btn"
            disabled={loading || !workspaceReady || !tauriRuntime}
            aria-label={t("topicNetwork.exportMd")}
            title={t("topicNetwork.exportMdIconTitle")}
            onClick={() => void onExportMd()}
          >
            <IconExportMarkdown />
          </button>
          {onTogglePanelWide ? (
            <button
              type="button"
              className="topic-network__toolbar-icon-btn"
              onClick={onTogglePanelWide}
              disabled={!workspaceReady || !tauriRuntime}
              aria-pressed={graphPanelWideExpanded}
              aria-label={
                graphPanelWideExpanded ? t("understandingGraph.collapseWide") : t("understandingGraph.expandWide")
              }
              title={
                graphPanelWideExpanded
                  ? t("understandingGraph.collapseWideTitle")
                  : t("understandingGraph.expandWideTitle")
              }
            >
              <svg
                className="topic-network__toolbar-icon-svg"
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
            </button>
          ) : null}
          <button
            type="button"
            className="topic-network__toolbar-icon-btn"
            onClick={() => void load()}
            disabled={loading || !workspaceReady || !tauriRuntime}
            aria-label={t("topicNetwork.rebuild")}
            title={t("topicNetwork.rebuildTitle")}
          >
            <IconRefreshCw />
          </button>
        </div>
      </div>
      {loadError ? <p className="topic-network__error">{loadError}</p> : null}
      {!tauriRuntime ? <p className="topic-network__empty">{t("understandingGraph.webEmpty")}</p> : null}
      {tauriRuntime && !loading && !loadError && data && data.topicNodes.length === 0 ? (
        <p className="topic-network__empty">
          {data.meta.extractSkippedNoLlm ? t("topicNetwork.emptyNoLlm") : t("topicNetwork.emptyGraph")}
        </p>
      ) : null}
      {loading ? <p className="topic-network__loading">{t("topicNetwork.scanning")}</p> : null}
      {showGraph ? (
        <svg
          ref={svgRef}
          className="topic-network__svg"
          width={dims.w}
          height={dims.h}
          role="img"
          aria-label={t("topicNetwork.graphAria")}
          onWheel={onWheelSvg}
          onPointerDown={onPointerDownSvg}
          onPointerMove={onPointerMoveSvg}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
        >
          <g transform={`translate(${view.x},${view.y}) scale(${view.k})`}>
            {data?.topicDocEdges.map((e, i) => {
              const s = nodes.find((n) => n.kind === "topic" && n.id === e.topicId);
              const d = nodes.find((n) => n.kind === "doc" && n.relPath === e.docRelPath);
              if (!s?.x || !s.y || !d?.x || !d.y) {
                return null;
              }
              return (
                <line
                  key={`td-${e.topicId}-${e.docRelPath}-${i}`}
                  className={`topic-network__edge${dimEdge(e, "doc") ? " topic-network__edge--dim" : ""}`}
                  x1={s.x}
                  y1={s.y}
                  x2={d.x}
                  y2={d.y}
                />
              );
            })}
            {data?.topicTopicEdges.map((e, i) => {
              const s = nodes.find((n) => n.kind === "topic" && n.id === e.sourceTopicId);
              const d = nodes.find((n) => n.kind === "topic" && n.id === e.targetTopicId);
              if (!s?.x || !s.y || !d?.x || !d.y) {
                return null;
              }
              return (
                <line
                  key={`tt-${e.sourceTopicId}-${e.targetTopicId}-${i}`}
                  className={`topic-network__edge${dimEdge(e, "topic") ? " topic-network__edge--dim" : ""}`}
                  x1={s.x}
                  y1={s.y}
                  x2={d.x}
                  y2={d.y}
                />
              );
            })}
            {nodes.map((n) => {
              const x = n.x ?? 0;
              const y = n.y ?? 0;
              const dim = dimNode(n);
              if (n.kind === "topic") {
                const label = n.name.length > 20 ? `${n.name.slice(0, 18)}…` : n.name;
                return (
                  <g
                    key={n.simId}
                    className={`topic-network__node-group${dim ? " topic-network__node-group--dim" : ""}`}
                    role="button"
                    tabIndex={0}
                    aria-label={t("topicNetwork.selectTopic", { name: n.name })}
                    onKeyDown={(ev) => {
                      if (ev.key === "Enter" || ev.key === " ") {
                        ev.preventDefault();
                        setSelectedTopicId((prev) => (prev === n.id ? null : n.id));
                      }
                    }}
                    onPointerDown={(ev) => onPointerDownNode(ev, n)}
                    onClick={() => {
                      if (suppressNextNodeClickRef.current) {
                        suppressNextNodeClickRef.current = false;
                        return;
                      }
                      setSelectedTopicId((prev) => (prev === n.id ? null : n.id));
                    }}
                  >
                    <circle
                      className="topic-network__node-topic"
                      cx={x}
                      cy={y}
                      r={n.r}
                      fill={topicFill(n.docCount, maxTopicDocForFill)}
                    />
                    <text className="topic-network__label" x={x} y={y + n.r + 12} textAnchor="middle">
                      {label}
                    </text>
                  </g>
                );
              }
              const label = basenameRel(n.relPath);
              const w = n.r * 2;
              const h = n.r * 2;
              return (
                <g
                  key={n.simId}
                  className={`topic-network__node-group${dim ? " topic-network__node-group--dim" : ""}`}
                  role="button"
                  tabIndex={0}
                  aria-label={t("understandingGraph.openNote", { path: n.relPath })}
                  onKeyDown={(ev) => {
                    if (ev.key === "Enter" || ev.key === " ") {
                      ev.preventDefault();
                      onOpenNote(n.relPath);
                    }
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
                  <rect
                    className="topic-network__node-doc"
                    x={x - w / 2}
                    y={y - h / 2}
                    width={w}
                    height={h}
                    rx={3}
                    fill={docFill(n)}
                  />
                  <text className="topic-network__label" x={x} y={y + h / 2 + 12} textAnchor="middle">
                    {label.length > 22 ? `${label.slice(0, 20)}…` : label}
                  </text>
                </g>
              );
            })}
          </g>
        </svg>
      ) : null}
    </div>
  );
}
