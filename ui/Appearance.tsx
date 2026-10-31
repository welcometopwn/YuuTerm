import { themeFor, themes } from "./themes";
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { X } from "lucide-react";
import { SettingsTabs } from "./SettingsTabs";
import { Modal } from "./Modal";
import { compileRegex } from "./highlight-policy";
import { terminalFontChoices } from "./terminal-font";
import type { Preferences } from "./types";
import { defaultHighlighting } from "./highlight-defaults";
import { HighlightImport } from "./HighlightImport";

export function Appearance({
  preferences,
  canSave,
  onPreview,
  onClose,
  onSaved,
}: {
  preferences: Preferences;
  canSave: boolean;
  onPreview: (draft: Preferences) => void;
  onClose: () => void;
  onSaved: (saved: Preferences) => void;
}) {
  const [draft, setDraft] = useState(() => structuredClone(preferences));
  const [custom, setCustom] = useState(
    !terminalFontChoices.includes(preferences.font_family),
  );
  const [tab, setTab] = useState("appearance");
  const [busy, setBusy] = useState(false);
  const saving = useRef(false);
  const [problem, setProblem] = useState("");
  const [confirmReset, setConfirmReset] = useState(false);
  useEffect(() => onPreview(draft), [draft, onPreview]);
  const updateHighlight = (patch: Partial<Preferences["highlighting"]>) =>
    setDraft((p) => ({ ...p, highlighting: { ...p.highlighting, ...patch } }));
  const updateRule = (
    index: number,
    patch: Partial<Preferences["highlighting"]["rules"][number]>,
  ) =>
    updateHighlight({
      rules: draft.highlighting.rules.map((r, i) =>
        i === index ? { ...r, ...patch } : r,
      ),
    });
  const issues: string[] = [];
  if (
    !Number.isInteger(draft.scrollback_lines) ||
    draft.scrollback_lines < 0 ||
    draft.scrollback_lines > 10000
  )
    issues.push("Terminal history must be 0–10,000 lines.");
  if (
    !Number.isFinite(draft.font_size) ||
    draft.font_size < 8 ||
    draft.font_size > 32
  )
    issues.push("Font size must be 8–32.");
  if (
    !draft.font_family.trim() ||
    new TextEncoder().encode(draft.font_family).length > 128 ||
    !/^[\p{L}\p{N} ._-]+$/u.test(draft.font_family)
  )
    issues.push("Enter a valid installed font family name.");
  draft.highlighting.rules.forEach((r, i) => {
    if (r.regex) {
      try {
        compileRegex(r.keywords);
      } catch (e) {
        issues.push(
          `Rule ${i + 1}: ${e instanceof Error ? e.message : String(e)}`,
        );
      }
    } else {
      const words = r.keywords.split(/[,\s]+/).filter(Boolean);
      if (
        !words.length ||
        words.length > 32 ||
        words.some((w) => !/^[a-z0-9_-]{1,32}$/i.test(w))
      )
        issues.push(
          `Rule ${i + 1}: use up to 32 words (letters, digits, _ or -).`,
        );
    }
  });
  const save = async () => {
    if (saving.current || issues.length || !canSave) return;
    saving.current = true;
    setBusy(true);
    setProblem("");
    try {
      onSaved(
        await invoke<Preferences>("save_appearance", {
          mocha: draft.mocha,
          theme: themeFor(draft.theme ?? "", draft.mocha).id,
          fontSize: draft.font_size,
          fontFamily: draft.font_family,
          scrollbackLines: draft.scrollback_lines,
          highlighting: draft.highlighting,
          statistics: draft.statistics,
        }),
      );
    } catch (e) {
      setProblem(String(e));
    } finally {
      saving.current = false;
      setBusy(false);
    }
  };
  return (
    <Modal title="Settings" onClose={busy ? undefined : onClose}>
      <SettingsTabs
        prefix="settings"
        label="Settings sections"
        tabs={["appearance", "highlighting", "statistics"]}
        value={tab}
        onChange={setTab}
      />
      <div
        role="tabpanel"
        id="settings-panel-appearance"
        aria-labelledby="settings-tab-appearance"
        hidden={tab !== "appearance"}
      >
        <label>
          Theme
          <select
            value={themeFor(draft.theme ?? "", draft.mocha).id}
            onChange={(e) =>
              setDraft((p) => ({
                ...p,
                theme: e.target.value,
                mocha: e.target.value === "catppuccin-mocha",
              }))
            }
          >
            {themes.map((t) => (
              <option key={t.id} value={t.id}>
                {t.label}
              </option>
            ))}
          </select>
        </label>
        <label>
          Terminal font
          <select
            aria-label="Terminal font"
            value={custom ? "custom" : draft.font_family}
            onChange={(e) => {
              const selected = e.target.value;
              setCustom(selected === "custom");
              if (selected !== "custom")
                setDraft((p) => ({ ...p, font_family: selected }));
            }}
          >
            {terminalFontChoices.map((name) => (
              <option key={name} value={name}>
                {name === "YuuTerm JetBrains Mono"
                  ? "JetBrains Mono Nerd Font (bundled)"
                  : name}
              </option>
            ))}
            <option value="custom">Other installed font</option>
          </select>
        </label>
        {custom && (
          <label>
            Font family
            <input
              aria-label="Terminal font family"
              value={draft.font_family}
              maxLength={128}
              onChange={(e) =>
                setDraft((p) => ({ ...p, font_family: e.target.value }))
              }
            />
          </label>
        )}
        <label>
          Default terminal font size
          <input
            aria-label="Default terminal font size"
            type="number"
            min={8}
            max={32}
            value={draft.font_size}
            onChange={(e) =>
              setDraft((p) => ({ ...p, font_size: Number(e.target.value) }))
            }
          />
        </label>
        <label>
          Terminal history (lines)
          <input
            aria-label="Terminal history lines"
            type="number"
            min={0}
            max={10000}
            step={100}
            value={draft.scrollback_lines}
            onChange={(e) =>
              setDraft((p) => ({
                ...p,
                scrollback_lines: Number(e.target.value),
              }))
            }
          />
        </label>
        <p className="muted">
          Applies after Save to all terminals. Reducing the limit discards older
          output. Zero keeps only the screen.
        </p>
      </div>
      <div
        role="tabpanel"
        id="settings-panel-highlighting"
        aria-labelledby="settings-tab-highlighting"
        hidden={tab !== "highlighting"}
      >
        <fieldset className="highlight-settings">
          <legend>Terminal highlighting</legend>
          <label className="check-label">
            <input
              type="checkbox"
              checked={draft.highlighting.enabled}
              onChange={(e) => updateHighlight({ enabled: e.target.checked })}
            />
            Highlight terminal output
          </label>
          <div className="ip-highlight-rules">
            {(["ipv4", "ipv6"] as const).map((version) => (
              <div className="highlight-rule ip-highlight-rule" key={version}>
                <label className="check-label">
                  <input
                    type="checkbox"
                    aria-label={`Highlight ${version.toUpperCase().replace("IPV", "IPv")}`}
                    checked={draft.highlighting[version]}
                    onChange={(e) =>
                      updateHighlight({ [version]: e.target.checked })
                    }
                  />
                  {version === "ipv4" ? "IPv4 addresses" : "IPv6 addresses"}
                </label>
                <input
                  type="color"
                  aria-label={`${version === "ipv4" ? "IPv4" : "IPv6"} highlight color`}
                  value={draft.highlighting[`${version}_color`]}
                  onChange={(e) =>
                    updateHighlight({ [`${version}_color`]: e.target.value })
                  }
                />
              </div>
            ))}
          </div>
          {draft.highlighting.rules.map((rule, index) => (
            <div className="highlight-rule" key={index}>
              <select
                aria-label={`Rule ${index + 1} type`}
                value={rule.regex ? "regex" : "words"}
                onChange={(e) =>
                  updateRule(index, { regex: e.target.value === "regex" })
                }
              >
                <option value="words">Words</option>
                <option value="regex">Regex</option>
              </select>
              <input
                aria-label={`Rule ${index + 1} words`}
                maxLength={rule.regex ? 128 : 1100}
                value={rule.keywords}
                placeholder={rule.regex ? "error|warn[0-9]+" : "error, failed"}
                onChange={(e) =>
                  updateRule(index, { keywords: e.target.value })
                }
              />
              <input
                type="color"
                aria-label={`Rule ${index + 1} color`}
                value={rule.color}
                onChange={(e) => updateRule(index, { color: e.target.value })}
              />
              <button
                aria-label={`Remove rule ${index + 1}`}
                onClick={() =>
                  updateHighlight({
                    rules: draft.highlighting.rules.filter(
                      (_, i) => i !== index,
                    ),
                  })
                }
              >
                <X size={13} />
              </button>
            </div>
          ))}
          <button
            disabled={draft.highlighting.rules.length >= 12}
            onClick={() =>
              updateHighlight({
                rules: [
                  ...draft.highlighting.rules,
                  { keywords: "custom", color: "#cba6f7", regex: false },
                ],
              })
            }
          >
            Add color rule
          </button>
          <HighlightImport
            remaining={12 - draft.highlighting.rules.length}
            onAdd={(rules) =>
              updateHighlight({
                rules: [...draft.highlighting.rules, ...rules],
              })
            }
          />
          <p className="hint">
            Case insensitive. Regex patterns without /slashes/; lookaround and
            backreferences are unsupported. Existing ANSI colors take priority.
          </p>
          <button type="button" onClick={() => setConfirmReset(true)}>
            Reset highlighting to defaults
          </button>
          {confirmReset && (
            <div className="reset-confirmation" role="alert">
              <p>
                Reset all highlighting rules, colors and switches to defaults?
                Changes stay in this draft until Save.
              </p>
              <button type="button" onClick={() => setConfirmReset(false)}>
                Keep my rules
              </button>
              <button
                type="button"
                onClick={() => {
                  setDraft((p) => ({
                    ...p,
                    highlighting: defaultHighlighting(),
                  }));
                  setConfirmReset(false);
                }}
              >
                Reset highlighting
              </button>
            </div>
          )}
        </fieldset>
      </div>
      <div
        role="tabpanel"
        id="settings-panel-statistics"
        aria-labelledby="settings-tab-statistics"
        hidden={tab !== "statistics"}
      >
        <fieldset className="statistics-settings">
          <legend>Host statistics</legend>
          <label className="check-label">
            <input
              type="checkbox"
              aria-label="Show host statistics by default"
              checked={draft.statistics.enabled}
              onChange={(e) =>
                setDraft((p) => ({
                  ...p,
                  statistics: { ...p.statistics, enabled: e.target.checked },
                }))
              }
            />
            Show host statistics by default
          </label>
          <p className="hint">
            For SSH connections. Each session can override this setting.
          </p>
          <label>
            Network rate unit
            <select
              aria-label="Network rate unit"
              value={draft.statistics.network_unit}
              onChange={(e) =>
                setDraft((p) => ({
                  ...p,
                  statistics: {
                    ...p.statistics,
                    network_unit: e.target
                      .value as Preferences["statistics"]["network_unit"],
                  },
                }))
              }
            >
              <option value="Megabytes">MB/s — megabytes per second</option>
              <option value="Kilobytes">kB/s — kilobytes per second</option>
              <option value="Megabits">Mbit/s — megabits per second</option>
            </select>
          </label>
        </fieldset>
      </div>
      {(problem || issues.length > 0) && (
        <p className="appearance-error" role="alert">
          {problem || issues[0]}
        </p>
      )}
      <div className="dialog-actions">
        <button disabled={busy} onClick={onClose}>
          Cancel
        </button>
        <button
          className="primary"
          disabled={!canSave || busy || issues.length > 0}
          onClick={() => void save()}
        >
          {busy ? "Saving…" : "Save settings"}
        </button>
      </div>
    </Modal>
  );
}
