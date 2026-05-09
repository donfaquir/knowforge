import { useCallback, useEffect, useMemo, useState } from "react";
import type { OutlineItem } from "../utils/extractOutline";

/** 每个标题在扁平列表中的父节点下标（更浅的最近前驱） */
function computeParents(items: OutlineItem[]): (number | null)[] {
  const parents: (number | null)[] = new Array(items.length);
  const stack: number[] = [];

  for (let i = 0; i < items.length; i++) {
    const level = items[i].level;
    while (stack.length > 0 && items[stack[stack.length - 1]].level >= level) {
      stack.pop();
    }
    parents[i] = stack.length > 0 ? stack[stack.length - 1] : null;
    stack.push(i);
  }
  return parents;
}

function computeHasChildren(items: OutlineItem[], parents: (number | null)[]): boolean[] {
  const h = items.map(() => false);
  for (const parent of parents) {
    if (parent !== null) {
      h[parent] = true;
    }
  }
  return h;
}

/** 每节点所在树的根下标；依赖 computeParents 保证 parent < i，故可一次线性 DP */
function computeComponentRoots(parents: (number | null)[]): number[] {
  const n = parents.length;
  const root: number[] = new Array(n);
  for (let i = 0; i < n; i++) {
    const p = parents[i];
    root[i] = p === null ? i : root[p];
  }
  return root;
}

/** `ancestor` 下整棵子树（不含 ancestor 自身）的下标集合，供 O(1) 成员检测 */
function strictDescendantIndexSet(ancestor: number, parents: (number | null)[]): Set<number> {
  const n = parents.length;
  const children: number[][] = Array.from({ length: n }, () => []);
  for (let i = 0; i < n; i++) {
    const p = parents[i];
    if (p !== null) {
      children[p].push(i);
    }
  }
  const out = new Set<number>();
  const stack = [...children[ancestor]];
  while (stack.length > 0) {
    const u = stack.pop()!;
    out.add(u);
    for (const c of children[u]) {
      stack.push(c);
    }
  }
  return out;
}

type CollapseAllFlatResult = {
  flatIndices: number[];
  outlineShellRoot: number | null;
};

function computeCollapseAllFlatAndShell(
  parents: (number | null)[],
  hasChildren: boolean[],
): CollapseAllFlatResult {
  const n = parents.length;
  const treeRoots: number[] = [];
  for (let i = 0; i < n; i++) {
    if (parents[i] === null) {
      treeRoots.push(i);
    }
  }
  if (treeRoots.length !== 1) {
    return { flatIndices: treeRoots, outlineShellRoot: null };
  }
  const r = treeRoots[0];
  if (!hasChildren[r]) {
    return { flatIndices: [r], outlineShellRoot: null };
  }
  const compRoot = computeComponentRoots(parents);
  for (let i = 0; i < n; i++) {
    if (i !== r && compRoot[i] !== r) {
      return { flatIndices: treeRoots, outlineShellRoot: null };
    }
  }
  const directChildren: number[] = [];
  for (let i = 0; i < n; i++) {
    if (parents[i] === r) {
      directChildren.push(i);
    }
  }
  return {
    flatIndices: directChildren.length > 0 ? directChildren : [r],
    outlineShellRoot: r,
  };
}

export type OutlineFoldModel = {
  items: OutlineItem[];
  hasChildren: boolean[];
  collapsedHeadings: Set<number>;
  visibleRows: { index: number; item: OutlineItem }[];
  indentBaseLevel: number;
  toggleHeadingCollapse: (index: number) => void;
  outlineHasBranches: boolean;
  allOutlineBranchesExpanded: boolean;
  toggleAllOutlineBranchesBulk: () => void;
};

/**
 * 大纲折叠状态与派生数据；供 OutlinePanel 与 RightPanelShell 顶栏 OutlineBulkToolbar 共用。
 * documentKey 变化时重置折叠（等价于按文档 remount OutlinePanel）。
 */
