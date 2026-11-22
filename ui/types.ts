export type Protocol = "Ssh" | "Telnet" | "Rdp" | "Vnc" | "Serial" | "Local";
export type LocalShell =
  "PowerShell" | "WindowsPowerShell" | "Cmd" | "GitBash" | "Wsl" | "Cygwin";
export const localShells: LocalShell[] = [
  "PowerShell",
  "WindowsPowerShell",
  "Cmd",
  "GitBash",
  "Wsl",
  "Cygwin",
];
export const localShellLabel = (s: LocalShell) =>
  ({
    PowerShell: "PowerShell 7",
    WindowsPowerShell: "Windows PowerShell",
    Cmd: "Command Prompt",
    GitBash: "Git Bash",
    Wsl: "WSL",
    Cygwin: "Cygwin",
  })[s];
export interface Profile {
  id: number;
  name: string;
  icon?: string;
  group?: string;
  mac_address?: string;
  /** Quick connect: in memory only, never saved. */
  temporary?: boolean;
  protocol: Protocol;
  host: string;
  port: number;
  username: string;
  font_size: number | null;
  font_family?: string | null;
  font_color?: string | null;
  statistics_enabled: boolean | null;
  statistics_hidden?: StatisticsItem[];
  logging: {
    enabled: boolean;
    file: string;
    rotation?: { max_mib: number; keep_files: number } | null;
  };
  telnet: { local_line_editing: boolean };
  serial: {
    baud_rate: number;
    data_bits: number;
    parity: "None" | "Odd" | "Even";
    stop_bits: number;
    flow_control: "None" | "Software" | "Hardware";
    dtr: boolean;
    rts: boolean;
  };
  rdp?: {
    clipboard: boolean;
    audio: "Local" | "Remote" | "Off";
    keyboard?: "Remote" | "Local";
    tools_sign_in?: RdpToolsSignIn;
    tools_key_file?: string;
  };
  local?: { shell: LocalShell; directory: string };
  vnc?: { auth: "Plain" | "Vnc"; clipboard?: boolean; resize_server?: boolean };
  // Backend-owned identity of a saved password; never edited in the UI.
  credential_id?: string;
  /** SSH gateways (jump hosts) in connection order; absent or empty is direct. */
  gateways?: Gateway[];
  ssh: {
    x11_forward?: boolean;
    auth: "WindowsAgent" | "KeyFile" | "Password";
    agent_pipe: string;
    identity_fingerprint: string;
    config_host: string;
    config_file: string;
    key_file: string;
  };
}
export interface ManualGateway {
  host: string;
  port: number;
  username: string;
  auth: "WindowsAgent" | "KeyFile" | "Password";
  key_file: string;
  agent_pipe: string;
  identity_fingerprint: string;
  // Backend-owned identity of the hop's saved password; never edited in the UI.
  credential_id?: string;
}
export type Gateway =
  { kind: "Saved"; profile_id: number } | ({ kind: "Manual" } & ManualGateway);
export interface Preferences {
  version: number;
  profiles: Profile[];
  mocha: boolean;
  /** Theme id from ui/themes.ts; empty for older settings. */
  theme?: string;
  font_size: number;
  font_family: string;
  scrollback_lines: number;
  highlighting: Highlighting;
  statistics: StatisticsSettings;
  local_shell?: LocalShell;
  update_check?: boolean;
  /** SSH tools layout; absent or empty keeps the built-in order. */
  ssh_commands?: CommandCategory[];
  rdp_commands?: CommandCategory[];
}
export type CommandMode = "run" | "confirm" | "insert";
export type CommandItem =
  | { kind: "builtin"; id: string }
  | { kind: "custom"; label: string; command: string; mode: CommandMode };
