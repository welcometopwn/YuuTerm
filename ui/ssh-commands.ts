// Command catalog for the SSH tools panel. Commands are typed into the
// session's visible terminal, so sudo prompts and interactive programs stay in
// front of the user. Inputs are validated strictly instead of relying on
// shell-specific quoting; the text works in sh, bash, zsh and fish.

export type Probe = {
  os: string;
  shell: string;
  uid: number | null;
  tools: string[];
  /** Membership of adm, systemd-journal, wheel and docker only. */
  groups: string[];
};

export type Param = "pid" | "unit" | "container" | "host" | "path";

export type Context = {
  /** Undefined when detection failed: every command stays enabled. */
  probe?: Probe;
  value?: string;
  action?: string;
};

export type SshCommand = {
  id: string;
  category: string;
  label: string;
  /** State-changing: always confirmed with the exact command shown. */
  danger?: boolean;
  param?: Param;
  /** Choices shown beside the input, e.g. restart/stop/start. */
  actions?: { value: string; label: string }[];
  /** Opens the storage view instead of typing a command. */
  storage?: boolean;
  /** Reason the command cannot run on this host, or null. */
  unavailable?: (probe: Probe) => string | null;
  build: (context: Context) => string;
};

const has = (probe: Probe | undefined, tool: string) =>
  !probe || probe.tools.includes(tool);
const needs =
  (...tools: string[]) =>
  (probe: Probe) =>
    tools.every((t) => probe.tools.includes(t))
      ? null
      : `Not installed on this host (${tools.filter((t) => !probe.tools.includes(t)).join(", ")})`;
const needsAny =
  (...tools: string[]) =>
  (probe: Probe) =>
    tools.some((t) => probe.tools.includes(t))
      ? null
      : `Not installed on this host (${tools.join(" or ")})`;
/** Root needs no sudo (and minimal containers often lack it). */
const sudo = (probe?: Probe) => (probe?.uid === 0 ? "" : "sudo ");
/** sudo only for a detected non-root account outside the given groups. */
const unless =
  (...groups: string[]) =>
  (probe?: Probe) =>
    probe &&
    probe.uid !== 0 &&
    !groups.some((g) => (probe.groups ?? []).includes(g))
      ? "sudo "
      : "";
// Reading the system journal needs one of these groups (wheel on Fedora/RHEL).
const journal = unless("adm", "systemd-journal", "wheel");
const docker = unless("docker");

export const PARAM_LABELS: Record<Param, string> = {
  pid: "Process ID",
  unit: "Service (systemd unit)",
  container: "Container name or ID",
  host: "Host name or IP address",
  path: "Absolute path",
};

const PATTERNS: Record<Exclude<Param, "path">, RegExp> = {
  pid: /^[1-9][0-9]{0,9}$/,
  unit: /^[A-Za-z0-9@_.:][A-Za-z0-9@_.:-]{0,255}$/,
  container: /^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/,
  host: /^[A-Za-z0-9.:][A-Za-z0-9.:-]{0,252}$/,
};

/** Returns a problem description, or null when the value is safe to type. */
export function validate(param: Param, value: string): string | null {
  if (param === "path") {
    if (!value.startsWith("/")) return "Enter an absolute path starting with /";
    if (value.length > 4096) return "Path is too long";
    // Backslashes would be reinterpreted inside fish single quotes.
    if (/[\u0000-\u001f\u007f\\]/.test(value))
      return "Path contains control characters or backslashes";
    return null;
  }
  return PATTERNS[param].test(value)
    ? null
    : `Invalid ${PARAM_LABELS[param].toLowerCase()}`;
}

