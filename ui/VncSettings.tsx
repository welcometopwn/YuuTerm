import type { Profile } from "./types";
export function VncSettings({
  profile,
  onChange,
}: {
  profile: Profile;
  onChange: (value: Partial<Profile>) => void;
}) {
  return (
    <fieldset>
      <legend>VNC security</legend>
      <label>
        Authentication inside TLS
        <select
          value={profile.vnc?.auth ?? "Plain"}
          onChange={(e) =>
            onChange({
              vnc: { ...profile.vnc, auth: e.target.value as "Plain" | "Vnc" },
            })
          }
        >
          <option value="Plain">
            VeNCrypt X509Plain (username and password)
          </option>
          <option value="Vnc">VeNCrypt X509Vnc (VNC password)</option>
        </select>
      </label>
      <label className="check-label">
        <input
          type="checkbox"
          checked={profile.vnc?.clipboard ?? false}
          onChange={(e) =>
            onChange({
              vnc: {
                auth: "Plain",
                ...profile.vnc,
                clipboard: e.target.checked,
              },
            })
          }
        />
        Allow sending local clipboard text on explicit action
      </label>
      <label className="check-label">
        <input
          type="checkbox"
          checked={profile.vnc?.resize_server ?? false}
          onChange={(e) =>
            onChange({
              vnc: {
                auth: "Plain",
                ...profile.vnc,
                resize_server: e.target.checked,
              },
            })
          }
        />
        Request server resolution changes to fit the visible pane
      </label>
      <ul className="hint hint-list">
        <li>
          Certificates require validation or explicit trust; changed
          certificates block.
        </li>
        <li>
          Sign in with the Windows prompt. Choose "Remember me" to save the
          password securely on this computer, just like SSH and RDP.
        </li>
        <li>
          The desktop scales when resize is disabled, unsupported or refused.
          Resizing can affect other clients of this desktop.
        </li>
        <li>
          Quiet desktops stay connected; incomplete protocol messages time out
          after 30 seconds.
        </li>
        <li>
          Clipboard sending supports Latin-1 text up to 16 KiB; received
          clipboard is always discarded.
        </li>
      </ul>
    </fieldset>
  );
}