export interface CommandCategory {
  name: string;
  hidden?: boolean;
  items: CommandItem[];
}
export interface StatisticsSettings {
  enabled: boolean;
  network_unit: "Megabytes" | "Kilobytes" | "Megabits";
}
export interface Highlighting {
  enabled: boolean;
  ipv4: boolean;
  ipv6: boolean;
  ipv4_color: string;
  ipv6_color: string;
  rules: { keywords: string; color: string; regex?: boolean }[];
}
export interface Bootstrap {
  rdp_probe: boolean;
  rdp_live: boolean;
  prefs: Preferences;
  can_save: boolean;
  notice: string;
  fixture: boolean;
}
export interface FileEntry {
  name: string;
  directory: boolean;
  symlink: boolean;
  size: number | null;
  permissions: number | null;
  uid: number | null;
  gid: number | null;
  owner: string | null;
  group: string | null;
  mtime: number | null;
}
export interface TrustPrompt {
  host: string;
  /** "Target" or "Gateway N of M". */
  role?: string;
  algorithm: string;
  fingerprint: string;
}
export interface Meta {
  protocol: Protocol;
  endpoint: string;
  status: string;
  connected: boolean;
  finished: boolean;
  trust: TrustPrompt | null;
  path: string;
  password_rejected?: boolean;
  files_status: string;
  files: FileEntry[] | null;
}
export interface Live {
  telnetLineEditing?: boolean;
  protocol: Protocol;
  id: number;
  profileId: number;
  meta?: Meta;
  logging?: OutputLogStatus | null;
}
export interface Poll {
  meta: Meta | null;
  chunks: number[][];
  more: boolean;
  logging: OutputLogStatus | null;
  /** The terminal output goes to an own window. */
  popped?: boolean;
}
export interface OutputLogStatus {
  durable_bytes?: number;
  active: boolean;
  finished: boolean;
  bytes: number;
  error: string | null;
  path: string | null;
}
export const statisticsItems = [
  ["Host", "Host name"],
  ["Cpu", "CPU"],
  ["Memory", "RAM"],
  ["Network", "Upload and download"],
  ["Uptime", "Uptime"],
  ["Disk", "Disk"],
  ["Latency", "Latency"],
] as const;
export type StatisticsItem = (typeof statisticsItems)[number][0];
export type RdpToolsSignIn = "Password" | "Agent" | "KeyFile";
export const rdpSettings = (p: Profile) => ({
  clipboard: true,
  audio: "Off" as "Local" | "Remote" | "Off",
  keyboard: "Remote" as "Remote" | "Local",
  tools_sign_in: "Password" as RdpToolsSignIn,
  tools_key_file: "",
  ...p.rdp,
});
// Protocols with an integrated session adapter.
export const connectable = (p: Protocol) =>
  p === "Ssh" ||
  p === "Telnet" ||
  p === "Serial" ||
  p === "Rdp" ||
  p === "Vnc" ||
  p === "Local";
export const blankProfile = (shell: LocalShell = "PowerShell"): Profile => ({
  id: 0,
  name: "",
  protocol: "Ssh",
  host: "",
  port: 22,
  username: "",
  font_size: null,
  statistics_enabled: null,
  logging: { enabled: false, file: "", rotation: null },
  telnet: { local_line_editing: false },
  serial: {
    baud_rate: 115200,
    data_bits: 8,
    parity: "None",
    stop_bits: 1,
    flow_control: "None",
    dtr: false,
    rts: false,
  },
  ssh: {
    auth: "WindowsAgent",
    agent_pipe: "\\\\.\\pipe\\openssh-ssh-agent",
    identity_fingerprint: "",
    config_host: "",
    config_file: "",
    key_file: "",
  },
  rdp: { clipboard: true, audio: "Off", keyboard: "Remote" },
  vnc: { auth: "Plain" },
  local: { shell, directory: "" },
});
export const protocolLabel = (p: Protocol) =>
  ({
    Ssh: "SSH",
    Telnet: "TELNET",
    Rdp: "RDP",
    Vnc: "VNC",
    Serial: "SERIAL",
    Local: "LOCAL",
  })[p];
/** Second line in the session list: host, or the shell of a local terminal. */
export const sessionDetail = (p: Profile) =>
  p.protocol === "Local"
    ? localShellLabel(p.local?.shell ?? "PowerShell")
    : p.host;