export function useOutlineFoldModel(
  items: OutlineItem[],
  documentKey: string | null,
): OutlineFoldModel {
  const [collapsedHeadings, setCollapsedHeadings] = useState<Set<number>>(() => new Set());

  useEffect(() => {
    setCollapsedHeadings(new Set());
  }, [documentKey]);

  const shallowestLevel = useMemo(() => {
    if (items.length === 0) {
      return 1;
    }
    return Math.min(...items.map((x) => x.level));
  }, [items]);

  const parents = useMemo(() => computeParents(items), [items]);
  const hasChildren = useMemo(() => computeHasChildren(items, parents), [items, parents]);

  const { flatIndices: collapseAllFlatIndices, outlineShellRoot } = useMemo(
    () => computeCollapseAllFlatAndShell(parents, hasChildren),
    [parents, hasChildren],
  );

  const strictDescendantsOfShellRoot = useMemo(
    () =>
      outlineShellRoot === null ? null : strictDescendantIndexSet(outlineShellRoot, parents),
    [outlineShellRoot, parents],
  );

  const collapseAllFlatMinLevel = useMemo(() => {
    if (collapseAllFlatIndices.length === 0) {
      return 1;
    }
    return Math.min(...collapseAllFlatIndices.map((i) => items[i].level));
  }, [collapseAllFlatIndices, items]);

  const toggleHeadingCollapse = useCallback(
    (index: number) => {
      setCollapsedHeadings((prev) => {
        const next = new Set(prev);
        if (next.has(index)) {
          next.delete(index);
          if (
            outlineShellRoot !== null &&
            strictDescendantsOfShellRoot !== null &&
            strictDescendantsOfShellRoot.has(index)
          ) {
            next.delete(outlineShellRoot);
          }
        } else {
          next.add(index);
        }
        return next;
      });
    },
    [outlineShellRoot, strictDescendantsOfShellRoot],
  );

  const collapseAllOutlineBranches = useCallback(() => {
    const next = new Set<number>();
    for (let i = 0; i < hasChildren.length; i++) {
      if (hasChildren[i]) {
        next.add(i);
      }
    }
    setCollapsedHeadings(next);
  }, [hasChildren]);

  const expandAllOutlineBranches = useCallback(() => {
    setCollapsedHeadings(new Set());
  }, []);

  const branchIndices = useMemo(
    () =>
      hasChildren
        .map((hasBranch, i) => (hasBranch ? i : null))
        .filter((i): i is number => i !== null),
    [hasChildren],
  );

  const allOutlineBranchesExpanded =
    branchIndices.length === 0 ||
    branchIndices.every((i) => !collapsedHeadings.has(i));

  const allBranchesCollapsed = useMemo(() => {
    if (branchIndices.length === 0) {
      return false;
    }
    return branchIndices.every((i) => collapsedHeadings.has(i));
  }, [branchIndices, collapsedHeadings]);

  const toggleAllOutlineBranchesBulk = useCallback(() => {
    if (allOutlineBranchesExpanded) {
      collapseAllOutlineBranches();
    } else {
      expandAllOutlineBranches();
    }
  }, [allOutlineBranchesExpanded, collapseAllOutlineBranches, expandAllOutlineBranches]);

  const visibleRows = useMemo(() => {
    const n = items.length;
    const rows: { index: number; item: OutlineItem }[] = [];
    if (allBranchesCollapsed) {
      const allow = new Set(collapseAllFlatIndices);
      for (let i = 0; i < n; i++) {
        if (allow.has(i)) {
          rows.push({ index: i, item: items[i] });
        }
      }
      return rows;
    }
    // 父下标 < i，沿文档序一次 DP：可见当且仅当父链上无折叠节点 —— O(n)，避免逐行爬链 O(n²)
    const ancOk = new Array<boolean>(n);
    for (let i = 0; i < n; i++) {
      const p = parents[i];
      ancOk[i] = p === null ? true : !collapsedHeadings.has(p) && ancOk[p];
      if (ancOk[i]) {
        rows.push({ index: i, item: items[i] });
      }
    }
    return rows;
  }, [items, parents, collapsedHeadings, allBranchesCollapsed, collapseAllFlatIndices]);

  const indentBaseLevel = allBranchesCollapsed ? collapseAllFlatMinLevel : shallowestLevel;
  const outlineHasBranches = hasChildren.some(Boolean);

  return {
    items,
    hasChildren,
    collapsedHeadings,
    visibleRows,
    indentBaseLevel,
    toggleHeadingCollapse,
    outlineHasBranches,
    allOutlineBranchesExpanded,
    toggleAllOutlineBranchesBulk,
  };
}
