import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { hostAPI, type WorkspaceGroup } from "../../host-api";
import { useI18n } from "../../use-i18n";
import { driverI18n } from "../../i18n";
import { instanceNameErrorCode, validateInstanceName } from "../../instance-name";
import { Button } from "../ui/button";
import { Field, FieldGroup, FieldLabel } from "../ui/field";
import { Input, Select } from "../ui/input";
import { ConfigurationFields, configurationDraft, type PortEntry } from "../../workbenches/ConfigurationFields";
import { workbenchDefinition } from "../../workbenches/definitions";
import { createTranslator } from "../../workbenches/common/stream-workbench/i18n";
import type { DriverConfigurationAPI } from "../../../../shared/index";

interface Props {
  driverId: string;
  onSubmit(config: unknown, name: string | undefined, workspaceId: string): Promise<void>;
  existingNames: readonly string[];
  workspaceGroups: readonly WorkspaceGroup[];
  activeWorkspaceId: string;
  disabled?: boolean;
  configurationAPI?: DriverConfigurationAPI;
}

/** Host-owned form. Creating an instance never opens the physical connection. */
export function DriverConfiguration({ driverId, onSubmit, existingNames, workspaceGroups, activeWorkspaceId, disabled = false, configurationAPI }: Props) {
  const { t } = useI18n();
  const [, refreshLocale] = useReducer((value: number) => value + 1, 0);
  const translate = createTranslator(configurationAPI?.i18n ?? driverI18n);
  useEffect(() => configurationAPI?.i18n?.subscribe(refreshLocale), [configurationAPI?.i18n]);
  const definition = workbenchDefinition(driverId);
  const [name, setName] = useState("");
  const [workspaceId, setWorkspaceId] = useState(activeWorkspaceId);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [ports, setPorts] = useState<PortEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const alive = useRef(false);
  const pending = useRef<Promise<void>>();
  const nameError = validateInstanceName(name, existingNames) ?? submitError;
  useEffect(() => {
    if (workspaceGroups.some(group => group.id === workspaceId)) return;
    setWorkspaceId(workspaceGroups.find(group => group.id === activeWorkspaceId)?.id ?? workspaceGroups[0]?.id ?? "default");
  }, [workspaceGroups, activeWorkspaceId, workspaceId]);
  const isSerial = driverId === "coterm-driver-serial" || driverId === "serial";
  const loadPorts = useCallback(() => {
    if (pending.current) return pending.current;
    const read = async () => {
      setLoading(true);
      try {
        if (!configurationAPI) await hostAPI.startDriver(driverId);
        if (!alive.current) return;
        const receipt = configurationAPI?.invoke
          ? await configurationAPI.invoke<{ ports: PortEntry[] }>("serial.driver_info", {})
          : await hostAPI.invokeDriverOperation<{ ports: PortEntry[] }>(driverId, "serial.driver_info", {});
        if (!alive.current) return;
        const entries = receipt.output?.ports;
        if (receipt.status !== "completed") throw new Error(receipt.error?.message ?? "Could not load serial ports");
        if (!Array.isArray(entries)) throw new Error(translate("Serial backend does not support port discovery. Rebuild the driver."));
        flushSync(() => { setPorts(entries); setError(""); });
      } catch (reason) { if (alive.current) { setPorts([]); setError(String(reason)); } }
      finally { if (alive.current) setLoading(false); }
    };
    pending.current = read().finally(() => { pending.current = undefined; });
    return pending.current;
  }, [driverId, configurationAPI]);
  useEffect(() => {
    alive.current = true;
    if (isSerial) void loadPorts();
    return () => { alive.current = false; };
  }, [isSerial, loadPorts]);
  if (!definition) return <p role="alert">{t("This driver does not provide a connection configuration screen.")}</p>;
  return <form className="driver-configuration" aria-busy={disabled || busy || loading} onSubmit={async event => {
    event.preventDefault();
    if (busy || disabled || nameError || (isSerial && loading)) return;
    const config = configurationDraft(definition.schema, definition.defaults, new FormData(event.currentTarget));
    setBusy(true); setError(""); setSubmitError(null);
    try { await onSubmit(config, name.trim() || undefined, workspaceId); }
    catch (reason) { if (alive.current) { setSubmitError(instanceNameErrorCode(reason)); setError(String(reason)); } }
    finally { if (alive.current) setBusy(false); }
  }}>
    <FieldGroup disabled={disabled || busy} aria-busy={disabled || busy || loading}>
      <Field>
        <FieldLabel htmlFor="new-instance-workspace">{t("Workspace")}</FieldLabel>
        <Select id="new-instance-workspace" value={workspaceId} onChange={event => setWorkspaceId(event.currentTarget.value)}>
          {workspaceGroups.map(group => <option key={group.id} value={group.id}>{group.id === "default" ? t("Default workspace") : group.name}</option>)}
        </Select>
      </Field>
      <Field data-invalid={!!nameError}>
        <FieldLabel htmlFor="new-instance-name">{t("Instance name (optional)")}</FieldLabel>
        <Input id="new-instance-name" autoComplete="off" maxLength={256} value={name}
          onChange={event => { setName(event.currentTarget.value); setSubmitError(null); }}
          placeholder={t("Leave empty to use the driver default")} aria-invalid={!!nameError}
          aria-describedby={nameError ? "new-instance-name-error" : undefined} />
        {nameError && <p id="new-instance-name-error" role="alert" className="text-xs text-destructive">{t(nameError)}</p>}
      </Field>
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
        <ConfigurationFields schema={definition.schema} defaults={definition.defaults} translate={translate} ports={ports}
          portMessage={loading ? "Loading ports…" : error ? "Could not load serial ports" : undefined} portEvents={{ onPointerDown: event => {
            if (event.button !== 0 || busy) return;
            const select = event.currentTarget;
            if (typeof select.showPicker !== "function") { void loadPorts(); return; }
            event.preventDefault(); select.focus();
            void loadPorts().then(() => { if (alive.current && document.activeElement === select) { try { select.showPicker(); } catch { /* Discovery can outlive user activation. */ } } });
          }, onKeyDown: event => { if (!event.repeat && ["F4", " ", "Enter", "ArrowDown"].includes(event.key)) void loadPorts(); } }} />
      </div>
      <p role="alert" className="error text-xs text-destructive">{error}</p>
      <Button type="submit" disabled={disabled || busy || !!nameError || (isSerial && (loading || !ports.length))}>{translate(busy ? "Working…" : definition.title)}</Button>
    </FieldGroup>
  </form>;
}
