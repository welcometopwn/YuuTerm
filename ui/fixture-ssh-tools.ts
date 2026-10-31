import { invoke } from "@tauri-apps/api/core";
// SSH tools panel against the explicitly selected Linux server: capability
// probe, typed commands (exact bytes via the capture input trace), confirmation
// of state-changing commands, and the du-based folder-size view.
export async function checkSshTools(ids: number[], checks: string[]) {
  const id = ids[0];
  const path = await invoke<string>("capture_external_path");
  const sizes = path + "/sizes";
  // Progress is written after every check so a stalled run shows its step.
  // It must say passed: a failed report closes the window. The runner still
  // requires a clean exit and the final screenshot before it accepts a pass.
  const note = (label: string) => {
    checks.push(label);
    void invoke("capture_report", { checks, passed: true }).catch(() => {});
  };
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const wait = async (f: () => boolean | Promise<boolean>, label: string) => {
    const end = Date.now() + 45000;
    while (Date.now() < end) {
      if (await f()) {
        note(label);
        return;
      }
      await sleep(100);
    }
    throw Error(label);
  };
  const button = (label: string) =>
    Array.from(document.querySelectorAll<HTMLButtonElement>("button")).find(
      (b) => b.textContent?.trim() === label,
    );
  const command = (key: string) =>
    document.querySelector<HTMLButtonElement>(`[data-command="${key}"]`);
  const row = (name: string) =>
    document.querySelector<HTMLButtonElement>(
      `[aria-label="Actions for ${name}"]`,
    );
  const rows = () => document.querySelector(".xterm-rows")?.textContent ?? "";
  const trace = async () =>
    (await invoke<number[][]>("capture_trace", { id })).map((b) =>
      new TextDecoder().decode(new Uint8Array(b)),
    );
  const setValue = (input: HTMLInputElement, value: string) => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  };
  const send = (text: string) =>
    invoke("session_input", {
      id,
      bytes: Array.from(new TextEncoder().encode(text + "\r")),
    });
  await send("unset HISTFILE; PS1='YuuTerm test$ '; clear");
  // Known sizes inside the generated run directory only.
  await send(
    `P='${sizes}'; mkdir -p "$P/big" "$P/small" && head -c 3145728 /dev/urandom > "$P/big/blob" && head -c 102400 /dev/urandom > "$P/small/blob" && printf x > "$P/note ' $(printf '\\316\\251').txt" && printf 'YT_%s\\n' SIZES_READY`,
  );
  await wait(
    () => rows().includes("YT_SIZES_READY"),
    "Test folders with known sizes exist on the server",
  );

  // Capability probe on its own exec channel.
  const probe = await invoke<{
    os: string;
    uid: number | null;
    tools: string[];
  }>("ssh_tools_probe", { id });
  if (probe.os !== "Linux" || !probe.tools.includes("systemctl"))
    throw Error("Probe did not detect a systemd Linux host");
  note(
    `Probe reports Linux, uid ${probe.uid === 0 ? "0" : "non-root"} and ${probe.tools.length} tools without touching the terminal`,
  );
  document
    .querySelector<HTMLButtonElement>('[aria-label="Remote tools"]')!
    .click();
  await wait(
    () => !!command("memory") && !command("memory")!.disabled,
    "Tools button opens the SSH tools panel with commands enabled after detection",
  );
  // Compact buttons: two per row at the default width, more when wider.
  const columns = () =>
    new Set(
      Array.from(
        document.querySelectorAll<HTMLElement>(
          ".ssh-command-list section:first-child .ssh-command",
        ),
      ).map((b) => Math.round(b.getBoundingClientRect().left)),
    ).size;
  const panel = () =>
    document.querySelector<HTMLElement>(".ssh-tools")!.getBoundingClientRect()
      .width;
  if (columns() !== 2 || command("memory")!.getBoundingClientRect().height > 26)
    throw Error(
      `Default layout: ${columns()} columns, ${command("memory")!.getBoundingClientRect().height}px buttons`,
    );
  note(
    `Default ${Math.round(panel())} px panel shows two compact command columns`,
  );
  const clipped = Array.from(
    document.querySelectorAll<HTMLElement>(".ssh-command > span:first-child"),
  )
    .filter((s) => s.scrollWidth > s.clientWidth)
    .map((s) => s.textContent);
  if (clipped.length) throw Error(`Clipped labels: ${clipped.join(", ")}`);
  note("No command label is cut off at the default width");
  const resizer = document.querySelector<HTMLElement>(
    '[aria-label="Resize remote tools"]',
  )!;
  // One key press per render, as a user's repeated presses arrive.
  for (let i = 0; i < 5; i++) {
    resizer.dispatchEvent(
      new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }),
    );
    await sleep(50);
  }
  await wait(
    () => Math.round(panel()) === 440 && columns() === 3,
    "Widening the panel by 100 px reflows the commands into three columns",
  );
  if (
    JSON.parse(localStorage.getItem("yuuterm.workspace-layout.v1")!)
      .toolsWidth !== 440
  )
    throw Error("Tools panel width was not saved");
  note("The panel width is saved in the workspace layout");
  resizer.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  await wait(
    () => Math.round(panel()) === 340 && columns() === 2,
    "Double-click resets the panel width",
  );
  if (!probe.tools.includes("docker")) {
    const docker = command("docker-ps")!;
    if (!docker.disabled || !docker.title.includes("Not installed"))
      throw Error("Missing Docker is not explained");
    note("Commands for tools missing on the host are disabled with a reason");
  }

  // A plain command is typed into this session only, followed by Enter.
  const before = (await trace()).length;
  command("memory")!.click();
  await wait(
    () => rows().includes("Mem:"),
    "Memory runs free -h in the visible Linux terminal",
  );
  const typed = (await trace()).slice(before);
  if (typed.length !== 1 || typed[0] !== "free -h\r")
    throw Error(`Unexpected typed input: ${JSON.stringify(typed)}`);
  note("Exactly the command text plus Enter reached the session");

  // State-changing commands stop at a confirmation that shows the command.
  const sudo = probe.uid === 0 ? "" : "sudo ";
  command("reboot")!.click();
  await wait(
    () =>
      document.querySelector('[role="alertdialog"] .ssh-preview')
        ?.textContent === `${sudo}reboot`,
    "Restart asks for confirmation and shows the exact command",
  );
  button("Cancel")!.click();
  await sleep(300);
  if ((await trace()).length !== before + 1)
    throw Error("Cancelling a confirmation still typed input");
  note("Cancelling the confirmation types nothing");

  // Root shell: sudo -i or su -, then commands are built as for root.
  const rootButton =
    document.querySelector<HTMLButtonElement>(".ssh-root-button")!;
  if (probe.uid === 0) {
    if (!rootButton.disabled || !rootButton.title.includes("already is root"))
      throw Error("Root shell offered to root");
    note("Root shell is disabled for an account that already is root");
  } else {
    const login = probe.tools.includes("sudo") ? "sudo -i" : "su -";
    if (rootButton.textContent !== `Root shell (${login})`)
      throw Error(`Unexpected root button: ${rootButton.textContent}`);
    const typedBefore = (await trace()).length;
    rootButton.click();
    await wait(
      () => /Password:|password for/.test(rows()),
      `Root shell types ${login} and the terminal asks for the password`,
    );
    const typedRoot = (await trace()).slice(typedBefore);
    if (typedRoot.length !== 1 || typedRoot[0] !== `${login}\r`)
      throw Error(`Unexpected root input: ${JSON.stringify(typedRoot)}`);
    command("reboot")!.click();
    await wait(
      () =>
        !!document.querySelector('[data-ssh-root="open"]') &&
        document.querySelector('[role="alertdialog"] .ssh-preview')
          ?.textContent === "reboot",
      "Inside the root shell commands leave out sudo",
    );
    button("Cancel")!.click();
    // Abort the password prompt; no password is ever entered by the test.
    await invoke("session_input", { id, bytes: [3] });
    button("Reset")!.click();
    command("reboot")!.click();
    await wait(
      () =>
        document.querySelector('[role="alertdialog"] .ssh-preview')
          ?.textContent === "sudo reboot",
      "Reset returns to sudo for the normal account",
    );
    button("Cancel")!.click();
  }

  // Parameters are validated before the command can run.
  command("service-status")!.click();
  const unit = await (async () => {
    await wait(
      () => !!document.querySelector('[aria-label="Service (systemd unit)"]'),
      "Service status asks for a unit name",
    );
    return document.querySelector<HTMLInputElement>(
      '[aria-label="Service (systemd unit)"]',
    )!;
  })();
  setValue(unit, "ssh; reboot");
  await wait(
    () => button("Run")?.disabled === true,
    "An unsafe unit name keeps Run disabled",
  );
  setValue(unit, "ssh");
  await wait(
    () =>
      document.querySelector(".ssh-preview")?.textContent ===
        "systemctl status --no-pager ssh" && button("Run")?.disabled === false,
    "A valid unit shows the command preview",
  );
  button("Run")!.click();
  await wait(
    () => rows().includes("Loaded:"),
    "Service status runs against the real systemd",
  );

  // A confirmed state-changing command really runs: end a process the test
  // started itself.
  await send("sleep 600 & printf 'YT_PID_%s\\n' $!");
  let pid = "";
  await wait(() => {
    pid = /YT_PID_(\d+)/.exec(rows())?.[1] ?? "";
    return !!pid;
  }, "A disposable process is running on the server");
  command("kill")!.click();
  await wait(
    () => !!document.querySelector('[aria-label="Process ID"]'),
    "End process asks for a process ID",
  );
  setValue(
    document.querySelector<HTMLInputElement>('[aria-label="Process ID"]')!,
    pid,
  );
  await wait(
    () =>
      document.querySelector('[role="alertdialog"] .ssh-preview')
        ?.textContent === `kill ${pid}`,
    "End process confirmation shows kill with the entered PID",
  );
  button("Run")!.click();
  await sleep(500);
  await send(`kill -0 ${pid} 2>/dev/null || printf 'YT_%s\\n' GONE`);
  await wait(
    () => rows().includes("YT_GONE"),
    "Confirmed End process terminated the process",
  );
  if (probe.uid === 0) {
    // The runner created a disposable oneshot unit that logs every start.
    const starts = async (expected: number) => {
      await send(
        "clear; printf 'YT_STARTS_%s\\n' $(wc -l < /run/yt-tools-probe.log)",
      );
      await wait(
        () => rows().includes(`YT_STARTS_${expected}`),
        `Disposable unit has started ${expected} time${expected > 1 ? "s" : ""}`,
      );
    };
    await starts(1);
    command("service-control")!.click();
    await wait(
      () => !!document.querySelector('[aria-label="Service (systemd unit)"]'),
      "Service control asks for a unit name",
    );
    setValue(
      document.querySelector<HTMLInputElement>(
        '[aria-label="Service (systemd unit)"]',
      )!,
      "yt-tools-probe",
    );
    await wait(
      () =>
        document.querySelector('[role="alertdialog"] .ssh-preview')
          ?.textContent === "systemctl restart yt-tools-probe",
      "As root the restart confirmation shows systemctl without sudo",
    );
    button("Run")!.click();
    await sleep(500);
    await starts(2);
    note("Confirmed service restart ran on the real systemd");
  }

  // Search narrows the catalog.
  const search = document.querySelector<HTMLInputElement>(
    '[aria-label="Search commands"]',
  )!;
  setValue(search, "docker");
  await wait(
    () =>
      !command("memory") && !!command("docker-ps") && !!command("docker-prune"),
    "Search narrows the command list",
  );
  setValue(search, "");

  // Folder sizes through the backend: sorting, totals, errors. du reports
  // allocated blocks; ZFS (the test container) allocates new data only at its
  // next transaction commit, so wait until the written sizes are visible.
  type Listing = {
    total_kib: number | null;
    entries: { name: string; kib: number; directory: boolean }[];
    filesystem: { total_kib: number } | null;
    incomplete: boolean;
  };
  let listing: Listing | undefined;
  for (let end = Date.now() + 30000; Date.now() < end; await sleep(1000)) {
    listing = await invoke<Listing>("ssh_folder_sizes", { id, path: sizes });
    if ((listing.entries[0]?.kib ?? 0) >= 3000) break;
  }
  const [first, second] = listing!.entries;
  if (
    first?.name !== "big" ||
    !first.directory ||
    first.kib < 3000 ||
    second?.name !== "small" ||
    second.kib < 90 ||
    !listing!.entries.some((e) => e.name === "note ' Ω.txt" && !e.directory) ||
    (listing!.total_kib ?? 0) < first.kib + second.kib ||
    !listing!.filesystem ||
    listing!.incomplete
  )
    throw Error(`Unexpected folder sizes: ${JSON.stringify(listing)}`);
  note(
    "du listing on Linux sorts folders by size, marks directories and keeps an apostrophe/Unicode name",
  );
  if (probe.uid !== 0) {
    const etc = await invoke<{ incomplete: boolean }>("ssh_folder_sizes", {
      id,
      path: "/etc",
    });
    if (!etc.incomplete) throw Error("/etc not reported as incomplete");
    note("Unreadable entries mark the listing as incomplete");
  }
  for (const [bad, expected] of [
    [path + "/missing", "not found"],
    ["relative", "Invalid folder path"],
  ]) {
    const problem = await invoke("ssh_folder_sizes", { id, path: bad }).then(
      () => "",
      String,
    );
    if (!problem.includes(expected))
      throw Error(`Unexpected error for ${bad}: ${problem}`);
  }
  note("Missing folders and relative paths are refused with a reason");
  // A newer request cancels the running one on the server.
  let superseded = "";
  for (
    let attempt = 0;
    attempt < 3 && !superseded.includes("Cancelled");
    attempt++
  ) {
    const slow = invoke("ssh_folder_sizes", { id, path: "/usr" }).then(
      () => "finished",
      String,
    );
    await sleep(30);
    const fast = invoke("ssh_folder_sizes", { id, path: sizes });
    superseded = await slow;
    await fast;
  }
  if (!superseded.includes("Cancelled"))
    throw Error(`Superseded listing was not cancelled: ${superseded}`);
  note("A newer folder-size request cancels the running one");
  let cancelled = "";
  for (
    let attempt = 0;
    attempt < 3 && !cancelled.includes("Cancelled");
    attempt++
  ) {
    const slow = invoke("ssh_folder_sizes", { id, path: "/usr" }).then(
      () => "finished",
      String,
    );
    await sleep(30);
    await invoke("ssh_folder_sizes_cancel", { id });
    cancelled = await slow;
  }
  if (!cancelled.includes("Cancelled"))
    throw Error(`Explicit cancel did not stop the listing: ${cancelled}`);
  note("Cancel stops a running folder-size listing");

  // Files menu opens the storage view for a folder.
  const input = document.querySelector<HTMLInputElement>(
    '[aria-label="Remote path"]',
  )!;
  setValue(input, path);
  await sleep(0);
  input.form!.requestSubmit();
  await wait(() => !!row("sizes"), "Files browses the test run");
  row("sizes")!.click();
  await wait(
    () => !!button("Show folder sizes"),
    "Folder context menu offers folder sizes",
  );
  button("Show folder sizes")!.click();
  const view = () =>
    document.querySelector<HTMLElement>(".ssh-storage")?.dataset.storagePath;
  const names = () =>
    Array.from(document.querySelectorAll(".ssh-sizes .ssh-size-name")).map(
      (n) => n.textContent,
    );
  await wait(
    () => view() === sizes && names()[0] === "big" && names()[1] === "small",
    "Show folder sizes opens the storage view, largest first",
  );
  Array.from(document.querySelectorAll<HTMLButtonElement>(".ssh-sizes button"))
    .find((b) => b.textContent?.includes("big"))!
    .click();
  await wait(
    () => view() === sizes + "/big" && names()[0] === "blob",
    "Clicking a folder drills down",
  );
  button("Up")!.click();
  await wait(
    () => view() === sizes && names()[0] === "big",
    "Up returns to the parent folder",
  );
  button("Open in Files")!.click();
  await wait(
    () => input.value === sizes && !!row("big"),
    "Open in Files browses the same folder",
  );

  await send("clear; printf 'SSH tools %s\\n' qualification");
  await wait(
    () =>
      rows().includes("SSH tools qualification") &&
      !rows().includes("yuuterm-test"),
    "Native capture terminal is clear of test account paths",
  );
  // Endpoint/account labels only are redacted for retained evidence, after all
  // protocol assertions.
  const profile = (
    await invoke<{ prefs: { profiles: { host: string; username: string }[] } }>(
      "bootstrap",
    )
  ).prefs.profiles[0];
  const run = path.split("/").pop()!;
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  while (walker.nextNode()) {
    const node = walker.currentNode;
    node.textContent = node
      .textContent!.replaceAll(profile.host, "<test-host>")
      .replaceAll(profile.username, "<test-account>")
      .replaceAll(run, "<isolated-run>");
  }
  input.value = "/home/<test-account>/yuuterm-test/<isolated-run>/sizes";
  // Storage still first; the final capture shows the compact command grid.
  await invoke("capture_still", { name: "ssh-tools-storage" });
  await sleep(500);
  button("Commands")!.click();
  await wait(
    () => !!command("memory"),
    "Commands tab is shown for the final capture",
  );
}
