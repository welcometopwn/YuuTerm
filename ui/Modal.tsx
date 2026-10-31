import { useEffect, useRef, type ReactNode } from "react";
import { X } from "lucide-react";
export function Modal({
  title,
  children,
  onClose,
  className,
}: {
  title: string;
  children: ReactNode;
  onClose?: () => void;
  className?: string;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    ref.current?.showModal();
  }, []);
  return (
    <dialog
      aria-label={title}
      className={className}
      ref={ref}
      onCancel={(e) => {
        e.preventDefault();
        onClose?.();
      }}
    >
      <div className="dialog-header">
        <h2>{title}</h2>
        {onClose && (
          <button className="icon" aria-label="Close dialog" onClick={onClose}>
            <X size={16} />
          </button>
        )}
      </div>
      {children}
    </dialog>
  );
}
