// Command catalog for the SSH tools panel. Commands are typed into the
// session's visible terminal, so sudo prompts and interactive programs stay in
// front of the user. Inputs are validated strictly instead of relying on
// shell-specific quoting; the text works in sh, bash, zsh and fish.

import type { CommandCategory, CommandItem } from "./types";

export type Probe = {
  os: string;
  shell: string;
  uid: number | null;
  tools: string[];
  /** Membership of adm, systemd-journal, wheel and docker only. */
  groups: string[];
};

export type Param =
  | "pid"
  | "unit"
  | "container"
  | "host"
  | "path"
  | "vmid"
  | "service"
  | "session";

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
  /** User-defined in Settings; typed without Enter when `insert` is set. */
  custom?: boolean;
  insert?: boolean;
  /** Opens the storage view instead of typing a command. */
  storage?: boolean;
  /** Shown only when the probe found this tool; hidden if detection failed. */
  platform?: string;
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
  vmid: "VM or container ID",
  service: "Windows service name",
  session: "Session ID (from Signed-in users)",
};

const PATTERNS: Record<Exclude<Param, "path">, RegExp> = {
  pid: /^[1-9][0-9]{0,9}$/,
  unit: /^[A-Za-z0-9@_.:][A-Za-z0-9@_.:-]{0,255}$/,
  container: /^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/,
  host: /^[A-Za-z0-9.:][A-Za-z0-9.:-]{0,252}$/,
  vmid: /^[1-9][0-9]{2,8}$/,
  service: /^[A-Za-z0-9_.@-][A-Za-z0-9 _.@-]{0,255}$/,
  session: /^[0-9]{1,5}$/,
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
        ? `${sudo(probe)}apt update && ${sudo(probe)}apt upgrade -y`
        : `${sudo(probe)}dnf upgrade -y`,
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
      `${sudo(probe)}apt update && ${sudo(probe)}apt full-upgrade -y`,
  },
  {
    id: "autoremove",
    category: "Packages",
    label: "Remove unused packages",
    danger: true,
    unavailable: needsAny("apt", "dnf"),
    build: ({ probe }) =>
      `${sudo(probe)}${has(probe, "apt") ? "apt" : "dnf"} autoremove -y`,
  },
  ...proxmox(),
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

function proxmox(): SshCommand[] {
  const base = { category: "Proxmox", platform: "pveversion" };
  const power = [
    { value: "start", label: "Start" },
    { value: "shutdown", label: "Shut down" },
    { value: "reboot", label: "Reboot" },
  ];
  const plain = (id: string, label: string, command: string): SshCommand => ({
    ...base,
    id,
    label,
    build: ({ probe }) => `${sudo(probe)}${command}`,
  });
  return [
    {
      ...base,
      id: "pve-version",
      label: "Versions",
      build: () => "pveversion -v",
    },
    plain("pve-vms", "Virtual machines", "qm list"),
    plain("pve-cts", "Containers", "pct list"),
    plain("pve-storage", "Storage", "pvesm status"),
    plain("pve-cluster", "Cluster status", "pvecm status"),
    plain("pve-ha", "HA status", "ha-manager status"),
    {
      ...base,
      id: "pve-vm-config",
      label: "VM config",
      param: "vmid",
      build: (c) => `${sudo(c.probe)}qm config ${value(c)}`,
    },
    {
      ...base,
      id: "pve-ct-config",
      label: "Container config",
      param: "vmid",
      build: (c) => `${sudo(c.probe)}pct config ${value(c)}`,
    },
    {
      ...base,
      id: "pve-ct-enter",
      label: "Enter container",
      param: "vmid",
      build: (c) => `${sudo(c.probe)}pct enter ${value(c)}`,
    },
    {
      ...base,
      id: "pve-vm-control",
      label: "Control VM",
      danger: true,
      param: "vmid",
      actions: power,
      build: (c) => `${sudo(c.probe)}qm ${c.action ?? "start"} ${value(c)}`,
    },
    {
      ...base,
      id: "pve-ct-control",
      label: "Control container",
      danger: true,
      param: "vmid",
      actions: power,
      build: (c) => `${sudo(c.probe)}pct ${c.action ?? "start"} ${value(c)}`,
    },
    {
      ...base,
      id: "pve-backup",
      label: "Back up now",
      danger: true,
      param: "vmid",
      build: (c) => `${sudo(c.probe)}vzdump ${value(c)} --mode snapshot`,
    },
  ];
}

export const CATEGORIES = [...new Set(COMMANDS.map((c) => c.category))];

export type Catalog = {
  commands: SshCommand[];
  categories: string[];
  description: string;
};
export const SSH_CATALOG: Catalog = {
  commands: COMMANDS,
  categories: CATEGORIES,
  description:
    "Commands for the SSH tools panel, used on every SSH host. Custom commands are typed exactly as entered; add sudo yourself where needed. Drag the grip to reorder.",
};

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

/** The built-in categories and commands in their default order. */
export function defaultLayout(catalog = SSH_CATALOG): CommandCategory[] {
  return catalog.categories.map((name) => ({
    name,
    items: catalog.commands
      .filter((c) => c.category === name)
      .map((c) => ({
        kind: "builtin" as const,
        id: c.id,
      })),
  }));
}

/**
 * The saved layout made complete: unknown or repeated built-in references are
 * dropped, and built-in commands added by a later version join their default
 * category (created at the end if the layout no longer has it).
 */
