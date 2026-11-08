import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { memoryPair, networkRate } from "./statistics-format";
import type { StatisticsSettings } from "./types";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import {
  Server,
  Cpu,
  MemoryStick,
  ArrowUp,
  ArrowDown,
  Clock3,
  HardDrive,
  Gauge,
} from "lucide-react";

type Sample = {
  request_ms: number;
  cpu_total: number;
  cpu_idle: number;
  ram_used_percent: number;
  disk_free_percent: number;
  disk_used_percent: number;
  disk_summary: string;
  ram_used_bytes: number;
  ram_total_bytes: number;
  uptime_seconds: number;
  hostname: string;
  os: string;
  kernel: string;
  interface: string | null;
  rx_bytes: number | null;
  tx_bytes: number | null;
};

function size(bytes: number) {
  return bytes >= 1024 ** 3
    ? `${(bytes / 1024 ** 3).toFixed(2)} GiB`
    : `${(bytes / 1024 ** 2).toFixed(0)} MiB`;
}
function uptime(seconds: number) {
  const days = Math.floor(seconds / 86400),
    hours = Math.floor(seconds / 3600) % 24,
    minutes = Math.floor(seconds / 60) % 60;
  return days
    ? `${days}d ${hours}h`
    : hours
      ? `${hours}h ${minutes}m`
      : `${minutes}m`;
}

type Process = { pid: number; cpu_percent: number; name: string };

/**
 * A native tooltip disappears and reappears whenever its text changes, so the
 * text is frozen while the pointer is over the item.
 */
function Stat({
  className,
  title,
  grow,
  children,
}: {
  className: string;
  title: string;
  /** Never shrink below the widest size reached, so the bar does not jitter. */
  grow?: boolean;
  children: ReactNode;
}) {
  const [frozen, setFrozen] = useState<string | null>(null);
  const element = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => {
    const el = element.current;
    if (!grow || !el) return;
    const width = el.getBoundingClientRect().width;
    if (width > (parseFloat(el.style.minWidth) || 0)) {
      el.style.minWidth = `${Math.ceil(width)}px`;
    }
  });
  return (
    <span
      ref={element}
      className={className}
      title={frozen ?? title}
      onMouseEnter={() => setFrozen(title)}
      onMouseLeave={() => setFrozen(null)}
    >
      {children}
    </span>
  );
}

