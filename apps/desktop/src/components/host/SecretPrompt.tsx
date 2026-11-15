import { useEffect, useId, useRef, useState } from "react";
import type { InteractionOptions, SelectionOption } from "../../../../shared/index";
import { useI18n } from "../../use-i18n";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/button";
import { Field, FieldLabel } from "../ui/field";
import { Input, Select } from "../ui/input";

export type InputKind = "password" | "text" | "confirmation" | SelectionOption[];
interface Props {
  request: InteractionOptions;
  kind: InputKind;
  deliver(value: string): Promise<void>;
  cancel(): void;
  reject(): void;
  unavailable(): void;
}

/** Passwords live only in the uncontrolled input and are cleared before awaiting delivery. */
export function SecretPrompt({ request, kind, deliver, cancel, reject, unavailable }: Props) {
  const { t } = useI18n();
  const input = useRef<HTMLInputElement | HTMLSelectElement>(null);
  const deciding = useRef(false);
  const mounted = useRef(false);
  const inputId = useId();
  useEffect(() => {
    mounted.current = true;
    const control = input.current;
    return () => { mounted.current = false; if (control) control.value = ""; };
  }, []);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  return <Modal title={t(request.title)} onClose={cancel} allowClose={request.allowCancel !== false} onOpenError={unavailable} className="host-secret-dialog w-[420px]">
    <form className="flex flex-col gap-4" onSubmit={async event => {
      event.preventDefault();
      if (deciding.current) return;
      deciding.current = true; setBusy(true); setError("");
      try {
        const result = deliver(input.current?.value ?? "");
        if (input.current) input.current.value = "";
        await result;
      } catch {
        if (!mounted.current) return;
        deciding.current = false; setBusy(false);
        setError(t("Unable to deliver the response. Try again or cancel."));
      }
    }}>
      <Field>
        <FieldLabel htmlFor={inputId}>{t(request.prompt)}</FieldLabel>
        {Array.isArray(kind) ? <Select id={inputId} ref={input as React.RefObject<HTMLSelectElement>} autoFocus disabled={busy}>
          {kind.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}
        </Select> : kind !== "confirmation" && <Input id={inputId} ref={input as React.RefObject<HTMLInputElement>} type={kind} autoComplete="off" maxLength={65536} autoFocus disabled={busy} />}
      </Field>
      {error && <p role="alert" className="text-xs text-destructive">{error}</p>}
      <div className="flex flex-wrap justify-end gap-2">
        {kind === "confirmation" && <Button type="button" variant="outline" onClick={reject}>{t("Reject")}</Button>}
        {request.allowCancel !== false && <Button type="button" variant="outline" onClick={cancel}>{t("Cancel")}</Button>}
        <Button type="submit" disabled={busy}>{t(kind === "confirmation" ? "Confirm" : "Continue")}</Button>
      </div>
    </form>
  </Modal>;
}