export function resolveLayout(
  saved?: CommandCategory[],
  catalog = SSH_CATALOG,
): CommandCategory[] {
  if (!saved?.length) return defaultLayout(catalog);
  const known = new Set(catalog.commands.map((c) => c.id));
  const seen = new Set<string>();
  const layout = saved.map((category) => ({
    ...category,
    items: category.items.filter(
      (item) =>
        item.kind === "custom" ||
        (known.has(item.id) && !seen.has(item.id) && !!seen.add(item.id)),
    ),
  }));
  for (const command of catalog.commands) {
    if (seen.has(command.id)) continue;
    let category = layout.find((c) => c.name === command.category);
    if (!category) {
      category = { name: command.category, items: [] };
      layout.push(category);
    }
    category.items.push({ kind: "builtin", id: command.id });
  }
  return layout;
}

/** Saved form: empty while the layout equals the built-in default. */
export function layoutToSave(
  layout: CommandCategory[],
  catalog = SSH_CATALOG,
): CommandCategory[] {
  const clean = layout.map((c) => ({
    name: c.name.trim(),
    ...(c.hidden ? { hidden: true } : {}),
    items: c.items.map((i) =>
      i.kind === "custom"
        ? { ...i, label: i.label.trim(), command: i.command.trim() }
        : i,
    ),
  }));
  return JSON.stringify(clean) === JSON.stringify(defaultLayout(catalog))
    ? []
    : clean;
}

export const isBuiltinCategory = (name: string, catalog = SSH_CATALOG) =>
  catalog.categories.includes(name);

/** Moves an entry so it ends up before position `to` of the original list. */
export function moveTo<T>(list: T[], from: number, to: number) {
  const next = [...list];
  const [entry] = next.splice(from, 1);
  next.splice(to > from ? to - 1 : to, 0, entry);
  return next;
}

/**
 * Moves a command before position `to.index` of category `to.category`.
 * Built-in commands stay in their category; custom ones may change category.
 */
export function moveCommand(
  layout: CommandCategory[],
  from: { category: number; index: number },
  to: { category: number; index: number },
) {
  const item = layout[from.category]?.items[from.index];
  if (!item || !layout[to.category]) return layout;
  if (from.category === to.category)
    return layout.map((c, i) =>
      i === to.category
        ? { ...c, items: moveTo(c.items, from.index, to.index) }
        : c,
    );
  if (item.kind === "builtin") return layout;
  return layout.map((c, i) => {
    if (i === from.category)
      return { ...c, items: c.items.filter((_, j) => j !== from.index) };
    if (i !== to.category) return c;
    const items = [...c.items];
    items.splice(to.index, 0, item);
    return { ...c, items };
  });
}

export function builtinLabel(id: string, catalog = SSH_CATALOG) {
  return catalog.commands.find((c) => c.id === id)?.label ?? id;
}

const utf8 = (s: string) => new TextEncoder().encode(s).length;
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/;

/** Problems in an edited layout; the backend applies the same limits. */
export function layoutProblems(layout: CommandCategory[]): string[] {
  const problems: string[] = [];
  const names = new Set<string>();
  let custom = 0;
  if (layout.length > 64) problems.push("At most 64 categories.");
  layout.forEach((category, c) => {
    const name = category.name.trim();
    const where = name || `Category ${c + 1}`;
    if (!name || utf8(name) > 48 || CONTROL.test(name))
      problems.push(`${where}: the name needs 1–48 bytes.`);
    else if (names.has(name.toLowerCase()))
      problems.push(`${where}: another category has this name.`);
    names.add(name.toLowerCase());
    if (category.items.length > 200)
      problems.push(`${where}: at most 200 commands.`);
    category.items.forEach((item, i) => {
      if (item.kind !== "custom") return;
      custom++;
      const label = item.label.trim();
      const command = item.command.trim();
      const at = `${where}, ${label || `command ${i + 1}`}`;
      if (!label || utf8(label) > 64 || CONTROL.test(label))
        problems.push(`${at}: the name needs 1–64 bytes.`);
      if (!command || utf8(command) > 4096 || CONTROL.test(command))
        problems.push(
          `${at}: enter one line (up to 4096 bytes) without control characters.`,
        );
    });
  });
  if (custom > 500) problems.push("At most 500 custom commands.");
  return problems;
}

function customCommand(
  category: string,
  item: Extract<CommandItem, { kind: "custom" }>,
  index: number,
): SshCommand {
  return {
    id: `custom-${index}`,
    category,
    label: item.label,
    custom: true,
    danger: item.mode === "confirm",
    insert: item.mode === "insert",
    build: () => item.command,
  };
}

/** Visible categories with their commands, in the user's order. */
export function panelCommands(
  saved?: CommandCategory[],
  catalog = SSH_CATALOG,
) {
  const byId = new Map(catalog.commands.map((c) => [c.id, c]));
  let index = 0;
  return resolveLayout(saved, catalog)
    .filter((c) => !c.hidden)
    .map((category) => ({
      name: category.name,
      commands: category.items.map((item) =>
        item.kind === "custom"
          ? customCommand(category.name, item, index++)
          : { ...byId.get(item.id)!, category: category.name },
      ),
    }));
}

/** Platform commands stay out of the list unless the probe found the platform. */
export function shown(command: SshCommand, probe?: Probe) {
  return !command.platform || !!probe?.tools.includes(command.platform);
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
