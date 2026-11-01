import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  Fragment,
  useState,
  type MouseEvent,
  type CSSProperties,
} from "react";
import { createRoot } from "react-dom/client";
import { LogRotationSettings } from "./LogRotationSettings";
import { WindowControls } from "./WindowControls";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Plus,
  Search,
  PanelLeft,
  LayoutGrid,
  Columns2,
  Square,
  Terminal as TerminalIcon,
  Folder,
  Settings2,
  ChevronDown,
  ChevronRight,
  Play,
  Unplug,
  X,
  Pencil,
  Check,
  ShieldCheck,
  Send,
  FileText,
  PanelRight,
  ArrowLeftRight,
} from "lucide-react";
import { TerminalPane } from "./TerminalPane";
import {
  useWorkspaceLayout,
  updateWorkspaceLayout,
} from "./use-workspace-layout";
import { toolsWidth } from "./workspace-layout";
import { connectionState } from "./connection-state";
import { MoveToGroup } from "./MoveToGroup";
import { ManageGroup } from "./ManageGroup";
import { GroupCombo } from "./GroupCombo";
import { sessionSelection, contextSelection } from "./session-selection";
import { validMac } from "./wol";
import { statusSummary } from "./status-text";
import { RdpProbe } from "./RdpProbe";
import { RdpPane } from "./RdpPane";
import { RdpTools } from "./RdpTools";
import { SshTools, type StorageRequest } from "./SshTools";
import { ToolsResizer } from "./ToolsResizer";
import { VncPane } from "./VncPane";
import { VncSettings } from "./VncSettings";
import { X11Settings } from "./X11Settings";
import { Broadcast } from "./Broadcast";
import { statisticsEnabled } from "./statistics-format";
import { Statistics } from "./Statistics";
import { ContextMenu, type MenuState } from "./ContextMenu";
import {
  blankProfile,
  connectable,
  rdpSettings,
  protocolLabel,
  localShellLabel,
  sessionDetail,
  type LocalShell,
  type Bootstrap,
  type Live,
  type Meta,
  type Preferences,
  type Profile,
  type Protocol,
} from "./types";
import {
  parseDestination,
  explicitDestination,
  findBookmark,
  quickProfile,
} from "./quick-connect";
import {
  groupSessions,
  groupNames,
  loadCollapsed,
  saveCollapsed,
} from "./session-groups";
import { EditMonitor } from "./EditMonitor";
import { RestoreEdits } from "./RestoreEdits";
import { Files } from "./Files";
import { SettingsTabs } from "./SettingsTabs";
import { Modal } from "./Modal";
import { Appearance } from "./Appearance";
import { SessionTransfer } from "./SessionTransfer";
import { IconPicker, SessionIcon } from "./SessionIcon";
import { RdpCertificate } from "./RdpCertificate";
import {
  assignPane,
  fillPanes,
  layoutForKey,
  showTab,
  swapPane,
} from "./input-policy";
import "./style.css";
import { applyChrome, themeFor } from "./themes";

const initial: Preferences = {
  version: 1,
  profiles: [],
  mocha: false,
  font_size: 14,
  statistics: { enabled: true, network_unit: "Megabytes" },
  font_family: "YuuTerm JetBrains Mono",
  scrollback_lines: 500,
  highlighting: {
    enabled: false,
    ipv4: true,
    ipv6: true,
    ipv4_color: "#89b4fa",
    ipv6_color: "#89b4fa",
    rules: [
      { keywords: "error, failed, failure, fatal, denied", color: "#f38ba8" },
      { keywords: "warning, warn, deprecated", color: "#f9e2af" },
      { keywords: "success, successful, passed, ready", color: "#a6e3a1" },
    ],
  },
};
type Paste = { id: number; text: string; send: () => void };
const QUICK_GROUP = "Quick connect · not saved";

