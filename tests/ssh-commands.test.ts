import { test } from "node:test";
import assert from "node:assert/strict";
import {
  COMMANDS,
  asRoot,
  rootShell,
  shown,
  defaultLayout,
  resolveLayout,
  layoutToSave,
  layoutProblems,
  panelCommands,
  isBuiltinCategory,
  moveCommand,
  moveTo,
  childPath,
  commandText,
  formatKib,
  matches,
  parentPath,
  quote,
  unavailable,
  validate,
  type Probe,
} from "../ui/ssh-commands.ts";

const command = (id: string) => COMMANDS.find((c) => c.id === id)!;
const probe = (tools: string[], uid = 1000, groups: string[] = []): Probe => ({
  os: "Linux",
  shell: "bash",
  uid,
  tools,
  groups,
});

test("every state-changing command is marked for confirmation", () => {
  const risky = COMMANDS.filter((c) => c.danger).map((c) => c.id);
  for (const id of [
    "journal-vacuum",
    "kill",
    "service-control",
    "docker-restart",
    "docker-prune",
    "upgrade",
    "full-upgrade",
    "autoremove",
    "reboot",
    "poweroff",
    "pve-vm-control",
    "pve-ct-control",
    "pve-backup",
  ])
    assert.ok(risky.includes(id), id);
  assert.equal(new Set(COMMANDS.map((c) => c.id)).size, COMMANDS.length);
});

test("parameters are validated strictly before anything is typed", () => {
  assert.equal(validate("pid", "1234"), null);
  for (const bad of ["0", "-1", "12a", "1; reboot", ""])
    assert.notEqual(validate("pid", bad), null, bad);
  assert.equal(validate("unit", "nginx.service"), null);
  assert.equal(validate("unit", "getty@tty1"), null);
  for (const bad of ["-x", "a b", "a;b", "$(id)", "a`b`"])
    assert.notEqual(validate("unit", bad), null, bad);
  assert.equal(validate("container", "web_1.prod-2"), null);
  assert.notEqual(validate("container", "-rm"), null);
  assert.equal(validate("host", "192.0.2.10"), null);
  assert.equal(validate("host", "::1"), null);
  assert.equal(validate("host", "nas.home.lan"), null);
  for (const bad of ["-f", "a b", "a&b"])
    assert.notEqual(validate("host", bad), null, bad);
  assert.equal(validate("path", "/srv/it's here"), null);
  for (const bad of ["relative", "/a\nb", "/a\\b", "/a\u0007"])
    assert.notEqual(validate("path", bad), null, JSON.stringify(bad));
  assert.throws(() =>
    commandText(command("kill"), { value: "1; reboot", action: "kill" }),
  );
});

test("paths are single-quoted the same way for POSIX shells and fish", () => {
  assert.equal(quote("/a b"), "'/a b'");
  assert.equal(quote("/it's"), "'/it'\\''s'");
  assert.match(
    commandText(command("largest-files"), {
      probe: probe(["numfmt"]),
      value: "/var/log",
    }),
    /^find '\/var\/log' -xdev .* \| numfmt --to=iec --field=1$/,
  );
  assert.doesNotMatch(
    commandText(command("largest-files"), { probe: probe([]), value: "/" }),
    /numfmt/,
  );
});

test("commands follow the detected host", () => {
  assert.equal(
    commandText(command("htop"), { probe: probe(["htop"]) }),
    "htop",
  );
  assert.equal(commandText(command("htop"), { probe: probe([]) }), "top");
  assert.equal(
    commandText(command("reboot"), { probe: probe([]) }),
    "sudo reboot",
  );
  assert.equal(
    commandText(command("reboot"), { probe: probe([], 0) }),
    "reboot",
  );
  assert.equal(
    commandText(command("upgrade"), { probe: probe(["apt"]) }),
    "sudo apt update && sudo apt upgrade -y",
  );
  assert.equal(
    commandText(command("upgrade"), { probe: probe(["dnf"], 0) }),
    "dnf upgrade -y",
  );
  assert.equal(
    commandText(command("full-upgrade"), { probe: probe(["apt"]) }),
    "sudo apt update && sudo apt full-upgrade -y",
  );
  assert.equal(
    commandText(command("autoremove"), { probe: probe(["apt"]) }),
    "sudo apt autoremove -y",
  );
  assert.equal(
    commandText(command("autoremove"), { probe: probe(["dnf"], 0) }),
    "dnf autoremove -y",
  );
  assert.equal(
    commandText(command("full-upgrade"), { probe: probe(["apt"], 0) }),
    "apt update && apt full-upgrade -y",
  );
  assert.equal(
    commandText(command("service-control"), {
      probe: probe(["systemctl"]),
      value: "nginx",
      action: "stop",
    }),
    "sudo systemctl stop nginx",
  );
  // An unknown action falls back to the first one, never to free text.
  assert.equal(
    commandText(command("kill"), { value: "42", action: "rm -rf" }),
    "kill 42",
  );
  assert.equal(
    commandText(command("kill"), { value: "42", action: "kill" }),
    "kill -9 42",
  );
  assert.equal(
    commandText(command("firewall"), { probe: probe(["nft"]) }),
    "sudo nft list ruleset",
  );
});

