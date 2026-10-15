export interface CommandBlock {
  id: string;
  index: number;
  command: string;
  exitCode: number | null;
  actor: "human" | "agent";
  isSelected: boolean;
  timestamp: string;
}

export interface GhostTextProposal {
  command: string;
  isRisky?: boolean;
  riskReason?: string;
}
