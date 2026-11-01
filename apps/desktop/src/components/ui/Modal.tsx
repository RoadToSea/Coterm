import { t } from "../../i18n";
import { useI18n } from "../../use-i18n";
import { useEffect, useRef, type ReactNode } from "react";
import { X } from "lucide-react";
import { Button } from "./button";
import { cn } from "../../lib/utils";

interface ModalProps {
  title: string;
  onClose: () => void;
  children: ReactNode;
  id?: string;
  className?: string;
  bodyClassName?: string;
}

/** Native dialog supplies focus trapping, Escape handling and a modal backdrop. */
export function Modal({ title, onClose, children, id, className, bodyClassName }: ModalProps) {
  useI18n();
  const dialogRef = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    const dialog = dialogRef.current;
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    dialog?.showModal();
    return () => {
      dialog?.close();
      if (previousFocus?.isConnected) previousFocus.focus({ preventScroll: true });
    };
  }, []);

  return (
    <dialog
      ref={dialogRef}
      id={id}
      aria-label={title}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      className={cn("m-auto max-h-[85dvh] w-[640px] max-w-[calc(100%-32px)] overflow-auto rounded-xl border border-border bg-card p-0 text-card-foreground shadow-2xl backdrop:bg-black/50", className)}
    >
      <header className="sticky top-0 flex h-14 items-center justify-between border-b border-border bg-card px-5">
        <h2 className="truncate text-sm font-semibold">{title}</h2>
        <Button variant="ghost" size="icon" type="button" aria-label={t("Close dialog")} onClick={onClose}>
          <X data-icon="inline-start" />
        </Button>
      </header>
      <div className={cn("p-5", bodyClassName)}>{children}</div>
    </dialog>
  );
}