export function Statistics({
  id,
  networkUnit,
}: {
  id: number;
  networkUnit: StatisticsSettings["network_unit"];
}) {
  const hostname = useRef<string | null>(null);
  const generation = useRef(0);
  const processCache = useRef<{ at: number; rows: Process[] } | null>(null);
  const processPending = useRef(false);
  const hoverTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const [processRows, setProcessRows] = useState<Process[] | null>(null);
  const [processError, setProcessError] = useState("");
  const [tooltip, setTooltip] = useState<{
    left: number;
    bottom: number;
  } | null>(null);
  const hideProcesses = () => {
    clearTimeout(hoverTimer.current);
    setTooltip(null);
  };
  const showProcesses = (element: HTMLElement) => {
    const rect = element.getBoundingClientRect();
    setTooltip({
      left: Math.max(8, Math.min(rect.left, window.innerWidth - 328)),
      bottom: window.innerHeight - rect.top + 8,
    });
    clearTimeout(hoverTimer.current);
    const cached = processCache.current;
    if (cached && performance.now() - cached.at < 10000) {
      setProcessRows(cached.rows);
      return;
    }
    if (processPending.current) return;
    hoverTimer.current = setTimeout(async () => {
      processPending.current = true;
      setProcessRows(null);
      setProcessError("");
      const incarnation = generation.current;
      try {
        const rows = await invoke<Process[]>("session_processes", { id });
        if (generation.current !== incarnation) return;
        processCache.current = { at: performance.now(), rows };
        setProcessRows(rows);
      } catch (e) {
        if (generation.current === incarnation) setProcessError(String(e));
      } finally {
        if (generation.current === incarnation) processPending.current = false;
      }
    }, 250);
  };
  const [sample, setSample] = useState<Sample | null>(null);
  const [cpu, setCpu] = useState<number | null>(null);
  const [error, setError] = useState("");
  const [network, setNetwork] = useState<{
    rx: number | null;
    tx: number | null;
  }>({ rx: null, tx: null });
  const [history, setHistory] = useState<number[]>([]);
  useEffect(() => {
    generation.current++;
    hostname.current = null;
    processCache.current = null;
    processPending.current = false;
    setTooltip(null);
    setProcessRows(null);
    setProcessError("");
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    let previous: Sample | null = null;
    let previousAt = 0;
    const poll = async () => {
      try {
        const next = await invoke<Sample>("session_statistics", { id });
        if (!active) return;
        const at = performance.now();
        const total = previous ? next.cpu_total - previous.cpu_total : 0;
        const idle = previous ? next.cpu_idle - previous.cpu_idle : 0;
        const usage =
          total > 0 && idle >= 0 && idle <= total
            ? 100 * (1 - idle / total)
            : null;
        setCpu(usage);
        if (usage !== null) setHistory((h) => [...h.slice(-23), usage]);
        const elapsed = (at - previousAt) / 1000;
        const perSecond = (key: "rx_bytes" | "tx_bytes") =>
          previous &&
          previous.interface === next.interface &&
          next[key] !== null &&
          previous[key] !== null &&
          elapsed > 0 &&
          next[key]! >= previous[key]!
            ? (next[key]! - previous[key]!) / elapsed
            : null;
        setNetwork({ rx: perSecond("rx_bytes"), tx: perSecond("tx_bytes") });
        previousAt = at;
        previous = next;
        hostname.current ??= next.hostname;
        setSample(next);
        setError("");
        timer = setTimeout(() => void poll(), 1000);
      } catch (e) {
        if (active) {
          setSample(null);
          setError(String(e));
        }
      }
    };
    setSample(null);
    setCpu(null);
    setError("");
    setNetwork({ rx: null, tx: null });
    setHistory([]);
    void poll();
    return () => {
      active = false;
      generation.current++;
      clearTimeout(hoverTimer.current);
      clearTimeout(timer);
    };
  }, [id]);
  return (
    <div
      className="session-statistics"
      role="group"
      aria-label="SSH host statistics"
    >
      {tooltip &&
        createPortal(
          <div
            className="cpu-process-tooltip"
            role="tooltip"
            id={`cpu-processes-${id}`}
            style={tooltip}
          >
            <strong>Top 5 processes</strong>
            <p>CPU % averaged over each process lifetime</p>
            {processError ? (
              <p>{processError}</p>
            ) : processRows ? (
              <table>
                <thead>
                  <tr>
                    <th>Process</th>
                    <th>PID</th>
                    <th>CPU</th>
                  </tr>
                </thead>
                <tbody>
                  {processRows.map((p) => (
                    <tr key={p.pid}>
                      <td>{p.name}</td>
                      <td>{p.pid}</td>
                      <td>{p.cpu_percent.toFixed(1)}%</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : (
              <p>Reading processes…</p>
            )}
            <p>Fetched on hover · cached for 10 seconds</p>
          </div>,
          document.querySelector(".app") ?? document.body,
        )}
      {error ? (
        <span className="stat-message" role="status">
          Stats unavailable: {error}
        </span>
      ) : sample ? (
        <>
          <Stat
            className="stat-host"
            title={`${sample.os}\n${sample.kernel}\nHost: ${sample.hostname}`}
          >
            <Server />
            <strong>{hostname.current ?? sample.hostname}</strong>
          </Stat>
          <span
            className="stat-cpu"
            tabIndex={0}
            aria-label="CPU usage; show top five processes"
            aria-describedby={tooltip ? `cpu-processes-${id}` : undefined}
            onMouseEnter={(e) => showProcesses(e.currentTarget)}
            onMouseLeave={hideProcesses}
            onFocus={(e) => showProcesses(e.currentTarget)}
            onBlur={hideProcesses}
            onKeyDown={(e) => {
              if (e.key === "Escape") hideProcesses();
            }}
          >
            <Cpu />
            <strong>
              <span className="cpu-value">
                {cpu === null ? "…" : cpu.toFixed(0)}
              </span>
              {cpu === null ? "" : "%"}
            </strong>
            <svg
              width="48"
              height="16"
              viewBox="0 0 48 16"
              aria-label="Recent CPU usage"
            >
              <polyline
                fill="none"
                stroke="currentColor"
                strokeWidth="1.5"
                points={history
                  .map(
                    (v, i) =>
                      `${48 - (history.length - 1 - i) * 2},${15 - v * 0.14}`,
                  )
                  .join(" ")}
              />
            </svg>
          </span>
          <Stat
            className="stat-memory"
            title={`RAM used: ${sample.ram_used_percent.toFixed(0)}% · used / total`}
          >
            <MemoryStick />
            <strong>
              {memoryPair(sample.ram_used_bytes, sample.ram_total_bytes)}
            </strong>
          </Stat>
          <Stat
            key={networkUnit}
            grow
            className="stat-upload"
            title={`Upload · ${sample.interface ?? "No interface"}\nTotal sent: ${sample.tx_bytes === null ? "Unavailable" : size(sample.tx_bytes)} (${sample.tx_bytes ?? "—"} bytes)\nInterface total since its counter reset; includes other connections`}
          >
            <ArrowUp />
            <strong>{networkRate(network.tx, networkUnit)}</strong>
          </Stat>
          <Stat
            key={networkUnit}
            grow
            className="stat-download"
            title={`Download · ${sample.interface ?? "No interface"}\nTotal received: ${sample.rx_bytes === null ? "Unavailable" : size(sample.rx_bytes)} (${sample.rx_bytes ?? "—"} bytes)\nInterface total since its counter reset; includes other connections`}
          >
            <ArrowDown />
            <strong>{networkRate(network.rx, networkUnit)}</strong>
          </Stat>
          <Stat
            className="stat-uptime"
            title={`Uptime: ${Math.floor(sample.uptime_seconds)} seconds`}
          >
            <Clock3 />
            <strong>{uptime(sample.uptime_seconds)}</strong>
          </Stat>
          <Stat
            className="stat-disk"
            title={`Root filesystem · df -Pk /\n${sample.disk_summary}`}
          >
            <HardDrive />
            <strong>/ {sample.disk_used_percent}%</strong>
          </Stat>
          <Stat
            className="stat-latency"
            title="SSH request time, including channel setup and server acknowledgement; not ICMP ping"
          >
            <Gauge />
            <strong>{sample.request_ms.toFixed(0)} ms</strong>
          </Stat>
        </>
      ) : (
        <span className="stat-message" role="status">
          Reading host statistics…
        </span>
      )}
    </div>
  );
}
