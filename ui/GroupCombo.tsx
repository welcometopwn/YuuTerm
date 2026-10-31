import { useId, useState, type InputHTMLAttributes } from "react";

type InputProps = Omit<
  InputHTMLAttributes<HTMLInputElement>,
  "value" | "onChange" | "list" | "role"
>;

// Text field with a styled suggestion list. Replaces <datalist>, whose
// WebView2 popup cannot be styled and is drawn detached from the field.
export function GroupCombo({
  value,
  options,
  onChange,
  ...input
}: InputProps & {
  value: string;
  options: string[];
  onChange: (value: string) => void;
}) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const needle = value.trim().toLowerCase();
  const shown = options.filter(
    (o) => o.toLowerCase().includes(needle) && o !== value,
  );
  const visible = open && shown.length > 0 && !input.disabled;
  const pick = (option: string) => {
    onChange(option);
    setOpen(false);
    setActive(-1);
  };
  return (
    <div className="combo">
      <input
        {...input}
        value={value}
        role="combobox"
        aria-expanded={visible}
        aria-controls={id}
        aria-autocomplete="list"
        aria-activedescendant={
          visible && active >= 0 ? `${id}-${active}` : undefined
        }
        autoComplete="off"
        onChange={(e) => {
          onChange(e.target.value);
          setOpen(true);
          setActive(-1);
        }}
        onFocus={(e) => {
          setOpen(true);
          input.onFocus?.(e);
        }}
        onBlur={(e) => {
          setOpen(false);
          setActive(-1);
          input.onBlur?.(e);
        }}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            setOpen(true);
            if (shown.length === 0) return;
            const step = e.key === "ArrowDown" ? 1 : -1;
            setActive((a) => (a + step + shown.length) % shown.length);
          } else if (e.key === "Enter" && visible && active >= 0) {
            e.preventDefault();
            pick(shown[active]);
          } else if (e.key === "Escape" && visible) {
            // Close the list only, not the surrounding dialog.
            e.preventDefault();
            e.stopPropagation();
            setOpen(false);
            setActive(-1);
          }
          input.onKeyDown?.(e);
        }}
      />
      {visible && (
        <ul className="combo-list" id={id} role="listbox">
          {shown.map((option, i) => (
            <li
              key={option}
              id={`${id}-${i}`}
              role="option"
              aria-selected={i === active}
              className={i === active ? "active" : undefined}
              onMouseDown={(e) => {
                e.preventDefault();
                pick(option);
              }}
              onMouseEnter={() => setActive(i)}
            >
              {option}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