test("journal and docker use sudo only without the granting group", () => {
  const text = (id: string, p?: Probe) =>
    commandText(command(id), { probe: p, value: "web" });
  assert.equal(text("journal-errors", probe([])), "sudo journalctl -p err -b");
  assert.equal(
    text("journal-errors", probe([], 1000, ["adm"])),
    "journalctl -p err -b",
  );
  assert.equal(
    text("journal-errors", probe([], 1000, ["wheel"])),
    "journalctl -p err -b",
  );
  assert.equal(text("journal-errors", probe([], 0)), "journalctl -p err -b");
  assert.match(
    text("ssh-failures", probe([])),
    /^sudo journalctl .* [|] grep -Ei/,
  );
  assert.equal(
    text("docker-logs", probe([])),
    "sudo docker logs -f --tail 200 web",
  );
  assert.equal(
    text("docker-logs", probe([], 1000, ["docker"])),
    "docker logs -f --tail 200 web",
  );
  // Without detection nothing is elevated.
  assert.equal(text("docker-ps"), "docker ps -a");
  assert.equal(text("journal-follow"), "journalctl -f");
});

test("root shell uses sudo -i where possible, su - otherwise, nothing as root", () => {
  assert.equal(rootShell(probe(["sudo"])), "sudo -i");
  assert.equal(rootShell(probe([])), "su -");
  assert.equal(rootShell(probe(["sudo"], 0)), null);
  assert.equal(rootShell(), "sudo -i");
  // Inside the root shell commands drop sudo and group-based elevation.
  const root = asRoot(probe(["sudo", "docker"]));
  assert.equal(commandText(command("reboot"), { probe: root }), "reboot");
  assert.equal(
    commandText(command("docker-ps"), { probe: root }),
    "docker ps -a",
  );
  assert.equal(
    commandText(command("journal-errors"), { probe: root }),
    "journalctl -p err -b",
  );
  assert.equal(asRoot(), undefined);
});

test("missing tools disable commands; failed detection keeps them enabled", () => {
  assert.match(unavailable(command("docker-ps"), probe([]))!, /docker/);
  assert.equal(unavailable(command("docker-ps"), probe(["docker"])), null);
  assert.equal(unavailable(command("docker-ps")), null);
  assert.match(unavailable(command("updates"), probe([]))!, /apt or dnf/);
  assert.match(unavailable(command("full-upgrade"), probe(["dnf"]))!, /apt/);
  assert.equal(unavailable(command("reboot"), probe([])), null);
});

test("search, sizes and path helpers", () => {
  assert.ok(matches(command("docker-ps"), "docker"));
  assert.ok(matches(command("ports"), "ss -tulpn"));
  assert.ok(!matches(command("ports"), "zzz"));
  assert.equal(formatKib(512), "512 KiB");
  assert.equal(formatKib(1536), "1.5 MiB");
  assert.equal(formatKib(300 * 1024 * 1024), "300 GiB");
  assert.equal(childPath("/", "var"), "/var");
  assert.equal(childPath("/var", "log"), "/var/log");
  assert.equal(parentPath("/var/log"), "/var");
  assert.equal(parentPath("/var"), "/");
  assert.equal(parentPath("/"), "/");
});

test("Proxmox commands appear only on a detected Proxmox host", () => {
  const pve = COMMANDS.filter((c) => c.category === "Proxmox");
  assert.ok(pve.length > 0);
  for (const c of pve) {
    assert.equal(shown(c), false, c.id);
    assert.equal(shown(c, probe(["apt", "qm"])), false, c.id);
    assert.equal(shown(c, probe(["pveversion"])), true, c.id);
  }
  assert.ok(shown(command("memory")));
  assert.equal(validate("vmid", "100"), null);
  assert.equal(validate("vmid", "999999999"), null);
  for (const bad of ["99", "0100", "1e3", "100 --purge", "-100", ""])
    assert.notEqual(validate("vmid", bad), null, bad);
  const pveProbe = probe(["pveversion"]);
  assert.equal(
    commandText(command("pve-vm-control"), {
      probe: pveProbe,
      value: "101",
      action: "shutdown",
    }),
    "sudo qm shutdown 101",
  );
  assert.equal(
    commandText(command("pve-ct-control"), {
      probe: probe(["pveversion"], 0),
      value: "200",
      action: "rm",
    }),
    "pct start 200",
  );
  assert.equal(
    commandText(command("pve-backup"), { probe: pveProbe, value: "101" }),
    "sudo vzdump 101 --mode snapshot",
  );
  assert.equal(
    commandText(command("pve-version"), { probe: pveProbe }),
    "pveversion -v",
  );
});