function App() {
  const workspaceLayout = useWorkspaceLayout();
  const sidebar = workspaceLayout.sidebarOpen;
  const setSidebar = (open: boolean) => {
    try {
      updateWorkspaceLayout({ sidebarOpen: open });
    } catch (e) {
      error(`Could not save sidebar layout: ${e}`);
    }
  };
  const sidebarWidth = workspaceLayout.sidebarWidth;
  const setSidebarWidth = (width: number) => {
    try {
      updateWorkspaceLayout({ sidebarWidth: width });
    } catch (e) {
      error(`Could not save sidebar layout: ${e}`);
    }
  };
  const toolsPanelWidth = workspaceLayout.toolsWidth;
  const resizeTools = (width: number) => {
    try {
      updateWorkspaceLayout({ toolsWidth: toolsWidth(width) });
    } catch (e) {
      error(`Could not save tools panel width: ${e}`);
    }
  };
  const resizeSidebar = (width: number) =>
    setSidebarWidth(
      Math.max(200, Math.min(Math.max(200, window.innerWidth - 160), width)),
    );
  const [contextMenu, setContextMenu] = useState<MenuState | null>(null);
  const [profileIconBusy, setProfileIconBusy] = useState(false);
  const closeContextMenu = useCallback(() => setContextMenu(null), []);
  const [rdpProbe, setRdpProbe] = useState(false);
  const [rdpLive, setRdpLive] = useState(false);
  const [appearance, setAppearance] = useState<Preferences | null>(null);
  const [editMonitoring, setEditMonitoring] = useState(false);
  const [autoUploadIds, setAutoUploadIds] = useState<number[]>([]);
  const [noticeBottom, setNoticeBottom] = useState(12);
  const [dispatchOpen, setDispatchOpen] = useState(false);
  const [findRequest, setFindRequest] = useState({ id: 0, revision: 0 });
  const [prefs, setPrefs] = useState(initial),
    [canSave, setCanSave] = useState(false),
    [notice, setNoticeText] = useState("");
  const [noticeKind, setNoticeKind] = useState<"info" | "success" | "error">(
    "info",
  );
  const noticeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const setNotice = useCallback(
    (text: string, kind: "info" | "success" | "error" = "info") => {
      clearTimeout(noticeTimer.current);
      setNoticeText(text);
      setNoticeKind(kind);
      if (text && kind === "success") {
        noticeTimer.current = setTimeout(() => setNoticeText(""), 5000);
      }
    },
    [],
  );
  useEffect(() => () => clearTimeout(noticeTimer.current), []);
  const [filter, setFilter] = useState(""),
    [side, setSide] = useState<"sessions" | "files">("sessions");
  const [open, setOpen] = useState<number[]>([]),
    [panes, setPanes] = useState<(number | null)[]>([null, null, null, null]),
    [focused, setFocused] = useState(0),
    [layout, setLayout] = useState(1);
  const [focusRequest, setFocusRequest] = useState(0);
  // RDP bookmarks whose remote-tools side panel is open. Their SSH tools
  // sessions live in `lives` under an internal key (backend RDP_TOOLS_KEY).
  const [rdpTools, setRdpTools] = useState<number[]>([]);
  // SSH bookmarks whose tools side panel is open, and the latest request to
  // open its storage view at a folder (from the Files menu).
  const [sshTools, setSshTools] = useState<number[]>([]);
  const [storageRequests, setStorageRequests] = useState<
    Record<number, StorageRequest>
  >({});
  const showFolderSizes = (profileId: number, path: string) => {
    setSshTools((t) => (t.includes(profileId) ? t : [...t, profileId]));
    setStorageRequests((r) => ({
      ...r,
      [profileId]: { path, revision: (r[profileId]?.revision ?? 0) + 1 },
    }));
  };
  const RDP_TOOLS_KEY = 2 ** 50;
  const toolsLive = (rdpId: number) =>
    lives.find((l) => l.profileId === RDP_TOOLS_KEY + rdpId);
  const startTools = async (rdpId: number) => {
    try {
      const { id, protocol, profileId } = await invoke<{
        id: number;
        protocol: Protocol;
        profileId: number;
      }>("rdp_tools_start", { profileId: rdpId });
      setLives((a) => [
        ...a.filter((l) => l.profileId !== profileId),
        { id, profileId, protocol },
      ]);
    } catch (e) {
      error(e);
    }
  };
  const stopTools = (rdpId: number) => {
    const l = toolsLive(rdpId);
    if (!l) return;
    void invoke("session_close", { id: l.id }).catch(error);
    setLives((a) => a.filter((v) => v.profileId !== l.profileId));
  };
  const [lives, setLives] = useState<Live[]>([]),
    [draft, setDraft] = useState<Profile | null>(null),
    [settings, setSettings] = useState(false),
    [paste, setPaste] = useState<Paste | null>(null),
    [fixture, setFixture] = useState(false);
  const [fixtureScope, setFixtureScope] = useState("");
  const [moveGroupId, setMoveGroupId] = useState<number | null>(null);
  const [manageGroup, setManageGroup] = useState<{
    name: string;
    remove: boolean;
  } | null>(null);
  const [deleteId, setDeleteId] = useState<number | null>(null);
  const [selectedSessions, setSelectedSessions] = useState<number[]>([]);
  const selectionAnchor = useRef<number | null>(null);
  const [bulkMoveIds, setBulkMoveIds] = useState<number[] | null>(null);
  const [bulkDeleteIds, setBulkDeleteIds] = useState<number[] | null>(null);
  const [actionBusy, setActionBusy] = useState(false);
  const actionWorking = useRef(false);
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const [draggingSessions, setDraggingSessions] = useState(false);
  const pointerSessionDrag = useRef<{
    x: number;
    y: number;
    id: number;
    pointer: number;
    ids: number[];
    active: boolean;
    destination: { group: string; before: number | null } | null;
  } | null>(null);
  const suppressSessionClick = useRef(false);
  useEffect(() => {
    setSelectedSessions([]);
    selectionAnchor.current = null;
  }, [filter]);
  useEffect(() => {
    setSelectedSessions((ids) =>
      ids.filter((id) => prefs.profiles.some((p) => p.id === id)),
    );
  }, [prefs.profiles]);
  const view = settings ? (appearance ?? prefs) : prefs;
  const theme = themeFor(view.theme ?? "", view.mocha);
  useEffect(() => applyChrome(theme), [theme]);
  const openAppearance = () => {
    setAppearance(null);
    setSettings(true);
  };
  const liveRef = useRef(lives);
  const starting = useRef(new Set<number>());
  const fixtureChecked = useRef(false);
  liveRef.current = lives;
  const activeId = panes[focused],
    active = prefs.profiles.find((p) => p.id === activeId),
    activeLive = lives.find((l) => l.profileId === activeId);
  const activeRef = useRef(activeId);
  activeRef.current = activeId;
  const openRef = useRef(open);
  openRef.current = open;
  const error = (e: unknown) => setNotice(String(e), "error");
  useEffect(() => {
    const pending = listen<string>("output-log-close-blocked", (e) =>
      setNotice(e.payload),
    );
    return () => {
      void pending.then((unlisten) => unlisten()).catch(() => {});
    };
  }, []);
  const rdpConnected = lives.some(
    (l) => l.protocol === "Rdp" && l.meta?.connected && !l.meta.finished,
  );
  const statsSignature = lives
    .map((l) => `${l.id}:${l.meta?.connected}:${l.meta?.finished}`)
    .join(",");
  useLayoutEffect(() => {
    if (!notice) return;
    const bars = Array.from(
      document.querySelectorAll<HTMLElement>(".session-statistics"),
    );
    const update = () =>
      setNoticeBottom(
        12 + Math.max(0, ...bars.map((b) => b.getBoundingClientRect().height)),
      );
    const observer = new ResizeObserver(update);
    bars.forEach((b) => observer.observe(b));
    update();
    return () => observer.disconnect();
  }, [notice, layout, sidebar, appearance, prefs, statsSignature]);
  useEffect(() => {
    void invoke<Bootstrap>("bootstrap")
      .then(async (b) => {
        setRdpProbe(b.rdp_probe);
        setRdpLive(b.rdp_live);
        setPrefs(b.prefs);
        setCanSave(b.can_save);
        if (!b.can_save && !b.fixture) setNotice(b.notice);
        setFixture(b.fixture);
        if (b.fixture) {
          const scope = await invoke<string>("capture_scope");
          setFixtureScope(scope);
          if (scope === "ssh-password")
            void import("./fixture-rdp-checks")
              .then((m) => m.checkSshPassword())
              .catch(error);
          if (scope === "ssh-two")
            void import("./fixture-rdp-checks")
              .then((m) => m.checkSshTwo())
              .catch(error);
          if (scope === "quick-connect") {
            // Nothing auto-connects: the check types destinations itself.
            setSide("sessions");
            void import("./fixture-rdp-checks")
              .then((m) =>
                m.checkQuickConnect(
                  setPrefs as never,
                  () => prefsRef.current.profiles,
                ),
              )
              .catch(error);
            return;
          }
          setSide(
            scope === "files" ||
              ["Telnet", "Vnc", "Local"].includes(b.prefs.profiles[0]?.protocol)
              ? "sessions"
              : "files",
          );
          const ids = b.prefs.profiles.map((p) => p.id);
          setOpen(ids);
          setPanes(ids.slice(0, 4));
          setLayout(scope === "local-readme" ? 2 : ids.length === 1 ? 1 : 4);
          ids.forEach(
            (id) =>
              void start(
                id,
                b.prefs.profiles.find((p) => p.id === id)?.protocol,
              ),
          );
          if (b.prefs.profiles[0]?.protocol === "Vnc" && scope === "vnc-real")
            void import("./fixture-vnc-real-checks")
              .then((m) => m.checkVncRealServer())
              .catch(error);
          else if (b.prefs.profiles[0]?.protocol === "Vnc")
            void import("./fixture-vnc-checks")
              .then((m) => m.checkVncFixture())
              .catch(error);
          if (b.prefs.profiles[0]?.protocol === "Rdp" && scope === "rdp-power")
            void import("./fixture-rdp-checks")
              .then((m) => m.checkRdpPower())
              .catch(error);
          if (
            b.prefs.profiles[0]?.protocol === "Rdp" &&
            scope === "rdp-network"
          )
            void import("./fixture-rdp-checks")
              .then((m) => m.checkRdpNetwork())
              .catch(error);
          if (
            b.prefs.profiles[0]?.protocol === "Rdp" &&
            scope === "rdp-refused"
          )
            void import("./fixture-rdp-checks")
              .then((m) => m.checkRdpRefused())
              .catch(error);
          if (b.prefs.profiles[0]?.protocol === "Rdp" && scope === "rdp")
            void import("./fixture-rdp-checks")
              .then((m) => m.checkRdpFixture(applyLayout))
              .catch(error);
        }
      })
      .catch(error);
  }, []);
  useEffect(() => {
    if (
      fixture &&
      !["Vnc", "Rdp"].includes(prefs.profiles[0]?.protocol ?? "") &&
      fixtureScope !== "ssh-password" &&
      fixtureScope !== "ssh-two" &&
      !fixtureChecked.current &&
      lives.length === prefs.profiles.length &&
      lives.every(
        (l) =>
          (l.meta?.connected ||
            (["telnet-refused", "telnet-external"].includes(fixtureScope) &&
              l.meta?.finished)) &&
          (fixtureScope === "windows-openssh" ||
            fixtureScope === "linux-openssh" ||
            fixtureScope === "telnet-external" ||
            fixtureScope === "linux-ssh-tools" ||
            fixtureScope === "zsh-input" ||
            prefs.profiles[0]?.protocol === "Telnet" ||
            prefs.profiles[0]?.protocol === "Local" ||
            (l.meta.files?.length ?? 0) >= 2),
      )
    ) {
      fixtureChecked.current = true;
      void import("./fixture-checks")
        .then((m) => {
          const ids = lives
            .slice()
            .sort((a, b) => a.profileId - b.profileId)
            .map((l) => l.id);
          // "local-hold": an external script measures the idle session.
          if (fixtureScope === "local-hold") return;
          if (fixtureScope === "local-readme")
            return import("./fixture-readme").then((f) => f.captureReadme(ids));
          if (fixtureScope === "telnet-refused")
            return import("./fixture-telnet-refused").then((f) =>
              f.checkTelnetRefused(),
            );
          if (fixtureScope === "local-flood") {
            // Sustained output for the renderer measurement
            // (scripts/Test-LocalTerminal.ps1 -Flood).
            void invoke("session_input", {
              id: ids[0],
              bytes: [
                ...new TextEncoder().encode(
                  "1..3000000 | % { 'line ' + $_ + ' ' + ('x' * 90) }\r",
                ),
              ],
            });
            return;
          }
          if (fixtureScope === "local-terminal")
            return import("./fixture-local-checks").then((f) =>
              f.checkLocalTerminal(ids),
            );
          if (fixtureScope === "zsh-input")
            return import("./fixture-zsh-input").then((f) =>
              f.checkZshInput(ids),
            );
          if (
            fixtureScope === "layout-prepare" ||
            fixtureScope === "layout-restore"
          )
            return import("./fixture-file-layout").then((f) =>
              f.checkFileLayout(fixtureScope === "layout-prepare"),
            );
          if (fixtureScope === "search-flood")
            return import("./fixture-search-flood").then((f) =>
              f.checkSearchFlood(ids, setPrefs),
            );
          if (
            [
              "highlight-wrap",
              "highlight-import",
              "log-rotation",
              "serial-import",
              "bookmark-groups",
              "session-actions",
              "telnet-line",
            ].includes(fixtureScope)
          )
            return import("./fixture-terminal-features").then((f) =>
              f.checkTerminalFeatures(ids, setPrefs, fixtureScope),
            );
          if (fixtureScope === "telnet-external") return m.checkFixture(ids);
          return prefs.profiles[0]?.protocol === "Telnet" &&
            fixtureScope !== "transfer"
            ? m.checkTelnetFixture(ids, setPrefs)
            : m.checkFixture(ids);
        })
        .catch(error);
    }
  }, [fixture, fixtureScope, lives, prefs.profiles]);
  const isOpenTab = (id: number) => open.includes(id);
  const select = (id: number) => {
    setFocusRequest((n) => n + 1);
    setOpen((o) => (o.includes(id) ? o : [...o, id]));
    const at = panes.indexOf(id);
    if (at >= 0 && at < layout) setFocused(at);
    setPanes((a) => showTab(a, layout, focused, id).panes);
  };
  const start = async (
    profileId: number,
    protocolOverride?: Protocol,
    replace = false,
  ) => {
    if (starting.current.has(profileId)) return;
    starting.current.add(profileId);
    try {
      const old = liveRef.current.find((l) => l.profileId === profileId);
      if (old && !old.meta?.finished && !replace) return;
      if (old && !old.meta?.finished) {
        // Unmount the running pane before the backend forgets its session;
        // otherwise its next placement reports "no longer exists".
        setLives((a) => a.filter((l) => l.id !== old.id));
        await new Promise((r) =>
          requestAnimationFrame(() => requestAnimationFrame(r)),
        );
      }
      if (old) await closeLive(old);
      const graphical =
        protocolOverride ??
        prefs.profiles.find((p) => p.id === profileId)?.protocol;
      const { id, protocol, telnetLineEditing } =
        graphical === "Rdp" || graphical === "Vnc"
          ? {
              id: await invoke<number>(
                graphical === "Rdp" ? "rdp_open" : "vnc_open",
                { profileId },
              ),
              protocol: graphical,
              telnetLineEditing: false,
            }
          : await invoke<{
              id: number;
              protocol: Protocol;
              telnetLineEditing: boolean;
            }>("session_start", {
              profileId,
            });
      setLives((a) => [
        ...a.filter((l) => l.profileId !== profileId),
        { id, profileId, protocol, telnetLineEditing },
      ]);
    } catch (e) {
      error(e);
    } finally {
      starting.current.delete(profileId);
    }
  };
  // RDP sessions live in native controls with their own incarnation IDs.
  const closeLive = (l: Live) =>
    l.protocol === "Vnc"
      ? invoke("vnc_close", { id: l.id, remove: true })
      : l.protocol === "Rdp"
        ? invoke("rdp_session", { id: l.id, action: "close" })
        : invoke("session_close", { id: l.id });
  const disconnectLive = (l: Live) =>
    void (
      l.protocol === "Vnc"
        ? invoke("vnc_close", { id: l.id, remove: false })
        : l.protocol === "Rdp"
          ? invoke("rdp_session", { id: l.id, action: "disconnect" })
          : invoke("session_disconnect", { id: l.id })
    ).catch(error);
  const close = async (profileId: number) => {
    const tools = liveRef.current.find(
      (l) => l.profileId === 2 ** 50 + profileId,
    );
    const l = liveRef.current.find((l) => l.profileId === profileId);
    // Remove the tab from the UI first: its terminal and file panes stop
    // asking about the connection before the backend forgets it.
    const rest = open.filter((i) => i !== profileId);
    const wasActive = profileId === activeRef.current;
    const next = wasActive ? rest[rest.length - 1] : undefined;
    setLives((a) =>
      a.filter(
        (v) => v.profileId !== profileId && v.profileId !== tools?.profileId,
      ),
    );
    setRdpTools((t) => t.filter((v) => v !== profileId));
    setSshTools((t) => t.filter((v) => v !== profileId));
    setOpen(rest);
    // No tab left: back to the single view with the home screen.
    if (!rest.length) applyLayout(1);
    const cleared = panes.map((i) => (i === profileId ? null : i));
    const shown = next === undefined ? -1 : cleared.indexOf(next);
    // A tab already visible in another pane just takes focus; it never moves.
    if (shown >= 0 && shown < layout) setFocused(shown);
    // Other hidden tabs then fill any pane left empty.
    setPanes((a) => {
      const left = a.map((i) => (i === profileId ? null : i));
      return fillPanes(
        next === undefined || (shown >= 0 && shown < layout)
          ? left
          : assignPane(left, focused, next),
        rest.length ? layout : 1,
        rest,
      );
    });
    if (wasActive) {
      // Show the last remaining tab; its files for a connected SSH session,
      // otherwise the session list (also when no tab is left: home screen).
      const nextLive = liveRef.current.find((v) => v.profileId === next);
      setSide(
        nextLive?.protocol === "Ssh" &&
          nextLive.meta?.connected &&
          !nextLive.meta.finished
          ? "files"
          : "sessions",
      );
    }
    if (tools) await invoke("session_close", { id: tools.id }).catch(error);
    if (l) await closeLive(l).catch(error);
  };
  const onMeta = (id: number, meta: Meta) => {
    const previous = liveRef.current.find((l) => l.id === id)?.meta;
    const owner = liveRef.current.find((l) => l.id === id)?.profileId ?? 0;
    // RDP remote-tools sessions show their files inside the tools panel.
    if (
      owner < 2 ** 50 &&
      meta.protocol === "Ssh" &&
      meta.connected &&
      !previous?.connected
    ) {
      setSide("files");
    }
    // The file sidebar follows the focused SSH connection: when it ends, show
    // the sessions again instead of a stale or empty file list.
    if (
      owner < 2 ** 50 &&
      previous?.connected &&
      !meta.connected &&
      owner === activeRef.current
    )
      setSide("sessions");
    setLives((a) => a.map((l) => (l.id === id ? { ...l, meta } : l)));
  };
  const onLogging = useCallback((id: number, logging: Live["logging"]) => {
    setLives((a) => a.map((l) => (l.id === id ? { ...l, logging } : l)));
  }, []);
  const focus = (index: number) => setFocused(index);
  const applyLayout = (n: number) => {
    setLayout(n);
    setFocused((i) => Math.min(i, n - 1));
    // More panes: open tabs that were hidden fill them side by side.
    setPanes((a) => fillPanes(a, n, openRef.current));
  };
  useEffect(() => {
    // F1/F2/F3 switch layouts everywhere; also suppresses WebView2's F3 find bar.
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof Element && e.target.closest(".vnc-surface"))
        return;
      const n = layoutForKey(e);
      if (!n) return;
      e.preventDefault();
      applyLayout(n);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);
  const save = async () => {
    if (!draft || profileIconBusy) return;
    try {
      const before = prefs.profiles.find((p) => p.id === draft.id);
      const p = await invoke<Preferences>("save_profile", {
        profile: { ...draft, group: draft.group?.trim() ?? "" },
      });
      setPrefs(p);
      setDraft(null);
      setNotice("");
      // RDP negotiates clipboard sharing only while connecting, so a running
      // session reconnects to apply the change (owner decision, #23).
      const live = liveRef.current.find((l) => l.profileId === draft.id);
      if (
        draft.protocol === "Rdp" &&
        live?.protocol === "Rdp" &&
        !live.meta?.finished &&
        (before?.rdp?.clipboard ?? true) !== (draft.rdp?.clipboard ?? true)
      )
        await start(draft.id, "Rdp", true);
    } catch (e) {
      error(e);
    }
  };
  const trust = lives.find((l) => l.meta?.trust);
  const duplicating = useRef(false);
  const duplicate = async (id: number) => {
    if (duplicating.current) return;
    duplicating.current = true;
    try {
      const updated = await invoke<Preferences>("duplicate_profile", {
        profileId: id,
      });
      setPrefs(updated);
      setNotice("");
    } catch (e) {
      error(e);
    } finally {
      duplicating.current = false;
    }
  };
  const [collapsed, setCollapsed] = useState(loadCollapsed);
  const toggleGroup = (name: string) =>
    setCollapsed((c) => {
      const next = new Set(c);
      if (!next.delete(name)) next.add(name);
      saveCollapsed(next);
      return next;
    });
  const prefsRef = useRef(prefs);
  prefsRef.current = prefs;
  const quickConnecting = useRef(false);
  // One click opens an unsaved PowerShell; an idle unsaved one is reused.
  const openLocal = async () => {
    if (quickConnecting.current) return;
    quickConnecting.current = true;
    try {
      const idle = prefs.profiles.find(
        (p) => p.temporary && p.protocol === "Local" && !open.includes(p.id),
      );
      let id = idle?.id;
      if (!id) {
        const names = new Set(prefs.profiles.map((p) => p.name));
        let name = "Local terminal";
        for (let n = 2; names.has(name); n++) name = `Local terminal ${n}`;
        const updated = await invoke<Preferences>("save_profile", {
          profile: {
            ...blankProfile(),
            name,
            protocol: "Local",
            port: 0,
            temporary: true,
          },
        });
        setPrefs(updated);
        id = updated.profiles.find((p) => p.temporary && p.name === name)?.id;
      }
      if (!id) throw "The local terminal could not be opened";
      select(id);
      await start(id, "Local");
    } catch (e) {
      error(e);
    } finally {
      quickConnecting.current = false;
    }
  };
  const quickConnect = async (text: string) => {
    const d = parseDestination(text);
    if (!d || quickConnecting.current) return;
    quickConnecting.current = true;
    try {
      let profile = findBookmark(prefs.profiles, d);
      if (!profile) {
        const updated = await invoke<Preferences>("save_profile", {
          profile: quickProfile(d),
        });
        setPrefs(updated);
        profile = findBookmark(updated.profiles, d);
      }
      if (!profile) throw "The quick-connect session was not saved";
      setFilter("");
      select(profile.id);
      await start(profile.id, profile.protocol);
    } catch (e) {
      error(e);
    } finally {
      quickConnecting.current = false;
    }
  };
  const matches = prefs.profiles.filter((p) =>
    `${p.name} ${p.host} ${p.group ?? ""}`
      .toLowerCase()
      .includes(filter.toLowerCase()),
  );
  // Quick-connect sessions are listed first and are never saved to disk.
  const quickSessions = matches.filter((p) => p.temporary);
  const sidebarGroups = [
    ...(quickSessions.length
      ? [{ name: QUICK_GROUP, profiles: quickSessions }]
      : []),
    ...groupSessions(matches.filter((p) => !p.temporary)),
  ];
  const visibleSessionIds = sidebarGroups
    .filter((g) => !g.name || !!filter.trim() || !collapsed.has(g.name))
    .flatMap((g) => g.profiles.map((p) => p.id));
  // The empty-pane chooser offers only sessions without an open tab.
  const closedProfiles = prefs.profiles.filter((p) => !open.includes(p.id));
  const savedCount = prefs.profiles.filter((p) => !p.temporary).length;
  // Enter connects to a destination when it cannot be a search term or nothing
  // matched; otherwise it opens the single matching bookmark.
  const quick =
    filter.trim() && (explicitDestination(filter) || matches.length === 0)
      ? parseDestination(filter)
      : null;
  const runSessionAction = async (action: () => Promise<void>) => {
    if (actionWorking.current) return;
    actionWorking.current = true;
    setActionBusy(true);
    try {
      await action();
    } catch (e) {
      error(e);
    } finally {
      actionWorking.current = false;
      setActionBusy(false);
    }
  };
  const wakeSessions = (ids: number[]) =>
    runSessionAction(async () => {
      const result = await invoke<{
        sent: number;
        skipped: number;
        failed: number;
      }>("wake_profiles", { profileIds: ids });
      setNotice(
        `Wake packets sent to ${result.sent} computer(s). ${result.skipped} skipped, ${result.failed} failed.`,
        result.sent && !result.failed ? "success" : "error",
      );
    });
  const moveSessions = (
    ids: number[],
    group: string,
    beforeId: number | null,
  ) =>
    runSessionAction(async () => {
      const updated = await invoke<Preferences>("move_profiles", {
        profileIds: ids,
        group,
        beforeId,
      });
      setPrefs(updated);
      setNotice(`${ids.length} session(s) moved`, "success");
      setCollapsed((old) => {
        const next = new Set(old);
        next.delete(group);
        saveCollapsed(next);
        return next;
      });
    });
  const moveSessionPointer = (event: React.PointerEvent) => {
    const drag = pointerSessionDrag.current;
    if (!drag || drag.pointer !== event.pointerId || actionWorking.current)
      return;
    if (
      !drag.active &&
      Math.hypot(event.clientX - drag.x, event.clientY - drag.y) < 6
    )
      return;
    if (!drag.active) {
      drag.active = true;
      setDraggingSessions(true);
      setSelectedSessions(drag.ids);
      event.currentTarget.setPointerCapture(event.pointerId);
    }
    event.preventDefault();
    const target = document
      .elementFromPoint(event.clientX, event.clientY)
      ?.closest<HTMLElement>(
        ".session-row[data-profile-id], [data-session-group]",
      );
    const profile = target?.dataset.profileId
      ? prefs.profiles.find((p) => p.id === Number(target.dataset.profileId))
      : undefined;
    drag.destination = profile
      ? { group: profile.group ?? "", before: profile.id }
      : target?.hasAttribute("data-session-group")
        ? { group: target.dataset.sessionGroup ?? "", before: null }
        : null;
    setDropTarget(
      profile
        ? `row:${profile.id}`
        : drag.destination
          ? `group:${drag.destination.group}`
          : null,
    );
  };
  const finishSessionPointer = (event: React.PointerEvent, cancel = false) => {
    const drag = pointerSessionDrag.current;
    if (!drag || drag.pointer !== event.pointerId) return;
    pointerSessionDrag.current = null;
    setDraggingSessions(false);
    setDropTarget(null);
    if (event.currentTarget.hasPointerCapture(event.pointerId))
      event.currentTarget.releasePointerCapture(event.pointerId);
    if (drag.active) {
      event.preventDefault();
      suppressSessionClick.current = true;
      requestAnimationFrame(() => {
        suppressSessionClick.current = false;
      });
      if (!cancel && drag.destination)
        void moveSessions(
          drag.ids,
          drag.destination.group,
          drag.destination.before,
        );
    }
  };
  const sessionMenu = (event: MouseEvent, p: Profile) => {
    event.preventDefault();
    event.stopPropagation();
    const live = liveRef.current.find((l) => l.profileId === p.id);
    const connected = live && !live.meta?.finished;
    const bounds = event.currentTarget.getBoundingClientRect();
    const targets = contextSelection(selectedSessions, p.id);
    if (!selectedSessions.includes(p.id)) selectionAnchor.current = p.id;
    setSelectedSessions(targets);
    if (targets.length > 1) {
      const profiles = prefs.profiles.filter((v) => targets.includes(v.id));
      setContextMenu({
        x: event.clientX || bounds.left + 12,
        y: event.clientY || bounds.top + 12,
        actions: [
          {
            label: "Connect selected sessions",
            disabled: actionBusy,
            run: () =>
              void runSessionAction(async () => {
                for (const profile of profiles) {
                  if (!connectable(profile.protocol)) continue;
                  const live = liveRef.current.find(
                    (v) => v.profileId === profile.id,
                  );
                  if (!live || live.meta?.finished) await start(profile.id);
                }
                setOpen((old) => [
                  ...new Set([
                    ...old,
                    ...profiles
                      .filter((v) => connectable(v.protocol))
                      .map((v) => v.id),
                  ]),
                ]);
              }),
          },
          {
            label: "Wake on LAN",
            disabled:
              actionBusy ||
              !profiles.some(
                (v) =>
                  v.mac_address &&
                  v.protocol !== "Local" &&
                  v.protocol !== "Serial",
              ),
            run: () => void wakeSessions(targets),
          },
          {
            label: "Move to group",
            disabled: actionBusy || (!canSave && !fixture),
            run: () => setBulkMoveIds(targets),
          },
          {
            label: "Copy hosts",
            run: () =>
              void navigator.clipboard
                .writeText(
                  [
                    ...new Set(
                      profiles
                        .filter(
                          (v) =>
                            v.protocol !== "Local" && v.protocol !== "Serial",
                        )
                        .map((v) => v.host)
                        .filter(Boolean),
                    ),
                  ].join("\n"),
                )
                .catch(error),
          },
          {
            label: "Delete sessions",
            destructive: true,
            disabled:
              actionBusy ||
              (!canSave && !fixture && profiles.some((v) => !v.temporary)),
            run: () => setBulkDeleteIds(targets),
          },
        ],
      });
      return;
    }
    setContextMenu({
      x: event.clientX || bounds.left + 12,
      y: event.clientY || bounds.top + 12,
      actions: [
        {
          label: connected ? "Show session" : "Connect",
          disabled: !connectable(p.protocol),
          run: () => {
            select(p.id);
            if (!connected) void start(p.id);
          },
        },
        {
          label: "Disconnect",
          disabled: !connected,
          run: () => {
            if (live) disconnectLive(live);
          },
        },
        ...(p.temporary
          ? [
              {
                label: "Save session",
                run: () =>
                  void invoke<Preferences>("save_profile", {
                    profile: { ...p, temporary: false },
                  })
                    .then((updated) => {
                      setPrefs(updated);
                      setNotice(`${p.name} saved`, "success");
                    })
                    .catch(error),
              },
            ]
          : []),
        { label: "Edit session", run: () => setDraft(structuredClone(p)) },
        {
          label: "Wake on LAN",
          disabled:
            actionBusy ||
            !p.mac_address ||
            p.protocol === "Local" ||
            p.protocol === "Serial",
          run: () => void wakeSessions([p.id]),
        },
        {
          label: "Move to group",
          disabled: !canSave && !fixture,
          run: () => setMoveGroupId(p.id),
        },
        ...(p.protocol === "Rdp" ||
        p.protocol === "Vnc" ||
        (p.protocol === "Ssh" && p.ssh.auth === "Password")
          ? [
              {
                label: "Forget saved password",
                run: () => {
                  void invoke<Preferences>("forget_saved_password", {
                    profileId: p.id,
                  })
                    .then((updated) => {
                      setPrefs(updated);
                      setNotice(
                        "Saved password removed for " + p.name,
                        "success",
                      );
                    })
                    .catch(error);
                },
              },
            ]
          : []),
        {
          label: "Duplicate session",
          disabled: !canSave && !fixture,
          run: () => {
            void duplicate(p.id);
          },
        },
        {
          label: "Copy host",
          run: () => {
            void navigator.clipboard.writeText(p.host).catch(error);
          },
        },
        {
          label: p.temporary ? "Remove session" : "Delete session",
          destructive: true,
          disabled: !canSave && !fixture && !p.temporary,
          run: () => setDeleteId(p.id),
        },
      ],
    });
  };
  if (rdpProbe) return <RdpProbe live={rdpLive} />;
  return (
    <div
      className={`app ${sidebar ? "" : "sidebar-hidden"}`}
      onPointerMove={moveSessionPointer}
      onPointerUp={(e) => finishSessionPointer(e)}
      onPointerCancel={(e) => finishSessionPointer(e, true)}
      style={{ "--sidebar-width": `${sidebarWidth}px` } as CSSProperties}
      onContextMenu={(event) => {
        if (
          (event.target as HTMLElement).closest(
            "input, textarea, [contenteditable=true]",
          )
        )
          return;
        event.preventDefault();
        if (document.querySelector("dialog[open]")) return;
        setContextMenu({
          x: event.clientX,
          y: event.clientY,
          actions: [
            { label: "New session", run: () => setDraft(blankProfile()) },
            { label: "Settings", run: openAppearance },
          ],
        });
      }}
    >
      {contextMenu && (
        <ContextMenu menu={contextMenu} onClose={closeContextMenu} />
      )}
      <header className="topbar" data-tauri-drag-region>
        <div className="brand" data-tauri-drag-region>
          <TerminalIcon size={19} />
          <strong>YuuTerm</strong>
          {__YUUTERM_DEV_BUILD__ && (
            <span className="dev-build">{__YUUTERM_DEV_BUILD__}</span>
          )}
        </div>
        <WindowControls />
      </header>
      <aside className="sidebar">
        <div className="side-top">
          <div className="new-row">
            <button
              className="new-session"
              onClick={() => setDraft(blankProfile())}
            >
              <Plus size={16} />
              New session
            </button>
            <button
              className="new-session local-terminal"
              aria-label="Open local terminal"
              title="Local terminal (not saved)"
              onClick={() => void openLocal()}
            >
              <TerminalIcon size={16} />
            </button>
          </div>
          <div className="side-tabs">
            <button
              className={side === "sessions" ? "selected" : ""}
              onClick={() => setSide("sessions")}
            >
              Sessions
            </button>
            <button
              className={side === "files" ? "selected" : ""}
              onClick={() => setSide("files")}
            >
              Files
            </button>
          </div>
        </div>
        {side === "sessions" ? (
          <>
            <div className="search">
              <Search size={14} />
              <input
                aria-label="Find a session"
                placeholder="Find a session or user@host"
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key !== "Enter") return;
                  e.preventDefault();
                  if (quick) void quickConnect(filter);
                  else if (matches.length === 1) {
                    select(matches[0].id);
                    if (connectable(matches[0].protocol))
                      void start(matches[0].id);
                  }
                }}
              />
            </div>
            <div className="section-label">
              SAVED SESSIONS
              <SessionTransfer
                enabled={canSave || fixture}
                count={prefs.profiles.length}
                onImported={setPrefs}
                error={error}
              />
            </div>
            <div
              className="session-list"
              role="listbox"
              aria-label="Saved sessions"
              aria-multiselectable="true"
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  setSelectedSessions([]);
                  selectionAnchor.current = null;
                }
              }}
            >
              {selectedSessions.length > 1 && (
                <div className="session-selection-count" role="status">
                  {selectedSessions.length} selected
                </div>
              )}
              {(draggingSessions ||
                sidebarGroups.some(
                  (g) => !g.name && g.profiles.length > 0,
                )) && (
                <div
                  className={`session-dropzone ${dropTarget === "group:" ? "drop-target" : ""}`}
                  aria-label="Move sessions to Ungrouped"
                  data-session-group=""
                >
                  Ungrouped
                </div>
              )}
              {sidebarGroups.map((g) => {
                const open =
                  !g.name || !!filter.trim() || !collapsed.has(g.name);
                return (
                  <Fragment key={`group:${g.name}`}>
                    {g.name && (
                      <button
                        className={`session-group ${dropTarget === `group:${g.name}` ? "drop-target" : ""}`}
                        data-session-group={g.name}
                        aria-expanded={open}
                        title={open ? "Collapse group" : "Expand group"}
                        onClick={() => toggleGroup(g.name)}
                        onContextMenu={(event) => {
                          event.preventDefault();
                          event.stopPropagation();
                          if (g.profiles.every((p) => p.temporary)) return;
                          setContextMenu({
                            x: event.clientX,
                            y: event.clientY,
                            actions: [
                              {
                                label: "Rename group",
                                disabled: !canSave && !fixture,
                                run: () =>
                                  setManageGroup({
                                    name: g.name,
                                    remove: false,
                                  }),
                              },
                              {
                                label: "Delete group",
                                destructive: true,
                                disabled: !canSave && !fixture,
                                run: () =>
                                  setManageGroup({
                                    name: g.name,
                                    remove: true,
                                  }),
                              },
                            ],
                          });
                        }}
                      >
                        {open ? (
                          <ChevronDown size={13} />
                        ) : (
                          <ChevronRight size={13} />
                        )}
                        <span>{g.name}</span>
                        <small>{g.profiles.length}</small>
                      </button>
                    )}
                    {open &&
                      g.profiles.map((p) => {
                        const l = lives.find((l) => l.profileId === p.id);
                        return (
                          <div
                            className={`session-row ${activeId === p.id ? "active" : ""} ${g.name ? "grouped" : ""} ${selectedSessions.includes(p.id) ? "selected" : ""} ${dropTarget === `row:${p.id}` ? "drop-before" : ""}`}
                            data-profile-id={p.id}
                            role="option"
                            aria-selected={selectedSessions.includes(p.id)}
                            draggable={false}
                            onPointerDown={(e) => {
                              if (
                                e.button !== 0 ||
                                actionBusy ||
                                (!canSave && !fixture) ||
                                (e.target as HTMLElement).closest(".edit")
                              )
                                return;
                              pointerSessionDrag.current = {
                                x: e.clientX,
                                y: e.clientY,
                                id: p.id,
                                pointer: e.pointerId,
                                ids: contextSelection(selectedSessions, p.id),
                                active: false,
                                destination: null,
                              };
                            }}
                            key={p.id}
                            onContextMenu={(event) => sessionMenu(event, p)}
                          >
                            <button
                              className="session-select"
                              onClick={(e) => {
                                if (suppressSessionClick.current) return;
                                const anchor = selectionAnchor.current;
                                if (
                                  !e.shiftKey ||
                                  anchor === null ||
                                  !visibleSessionIds.includes(anchor)
                                )
                                  selectionAnchor.current = p.id;
                                setSelectedSessions((ids) =>
                                  sessionSelection(
                                    ids,
                                    p.id,
                                    e.ctrlKey || e.metaKey,
                                    {
                                      shift: e.shiftKey,
                                      anchor,
                                      order: visibleSessionIds,
                                    },
                                  ),
                                );
                                // Single click never opens a tab; double-click connects.
                                if (
                                  !e.ctrlKey &&
                                  !e.metaKey &&
                                  !e.shiftKey &&
                                  isOpenTab(p.id)
                                )
                                  select(p.id);
                              }}
                              onDoubleClick={(e) => {
                                if (e.ctrlKey || e.metaKey || e.shiftKey)
                                  return;
                                select(p.id);
                                if (connectable(p.protocol)) void start(p.id);
                              }}
                              title="Double-click to connect"
                            >
                              <span
                                className={`dot ${l?.meta?.connected ? "connected" : ""}`}
                              />
                              {p.icon && (
                                <SessionIcon icon={p.icon} size={16} />
                              )}
                              <span className="session-info">
                                <span>{p.name}</span>
                                <small>
                                  {protocolLabel(p.protocol)}
                                  {sessionDetail(p)
                                    ? ` · ${sessionDetail(p)}`
                                    : ""}
                                </small>
                              </span>
                            </button>
                            <button
                              className="icon edit"
                              title={`Edit ${p.name}`}
                              aria-label={`Edit ${p.name}`}
                              onClick={() => setDraft(structuredClone(p))}
                            >
                              <Pencil size={13} />
                            </button>
                          </div>
                        );
                      })}
                  </Fragment>
                );
              })}
              {prefs.profiles.length === 0 && (
                <div className="side-empty">
                  Your connections live here.
                  <br />
                  Add a session to get started.
                </div>
              )}
            </div>
            <footer className="sessions-footer">
              <span>
                {savedCount} saved {savedCount === 1 ? "session" : "sessions"}
              </span>
              <span className="app-version">v{__YUUTERM_VERSION__}</span>
            </footer>
          </>
        ) : ["Telnet", "Serial", "Rdp", "Vnc"].includes(
            activeLive?.protocol ?? active?.protocol ?? "",
          ) ? (
          <div className="side-empty">
            File browsing is available for SSH sessions.
          </div>
        ) : (
          <Files
            key={activeLive?.id ?? "none"}
            expandable
            expanded={sidebarWidth >= 500}
            onResize={(expanded) => resizeSidebar(expanded ? 740 : 248)}
            live={activeLive}
            error={error}
            onFolderSizes={
              activeLive
                ? (path) => showFolderSizes(activeLive.profileId, path)
                : undefined
            }
            onEditOpened={() => setEditMonitoring(true)}
            onFocusTerminal={() => setFocusRequest((n) => n + 1)}
            autoUpload={autoUploadIds.includes(activeLive?.id ?? -1)}
            onAskUploads={() =>
              setAutoUploadIds((ids) =>
                ids.filter((id) => id !== activeLive?.id),
              )
            }
          />
        )}
        <div
          className="sidebar-resizer"
          role="separator"
          aria-label="Resize sidebar"
          aria-orientation="vertical"
          aria-valuemin={200}
          aria-valuemax={Math.max(200, window.innerWidth - 160)}
          aria-valuenow={sidebarWidth}
          tabIndex={0}
          title="Drag to resize sidebar; double-click to reset"
          onPointerDown={(event) => {
            if (event.button !== 0) return;
            event.preventDefault();
            event.currentTarget.setPointerCapture(event.pointerId);
          }}
          onPointerMove={(event) => {
            if (event.currentTarget.hasPointerCapture(event.pointerId))
              resizeSidebar(
                event.clientX -
                  event.currentTarget.closest(".app")!.getBoundingClientRect()
                    .left,
              );
          }}
          onPointerUp={(event) => {
            if (event.currentTarget.hasPointerCapture(event.pointerId))
              event.currentTarget.releasePointerCapture(event.pointerId);
          }}
          onDoubleClick={() => setSidebarWidth(248)}
          onKeyDown={(event) => {
            if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key))
              return;
            event.preventDefault();
            resizeSidebar(
              event.key === "Home"
                ? 200
                : event.key === "End"
                  ? window.innerWidth - 160
                  : sidebarWidth + (event.key === "ArrowRight" ? 20 : -20),
            );
          }}
        />
      </aside>
      <main className="main">
        <div className="workspace-bar">
          <nav className="tabs">
            {open.map((id) => {
              const p = prefs.profiles.find((p) => p.id === id);
              return (
                p && (
                  <div
                    className={`tab ${activeId === id ? "active" : ""}`}
                    key={id}
                    onContextMenu={(event) => sessionMenu(event, p)}
                    onMouseDown={(event) => {
                      if (event.button === 1) event.preventDefault();
                    }}
                    onAuxClick={(event) => {
                      if (event.button !== 1) return;
                      event.preventDefault();
                      void close(id);
                    }}
                  >
                    <button onClick={() => select(id)}>
                      <SessionIcon icon={p.icon} size={13} />
                      {p.name}
                    </button>
                    <button
                      className="tab-close"
                      title={`Close ${p.name}`}
                      aria-label={`Close ${p.name}`}
                      onClick={() => void close(id)}
                    >
                      <X size={12} />
                    </button>
                  </div>
                )
              );
            })}
          </nav>
          <div className="top-actions">
            <div className="workspace-actions">
              <button
                className="icon"
                title="Send to terminals"
                aria-label="Send to terminals"
                onClick={() => setDispatchOpen(true)}
              >
                <Send size={15} />
              </button>
              <button
                className="icon"
                title="Show remote files"
                aria-label="Show remote files"
                onClick={() => {
                  setSidebar(true);
                  setSide("files");
                }}
              >
                <Folder />
              </button>
              <details className="view-menu">
                <summary title="Workspace layout">
                  <LayoutGrid size={15} />
                  <ChevronDown size={11} />
                </summary>
                <div className="menu">
                  <label>VIEW LAYOUT</label>
                  {[
                    [1, "Single pane", Square, "F1"],
                    [2, "Two panes", Columns2, "F2"],
                    [4, "Four panes", LayoutGrid, "F3"],
                  ].map(([n, label, Icon, key]) => {
                    const I = Icon as typeof Square;
                    return (
                      <button
                        key={Number(n)}
                        data-shortcut={String(key)}
                        title={`${String(label)} (${String(key)})`}
                        onClick={(e) => {
                          applyLayout(Number(n));
                          e.currentTarget
                            .closest("details")
                            ?.removeAttribute("open");
                        }}
                      >
                        <I size={15} />
                        {String(label)}
                        {layout === n && <Check size={13} />}
                      </button>
                    );
                  })}
                </div>
              </details>
            </div>
            <button
              className="icon"
              title="Toggle sidebar"
              aria-label="Toggle sidebar"
              aria-expanded={sidebar}
              onClick={() => setSidebar(!sidebar)}
            >
              <PanelLeft />
            </button>
            <button
              className="icon"
              title="Settings"
              aria-label="Settings"
              onClick={openAppearance}
            >
              <Settings2 />
            </button>
          </div>
        </div>
        <div
          className={`workspace layout-${layout}`}
          data-panes={panes.join(",")}
        >
          {open.map((id) => {
            const p = prefs.profiles.find((p) => p.id === id);
            if (!p) return null;
            const index = panes.indexOf(id),
              visible = index >= 0 && index < layout,
              l = lives.find((l) => l.profileId === id);
            const currentProtocol = l?.protocol ?? p.protocol;
            return (
              <section
                data-profile-id={id}
                data-profile-protocol={p.protocol}
                data-live-protocol={l?.protocol}
                data-live-id={l?.id}
                className={`pane ${focused === index ? "focused" : ""}`}
                key={id}
                style={{
                  display: visible ? "flex" : "none",
                  gridColumn: layout === 1 ? 1 : (index % 2) + 1,
                  gridRow: layout === 4 ? Math.floor(index / 2) + 1 : 1,
                }}
              >
                <div className="pane-bar">
                  <button
                    className="pane-title"
                    onClick={() => focus(index)}
                    title={connectionState(l)}
                  >
                    <span
                      className={`dot ${l?.meta?.connected ? "connected" : ""}`}
                    />
                    {connectionState(l)}
                  </button>
                  {layout > 1 && open.length > 1 && (
                    <button
                      className="icon"
                      title="Switch session in this pane"
                      aria-label="Switch session in this pane"
                      onClick={(event) => {
                        const at = event.currentTarget.getBoundingClientRect();
                        setContextMenu({
                          x: at.left,
                          y: at.bottom + 4,
                          actions: open
                            .filter((other) => other !== id)
                            .map((other) => {
                              const shown = panes.indexOf(other);
                              const name =
                                prefs.profiles.find((q) => q.id === other)
                                  ?.name ?? "Session";
                              return {
                                label:
                                  shown >= 0 && shown < layout
                                    ? `${name} (swap with pane ${shown + 1})`
                                    : name,
                                run: () => {
                                  setPanes((a) => swapPane(a, index, other));
                                  setFocused(index);
                                  setFocusRequest((n) => n + 1);
                                },
                              };
                            }),
                        });
                      }}
                    >
                      <ArrowLeftRight size={14} />
                    </button>
                  )}
                  <span className="endpoint">
                    {l?.meta?.endpoint ??
                      `${p.username ? `${p.username}@` : ""}${p.host}`}
                  </span>
                  {l &&
                    currentProtocol !== "Rdp" &&
                    currentProtocol !== "Vnc" && (
                      <button
                        className="icon"
                        title="Find in terminal (Ctrl+F)"
                        aria-label="Find in terminal"
                        onClick={() => {
                          focus(index);
                          setFindRequest((r) => ({
                            id: l.id,
                            revision: r.revision + 1,
                          }));
                        }}
                      >
                        <Search size={14} />
                      </button>
                    )}
                  {l?.logging?.active && (
                    <span
                      className="logging-indicator"
                      role="img"
                      aria-label="Output logging enabled"
                      title={
                        l.logging.path
                          ? `Logging to ${l.logging.path}`
                          : "Opening output log…"
                      }
                    >
                      <FileText size={14} />
                    </span>
                  )}
                  {currentProtocol === "Rdp" && (
                    <button
                      className={`icon ${rdpTools.includes(id) ? "active" : ""}`}
                      aria-label="Remote tools"
                      title="Remote tools (files, PowerShell, restart over OpenSSH)"
                      aria-pressed={rdpTools.includes(id)}
                      onClick={() =>
                        setRdpTools((t) =>
                          t.includes(id)
                            ? t.filter((v) => v !== id)
                            : [...t, id],
                        )
                      }
                    >
                      <PanelRight size={14} />
                    </button>
                  )}
                  {currentProtocol === "Ssh" && (
                    <button
                      className={`icon ${sshTools.includes(id) ? "active" : ""}`}
                      aria-label="Remote tools"
                      title="Remote tools (commands, folder sizes)"
                      aria-pressed={sshTools.includes(id)}
                      onClick={() =>
                        setSshTools((t) =>
                          t.includes(id)
                            ? t.filter((v) => v !== id)
                            : [...t, id],
                        )
                      }
                    >
                      <PanelRight size={14} />
                    </button>
                  )}
                  {l && !l.meta?.finished ? (
                    <button
                      className="icon"
                      aria-label="Disconnect"
                      title="Disconnect"
                      onClick={() => disconnectLive(l)}
                    >
                      <Unplug size={14} />
                    </button>
                  ) : (
                    <button
                      className="connect"
                      disabled={!connectable(p.protocol)}
                      onClick={() => void start(p.id)}
                    >
                      <Play size={12} />
                      {l ? "Reconnect" : "Connect"}
                    </button>
                  )}
                </div>
                {currentProtocol === "Telnet" && (
                  <div className="plaintext-warning">
                    TELNET · Unencrypted — input, passwords and output can be
                    read on the network.
                  </div>
                )}
                {l?.meta?.status && (!l.meta.connected || l.meta.finished) && (
                  <div
                    className={`connection-message ${l.meta.finished ? "ended" : ""}`}
                  >
                    {statusSummary(l.meta.status)}
                  </div>
                )}
                {currentProtocol === "Rdp" && (l || rdpTools.includes(id)) ? (
                  // One stable container: opening the tools panel must not
                  // remount the RDP pane (a remount would reconnect).
                  <div
                    className="rdp-split"
                    style={
                      {
                        "--tools-width": `${toolsPanelWidth}px`,
                      } as CSSProperties
                    }
                  >
                    {l ? (
                      <RdpPane
                        // A reconnect is a new native session: fresh component state.
                        key={l.id}
                        live={l}
                        visible={visible}
                        active={visible && focused === index}
                        focusRequest={focusRequest}
                        onMeta={onMeta}
                        onError={error}
                        onFocus={() => focus(index)}
                      />
                    ) : (
                      <div className="rdp-surface">Not connected</div>
                    )}
                    {rdpTools.includes(id) && (
                      <ToolsResizer
                        width={toolsPanelWidth}
                        onWidth={resizeTools}
                      />
                    )}
                    {rdpTools.includes(id) && (
                      <RdpTools
                        profile={p}
                        rdpId={l && !l.meta?.finished ? l.id : undefined}
                        live={toolsLive(id)}
                        onConnect={() => startTools(id)}
                        onDisconnect={() => stopTools(id)}
                        onClose={() => {
                          stopTools(id);
                          setRdpTools((t) => t.filter((v) => v !== id));
                        }}
                        files={
                          toolsLive(id) && (
                            <Files
                              key={toolsLive(id)!.id}
                              live={toolsLive(id)}
                              error={error}
                              onEditOpened={() => setEditMonitoring(true)}
                              onFocusTerminal={() => {}}
                              autoUpload={false}
                              onAskUploads={() => {}}
                            />
                          )
                        }
                        terminal={
                          toolsLive(id) && (
                            <TerminalPane
                              key={toolsLive(id)!.id}
                              findRequest={0}
                              active={false}
                              focusRequest={0}
                              live={toolsLive(id)!}
                              protocol="Ssh"
                              fontSize={prefs.font_size}
                              fontFamily={view.font_family}
                              scrollbackLines={prefs.scrollback_lines}
                              theme={theme.terminal}
                              highlighting={view.highlighting}
                              onMeta={onMeta}
                              onLogging={onLogging}
                              onError={error}
                              onPaste={(pid, text, send) =>
                                setPaste({ id: pid, text, send })
                              }
                              onFocus={() => focus(index)}
                              fixture={fixture}
                              fixtureProbe={fixtureScope !== "telnet-external"}
                              gpu={
                                !fixture ||
                                fixtureScope.startsWith("local-") ||
                                fixtureScope === "zsh-input"
                              }
                            />
                          )
                        }
                      />
                    )}
                  </div>
                ) : l && currentProtocol === "Vnc" ? (
                  <VncPane
                    key={l.id}
                    live={l}
                    visible={visible}
                    active={visible && focused === index}
                    focusRequest={focusRequest}
                    onMeta={onMeta}
                    onError={error}
                    onFocus={() => focus(index)}
                  />
                ) : currentProtocol === "Ssh" &&
                  (l || sshTools.includes(id)) ? (
                  // One stable container: toggling the tools panel must not
                  // remount the terminal.
                  <div
                    className="rdp-split"
                    style={
                      {
                        "--tools-width": `${toolsPanelWidth}px`,
                      } as CSSProperties
                    }
                  >
                    {l ? (
                      <TerminalPane
                        findRequest={
                          findRequest.id === l.id ? findRequest.revision : 0
                        }
                        active={visible && focused === index}
                        focusRequest={focusRequest}
                        live={l}
                        protocol={currentProtocol}
                        fontSize={
                          p.font_size ??
                          (Number.isFinite(view.font_size) &&
                          view.font_size >= 8 &&
                          view.font_size <= 32
                            ? view.font_size
                            : prefs.font_size)
                        }
                        fontFamily={view.font_family}
                        scrollbackLines={prefs.scrollback_lines}
                        theme={theme.terminal}
                        highlighting={view.highlighting}
                        onMeta={onMeta}
                        onLogging={onLogging}
                        onError={error}
                        onPaste={(id, text, send) =>
                          setPaste({ id, text, send })
                        }
                        onFocus={() => focus(index)}
                        fixture={fixture}
                        fixtureProbe={fixtureScope !== "telnet-external"}
                        gpu={
                          !fixture ||
                          fixtureScope.startsWith("local-") ||
                          fixtureScope === "zsh-input"
                        }
                      />
                    ) : (
                      <div className="rdp-surface">Not connected</div>
                    )}
                    {sshTools.includes(id) && (
                      <ToolsResizer
                        width={toolsPanelWidth}
                        onWidth={resizeTools}
                      />
                    )}
                    {sshTools.includes(id) && (
                      <SshTools
                        profile={p}
                        live={l}
                        storageRequest={storageRequests[id]}
                        onClose={() =>
                          setSshTools((t) => t.filter((v) => v !== id))
                        }
                        onFocusTerminal={() => {
                          focus(index);
                          setFocusRequest((n) => n + 1);
                        }}
                        onOpenInFiles={(path) => {
                          if (!l) return;
                          focus(index);
                          setSide("files");
                          void invoke("session_browse", {
                            id: l.id,
                            path,
                          }).catch(error);
                        }}
                        error={error}
                      />
                    )}
                  </div>
                ) : l ? (
                  <TerminalPane
                    findRequest={
                      findRequest.id === l.id ? findRequest.revision : 0
                    }
                    active={visible && focused === index}
                    focusRequest={focusRequest}
                    live={l}
                    protocol={currentProtocol}
                    fontSize={
                      p.font_size ??
                      (Number.isFinite(view.font_size) &&
                      view.font_size >= 8 &&
                      view.font_size <= 32
                        ? view.font_size
                        : prefs.font_size)
                    }
                    fontFamily={view.font_family}
                    scrollbackLines={prefs.scrollback_lines}
                    theme={theme.terminal}
                    highlighting={view.highlighting}
                    onMeta={onMeta}
                    onLogging={onLogging}
                    onError={error}
                    onPaste={(id, text, send) => setPaste({ id, text, send })}
                    onFocus={() => focus(index)}
                    fixture={fixture}
                    fixtureProbe={fixtureScope !== "telnet-external"}
                    gpu={
                      !fixture ||
                      fixtureScope.startsWith("local-") ||
                      fixtureScope === "zsh-input"
                    }
                  />
                ) : (
                  <div className="pane-empty">
                    <TerminalIcon size={28} />
                    <h2>{p.name}</h2>
                    <p>
                      {p.protocol === "Ssh"
                        ? "Ready to connect securely."
                        : p.protocol === "Telnet"
                          ? "Ready to connect without encryption."
                          : p.protocol === "Serial"
                            ? "Ready to open the COM device. Connecting may change control lines."
                            : p.protocol === "Rdp"
                              ? "Ready to open the remote desktop. Windows asks for credentials and, if needed, about the server certificate."
                              : "Ready to open VNC over validated TLS. You will review unknown certificates and enter a password."}
                    </p>
                    {connectable(p.protocol) && (
                      <button
                        className="primary"
                        onClick={() => void start(p.id)}
                      >
                        <Play size={13} />
                        Connect via {protocolLabel(p.protocol)}
                      </button>
                    )}
                    <button
                      className="text-button"
                      onClick={() => setDraft(structuredClone(p))}
                    >
                      Edit connection settings
                    </button>
                  </div>
                )}
                {l &&
                  visible &&
                  currentProtocol === "Ssh" &&
                  l.meta?.connected &&
                  !l.meta.finished &&
                  statisticsEnabled(
                    p.statistics_enabled,
                    view.statistics.enabled,
                  ) && (
                    <Statistics
                      key={l.id}
                      id={l.id}
                      networkUnit={view.statistics.network_unit}
                    />
                  )}
              </section>
            );
          })}
          {panes.slice(0, layout).map((p, index) =>
            p === null ? (
              <section
                key={`empty-${index}`}
                className={`pane ${focused === index ? "focused" : ""}`}
                style={{
                  gridColumn: layout === 1 ? 1 : (index % 2) + 1,
                  gridRow: layout === 4 ? Math.floor(index / 2) + 1 : 1,
                }}
                onMouseDown={() => focus(index)}
              >
                {layout > 1 && (
                  <div className="empty-heading">
                    <span>PANE {index + 1}</span>
                  </div>
                )}
                <div className="pane-empty">
                  <TerminalIcon size={32} />
                  <h1>
                    {layout === 1
                      ? "Your workspace, ready."
                      : "Choose a connection"}
                  </h1>
                  <p>
                    {layout === 1
                      ? "Open a saved session or create your first connection."
                      : "Choose a saved connection below to open it in this pane."}
                  </p>
                  {layout > 1 && closedProfiles.length > 0 ? (
                    <div
                      className="pane-connections"
                      aria-label={`Connections for pane ${index + 1}`}
                    >
                      {closedProfiles.map((profile) => (
                        <button
                          key={profile.id}
                          onClick={() => {
                            setFocused(index);
                            setPanes((a) => assignPane(a, index, profile.id));
                            setOpen((o) =>
                              o.includes(profile.id) ? o : [...o, profile.id],
                            );
                            setFocusRequest((n) => n + 1);
                            if (connectable(profile.protocol))
                              void start(profile.id);
                          }}
                        >
                          <SessionIcon icon={profile.icon} size={16} />
                          <span>
                            {profile.name}
                            <small>
                              {protocolLabel(profile.protocol)}
                              {profile.host ? ` · ${profile.host}` : ""}
                            </small>
                          </span>
                        </button>
                      ))}
                    </div>
                  ) : (
                    <button
                      className="primary"
                      onClick={() => setDraft(blankProfile())}
                    >
                      <Plus size={14} />
                      New session
                    </button>
                  )}
                  <div className="protocols">
                    SSH
                    <span />
                    TELNET
                    <span />
                    RDP
                    <span />
                    VNC
                    <span />
                    SERIAL
                  </div>
                </div>
              </section>
            ) : null,
          )}
        </div>
      </main>
      {notice && (
        <div
          // A native RDP desktop would cover the bottom corner; while one is
          // connected, notices sit in the tab row, which is never covered.
          className={`app-notice ${noticeKind} ${rdpConnected ? "above-rdp" : ""}`}
          role={noticeKind === "success" ? "status" : "alert"}
          style={rdpConnected ? undefined : { bottom: noticeBottom }}
        >
          <span>{notice}</span>
          <button
            className="icon"
            aria-label="Dismiss notification"
            onClick={() => setNotice("")}
          >
            <X size={14} />
          </button>
        </div>
      )}
      <RestoreEdits
        enabled={canSave || fixture}
        lives={lives}
        onManual={(id) => {
          setAutoUploadIds((ids) => ids.filter((value) => value !== id));
          setEditMonitoring(true);
        }}
      />
      <EditMonitor
        enabled={editMonitoring}
        lives={lives}
        autoIds={autoUploadIds}
        onAutoChange={setAutoUploadIds}
        error={error}
      />
      {bulkDeleteIds && (
        <Modal
          title="Delete sessions"
          onClose={actionBusy ? undefined : () => setBulkDeleteIds(null)}
        >
          <p>
            Delete {bulkDeleteIds.length} selected sessions? Their saved
            passwords are removed too. Open connections close first.
          </p>
          <div className="dialog-actions">
            <button
              disabled={actionBusy}
              onClick={() => setBulkDeleteIds(null)}
            >
              Cancel
            </button>
            <button
              className="danger"
              disabled={actionBusy}
              onClick={() =>
                void runSessionAction(async () => {
                  const ids = [...bulkDeleteIds];
                  const closing = liveRef.current.filter(
                    (l) =>
                      ids.includes(l.profileId) ||
                      ids.some((id) => l.profileId === 2 ** 50 + id),
                  );
                  setLives((old) =>
                    old.filter((l) => !closing.some((v) => v.id === l.id)),
                  );
                  setOpen((old) => old.filter((id) => !ids.includes(id)));
                  setPanes((old) =>
                    old.map((id) =>
                      id !== null && ids.includes(id) ? null : id,
                    ),
                  );
                  setRdpTools((old) => old.filter((id) => !ids.includes(id)));
                  setSshTools((old) => old.filter((id) => !ids.includes(id)));
                  for (const live of closing) await closeLive(live);
                  setPrefs(
                    await invoke<Preferences>("delete_profiles", {
                      profileIds: ids,
                    }),
                  );
                  setBulkDeleteIds(null);
                  setSelectedSessions([]);
                  setNotice(`${ids.length} sessions deleted`, "success");
                })
              }
            >
              Delete
            </button>
          </div>
        </Modal>
      )}
      {bulkMoveIds && prefs.profiles.some((p) => p.id === bulkMoveIds[0]) && (
        <MoveToGroup
          key={bulkMoveIds.join(",")}
          profile={prefs.profiles.find((p) => p.id === bulkMoveIds[0])!}
          profileIds={bulkMoveIds}
          profiles={prefs.profiles}
          onSaved={setPrefs}
          onClose={() => setBulkMoveIds(null)}
        />
      )}
      {deleteId !== null &&
        (() => {
          const p = prefs.profiles.find((v) => v.id === deleteId);
          if (!p) return null;
          return (
            <Modal title="Delete session" onClose={() => setDeleteId(null)}>
              <p className="remote-target">{p.name}</p>
              <p>
                Delete this session? Its saved password is removed too. Open
                connections close first.
              </p>
              <div className="dialog-actions">
                <button onClick={() => setDeleteId(null)}>Cancel</button>
                <button
                  className="danger"
                  onClick={() => {
                    setDeleteId(null);
                    void (async () => {
                      if (open.includes(p.id)) await close(p.id);
                      const updated = await invoke<Preferences>(
                        "delete_profile",
                        { profileId: p.id },
                      );
                      setPrefs(updated);
                      setNotice(`${p.name} deleted`, "success");
                    })().catch(error);
                  }}
                >
                  Delete
                </button>
              </div>
            </Modal>
          );
        })()}
      {manageGroup && (
        <ManageGroup
          group={manageGroup.name}
          remove={manageGroup.remove}
          onClose={() => setManageGroup(null)}
          onSaved={(updated, name) => {
            setPrefs(updated);
            setCollapsed((current) => {
              const next = new Set(current);
              const wasCollapsed = next.delete(manageGroup.name);
              if (name && wasCollapsed) next.add(name);
              saveCollapsed(next);
              return next;
            });
            setNotice(
              name ? "Group renamed" : "Group deleted; all sessions kept",
              "success",
            );
          }}
        />
      )}
      {moveGroupId !== null &&
        prefs.profiles.some((p) => p.id === moveGroupId) && (
          <MoveToGroup
            key={moveGroupId}
            profile={prefs.profiles.find((p) => p.id === moveGroupId)!}
            profiles={prefs.profiles}
            onSaved={setPrefs}
            onClose={() => setMoveGroupId(null)}
          />
        )}
      {draft && (
        <Modal
          title={draft.id ? "Edit session" : "New session"}
          onClose={() => setDraft(null)}
        >
          <ProfileEditor
            value={draft}
            onChange={setDraft}
            statisticsEnabled={prefs.statistics.enabled}
            globalFontSize={prefs.font_size}
            onIconBusy={setProfileIconBusy}
            groups={groupNames(prefs.profiles)}
          />
          <div className="dialog-actions">
            <button onClick={() => setDraft(null)}>Cancel</button>
            <button
              className="primary"
              disabled={
                (!canSave && !fixture) ||
                profileIconBusy ||
                !validMac(draft.mac_address ?? "") ||
                (draft.font_size !== null &&
                  (!Number.isFinite(draft.font_size) ||
                    draft.font_size < 8 ||
                    draft.font_size > 32)) ||
                !draft.name.trim() ||
                (!draft.host.trim() && draft.protocol !== "Local")
              }
              onClick={() => void save()}
            >
              Save session
            </button>
          </div>
        </Modal>
      )}
      {dispatchOpen && (
        <Broadcast
          lives={lives}
          profiles={prefs.profiles}
          onClose={() => setDispatchOpen(false)}
        />
      )}
      {settings && (
        <Appearance
          preferences={prefs}
          canSave={canSave || fixture}
          onPreview={setAppearance}
          onClose={() => {
            setAppearance(null);
            setSettings(false);
          }}
          onSaved={(saved) => {
            setPrefs(saved);
            setAppearance(null);
            setSettings(false);
            setNotice("");
          }}
        />
      )}
      {trust?.meta?.trust && (
        <Modal title="Verify SSH host key">
          <div className="trust-heading">
            <ShieldCheck size={22} />
            <span>{trust.meta.trust.host}</span>
          </div>
          <p>
            Compare this fingerprint with a trusted source before approving.
          </p>
          <small>{trust.meta.trust.algorithm}</small>
          <code>{trust.meta.trust.fingerprint}</code>
          <p className="hint">
            YuuTerm pins the key separately. Changed keys are blocked.
          </p>
          <div className="dialog-actions">
            <button
              onClick={() =>
                void invoke("session_trust", {
                  id: trust.id,
                  accept: false,
                }).catch(error)
              }
            >
              Reject
            </button>
            <button
              className="primary"
              onClick={() =>
                void invoke("session_trust", {
                  id: trust.id,
                  accept: true,
                }).catch(error)
              }
            >
              Trust and save
            </button>
          </div>
        </Modal>
      )}
      {paste && (
        <Modal title="Confirm terminal paste" onClose={() => setPaste(null)}>
          <p>
            This paste contains line breaks or control characters and may
            execute commands.
          </p>
          <pre>{JSON.stringify(paste.text)}</pre>
          <div className="dialog-actions">
            <button onClick={() => setPaste(null)}>Cancel</button>
            <button
              className="primary"
              onClick={() => {
                if (
                  liveRef.current.some(
                    (l) => l.id === paste.id && l.meta?.connected,
                  )
                )
                  paste.send();
                else
                  error(
                    "Original connection is no longer active; paste discarded.",
                  );
                setPaste(null);
              }}
            >
              Paste to original session
            </button>
          </div>
        </Modal>
      )}
    </div>
  );
}

