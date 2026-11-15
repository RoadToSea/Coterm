import type { ButtonHTMLAttributes } from "react";
import { Button } from "./button";
import { cn } from "../../lib/utils";

interface Props extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "onChange"> {
  checked: boolean;
  onCheckedChange(checked: boolean): void;
}

/** Accessible binary control. Disabled controls never call onCheckedChange. */
export function Switch({ checked, onCheckedChange, className, ...props }: Props) {
  return <Button {...props} type="button" role="switch" aria-checked={checked} variant={checked ? "default" : "outline"}
    className={cn("h-6 w-10 justify-start p-1", className)} onClick={() => onCheckedChange(!checked)}>
    <span aria-hidden="true" className={cn("size-3.5 rounded-sm transition-transform motion-reduce:transition-none", checked ? "translate-x-4 bg-primary-foreground" : "bg-muted-foreground")} />
  </Button>;
}
