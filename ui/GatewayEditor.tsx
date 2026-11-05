// SSH gateway (jump host) list of an SSH bookmark.
import { ArrowDown, ArrowUp, Trash2 } from "lucide-react";
import {
  MAX_GATEWAYS,
  brokenGateways,
  gatewayPath,
  newManualGateway,
  savedChoices,
  savedSummary,
} from "./gateways";
import type { Gateway, ManualGateway, Profile } from "./types";

export function GatewayEditor({
  profile,
  profiles,
  onChange,
}: {
  profile: Profile;
  profiles: Profile[];
  onChange: (gateways: Gateway[]) => void;
}) {
  const gateways = profile.gateways ?? [];
  const choices = savedChoices(profile, profiles);
  const broken = brokenGateways(profile, profiles);
  const replace = (index: number, gateway: Gateway) =>
    onChange(gateways.map((g, i) => (i === index ? gateway : g)));
  const move = (index: number, by: number) => {
    const next = [...gateways];
    [next[index], next[index + by]] = [next[index + by], next[index]];
    onChange(next);
  };
  const manual = (index: number, patch: Partial<ManualGateway>) => {
    const g = gateways[index];
    if (g.kind === "Manual") replace(index, { ...g, ...patch });
  };
  return (
    <fieldset className="session-statistics-settings gateway-settings">
      <legend>SSH gateway (jump hosts)</legend>
      {!gateways.length && <p className="hint">Direct connection</p>}
      {gateways.map((g, index) => {
        const n = index + 1;
        const source =
          g.kind === "Saved"
            ? profiles.find((p) => p.id === g.profile_id)
            : undefined;
        return (
          <div
            key={index}
            className={`gateway-row${broken.includes(index) ? " invalid" : ""}`}
          >
            <div className="form-row">
              <label>
                Gateway {n}
                <select
                  aria-label={`Gateway ${n} source`}
                  value={g.kind}
                  onChange={(e) =>
                    replace(
                      index,
                      e.target.value === "Manual"
                        ? newManualGateway()
                        : { kind: "Saved", profile_id: choices[0]?.id ?? 0 },
                    )
                  }
                >
                  <option value="Saved">Saved session</option>
                  <option value="Manual">Manual</option>
                </select>
              </label>
              <div className="gateway-actions">
                <button
                  type="button"
                  aria-label={`Move gateway ${n} up`}
                  disabled={index === 0}
                  onClick={() => move(index, -1)}
                >
                  <ArrowUp size={14} />
                </button>
                <button
                  type="button"
                  aria-label={`Move gateway ${n} down`}
                  disabled={index === gateways.length - 1}
                  onClick={() => move(index, 1)}
                >
                  <ArrowDown size={14} />
                </button>
                <button
                  type="button"
                  aria-label={`Remove gateway ${n}`}
                  onClick={() =>
                    onChange(gateways.filter((_, i) => i !== index))
                  }
                >
                  <Trash2 size={14} />
                </button>
              </div>
            </div>
            {g.kind === "Saved" ? (
              <>
                <select
                  aria-label={`Gateway ${n} saved session`}
                  value={g.profile_id}
                  onChange={(e) =>
                    replace(index, {
                      kind: "Saved",
                      profile_id: Number(e.target.value),
                    })
                  }
                >
                  {!source && (
                    <option value={g.profile_id}>Choose a session</option>
                  )}
                  {choices.map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.name}
                    </option>
                  ))}
                </select>
                {source && broken.includes(index) === false && (
                  <small>{savedSummary(source)}</small>
                )}
                {broken.includes(index) && (
                  <p role="alert">
                    Choose an existing SSH session other than this one.
                  </p>
                )}
              </>
            ) : (
              <>
                <div className="form-row">
                  <label>
                    Host
                    <input
                      aria-label={`Gateway ${n} host`}
                      value={g.host}
                      maxLength={253}
                      onChange={(e) =>
                        manual(index, { host: e.target.value.trim() })
                      }
                    />
                  </label>
                  <label className="port-field">
                    Port
                    <input
                      aria-label={`Gateway ${n} port`}
                      type="number"
                      min={1}
                      max={65535}
                      value={g.port}
                      onChange={(e) =>
                        manual(index, { port: Number(e.target.value) })
                      }
                    />
                  </label>
                </div>
                <div className="form-row">
                  <label>
                    Username
                    <input
                      aria-label={`Gateway ${n} username`}
                      value={g.username}
                      maxLength={256}
                      onChange={(e) =>
                        manual(index, { username: e.target.value })
                      }
                    />
                  </label>
                  <label>
                    Authentication
                    <select
                      aria-label={`Gateway ${n} authentication`}
                      value={g.auth}
                      onChange={(e) =>
                        manual(index, {
                          auth: e.target.value as ManualGateway["auth"],
                        })
                      }
                    >
                      <option value="WindowsAgent">Windows SSH agent</option>
                      <option value="KeyFile">Private-key file</option>
                      <option value="Password">Password</option>
                    </select>
                  </label>
                </div>
                {g.auth === "KeyFile" && (
                  <label>
                    Private-key path
                    <input
                      aria-label={`Gateway ${n} private-key path`}
                      value={g.key_file}
                      onChange={(e) =>
                        manual(index, { key_file: e.target.value })
                      }
                    />
                  </label>
                )}
              </>
            )}
          </div>
        );
      })}
      <button
        type="button"
        disabled={gateways.length >= MAX_GATEWAYS}
        onClick={() =>
          onChange([
            ...gateways,
            choices.length
              ? { kind: "Saved", profile_id: choices[0].id }
              : newManualGateway(),
          ])
        }
      >
        Add gateway
      </button>
      {gateways.length >= MAX_GATEWAYS && <small>Up to 4 gateways</small>}
      {gateways.length > 0 && (
        <p className="gateway-path" aria-label="Connection path">
          {gatewayPath(profile, profiles)}
        </p>
      )}
      <p className="hint">
        Each gateway's host key is verified separately; a changed key blocks the
        connection. Passwords are asked for in the Windows prompt.
      </p>
    </fieldset>
  );
}
