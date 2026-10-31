import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface Surface {
  id: number;
  version: string;
  connected: number;
  visible: boolean;
  parent_matches: boolean;
  focus_inside: boolean;
  top_at_center: boolean;
  bounds: { x: number; y: number; width: number; height: number };
  policy: { property: string; expected: number; actual: number }[];
  events: { kind: string; code: number | null }[];
  dropped_events: number;
  fixture: {
    accepted: boolean;
    negotiation_received: boolean;
    finished: boolean;
  } | null;
  key_sha256: string | null;
  key_pinned: boolean;
}
// Mounted only in the opt-in developer build. Real COM controls and an owned
// loopback disconnect fixture; the separate live mode connects one pinned host.
export function RdpProbe({ live = false }: { live?: boolean }) {
  const box = useRef<HTMLDivElement>(null);
  const [log, setLog] = useState<string[]>([]);
  const [message, setMessage] = useState(
    "Starting isolated composition probe…",
  );
  const started = useRef(false);
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    const checks: string[] = [];
    const surfaces: Surface[] = [];
    const paint = () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      );
    const check = (value: boolean, name: string) => {
      if (!value) throw new Error(name);
      checks.push(name);
      setLog([...checks]);
    };
    let id: number | undefined;
    const call = (action: string, bounds?: number[]) =>
      invoke<Surface | null>("rdp_probe_surface", {
        action,
        bounds: bounds ?? null,
        id: id ?? null,
      });
    const place = async () => {
      const r = box.current!.getBoundingClientRect(),
        scale = window.devicePixelRatio;
      const result = await call(
        "place",
        [r.x, r.y, r.width, r.height].map((value) => Math.round(value * scale)),
      );
      if (!result) throw new Error("Native surface missing");
      id = result.id;
      surfaces.push(result);
      check(
        result.connected === 0 && result.parent_matches && result.visible,
        "Real Microsoft RDP control is disconnected, visible and parented to YuuTerm",
      );
      check(
        result.bounds.width === Math.round(r.width * scale) &&
          result.bounds.height === Math.round(r.height * scale),
        "Native child size matches physical WebView pane dimensions",
      );
      return result;
    };
    // Live mode: one pinned, authenticated session against the host-configured
    // target. Reports carry only event kinds/codes and the public key hash.
    const runLive = async () => {
      let last: Surface | null = null;
      const ended = (s: Surface) =>
        s.events.find((e) =>
          [
            "disconnected",
            "fatal_error",
            "logon_error",
            "public_key_refused",
            "authentication_warning_cancelled",
          ].includes(e.kind),
        );
      try {
        const top = box.current!.getBoundingClientRect().top;
        box.current!.style.width = `${Math.min(1024, window.innerWidth - 80)}px`;
        box.current!.style.height = `${Math.max(240, Math.min(640, window.innerHeight - top - 24))}px`;
        await paint();
        await place();
        await call("live_connect");
        const deadline = Date.now() + 60000;
        for (;;) {
          last = await call("status");
          if (!last) throw new Error("Live surface vanished");
          setMessage(last.events.map((e) => e.kind).join(" → "));
          if (last.events.some((e) => e.kind === "login_complete")) break;
          const end = ended(last);
          if (end)
            throw new Error(
              `Live connection ended: ${end.kind}${end.code !== null ? ` ${end.code}` : ""}`,
            );
          if (Date.now() > deadline)
            throw new Error("Live connection did not complete within 60 s");
          await new Promise((r) => setTimeout(r, 500));
        }
        check(
          last.key_pinned && !!last.key_sha256,
          "Server public key matched the confirmed pin before any password was handed over",
        );
        check(
          last.connected === 1,
          "Embedded control reports an active session",
        );
        check(
          last.events.some((e) => e.kind === "login_complete"),
          "Windows logon completed inside the YuuTerm pane",
        );
        // Let the remote desktop paint before the owned-window capture.
        await new Promise((r) => setTimeout(r, 4000));
        last = await call("status");
        check(
          last?.connected === 1 && !ended(last),
          "Session stays connected after the first frames",
        );
        await invoke("rdp_probe_report", {
          report: {
            passed: true,
            checks,
            key_sha256: last?.key_sha256 ?? null,
            events: last?.events ?? [],
          },
        });
      } catch (error) {
        setMessage(String(error));
        await invoke("rdp_probe_report", {
          report: {
            passed: false,
            checks,
            failure: String(error),
            key_sha256: last?.key_sha256 ?? null,
            key_pinned: last?.key_pinned ?? false,
            events: last?.events ?? [],
          },
        }).catch(() => {});
      }
    };
    void (async () => {
      if (live) return runLive();
      try {
        await paint();
        const first = await place();
        check(
          first.policy.length === 13 &&
            first.policy.every((p) => p.expected === p.actual),
          "Thirteen explicit RDP policy settings were set and read back on the native control",
        );
        check(
          first.policy.find((p) => p.property === "AuthenticationLevel")
            ?.actual === 1,
          "Server authentication is required; warning-and-continue mode is not selected",
        );
        check(
          first.policy.find((p) => p.property === "EnableCredSspSupport")
            ?.actual === 1,
          "CredSSP support is enabled (negotiated NLA remains untested)",
        );
        box.current!.style.width = "480px";
        await paint();
        const resized = await place();
        check(
          resized.id === first.id,
          "Resize retains the same native COM surface incarnation",
        );
        const focused = await call("focus");
        check(
          !!focused?.focus_inside,
          "Win32 keyboard focus belongs to the native child subtree (no remote input tested)",
        );
        const hidden = await call("hide");
        check(
          hidden?.visible === false,
          "Native child is hidden before opening a WebView modal",
        );
        const dialog = document.createElement("dialog");
        const content = document.createElement("p");
        content.textContent =
          "Local overlay test — no server or certificate prompt.";
        dialog.append(content);
        document.body.append(dialog);
        dialog.showModal();
        await paint();
        check(
          dialog.open && (await call("status"))?.visible === false,
          "WebView modal remains open with the native child hidden",
        );
        dialog.close();
        dialog.remove();
        box.current!.style.width = "640px";
        await paint();
        await place();
        const destroyed = await call("destroy");
        check(
          destroyed === null,
          "Explicit close destroys the native surface and COM owner",
        );
        let rejected = false;
        try {
          await call("focus");
        } catch {
          rejected = true;
        }
        check(
          rejected,
          "A stale surface incarnation cannot receive focus after destruction",
        );
        id = undefined;
        const recreated = await place();
        check(
          recreated.id !== first.id,
          "Recreated native control receives a fresh incarnation ID",
        );
        // This stage uses four real COM owners. It is still disconnected: no
        // synthetic desktop image is substituted for a protocol session.
        const root = box.current!;
        root.style.width = "900px";
        root.style.height = "280px";
        root.style.display = "grid";
        root.style.gridTemplateColumns = "1fr 1fr";
        root.style.gap = "16px";
        const panes = Array.from({ length: 4 }, (_, index) => {
          const cell = document.createElement("div");
          cell.style.cssText =
            "display:flex;flex-direction:column;min-height:0;gap:4px";
          const label = document.createElement("span");
          label.textContent = `RDP ${index + 1} · disconnected native control`;
          label.style.fontSize = "12px";
          const body = document.createElement("div");
          body.style.cssText =
            "flex:1;min-height:64px;border:1px solid #707580";
          cell.append(label, body);
          root.append(cell);
          return body;
        });
        await paint();
        const boundsFor = (el: HTMLElement) => {
          const r = el.getBoundingClientRect();
          return [r.x, r.y, r.width, r.height].map((v) =>
            Math.round(v * window.devicePixelRatio),
          );
        };
        const surfaceCall = (
          action: string,
          target: number | null,
          bounds: number[] | null = null,
        ) =>
          invoke<Surface | null>("rdp_probe_surface", {
            action,
            id: target,
            bounds,
          });
        const owners: Surface[] = [];
        for (let i = 0; i < 4; i++) {
          const expected = boundsFor(panes[i]);
          const s = (await surfaceCall(
            "place",
            i === 0 ? recreated.id : null,
            expected,
          ))!;
          owners.push(s);
          surfaces.push(s);
          check(
            s.parent_matches &&
              s.visible &&
              s.connected === 0 &&
              [s.bounds.x, s.bounds.y, s.bounds.width, s.bounds.height].every(
                (v, j) => v === expected[j],
              ),
            `Pane ${i + 1} owns a disconnected control at the exact physical rectangle`,
          );
          check(
            s.policy.length === 13 &&
              s.policy.every((v) => v.expected === v.actual),
            `Pane ${i + 1} independently retains the required RDP policy`,
          );
        }
        check(
          new Set(owners.map((s) => s.id)).size === 4,
          "Four native surfaces have distinct incarnation IDs",
        );
        let fifthRejected = false;
        try {
          await surfaceCall("place", null, boundsFor(panes[0]));
        } catch {
          fifthRejected = true;
        }
        check(
          fifthRejected,
          "Developer probe refuses a fifth native surface without replacing an existing owner",
        );
        for (let i = 0; i < 4; i++) {
          const focused = (await surfaceCall("focus", owners[i].id))!;
          const previous = (await surfaceCall(
            "status",
            owners[(i + 3) % 4].id,
          ))!;
          check(
            focused.focus_inside && !previous.focus_inside,
            `Native focus transfers exclusively to pane ${i + 1}`,
          );
        }
        for (const s of owners.slice(1)) await surfaceCall("hide", s.id);
        const single = (await surfaceCall(
          "place",
          owners[0].id,
          boundsFor(root),
        ))!;
        check(
          single.id === owners[0].id &&
            (await surfaceCall("status", owners[3].id))?.visible === false,
          "Single-pane transition hides retained siblings without recreating controls",
        );
        for (let i = 0; i < 2; i++)
          await surfaceCall("place", owners[i].id, boundsFor(panes[i]));
        check(
          (await surfaceCall("status", owners[1].id))?.visible === true &&
            (await surfaceCall("status", owners[2].id))?.visible === false,
          "Two-pane transition restores only the selected native owners",
        );
        for (let i = 0; i < 4; i++)
          await surfaceCall("place", owners[i].id, boundsFor(panes[i]));
        check(
          (await surfaceCall("status", owners[3].id))?.visible === true,
          "Four-pane transition restores the existing control identities",
        );
        for (const s of owners) await surfaceCall("hide", s.id);
        const overlay = document.createElement("dialog");
        overlay.textContent =
          "Coordinated local overlay; all native children acknowledged hidden.";
        document.body.append(overlay);
        overlay.showModal();
        await paint();
        const hiddenOwners = await Promise.all(
          owners.map((s) => surfaceCall("status", s.id)),
        );
        check(
          overlay.open &&
            hiddenOwners.every((s) => s?.visible === false && !s.top_at_center),
          "All four native surfaces acknowledge hidden before the DOM modal opens",
        );
        overlay.close();
        overlay.remove();
        for (let i = 0; i < 4; i++)
          await surfaceCall("place", owners[i].id, boundsFor(panes[i]));
        const last = owners[3];
        await surfaceCall("destroy", last.id);
        let staleRejected = false;
        try {
          await surfaceCall("place", last.id, boundsFor(panes[3]));
        } catch {
          staleRejected = true;
        }
        check(
          staleRejected,
          "A stale place operation cannot recreate or redirect a closed native owner",
        );
        const replacement = (await surfaceCall(
          "place",
          null,
          boundsFor(panes[3]),
        ))!;
        check(
          replacement.id !== last.id &&
            (await surfaceCall("status", owners[0].id))?.id === owners[0].id,
          "Replacing one closed control preserves unrelated native owners",
        );
        await surfaceCall("test_refusal", replacement.id);
        let refused = (await surfaceCall("status", replacement.id))!;
        const deadline = Date.now() + 10000;
        while (
          Date.now() < deadline &&
          !(
            refused.fixture?.finished &&
            refused.events.some((e) => e.kind === "disconnected")
          )
        ) {
          await new Promise((resolve) => setTimeout(resolve, 100));
          refused = (await surfaceCall("status", replacement.id))!;
        }
        check(
          !!refused.fixture?.accepted && !!refused.fixture.negotiation_received,
          "Real RDP control contacts only the owned loopback fixture and sends a TPKT negotiation header",
        );
        check(
          refused.connected === 0 &&
            refused.events.some((e) => e.kind === "connecting") &&
            refused.events.some(
              (e) => e.kind === "disconnected" && e.code !== null,
            ),
          "Native COM events report connecting then a coded disconnect after the fixture closes",
        );
        check(
          !refused.events.some(
            (e) => e.kind === "login_complete" || e.kind === "connected",
          ) && refused.dropped_events === 0,
          "Early transport failure never becomes an authenticated or connected desktop",
        );
        check(
          (await surfaceCall("status", owners[0].id))!.events.length === 0,
          "Failure callbacks remain isolated from another surface incarnation",
        );
        let repeatRejected = false;
        try {
          await surfaceCall("test_refusal", replacement.id);
        } catch {
          repeatRejected = true;
        }
        check(
          repeatRejected,
          "Fixture retry requires a new surface rather than implicit reconnect",
        );
        await surfaceCall("destroy", replacement.id);
        let closedRejected = false;
        try {
          await surfaceCall("test_refusal", replacement.id);
        } catch {
          closedRejected = true;
        }
        check(
          closedRejected,
          "A destroyed native incarnation cannot initiate a connection",
        );
        await surfaceCall("place", null, boundsFor(panes[3]));
        const report = {
          passed: true,
          scope:
            "Native RDP composition and owned loopback early-disconnect fixture; no authenticated server",
          checks,
          refusal: refused,
          surfaces: surfaces.map(({ policy: _policy, ...surface }) => surface),
          policy: first.policy,
          scale: window.devicePixelRatio,
          limits: [
            "No remote bitmap/input, NLA or certificate validation",
            "No clipboard/audio or mixed-monitor DPI validation",
            "No physical keyboard test",
            "Modal uses explicit hide-before-open; arbitrary overlay interactions remain unproven",
          ],
        };
        setMessage(
          `${checks.length} native checks passed, including a loopback negotiation followed by disconnect. No authenticated desktop.`,
        );
        await paint();
        // Automatic report completion is separately opted into through the host env.
        await invoke("rdp_probe_report", { report });
      } catch (error) {
        const failure = String(error);
        setMessage(failure);
        await invoke("rdp_probe_report", {
          report: {
            passed: false,
            checks,
            failure,
            surfaces: surfaces.map(
              ({ policy: _policy, ...surface }) => surface,
            ),
          },
        }).catch(() => {});
      }
    })();
  }, []);
  return (
    <main style={{ padding: 32 }}>
      <h1>RDP composition probe</h1>
      <p>
        Real Microsoft controls · loopback failure fixture · no credentials or
        authenticated desktop
      </p>
      <p role="status">{message}</p>
      <div
        ref={box}
        style={{
          width: 640,
          height: 240,
          border: "1px solid #707580",
          margin: "24px 0",
        }}
      />
      <ol>
        {log.map((line, i) => (
          <li key={i}>{line}</li>
        ))}
      </ol>
    </main>
  );
}
