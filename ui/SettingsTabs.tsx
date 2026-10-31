export function SettingsTabs({
  prefix,
  label,
  tabs,
  value,
  onChange,
}: {
  prefix: string;
  label: string;
  tabs: readonly string[];
  value: string;
  onChange: (tab: string) => void;
}) {
  return (
    <div className="settings-tabs" role="tablist" aria-label={label}>
      {tabs.map((name, index) => (
        <button
          key={name}
          role="tab"
          id={`${prefix}-tab-${name}`}
          aria-controls={`${prefix}-panel-${name}`}
          aria-selected={value === name}
          tabIndex={value === name ? 0 : -1}
          onClick={() => onChange(name)}
          onKeyDown={(e) => {
            const next =
              e.key === "ArrowRight"
                ? tabs[(index + 1) % tabs.length]
                : e.key === "ArrowLeft"
                  ? tabs[(index + tabs.length - 1) % tabs.length]
                  : e.key === "Home"
                    ? tabs[0]
                    : e.key === "End"
                      ? tabs[tabs.length - 1]
                      : null;
            if (next) {
              e.preventDefault();
              onChange(next);
              document.getElementById(`${prefix}-tab-${next}`)?.focus();
            }
          }}
        >
          {name[0].toUpperCase() + name.slice(1)}
        </button>
      ))}
    </div>
  );
}
