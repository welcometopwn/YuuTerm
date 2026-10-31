export type Protocol = "Ssh" | "Telnet" | "Rdp" | "Vnc" | "Serial" | "Local";
export type LocalShell = "PowerShell" | "WindowsPowerShell" | "Cmd";
export const localShellLabel = (s: LocalShell) =>
  ({
    PowerShell: "PowerShell 7",
    WindowsPowerShell: "Windows PowerShell",
    Cmd: "Command Prompt",
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
  statistics_enabled: boolean | null;
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
  };
  local?: { shell: LocalShell; directory: string };
  vnc?: { auth: "Plain" | "Vnc"; clipboard?: boolean; resize_server?: boolean };
  // Backend-owned identity of a saved password; never edited in the UI.
  credential_id?: string;
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
}
export interface OutputLogStatus {
  durable_bytes?: number;
  active: boolean;
  finished: boolean;
  bytes: number;
  error: string | null;
  path: string | null;
}
export const rdpSettings = (p: Profile) => ({
  clipboard: true,
  audio: "Off" as "Local" | "Remote" | "Off",
  keyboard: "Remote" as "Remote" | "Local",
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
export const blankProfile = (): Profile => ({
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
  local: { shell: "PowerShell", directory: "" },
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
