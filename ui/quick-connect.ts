// Quick connect: the session search field also accepts a destination such as
// "user@host", "host:2222" or "rdp://host". It reuses a matching bookmark or
// adds an unsaved, in-memory session for this run (no stored password).
import { blankProfile, protocolLabel } from "./types.ts";
import type { Profile, Protocol } from "./types.ts";

export interface Destination {
  protocol: Protocol;
  host: string;
  port: number;
  username: string;
}

export const defaultPorts: Record<
  Exclude<Protocol, "Serial" | "Local">,
  number
> = {
  Ssh: 22,
  Telnet: 23,
  Rdp: 3389,
  Vnc: 5900,
};

const schemes: Record<string, Exclude<Protocol, "Serial" | "Local">> = {
  ssh: "Ssh",
  telnet: "Telnet",
  rdp: "Rdp",
  vnc: "Vnc",
};

const hostName = /^[A-Za-z0-9_](?:[A-Za-z0-9._-]{0,252})$/;
const ipv6 = /^[0-9A-Fa-f:.]{2,45}$/;

/** Parses a destination; `null` when the text is not one. */
export function parseDestination(text: string): Destination | null {
  let rest = text.trim();
  if (!rest || rest.length > 300 || /[\s\u0000-\u001f\u007f]/.test(rest))
    return null;
  let protocol: Exclude<Protocol, "Serial" | "Local"> = "Ssh";
  const scheme = /^([A-Za-z]+):\/\//.exec(rest);
  if (scheme) {
    const known = schemes[scheme[1].toLowerCase()];
    if (!known) return null;
    protocol = known;
    rest = rest.slice(scheme[0].length).replace(/\/$/, "");
  }
  let username = "";
  const at = rest.lastIndexOf("@");
  if (at >= 0) {
    username = rest.slice(0, at);
    rest = rest.slice(at + 1);
    if (!username || username.length > 256) return null;
  }
  let host = rest;
  let port = defaultPorts[protocol];
  const bracket = /^\[([^\]]+)\](?::(\d{1,5}))?$/.exec(rest);
  if (bracket) {
    host = bracket[1];
    if (!ipv6.test(host)) return null;
    if (bracket[2]) port = Number(bracket[2]);
  } else if ((rest.match(/:/g) ?? []).length > 1) {
    if (!ipv6.test(rest)) return null;
  } else {
    const colon = rest.indexOf(":");
    if (colon >= 0) {
      host = rest.slice(0, colon);
      const digits = rest.slice(colon + 1);
      if (!/^\d{1,5}$/.test(digits)) return null;
      port = Number(digits);
    }
    if (!hostName.test(host)) return null;
  }
  if (port < 1 || port > 65535) return null;
  return { protocol, host, port, username };
}

/** Text that can only mean a destination, never a search term. */
export const explicitDestination = (text: string) =>
  /@|:\/\/|:\d+\s*$|^\s*\[/.test(text) && parseDestination(text) !== null;

const same = (p: Profile, d: Destination) =>
  p.protocol === d.protocol &&
  p.host.toLowerCase() === d.host.toLowerCase() &&
  p.port === d.port &&
  p.username === d.username;

/** The newest bookmark for exactly this destination. */
export const findBookmark = (profiles: Profile[], d: Destination) =>
  profiles.filter((p) => same(p, d)).sort((a, b) => b.id - a.id)[0];

export function describe(d: Destination) {
  const host = d.host.includes(":") ? `[${d.host}]` : d.host;
  const port =
    d.port === defaultPorts[d.protocol as keyof typeof defaultPorts]
      ? ""
      : `:${d.port}`;
  const user = d.username ? `${d.username}@` : "";
  return `${user}${host}${port}`;
}

/** A new bookmark for the destination with the usual defaults. */
export function quickProfile(d: Destination): Profile {
  const profile = blankProfile();
  profile.protocol = d.protocol;
  profile.host = d.host;
  profile.port = d.port;
  profile.username = d.username;
  profile.temporary = true;
  profile.name =
    d.protocol === "Ssh"
      ? describe(d)
      : `${protocolLabel(d.protocol)} ${describe(d)}`;
  return profile;
}