test("saved layouts keep their order and still gain new built-in commands", () => {
  const defaults = defaultLayout();
  assert.deepEqual(resolveLayout(), defaults);
  assert.deepEqual(resolveLayout([]), defaults);
  assert.deepEqual(layoutToSave(defaults), []);

  const resolved = resolveLayout([
    {
      name: "Deploy",
      items: [
        { kind: "custom", label: "Pull", command: "git pull", mode: "run" },
      ],
    },
    {
      name: "Docker",
      hidden: true,
      items: [
        {
          kind: "custom",
          label: "Up",
          command: "docker compose up -d",
          mode: "confirm",
        },
        { kind: "builtin", id: "docker-logs" },
        { kind: "builtin", id: "docker-logs" },
        { kind: "builtin", id: "removed-in-this-version" },
      ],
    },
  ]);
  assert.deepEqual(resolved.map((c) => c.name).slice(0, 2), [
    "Deploy",
    "Docker",
  ]);
  const docker = resolved[1].items.map((i) =>
    i.kind === "builtin" ? i.id : i.label,
  );
  assert.deepEqual(docker.slice(0, 2), ["Up", "docker-logs"]);
  assert.equal(docker.filter((id) => id === "docker-logs").length, 1);
  assert.ok(!docker.includes("removed-in-this-version"));
  assert.ok(
    docker.includes("docker-ps"),
    "missing built-ins rejoin their category",
  );
  const ids = resolved.flatMap((c) =>
    c.items.flatMap((i) => (i.kind === "builtin" ? [i.id] : [])),
  );
  assert.equal(new Set(ids).size, COMMANDS.length);

  const panel = panelCommands([...resolved]);
  assert.ok(!panel.some((c) => c.name === "Docker"), "hidden category");
  const pull = panel[0].commands[0];
  assert.equal(commandText(pull, {}), "git pull");
  assert.ok(pull.custom && !pull.danger && !pull.insert);
  const saved = layoutToSave(resolved);
  assert.equal(saved[1].hidden, true);
  assert.equal(layoutToSave(resolveLayout(saved)).length, saved.length);
});

test("custom command modes and layout problems", () => {
  const layout = (
    command: string,
    mode: "run" | "confirm" | "insert" = "run",
  ) => [
    {
      name: "Mine",
      items: [{ kind: "custom" as const, label: "Go", command, mode }],
    },
  ];
  const first = (mode: "run" | "confirm" | "insert") =>
    panelCommands(layout("ls -la", mode))[0].commands[0];
  assert.ok(first("confirm").danger);
  assert.ok(first("insert").insert && !first("insert").danger);
  assert.deepEqual(layoutProblems(layout("uptime")), []);
  for (const bad of [
    "",
    "   ",
    "uptime\nreboot",
    "a\rb",
    "printf '\u001b'",
    "x\u0085",
  ])
    assert.equal(layoutProblems(layout(bad)).length, 1, JSON.stringify(bad));
  assert.equal(layoutProblems(layout("x".repeat(4097))).length, 1);
  assert.equal(
    layoutProblems([...layout("a"), { name: "mine ", items: [] }]).length,
    1,
  );
  assert.equal(layoutProblems([{ name: " ", items: [] }]).length, 1);
  assert.ok(isBuiltinCategory("Docker") && !isBuiltinCategory("Mine"));
});

test("drag and drop keeps built-in commands in their category", () => {
  assert.deepEqual(moveTo(["a", "b", "c"], 0, 2), ["b", "a", "c"]);
  assert.deepEqual(moveTo(["a", "b", "c"], 0, 3), ["b", "c", "a"]);
  assert.deepEqual(moveTo(["a", "b", "c"], 2, 0), ["c", "a", "b"]);
  assert.deepEqual(moveTo(["a", "b", "c"], 1, 2), ["a", "b", "c"]);
  const layout = [
    {
      name: "Mine",
      items: [
        {
          kind: "custom" as const,
          label: "Go",
          command: "go",
          mode: "run" as const,
        },
      ],
    },
    {
      name: "Docker",
      items: [
        { kind: "builtin" as const, id: "docker-ps" },
        { kind: "builtin" as const, id: "docker-stats" },
      ],
    },
  ];
  const moved = moveCommand(
    layout,
    { category: 0, index: 0 },
    { category: 1, index: 1 },
  );
  assert.equal(moved[0].items.length, 0);
  assert.deepEqual(
    moved[1].items.map((i) => (i.kind === "builtin" ? i.id : i.label)),
    ["docker-ps", "Go", "docker-stats"],
  );
  assert.equal(
    moveCommand(layout, { category: 1, index: 0 }, { category: 0, index: 0 }),
    layout,
    "built-in commands cannot leave their category",
  );
  const within = moveCommand(
    layout,
    { category: 1, index: 0 },
    { category: 1, index: 2 },
  );
  assert.deepEqual(
    within[1].items.map((i) => (i.kind === "builtin" ? i.id : "")),
    ["docker-stats", "docker-ps"],
  );
});
