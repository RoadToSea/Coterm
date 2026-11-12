import { hostAPI } from "../../host-api";
import { t } from "../../i18n";
import { useI18n } from "../../use-i18n";
import { useEffect, useRef, useState } from "react";
import type { InvokeOptions, DriverFrontend } from "../../../../shared/index";
import { loadDriverFrontend } from "../../driver-loader";
import { driverI18n } from "../../i18n";
import { cn } from "../../lib/utils";
import { instanceNameErrorCode, validateInstanceName } from "../../instance-name";

export function DriverConfiguration({ driverId, onSubmit, existingNames, disabled = false }: { driverId: string; onSubmit(config: unknown, name?: string): Promise<void>; existingNames: readonly string[]; disabled?: boolean }) {
  useI18n();
  const containerRef = useRef<HTMLDivElement>(null);
  const submitRef = useRef(onSubmit);
  submitRef.current = onSubmit;
  const nameRef = useRef("");
  const [name, setName] = useState("");
  const namesRef = useRef(existingNames);
  namesRef.current = existingNames;
  const [submitError, setSubmitError] = useState<string | null>(null);
  const nameError = validateInstanceName(name, existingNames) ?? submitError;
  const [error, setError] = useState<string | null>(null);
  useEffect(() => { if (containerRef.current) containerRef.current.inert = disabled; }, [disabled]);
  useEffect(() => {
    let disposed = false;
    let frontend: DriverFrontend | undefined;
    const container = containerRef.current;
    const subscriptions = new Set<() => void>();
    if (!container) return;
    setError(null);
    void loadDriverFrontend(driverId).then((module) => {
      if (disposed) return;
      if (!module?.configure) throw new Error(t("This driver does not provide a connection configuration screen."));
      frontend = module.configure(container, {
        submit: async (config) => {
          const trimmedName = nameRef.current.trim();
          const validationError = validateInstanceName(trimmedName, namesRef.current);
          if (validationError) {
            setSubmitError(validationError);
            throw new Error(validationError);
          }
          setSubmitError(null);
          try {
            await submitRef.current(config, trimmedName || undefined);
          } catch (reason) {
            setSubmitError(instanceNameErrorCode(reason));
            throw reason;
          }
        },
        invoke: async <T,>(operation: string, args?: unknown, options?: InvokeOptions) => {
          if (disposed) throw new Error("driver_configuration_closed");
          // Configuration can need backend data before the first instance exists.
          // Runtime start is idempotent and completes the operation-catalog handshake.
          await hostAPI.startDriver(driverId);
          if (disposed) throw new Error("driver_configuration_closed");
          return hostAPI.invokeDriverOperation<T>(driverId, operation, args ?? {}, options?.waitTimeoutMs);
        },
        i18n: {
          get locale() { return driverI18n.locale; },
          subscribe: (listener) => {
            if (disposed) return () => {};
            const stop = driverI18n.subscribe(listener);
            const cancel = () => { subscriptions.delete(cancel); stop(); };
            subscriptions.add(cancel);
            return cancel;
          },
        },
      });
    }).catch((reason) => { if (!disposed) setError(String(reason)); });
    return () => { disposed = true; for (const stop of subscriptions) stop(); frontend?.unmount(); container.replaceChildren(); };
  }, [driverId]);
  nameRef.current = name;
  return <fieldset disabled={disabled} aria-busy={disabled} className="m-0 min-w-0 border-0 p-0">
    {error && <p role="alert" className="mb-3 text-xs text-destructive">{error}</p>}
    <label className="mb-4 block text-sm font-medium" htmlFor="new-instance-name">
      <span className="mb-1.5 block">{t("Instance name (optional)")}</span>
      <input id="new-instance-name" type="text" autoComplete="off" maxLength={256} value={name}
        onChange={event => { setName(event.currentTarget.value); setSubmitError(null); }}
        placeholder={t("Leave empty to use the driver default")}
        aria-invalid={!!nameError}
        aria-describedby={nameError ? "new-instance-name-error" : undefined}
        className={cn("w-full rounded-md border border-input bg-background px-3 py-2 text-sm font-normal text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring", nameError && "border-destructive focus-visible:ring-destructive")} />
    </label>
    {nameError && <p id="new-instance-name-error" role="alert" className="mb-3 text-xs text-destructive">{t(nameError)}</p>}
    <div ref={containerRef} />
  </fieldset>;
}
