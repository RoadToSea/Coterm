import { useEffect, useRef, type ReactNode } from "react";
import { X } from "lucide-react";

interface ModalProps {
  title: string;
  onClose: () => void;
  children: ReactNode;
}

/** Native dialog supplies focus trapping, Escape handling and a modal backdrop. */
export function Modal({ title, onClose, children }: ModalProps) {
  const dialogRef = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    const dialog = dialogRef.current;
    dialog?.showModal();
    return () => dialog?.close();
  }, []);

  return (
    <dialog
      ref={dialogRef}
      aria-label={title}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      className="m-auto max-h-[85dvh] w-[640px] max-w-[calc(100%-32px)] overflow-auto border border-[#454545] bg-[#252526] p-0 text-[#cccccc] shadow-xl backdrop:bg-black/50"
    >
      <header className="sticky top-0 flex h-10 items-center justify-between border-b border-[#383838] bg-[#252526] px-4">
        <h2 className="truncate text-xs font-medium">{title}</h2>
        <button type="button" aria-label="Close dialog" onClick={onClose} className="icon-button">
          <X size={16} />
        </button>
      </header>
      <div className="p-4">{children}</div>
    </dialog>
  );
}
