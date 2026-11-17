import type { ComponentProps } from "react";
import { Cable, Globe, KeyRound, Network, PlugZap, TerminalSquare } from "lucide-react";

const icons = {
  ssh: KeyRound,
  serial: Cable,
  local: TerminalSquare,
  telnet: Globe,
  tcp: Network,
} as const;

type DriverIconProps = ComponentProps<typeof PlugZap> & { driverId: string };

export function DriverIcon({ driverId, ...props }: DriverIconProps) {
  const key = driverId.replace(/^coterm-(?:driver|plugin)-/, "").toLowerCase();
  const iconKey = key === "raw-tcp" ? "tcp" : key;
  const Icon = icons[iconKey as keyof typeof icons] ?? PlugZap;
  return <Icon {...props} data-driver-icon={iconKey} aria-hidden="true" />;
}