function ProfileEditor({
  value: p,
  onChange: set,
  statisticsEnabled,
  globalFontSize,
  onIconBusy,
  groups,
}: {
  groups: string[];
  globalFontSize: number;
  statisticsEnabled: boolean;
  value: Profile;
  onChange: (p: Profile) => void;
  onIconBusy: (busy: boolean) => void;
}) {
  const [tab, setTab] = useState("connection");
  // A protocol change can remove the open tab; fall back to the connection tab.
  useEffect(() => {
    if (
      (tab === "terminal" && (p.protocol === "Rdp" || p.protocol === "Vnc")) ||
      (tab === "statistics" && p.protocol !== "Ssh")
    )
      setTab("connection");
  }, [p.protocol, tab]);
  const change = (patch: Partial<Profile>) => set({ ...p, ...patch });
  const [ports, setPorts] = useState<string[]>([]);
  const [portStatus, setPortStatus] = useState("");
  const refreshPorts = () => {
    setPortStatus("Checking available ports…");
    void invoke<string[]>("serial_ports")
      .then((names) => {
        setPorts(names);
        setPortStatus(
          names.length
            ? `${names.length} port(s) found`
            : "No COM ports found; you can enter a device manually.",
        );
      })
      .catch(() =>
        setPortStatus(
          "Port enumeration failed; enter the COM device manually.",
        ),
      );
  };
  const ssh = (patch: Partial<Profile["ssh"]>) =>
    change({ ssh: { ...p.ssh, ...patch } });
  return (
    <>
      <SettingsTabs
        prefix="session"
        label="Session settings sections"
        tabs={
          p.protocol === "Ssh"
            ? ["connection", "terminal", "statistics"]
            : // Remote desktops have no terminal settings (the RDP tools
              // terminal uses the global font).
              p.protocol === "Rdp" || p.protocol === "Vnc"
              ? ["connection"]
              : ["connection", "terminal"]
        }
        value={tab}
        onChange={setTab}
      />
      <div
        role="tabpanel"
        id="session-panel-connection"
        aria-labelledby="session-tab-connection"
        hidden={tab !== "connection"}
      >
        <label>
          Session name
          <input
            autoFocus
            value={p.name}
            maxLength={128}
            onChange={(e) => change({ name: e.target.value })}
            placeholder="A name you recognize"
          />
        </label>
        <label>
          Group
          <GroupCombo
            value={p.group ?? ""}
            options={groups}
            maxLength={64}
            onChange={(group) => change({ group })}
            onBlur={(e) => change({ group: e.target.value.trim() })}
            placeholder="Optional, for example Development"
          />
        </label>
        <IconPicker
          value={p.icon}
          onChange={(icon) => change({ icon })}
          onBusy={onIconBusy}
        />
        {p.protocol !== "Local" && p.protocol !== "Serial" && (
          <>
            <label>
              MAC address
              <input
                aria-label="MAC address for Wake-on-LAN"
                value={p.mac_address ?? ""}
                maxLength={17}
                placeholder="02:11:22:33:44:55"
                onChange={(e) => change({ mac_address: e.target.value })}
                onBlur={(e) =>
                  change({
                    mac_address: e.target.value
                      .trim()
                      .replaceAll("-", ":")
                      .toUpperCase(),
                  })
                }
              />
            </label>
            <p>Optional. Used to wake this computer with Wake-on-LAN.</p>
            {!validMac(p.mac_address ?? "") && (
              <p role="alert">
                Enter a valid MAC address, for example 02:11:22:33:44:55.
              </p>
            )}
          </>
        )}
        <div className="form-row">
          <label>
            Protocol
            <select
              value={p.protocol}
              onChange={(e) => {
                const protocol = e.target.value as Protocol;
                change({
                  protocol,
                  port: {
                    Ssh: 22,
                    Telnet: 23,
                    Rdp: 3389,
                    Vnc: 5900,
                    Serial: 0,
                    Local: 0,
                  }[protocol],
                  // A local terminal has no host; other types start empty too.
                  ...(protocol === "Local" || p.protocol === "Local"
                    ? { host: "" }
                    : {}),
                  ...(protocol === "Local"
                    ? {
                        local: p.local ?? {
                          shell: "PowerShell",
                          directory: "",
                        },
                        ssh: { ...p.ssh, x11_forward: false },
                      }
                    : {}),
                });
              }}
            >
              {(
                ["Ssh", "Telnet", "Rdp", "Vnc", "Serial", "Local"] as Protocol[]
              ).map((v) => (
                <option key={v} value={v}>
                  {protocolLabel(v)}
                </option>
              ))}
            </select>
          </label>
          {p.protocol === "Local" && (
            <label>
              Shell
              <select
                value={p.local?.shell ?? "PowerShell"}
                onChange={(e) =>
                  change({
                    local: {
                      directory: p.local?.directory ?? "",
                      shell: e.target.value as LocalShell,
                    },
                  })
                }
              >
                {(
                  ["PowerShell", "WindowsPowerShell", "Cmd"] as LocalShell[]
                ).map((v) => (
                  <option key={v} value={v}>
                    {localShellLabel(v)}
                  </option>
                ))}
              </select>
            </label>
          )}
          {p.protocol !== "Local" && (
            <label>
              {p.protocol === "Serial" ? "COM device" : "Host / IP"}
              <input
                value={p.host}
                list={p.protocol === "Serial" ? "serial-ports" : undefined}
                maxLength={253}
                onChange={(e) => change({ host: e.target.value })}
              />
            </label>
          )}
          {p.protocol !== "Serial" && p.protocol !== "Local" && (
            <label className="port">
              Port
              <input
                type="number"
                min={1}
                max={65535}
                value={p.port}
                onChange={(e) => change({ port: Number(e.target.value) })}
              />
            </label>
          )}
        </div>
        {p.protocol === "Local" && (
          <label>
            Start directory
            <input
              value={p.local?.directory ?? ""}
              maxLength={1024}
              placeholder="Empty: your user folder, for example C:\Work"
              onChange={(e) =>
                change({
                  local: {
                    shell: p.local?.shell ?? "PowerShell",
                    directory: e.target.value,
                  },
                })
              }
            />
          </label>
        )}
        {p.protocol === "Serial" && (
          <>
            <datalist id="serial-ports">
              {ports.map((port) => (
                <option key={port} value={port} />
              ))}
            </datalist>
            <button type="button" onClick={refreshPorts}>
              Refresh COM ports
            </button>
            <small role="status">
              {portStatus ||
                "Enter a COM device or refresh the list. Ports are not opened during discovery."}
            </small>
            <div className="form-row">
              <label>
                Baud rate
                <input
                  type="number"
                  min={50}
                  max={4000000}
                  value={p.serial.baud_rate}
                  onChange={(e) =>
                    change({
                      serial: {
                        ...p.serial,
                        baud_rate: Number(e.target.value),
                      },
                    })
                  }
                />
              </label>
              <label>
                Data bits
                <select
                  value={p.serial.data_bits}
                  onChange={(e) =>
                    change({
                      serial: {
                        ...p.serial,
                        data_bits: Number(e.target.value),
                      },
                    })
                  }
                >
                  {[5, 6, 7, 8].map((v) => (
                    <option key={v}>{v}</option>
                  ))}
                </select>
              </label>
              <label>
                Parity
                <select
                  value={p.serial.parity}
                  onChange={(e) =>
                    change({
                      serial: {
                        ...p.serial,
                        parity: e.target.value as Profile["serial"]["parity"],
                      },
                    })
                  }
                >
                  {["None", "Odd", "Even"].map((v) => (
                    <option key={v}>{v}</option>
                  ))}
                </select>
              </label>
              <label>
                Stop bits
                <select
                  value={p.serial.stop_bits}
                  onChange={(e) =>
                    change({
                      serial: {
                        ...p.serial,
                        stop_bits: Number(e.target.value),
                      },
                    })
                  }
                >
                  {[1, 2].map((v) => (
                    <option key={v}>{v}</option>
                  ))}
                </select>
              </label>
            </div>
            <label>
              Flow control
              <select
                value={p.serial.flow_control}
                onChange={(e) =>
                  change({
                    serial: {
                      ...p.serial,
                      flow_control: e.target
                        .value as Profile["serial"]["flow_control"],
                    },
                  })
                }
              >
                <option value="None">None</option>
                <option value="Software">XON/XOFF</option>
                <option value="Hardware">RTS/CTS</option>
              </select>
            </label>
            <div className="form-row serial-lines">
              <label>
                <input
                  type="checkbox"
                  checked={p.serial.dtr}
                  onChange={(e) =>
                    change({ serial: { ...p.serial, dtr: e.target.checked } })
                  }
                />
                Assert DTR on connect
              </label>
              <label>
                <input
                  type="checkbox"
                  disabled={p.serial.flow_control === "Hardware"}
                  checked={p.serial.rts}
                  onChange={(e) =>
                    change({ serial: { ...p.serial, rts: e.target.checked } })
                  }
                />
                Assert RTS (without RTS/CTS)
              </label>
            </div>
            <p className="warning">
              Connecting can change control lines or reset a device. No
              automatic commands or local echo are sent. Enter sends CR.
            </p>
          </>
        )}
        {p.protocol !== "Serial" &&
          p.protocol !== "Telnet" &&
          p.protocol !== "Local" && (
            <label>
              Username
              <input
                value={p.username}
                maxLength={256}
                onChange={(e) => change({ username: e.target.value })}
              />
            </label>
          )}
        {p.protocol === "Vnc" && <VncSettings profile={p} onChange={change} />}
        {p.protocol === "Rdp" && (
          <fieldset className="session-statistics-settings">
            <legend>Remote desktop</legend>
            <label className="check-label">
              <input
                type="checkbox"
                aria-label="Share clipboard"
                checked={p.rdp?.clipboard ?? true}
                onChange={(e) =>
                  change({
                    rdp: { ...rdpSettings(p), clipboard: e.target.checked },
                  })
                }
              />
              Share the clipboard with the remote computer
            </label>
            <small>
              Saving a change reconnects a running session; open programs on the
              remote computer stay open.
            </small>
            <label>
              Remote audio
              <select
                value={p.rdp?.audio ?? "Off"}
                onChange={(e) =>
                  change({
                    rdp: {
                      ...rdpSettings(p),
                      audio: e.target.value as "Local" | "Remote" | "Off",
                    },
                  })
                }
              >
                <option value="Off">Do not play</option>
                <option value="Local">Play on this computer</option>
                <option value="Remote">Play on the remote computer</option>
              </select>
            </label>
            <label>
              Windows key combinations
              <select
                value={p.rdp?.keyboard ?? "Remote"}
                onChange={(e) =>
                  change({
                    rdp: {
                      ...rdpSettings(p),
                      keyboard: e.target.value as "Remote" | "Local",
                    },
                  })
                }
              >
                <option value="Remote">
                  On the remote computer while it has focus
                </option>
                <option value="Local">On this computer</option>
              </select>
            </label>
            <RdpCertificate profileId={p.id} />
            <p className="hint">
              Drives, printers, smart cards and other devices are not shared. A
              Windows prompt asks for the password; "Remember me" keeps it in
              Windows Credential Manager on this computer. Windows shows its own
              warning when it cannot verify the server.
            </p>
          </fieldset>
        )}
        {p.protocol === "Ssh" && (
          <>
            <X11Settings
              enabled={p.ssh.x11_forward ?? false}
              onChange={(x11_forward) => ssh({ x11_forward })}
            />
            <label>
              Authentication
              <select
                value={p.ssh.auth}
                onChange={(e) =>
                  ssh({ auth: e.target.value as Profile["ssh"]["auth"] })
                }
              >
                <option value="WindowsAgent">Windows SSH agent</option>
                <option value="KeyFile">Private-key file</option>
                <option value="Password">Password</option>
              </select>
            </label>
            {p.ssh.auth === "WindowsAgent" ? (
              <>
                <label>
                  Identity fingerprint{" "}
                  <span className="optional">optional</span>
                  <input
                    value={p.ssh.identity_fingerprint}
                    onChange={(e) =>
                      ssh({ identity_fingerprint: e.target.value })
                    }
                    placeholder="SHA256:…"
                  />
                </label>
                <details className="advanced">
                  <summary>Agent and OpenSSH identity rules</summary>
                  <label>
                    OpenSSH identity host{" "}
                    <span className="optional">optional</span>
                    <input
                      value={p.ssh.config_host}
                      maxLength={253}
                      onChange={(e) => ssh({ config_host: e.target.value })}
                      placeholder="Uses Host / IP or a unique matching HostName entry"
                    />
                  </label>
                  <small>
                    Reads IdentityFile / IdentitiesOnly from your user SSH
                    config. Endpoint fields remain as entered. An explicit
                    fingerprint overrides identity-config lookup.
                  </small>
                  <label>
                    SSH identity config file{" "}
                    <span className="optional">optional</span>
                    <input
                      value={p.ssh.config_file}
                      onChange={(e) => ssh({ config_file: e.target.value })}
                      placeholder="Default: your user .ssh/config"
                    />
                  </label>
                  <label>
                    Local named pipe
                    <input
                      value={p.ssh.agent_pipe}
                      onChange={(e) => ssh({ agent_pipe: e.target.value })}
                    />
                  </label>
                </details>
              </>
            ) : p.ssh.auth === "Password" ? (
              <small>
                A Windows prompt asks for the password when connecting;
                "Remember me" keeps it in the Windows Credential Manager for
                this saved connection on this computer, never in the bookmark
                file.
              </small>
            ) : (
              <label>
                Private-key path
                <input
                  value={p.ssh.key_file}
                  onChange={(e) => ssh({ key_file: e.target.value })}
                />
                <small>
                  For encrypted keys, load the key into the Windows agent.
                </small>
              </label>
            )}
          </>
        )}
      </div>
      {p.protocol === "Ssh" && (
        <div
          role="tabpanel"
          id="session-panel-statistics"
          aria-labelledby="session-tab-statistics"
          hidden={tab !== "statistics"}
        >
          <fieldset className="session-statistics-settings">
            <legend>Host statistics</legend>
            <label className="check-label">
              <input
                type="checkbox"
                aria-label="Show session host statistics"
                checked={p.statistics_enabled ?? statisticsEnabled}
                onChange={(e) =>
                  change({ statistics_enabled: e.target.checked })
                }
              />
              Show host statistics
            </label>
            <div className="setting-inheritance">
              <small>
                {p.statistics_enabled === null
                  ? `Using global setting (${statisticsEnabled ? "on" : "off"})`
                  : "Session override"}
              </small>
              <button
                disabled={p.statistics_enabled === null}
                onClick={() => change({ statistics_enabled: null })}
              >
                Use global setting
              </button>
            </div>
          </fieldset>
        </div>
      )}
      <div
        role="tabpanel"
        id="session-panel-terminal"
        aria-labelledby="session-tab-terminal"
        hidden={tab !== "terminal"}
      >
        <label>
          Font size override
          <input
            aria-label="Session font size"
            type="number"
            min={8}
            max={32}
            step="any"
            placeholder={`Global (${globalFontSize} px)`}
            value={p.font_size ?? ""}
            onChange={(e) =>
              change({
                font_size:
                  e.target.value === "" ? null : Number(e.target.value),
              })
            }
          />
          <small>8–32 px. Leave empty to use the global setting.</small>
        </label>
        {["Ssh", "Telnet", "Serial", "Local"].includes(p.protocol) && (
          <fieldset className="session-statistics-settings">
            <legend>Terminal output logging</legend>
            <label className="check-label">
              <input
                type="checkbox"
                aria-label="Log terminal output"
                checked={p.logging.enabled}
                onChange={(e) =>
                  change({
                    logging: { ...p.logging, enabled: e.target.checked },
                  })
                }
              />
              Log terminal output automatically
            </label>
            <label className="log-file-setting">
              Log file (optional)
              <input
                aria-label="Log file path"
                disabled={!p.logging.enabled}
                value={p.logging.file}
                maxLength={1024}
                placeholder={`Documents\\YuuTerm\\${p.name || "Session"}\\${p.logging.rotation ? "current.log" : "<timestamp>.log"}`}
                onChange={(e) =>
                  change({ logging: { ...p.logging, file: e.target.value } })
                }
              />
            </label>
            <p className="hint">
              Leave empty for{" "}
              {p.logging.rotation
                ? "current.log reused across reconnects"
                : "a new timestamped file"}{" "}
              in Documents\YuuTerm\
              {p.name || "Session"}. A custom file is appended to. Plaintext
              output; stops at 64 MiB unless rotation is enabled. Applies on the
              next connection and reconnects.
            </p>
            <LogRotationSettings
              value={p.logging}
              onChange={(logging) => change({ logging })}
            />
          </fieldset>
        )}
        {p.protocol === "Telnet" && (
          <label className="check-label">
            <input
              type="checkbox"
              aria-label="TELNET local line editing"
              checked={p.telnet.local_line_editing}
              onChange={(e) =>
                change({ telnet: { local_line_editing: e.target.checked } })
              }
            />
            Local line editing (next connection; Enter sends, maximum 4096 UTF-8
            bytes)
          </label>
        )}
      </div>
      {p.protocol === "Telnet" && (
        <p className="warning">TELNET transmits data without encryption.</p>
      )}
    </>
  );
}
createRoot(document.getElementById("root")!).render(<App />);
