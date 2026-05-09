type PerfMetaValue = string | number | boolean | null | undefined;

export type PerfMeta = Record<string, PerfMetaValue>;

export type PerfTrace = {
  label: string;
  startMs: number;
  meta?: PerfMeta;
};

const PERF_LOG_STORAGE_KEY = "knowforge:perfLogs";

function perfTraceEnabled(): boolean {
  if (import.meta.env.DEV) {
    return true;
  }
  try {
    return window.localStorage.getItem(PERF_LOG_STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

function roundMs(ms: number): number {
  return Math.round(ms * 100) / 100;
}

export function startPerfTrace(label: string, meta?: PerfMeta): PerfTrace {
  return {
    label,
    startMs: performance.now(),
    meta,
  };
}

export function endPerfTrace(trace: PerfTrace, meta?: PerfMeta): void {
  if (!perfTraceEnabled()) {
    return;
  }
  const durationMs = roundMs(performance.now() - trace.startMs);
  console.debug(`[knowforge:perf] ${trace.label}`, {
    durationMs,
    ...trace.meta,
    ...meta,
  });
}

export function logPerfMark(label: string, meta?: PerfMeta): void {
  if (!perfTraceEnabled()) {
    return;
  }
  console.debug(`[knowforge:perf] ${label}`, meta ?? {});
}
