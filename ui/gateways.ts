// SSH gateways (jump hosts) of a bookmark. Pure helpers for the
// editor and the session header; the backend validates and resolves again.
import type { Gateway, ManualGateway, Profile } from "./types";

export const MAX_GATEWAYS = 4;

export const newManualGateway = (): Gateway => ({
  kind: "Manual",
  host: "",
  port: 22,
  username: "",
  auth: "WindowsAgent",
  key_file: "",
  agent_pipe: "\\\\.\\pipe\\openssh-ssh-agent",
  identity_fingerprint: "",
});

const referenced = (g: Gateway, profiles: Profile[]) =>
  g.kind === "Saved"
    ? profiles.find((p) => p.id === g.profile_id && !p.temporary)
    : undefined;

/** A hop's display name: the saved session's name or the manual host. */
export function gatewayName(g: Gateway, profiles: Profile[]): string {
  if (g.kind === "Manual") return g.host;
  return referenced(g, profiles)?.name ?? "missing session";
}

export function gatewayPath(profile: Profile, profiles: Profile[]): string {
  return [
    "This PC",
    ...(profile.gateways ?? []).map((g) => gatewayName(g, profiles)),
    `${profile.host}:${profile.port}`,
  ].join(" → ");
}

export function viaLabel(profile: Profile, profiles: Profile[]): string {
  const gateways = profile.gateways ?? [];
  if (!gateways.length) return "";
  if (gateways.length === 1) return `via ${gatewayName(gateways[0], profiles)}`;
  return `via ${gateways.length} gateways`;
}

/** Indices of references to deleted, non-SSH or the same session. */
export function brokenGateways(
  profile: Profile,
  profiles: Profile[],
): number[] {
  return (profile.gateways ?? []).flatMap((g, index) => {
    if (g.kind === "Manual") return [];
    const source = referenced(g, profiles);
    return !source || source.protocol !== "Ssh" || source.id === profile.id
      ? [index]
      : [];
  });
}

/** Import preview: every hop of an imported session (imports are manual only). */
export function importGatewayNote(profile: Profile): string {
  const hops = (profile.gateways ?? []).map((g) =>
    g.kind === "Manual"
      ? `${g.username ? `${g.username}@` : ""}${g.host}:${g.port}`
      : "saved session",
  );
  return hops.length ? `Through SSH gateways: ${hops.join(" → ")}` : "";
}

/** Sessions that stay and use one of `deleted` as gateway. */
export function gatewayDependents(
  profiles: Profile[],
  deleted: number[],
): number {
  return profiles.filter(
    (p) =>
      !deleted.includes(p.id) &&
      (p.gateways ?? []).some(
        (g) => g.kind === "Saved" && deleted.includes(g.profile_id),
      ),
  ).length;
}

export function savedChoices(profile: Profile, profiles: Profile[]): Profile[] {
  return profiles.filter(
    (p) => p.protocol === "Ssh" && !p.temporary && p.id !== profile.id,
  );
}

const authLabel = (auth: ManualGateway["auth"]) =>
  ({ WindowsAgent: "Agent", KeyFile: "Key file", Password: "Password" })[auth];

/** "bastion · admin@bastion.example:22 · Agent" */
export function savedSummary(p: Profile): string {
  return `${p.name} · ${p.username ? `${p.username}@` : ""}${p.host}:${p.port} · ${authLabel(p.ssh.auth)}`;
}