/** Single-quoted shell word; `'` becomes `'\''`, valid in sh, bash, zsh and fish. */
export function quote(value: string) {
  return `'${value.replace(/'/g, `'\\''`)}'`;
}

const value = (context: Context) => context.value ?? "";

export const COMMANDS: SshCommand[] = [
  {
    id: "htop",
    category: "System",
    label: "Processes (htop)",
    build: ({ probe }) => (has(probe, "htop") ? "htop" : "top"),
  },
  { id: "memory", category: "System", label: "Memory", build: () => "free -h" },
  {
    id: "sensors",
    category: "System",
    label: "Temperatures",
    unavailable: needs("sensors"),
    build: () => "sensors",
  },
  {
    id: "folder-sizes",
    category: "Storage",
    label: "Folder sizes",
    storage: true,
    build: () => "",
  },
  {
    id: "lsblk",
    category: "Storage",
    label: "Block devices",
    unavailable: needs("lsblk"),
    build: () => "lsblk",
  },
  {
    id: "largest-files",
    category: "Storage",
    label: "Largest files",
    param: "path",
    build: (c) =>
      `find ${quote(value(c))} -xdev -type f -printf '%s\\t%p\\n' 2>/dev/null | sort -nr | head -n 20` +
      (has(c.probe, "numfmt") ? " | numfmt --to=iec --field=1" : ""),
  },
  {
    id: "journal-vacuum",
    category: "Storage",
    label: "Trim journal to 7 days",
    danger: true,
    unavailable: needs("journalctl"),
    build: ({ probe }) => `${sudo(probe)}journalctl --vacuum-time=7d`,
  },
  {
    id: "top-memory",
    category: "Processes",
    label: "Top memory",
    build: () => "ps aux --sort=-%mem | head -n 15",
  },
  {
    id: "top-cpu",
    category: "Processes",
    label: "Top CPU",
    build: () => "ps aux --sort=-%cpu | head -n 15",
  },
  {
    id: "kill",
    category: "Processes",
    label: "End process",
    danger: true,
    param: "pid",
    actions: [
      { value: "term", label: "End" },
      { value: "kill", label: "Force (kill -9)" },
    ],
    build: (c) => `kill ${c.action === "kill" ? "-9 " : ""}${value(c)}`,
  },
  {
    id: "failed-services",
    category: "Services & logs",
    label: "Failed services",
    unavailable: needs("systemctl"),
    build: () => "systemctl --failed",
  },
  {
    id: "service-status",
    category: "Services & logs",
    label: "Service status",
    param: "unit",
    unavailable: needs("systemctl"),
    build: (c) => `systemctl status --no-pager ${value(c)}`,
  },
  {
    id: "service-control",
    category: "Services & logs",
    label: "Control service",
    danger: true,
    param: "unit",
    actions: [
      { value: "restart", label: "Restart" },
      { value: "stop", label: "Stop" },
      { value: "start", label: "Start" },
    ],
    unavailable: needs("systemctl"),
    build: (c) =>
      `${sudo(c.probe)}systemctl ${c.action ?? "restart"} ${value(c)}`,
  },
  {
    id: "journal-follow",
    category: "Services & logs",
    label: "Live system log",
    unavailable: needs("journalctl"),
    build: ({ probe }) => `${journal(probe)}journalctl -f`,
  },
  {
    id: "journal-errors",
    category: "Services & logs",
    label: "Errors since boot",
    unavailable: needs("journalctl"),
    build: ({ probe }) => `${journal(probe)}journalctl -p err -b`,
  },
  {
    id: "service-log",
    category: "Services & logs",
    label: "Service log",
    param: "unit",
    unavailable: needs("journalctl"),
    build: (c) => `${journal(c.probe)}journalctl -u ${value(c)} -n 200 -f`,
  },
  {
    id: "last",
    category: "Services & logs",
    label: "Recent logins",
    unavailable: needs("last"),
    build: () => "last -n 20",
  },
  {
    id: "ssh-failures",
    category: "Services & logs",
    label: "Failed SSH logins",
    unavailable: needs("journalctl"),
    // Debian names the unit ssh, Fedora/RHEL sshd.
    build: ({ probe }) =>
      `${journal(probe)}journalctl -u ssh -u sshd --since -24h --no-pager | grep -Ei 'failed|invalid'`,
  },
  {
    id: "addresses",
    category: "Network",
    label: "Addresses",
    unavailable: needs("ip"),
    build: () => "ip -br a",
  },
  {
    id: "ports",
    category: "Network",
    label: "Listening ports",
    unavailable: needs("ss"),
    build: ({ probe }) => `${sudo(probe)}ss -tulpn`,
  },
  {
    id: "connections",
    category: "Network",
    label: "Active connections",
    unavailable: needs("ss"),
    build: () => "ss -tnp",
  },
  {
    id: "ping",
    category: "Network",
    label: "Ping",
    param: "host",
    build: (c) => `ping -c 4 ${value(c)}`,
  },
  {
    id: "firewall",
    category: "Network",
    label: "Firewall status",
    unavailable: needsAny("ufw", "nft"),
    build: ({ probe }) =>
      has(probe, "ufw")
        ? `${sudo(probe)}ufw status verbose`
        : `${sudo(probe)}nft list ruleset`,
  },
  {
    id: "docker-ps",
    category: "Docker",
    label: "Containers",
    unavailable: needs("docker"),
    build: ({ probe }) => `${docker(probe)}docker ps -a`,
  },
  {
    id: "docker-stats",
    category: "Docker",
    label: "Live usage",
    unavailable: needs("docker"),
    build: ({ probe }) => `${docker(probe)}docker stats`,
  },
  {
    id: "docker-logs",
    category: "Docker",
    label: "Container log",
    param: "container",
    unavailable: needs("docker"),
    build: (c) => `${docker(c.probe)}docker logs -f --tail 200 ${value(c)}`,
  },
  {
    id: "docker-restart",
    category: "Docker",
    label: "Restart container",
    danger: true,
    param: "container",
    unavailable: needs("docker"),
    build: (c) => `${docker(c.probe)}docker restart ${value(c)}`,
  },
  {
    id: "docker-prune",
    category: "Docker",
    label: "Prune unused data",
    danger: true,
    unavailable: needs("docker"),
    build: ({ probe }) => `${docker(probe)}docker system prune`,
  },
  {
    id: "updates",
    category: "Packages",
    label: "Available updates",
    unavailable: needsAny("apt", "dnf"),
    build: ({ probe }) =>
      has(probe, "apt") ? "apt list --upgradable" : "dnf check-update",
  },
  {
    id: "upgrade",
    category: "Packages",
    label: "Install updates",
    danger: true,
    unavailable: needsAny("apt", "dnf"),
    build: ({ probe }) =>
      has(probe, "apt")
        ? `${sudo(probe)}apt update && ${sudo(probe)}apt upgrade`
        : `${sudo(probe)}dnf upgrade`,
  },
  {
    // Listed before Full upgrade so that lands under Install updates in two columns.
    id: "reboot-required",
    category: "Packages",
    label: "Reboot required?",
    unavailable: (probe) =>
      probe.tools.includes("apt") || probe.tools.includes("needs-restarting")
        ? null
        : "Not available on this host (apt or needs-restarting)",
    build: ({ probe }) =>
      has(probe, "apt")
        ? "cat /var/run/reboot-required 2>/dev/null || echo 'No reboot required'"
        : "needs-restarting -r",
  },
  {
    // May remove packages to resolve conflicts; Proxmox VE requires it.
    id: "full-upgrade",
    category: "Packages",
    label: "Full upgrade",
    danger: true,
    unavailable: needs("apt"),
    build: ({ probe }) =>
      `${sudo(probe)}apt update && ${sudo(probe)}apt full-upgrade`,
  },
  {
    id: "reboot",
    category: "Power",
    label: "Restart",
    danger: true,
    build: ({ probe }) => `${sudo(probe)}reboot`,
  },
  {
    id: "poweroff",
    category: "Power",
    label: "Shut down",
    danger: true,
    build: ({ probe }) => `${sudo(probe)}poweroff`,
  },
];

export const CATEGORIES = [...new Set(COMMANDS.map((c) => c.category))];

/**
 * Command that opens a root login shell in the terminal, or null when the
 * account already is root. `sudo -i` where sudo exists, otherwise `su -`.
 */
export function rootShell(probe?: Probe) {
  if (probe?.uid === 0) return null;
  return probe && !probe.tools.includes("sudo") ? "su -" : "sudo -i";
}

/** The probe as seen from a root shell opened with `rootShell`. */
export function asRoot(probe?: Probe): Probe | undefined {
  return probe && { ...probe, uid: 0 };
}

/** Why the command cannot run, or null. Without a probe everything is allowed. */
export function unavailable(command: SshCommand, probe?: Probe) {
  if (!probe) return null;
  return command.unavailable?.(probe) ?? null;
}

/** Text typed into the terminal; throws on invalid input. */
export function commandText(command: SshCommand, context: Context) {
  if (command.param) {
    const problem = validate(command.param, context.value ?? "");
    if (problem) throw Error(problem);
  }
  if (
    command.actions &&
    !command.actions.some((a) => a.value === context.action)
  )
    context = { ...context, action: command.actions[0].value };
  return command.build(context);
}

export function matches(command: SshCommand, query: string) {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return [command.label, command.category, command.build({})]
    .join(" ")
    .toLowerCase()
    .includes(q);
}

/** Human-readable size from KiB (du/df units). */
export function formatKib(kib: number) {
  const units = ["KiB", "MiB", "GiB", "TiB", "PiB"];
  let size = kib,
    unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit++;
  }
  return `${size >= 100 || unit === 0 ? Math.round(size) : size.toFixed(1)} ${units[unit]}`;
}

/** Joins a child name to an absolute directory path. */
export function childPath(directory: string, name: string) {
  return directory.endsWith("/") ? directory + name : `${directory}/${name}`;
}

/** Parent of an absolute path; `/` stays `/`. */
export function parentPath(path: string) {
  const trimmed = path.replace(/\/+$/, "");
  const index = trimmed.lastIndexOf("/");
  return index <= 0 ? "/" : trimmed.slice(0, index);
}
