import { useLayoutEffect, useRef } from "react";
import { createPortal } from "react-dom";

export type MenuAction = {
  label: string;
  disabled?: boolean;
  destructive?: boolean;
  run: () => void;
};
export type MenuState = { x: number; y: number; actions: MenuAction[] };

export function ContextMenu({
  menu,
  onClose,
  compact = false,
}: {
  menu: MenuState;
  onClose: () => void;
  compact?: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  useLayoutEffect(() => {
    const element = ref.current!;
    const previous = document.activeElement;
    const bounds = element.getBoundingClientRect();
    element.style.left = `${Math.max(8, Math.min(menu.x, innerWidth - bounds.width - 8))}px`;
    element.style.top = `${Math.max(8, Math.min(menu.y, innerHeight - bounds.height - 8))}px`;
    element.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    const dismiss = (e: Event) => {
      if (!element.contains(e.target as Node)) closeRef.current();
    };
    const close = () => closeRef.current();
    document.addEventListener("pointerdown", dismiss, true);
    document.addEventListener("scroll", dismiss, true);
    window.addEventListener("resize", close);
    window.addEventListener("blur", close);
    return () => {
      document.removeEventListener("pointerdown", dismiss, true);
      document.removeEventListener("scroll", dismiss, true);
      window.removeEventListener("resize", close);
      window.removeEventListener("blur", close);
      // Never steal focus from a newly opened dialog or another application.
      if (
        element.contains(document.activeElement) &&
        document.hasFocus() &&
        previous instanceof HTMLElement &&
        previous.isConnected
      )
        previous.focus();
    };
  }, [menu.x, menu.y]);
  return createPortal(
    <div
      className={`context-menu${compact ? " compact" : ""}`}
      role="menu"
      aria-label="Context actions"
      ref={ref}
      style={{ left: menu.x, top: menu.y }}
      onContextMenu={(e) => {
        e.preventDefault();
        e.stopPropagation();
      }}
      onKeyDown={(e) => {
        const buttons = Array.from(
          ref.current!.querySelectorAll<HTMLButtonElement>(
            "button:not(:disabled)",
          ),
        );
        const index = buttons.indexOf(
          document.activeElement as HTMLButtonElement,
        );
        if (["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) {
          e.preventDefault();
          buttons[
            e.key === "Home"
              ? 0
              : e.key === "End"
                ? buttons.length - 1
                : (index + (e.key === "ArrowUp" ? -1 : 1) + buttons.length) %
                  buttons.length
          ]?.focus();
        } else if (e.key === "Escape" || e.key === "Tab") {
          e.preventDefault();
          onClose();
        }
      }}
    >
      {menu.actions.map((action) => (
        <button
          key={action.label}
          role="menuitem"
          disabled={action.disabled}
          className={action.destructive ? "destructive" : undefined}
          onClick={() => {
            onClose();
            action.run();
          }}
        >
          {action.label}
        </button>
      ))}
    </div>,
    document.body,
  );
}
