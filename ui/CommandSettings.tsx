import { useRef, useState, type PointerEvent } from "react";
import { ArrowDown, ArrowUp, GripVertical, Plus, Trash2 } from "lucide-react";
import {
  builtinLabel,
  defaultLayout,
  isBuiltinCategory,
  moveCommand,
  moveTo,
  SSH_CATALOG,
  type Catalog,
} from "./ssh-commands";
import type { CommandCategory, CommandItem, CommandMode } from "./types";

const MODES: { value: CommandMode; label: string }[] = [
  { value: "run", label: "Run" },
  { value: "confirm", label: "Run after confirmation" },
  { value: "insert", label: "Insert without Enter" },
];

type Drag =
  | { kind: "category"; category: number }
  | { kind: "command"; category: number; index: number; builtin: boolean };
/** Where the dragged entry lands: before `index` (categories or commands). */
type Drop = { category: number; index?: number };

const lowerHalf = (el: Element, y: number) => {
  const box = el.getBoundingClientRect();
  return y > box.top + box.height / 2;
};

// Settings → Commands: the SSH tools layout. Built-in commands can be
// reordered within their category and built-in categories hidden; custom
// categories and commands can be added anywhere. Entries are dragged by their
// grip with pointer capture (WebView2's file-drop handler intercepts HTML drag
// and drop, see docs/session-actions.md); the arrow buttons do the same from
// the keyboard.
export function CommandSettings({
  layout,
  onChange,
  catalog = SSH_CATALOG,
}: {
  layout: CommandCategory[];
  onChange: (layout: CommandCategory[]) => void;
  catalog?: Catalog;
}) {
  const [drag, setDrag] = useState<Drag | null>(null);
  const [drop, setDrop] = useState<Drop | null>(null);
  const pointer = useRef<{
    value: Drag;
    x: number;
    y: number;
    active: boolean;
    drop: Drop | null;
  } | null>(null);
  const setCategory = (index: number, patch: Partial<CommandCategory>) =>
    onChange(layout.map((c, i) => (i === index ? { ...c, ...patch } : c)));
  const setItems = (index: number, items: CommandItem[]) =>
    setCategory(index, { items });
  const newName = () => {
    for (let n = 1; ; n++) {
      const name = n === 1 ? "My commands" : `My commands ${n}`;
      if (
        !layout.some((c) => c.name.trim().toLowerCase() === name.toLowerCase())
      )
        return name;
    }
  };
  const locate = (value: Drag, x: number, y: number): Drop | null => {
    const hit = document.elementFromPoint(x, y);
    const section = hit?.closest<HTMLElement>("[data-drop-category]");
    if (!section) return null;
    const category = Number(section.dataset.dropCategory);
    if (value.kind === "category")
      return { category: lowerHalf(section, y) ? category + 1 : category };
    if (value.builtin && value.category !== category) return null;
    const row = hit?.closest<HTMLElement>("[data-drop-index]");
    if (!row) return { category };
    const index = Number(row.dataset.dropIndex);
    return { category, index: lowerHalf(row, y) ? index + 1 : index };
  };
  const end = () => {
    pointer.current = null;
    setDrag(null);
    setDrop(null);
  };
  const grip = (label: string, value: Drag) => (
    <span
      className="command-grip"
      title="Drag to reorder"
      aria-label={label}
      onPointerDown={(e: PointerEvent<HTMLElement>) => {
        if (e.button !== 0) return;
        e.preventDefault();
        e.currentTarget.setPointerCapture(e.pointerId);
        pointer.current = {
          value,
          x: e.clientX,
          y: e.clientY,
          active: false,
          drop: null,
        };
      }}
      onPointerMove={(e: PointerEvent<HTMLElement>) => {
        const p = pointer.current;
        if (!p || !e.currentTarget.hasPointerCapture(e.pointerId)) return;
        if (!p.active) {
          if (Math.hypot(e.clientX - p.x, e.clientY - p.y) < 6) return;
          p.active = true;
          setDrag(p.value);
        }
        p.drop = locate(p.value, e.clientX, e.clientY);
        setDrop(p.drop);
      }}
      onPointerUp={() => {
        const p = pointer.current;
        if (p?.active && p.drop) {
          const { value, drop: target } = p;
          if (value.kind === "category")
            onChange(moveTo(layout, value.category, target.category));
          else
            onChange(
              moveCommand(layout, value, {
                category: target.category,
                index: target.index ?? layout[target.category].items.length,
              }),
            );
        }
        end();
      }}
      onPointerCancel={end}
    >
      <GripVertical size={13} />
    </span>
  );
  return (
    <div className="command-settings">
      <p className="muted">{catalog.description}</p>
      {layout.map((category, c) => {
        const builtin = isBuiltinCategory(category.name, catalog);
        const marker =
          drag?.kind === "command" &&
          drop?.category === c &&
          drop.index === undefined
            ? "drop-into"
            : drag?.kind === "category" && drop && drop.index === undefined
              ? drop.category === c
                ? "drop-before"
                : drop.category === c + 1 && c === layout.length - 1
                  ? "drop-after"
                  : ""
              : "";
        return (
          <section
            key={c}
            className={`command-category ${category.hidden ? "hidden-category" : ""} ${marker} ${drag?.kind === "category" && drag.category === c ? "dragging" : ""}`}
            data-category={category.name}
            data-drop-category={c}
          >
            <header>
              {grip(`Drag ${category.name || "category"}`, {
                kind: "category",
                category: c,
              })}
              {builtin ? (
                <strong>{category.name}</strong>
              ) : (
                <input
                  aria-label="Category name"
                  value={category.name}
                  maxLength={48}
                  onChange={(e) => setCategory(c, { name: e.target.value })}
                />
              )}
              <button
                className="icon"
                aria-label={`Move ${category.name || "category"} up`}
                title="Move up"
                disabled={c === 0}
                onClick={() => onChange(moveTo(layout, c, c - 1))}
              >
                <ArrowUp size={13} />
              </button>
              <button
                className="icon"
                aria-label={`Move ${category.name || "category"} down`}
                title="Move down"
                disabled={c === layout.length - 1}
                onClick={() => onChange(moveTo(layout, c, c + 2))}
              >
                <ArrowDown size={13} />
              </button>
              {builtin ? (
                <label className="check-label">
                  <input
                    type="checkbox"
                    checked={!category.hidden}
                    onChange={(e) =>
                      setCategory(c, { hidden: !e.target.checked })
                    }
                  />
                  Show
                </label>
              ) : (
                <button
                  className="icon"
                  aria-label={`Delete ${category.name || "category"}`}
                  title="Delete category and its commands"
                  onClick={() => onChange(layout.filter((_, i) => i !== c))}
                >
                  <Trash2 size={13} />
                </button>
              )}
            </header>
            {category.items.map((item, i) => {
              const name =
                item.kind === "builtin"
                  ? builtinLabel(item.id, catalog)
                  : item.label || "command";
              const rowMarker =
                drag?.kind === "command" && drop?.category === c
                  ? drop.index === i
                    ? "drop-before"
                    : drop.index === i + 1 && i === category.items.length - 1
                      ? "drop-after"
                      : ""
                  : "";
              const rowDrag = { "data-drop-index": i };
              const dragging =
                drag?.kind === "command" &&
                drag.category === c &&
                drag.index === i
                  ? "dragging"
                  : "";
              const handle = grip(`Drag ${name}`, {
                kind: "command",
                category: c,
                index: i,
                builtin: item.kind === "builtin",
              });
              const arrows = (
                <>
                  <button
                    className="icon"
                    aria-label={`Move ${name} up`}
                    title="Move up"
                    disabled={i === 0}
                    onClick={() =>
                      setItems(c, moveTo(category.items, i, i - 1))
                    }
                  >
                    <ArrowUp size={13} />
                  </button>
                  <button
                    className="icon"
                    aria-label={`Move ${name} down`}
                    title="Move down"
                    disabled={i === category.items.length - 1}
                    onClick={() =>
                      setItems(c, moveTo(category.items, i, i + 2))
                    }
                  >
                    <ArrowDown size={13} />
                  </button>
                </>
              );
              if (item.kind === "builtin")
                return (
                  <div
                    className={`command-row builtin ${rowMarker} ${dragging}`}
                    key={item.id}
                    {...rowDrag}
                  >
                    {handle}
                    <span className="command-name">{name}</span>
                    <span className="muted">Built-in</span>
                    {arrows}
                  </div>
                );
              const update = (patch: Partial<typeof item>) =>
                setItems(
                  c,
                  category.items.map((v, j) =>
                    j === i ? { ...item, ...patch } : v,
                  ),
                );
              return (
                <div
                  className={`command-row custom ${rowMarker} ${dragging}`}
                  key={`custom-${i}`}
                  {...rowDrag}
                >
                  {handle}
                  <input
                    aria-label="Command name"
                    placeholder="Name"
                    value={item.label}
                    maxLength={64}
                    onChange={(e) => update({ label: e.target.value })}
                  />
                  <input
                    aria-label={`Command for ${name}`}
                    className="command-text"
                    placeholder="Command, e.g. docker compose pull"
                    spellCheck={false}
                    value={item.command}
                    maxLength={4096}
                    onChange={(e) => update({ command: e.target.value })}
                  />
                  <select
                    aria-label={`How ${name} runs`}
                    value={item.mode}
                    onChange={(e) =>
                      update({ mode: e.target.value as CommandMode })
                    }
                  >
                    {MODES.map((m) => (
                      <option key={m.value} value={m.value}>
                        {m.label}
                      </option>
                    ))}
                  </select>
                  {arrows}
                  <button
                    className="icon"
                    aria-label={`Delete ${name}`}
                    title="Delete command"
                    onClick={() =>
                      setItems(
                        c,
                        category.items.filter((_, j) => j !== i),
                      )
                    }
                  >
                    <Trash2 size={13} />
                  </button>
                </div>
              );
            })}
            <button
              className="command-add"
              onClick={() =>
                setItems(c, [
                  ...category.items,
                  { kind: "custom", label: "", command: "", mode: "run" },
                ])
              }
            >
              <Plus size={13} />
              Add command
            </button>
          </section>
        );
      })}
      <div className="dialog-actions command-settings-actions">
        <button
          title="Removes custom commands and restores the built-in order"
          onClick={() => onChange(defaultLayout(catalog))}
        >
          Reset to defaults
        </button>
        <button
          onClick={() => onChange([...layout, { name: newName(), items: [] }])}
        >
          <Plus size={13} />
          Add category
        </button>
      </div>
    </div>
  );
}
