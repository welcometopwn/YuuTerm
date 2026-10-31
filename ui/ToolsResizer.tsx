import { TOOLS_MAX_WIDTH, TOOLS_MIN_WIDTH } from "./workspace-layout";

// Drag handle on the left edge of the RDP/SSH remote-tools panel. The panel
// sits at the right of `.rdp-split`, so its width is the distance from the
// pointer to the container's right edge.
export function ToolsResizer({
  width,
  onWidth,
}: {
  width: number;
  onWidth: (width: number) => void;
}) {
  return (
    <div
      className="tools-resizer"
      role="separator"
      aria-label="Resize remote tools"
      aria-orientation="vertical"
      aria-valuemin={TOOLS_MIN_WIDTH}
      aria-valuemax={TOOLS_MAX_WIDTH}
      aria-valuenow={width}
      tabIndex={0}
      title="Drag to resize remote tools; double-click to reset"
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        event.preventDefault();
        event.currentTarget.setPointerCapture(event.pointerId);
      }}
      onPointerMove={(event) => {
        if (!event.currentTarget.hasPointerCapture(event.pointerId)) return;
        const split = event.currentTarget.closest(".rdp-split")!;
        const bounds = split.getBoundingClientRect();
        // Keep at least 200 px for the terminal or remote desktop.
        onWidth(Math.min(bounds.width - 200, bounds.right - event.clientX));
      }}
      onPointerUp={(event) => {
        if (event.currentTarget.hasPointerCapture(event.pointerId))
          event.currentTarget.releasePointerCapture(event.pointerId);
      }}
      onDoubleClick={() => onWidth(340)}
      onKeyDown={(event) => {
        if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key))
          return;
        event.preventDefault();
        onWidth(
          event.key === "Home"
            ? TOOLS_MAX_WIDTH
            : event.key === "End"
              ? TOOLS_MIN_WIDTH
              : width + (event.key === "ArrowLeft" ? 20 : -20),
        );
      }}
    />
  );
}
