import { useId, useState, type ComponentProps } from "react";
import { Field, FieldLabel } from "../components/ui/field";
import { Input, Select } from "../components/ui/input";
import type { ConfigurationSchema } from "./definitions";
import { useI18n } from "../use-i18n";

export interface PortEntry { portName: string; portType: { kind: string; product?: string | null } }

/** Collapse OS product suffixes without assuming Windows COM naming. */
export function portDisplayName({ portName, portType }: PortEntry): string {
  const product = portType?.product?.trim() ?? "";
  const suffix = `(${portName})`;
  const description = product.endsWith(suffix) ? product.slice(0, -suffix.length).trimEnd() : product;
  return description && description !== portName ? `${portName} (${description})` : portName;
}

interface Props {
  schema: ConfigurationSchema;
  defaults: Record<string, unknown>;
  translate(message: string): string;
  ports?: PortEntry[];
  portMessage?: string;
  portEvents?: Pick<ComponentProps<"select">, "onPointerDown" | "onKeyDown" | "onFocus">;
  prefix?: string;
  manageConditions?: boolean;
}

/** Render the supported connection-schema subset; the backend remains the final validator. */
export function ConfigurationFields({ schema, defaults, translate: t, ports = [], portMessage, portEvents, prefix = "", manageConditions = true }: Props) {
  useI18n();
  const id = useId();
  const [values, setValues] = useState(defaults);
  const proxyType = values.type ?? "direct";
  const proxy = prefix === "proxy.";
  return <>{Object.entries(schema.properties ?? {}).map(([key, field]) => {
    const name = `${prefix}${key}`;
    const value = defaults[key];
    if (field.type === "object") return <Field key={name} className="col-span-full">
      <ConfigurationFields schema={field} defaults={(value ?? {}) as Record<string, unknown>} translate={t} prefix={`${name}.`} manageConditions={manageConditions} />
    </Field>;
    const controlId = `${id}-${name}`;
    const conditional = manageConditions && proxy && key !== "type";
    const inactive = (conditional && proxyType === "direct") || (manageConditions && !!field["x-enabledBy"] && !values[field["x-enabledBy"]]);
    const required = schema.required?.includes(key) || (conditional && !inactive);
    const common = { id: controlId, name, required, disabled: inactive, title: field["x-help"] ? t(field["x-help"]) : undefined,
      "aria-description": field["x-help"] ? t(field["x-help"]) : undefined };
    const serialPort = field["x-source"] === "serial.ports";
    return <Field key={name} data-field={name} hidden={conditional && proxyType === "direct"} className={field.type === "boolean" ? "flex-row items-center" : undefined}>
      <FieldLabel htmlFor={controlId}>{t(field.title ?? key)}</FieldLabel>
      {serialPort ? <Select {...common} defaultValue={String(value ?? "")} {...portEvents}>
        {!ports.length && <option value="">{t(portMessage ?? "No serial ports detected")}</option>}
        {ports.map(port => <option key={port.portName} value={port.portName}>{portDisplayName(port)}</option>)}
      </Select> : field.enum ? <Select {...common} defaultValue={String(value ?? field.enum[0])}
        onChange={proxy && key === "type" && manageConditions ? event => setValues({ ...values, type: event.currentTarget.value }) : undefined}>
        {field.enum.map((option, index) => <option key={option} value={option}>{t(field["x-enumNames"]?.[index] ?? String(option))}</option>)}
      </Select> : field.type === "boolean" ? <Input {...common} type="checkbox" defaultChecked={value === true} className="size-4 accent-primary"
        onChange={manageConditions ? event => setValues({ ...values, [key]: event.currentTarget.checked }) : undefined} />
        : <Input {...common} type={field.type === "integer" ? "number" : "text"} defaultValue={value == null ? "" : String(value)}
          min={field.minimum} max={field.maximum} maxLength={field.maxLength} step={field.type === "integer" ? 1 : undefined}
          list={field["x-suggestions"] ? `${controlId}-suggestions` : undefined} autoComplete="off" spellCheck={false} />}
      {field["x-suggestions"] && <datalist id={`${controlId}-suggestions`}>{field["x-suggestions"].map(item => <option key={item} value={item} />)}</datalist>}
    </Field>;
  })}</>;
}

/** Read one form draft; only fields declared numeric by the schema are converted. */
export function configurationDraft(schema: ConfigurationSchema, defaults: Record<string, unknown>, data: FormData, prefix = ""): Record<string, unknown> {
  const result = { ...defaults };
  for (const [key, field] of Object.entries(schema.properties ?? {})) {
    const name = `${prefix}${key}`;
    if (field.type === "object") {
      const proxy = configurationDraft(field, (defaults[key] ?? {}) as Record<string, unknown>, data, `${name}.`);
      result[key] = proxy.type === "direct" ? { type: "direct" } : proxy;
    } else if (field.type === "boolean") result[key] = data.has(name);
    else if (field.type === "integer") result[key] = data.has(name) ? Number(data.get(name)) : defaults[key];
    else result[key] = String(data.get(name) ?? "");
  }
  if ("username" in result && result.username === "" && defaults.username === null) result.username = null;
  return result;
}
