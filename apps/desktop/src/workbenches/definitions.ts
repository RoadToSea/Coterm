import local from "../../../../crates/coterm-driver/manifests/local.json";
import ssh from "../../../../crates/coterm-driver/manifests/ssh.json";
import serial from "../../../../crates/coterm-driver/manifests/serial.json";
import tcp from "../../../../crates/coterm-driver/manifests/tcp.json";
import telnet from "../../../../crates/coterm-driver/manifests/telnet.json";

export interface ConfigurationSchema {
  type: string;
  title?: string;
  enum?: readonly (string | number)[];
  minimum?: number;
  maximum?: number;
  maxLength?: number;
  required?: readonly string[];
  properties?: Record<string, ConfigurationSchema>;
  "x-source"?: string;
  "x-help"?: string;
  "x-suggestions"?: readonly number[];
  "x-enumNames"?: readonly string[];
  "x-enabledBy"?: string;
  "x-visibleWhen"?: { field: string; equals: string };
}
export interface WorkbenchDefinition {
  workbench: "terminal" | "stream";
  title: string;
  schema: ConfigurationSchema;
  defaults: Record<string, unknown>;
}
const definitions = Object.fromEntries([local, ssh, serial, tcp, telnet].map(manifest => [manifest.id, manifest.ui as WorkbenchDefinition]));

/** Built-in metadata only. Unknown driver ids return undefined; no source code is loaded. */
export function workbenchDefinition(driverId: string): WorkbenchDefinition | undefined {
  return definitions[driverId.startsWith("coterm-driver-") ? driverId : `coterm-driver-${driverId}`];
}
