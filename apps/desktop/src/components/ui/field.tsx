import type { ComponentProps } from "react";
import { cn } from "../../lib/utils";

/** Accepts native fieldset attributes, including disabled; renders a vertical form group. */
export function FieldGroup({ className, ...props }: ComponentProps<"fieldset">) {
  return <fieldset className={cn("m-0 flex min-w-0 flex-col gap-5 border-0 p-0", className)} {...props} />;
}

/** Accepts native div attributes; data-invalid may mark a field's validation state. */
export function Field({ className, ...props }: ComponentProps<"div">) {
  return <div className={cn("flex min-w-0 flex-col gap-2", className)} {...props} />;
}

/** Accepts native label attributes; htmlFor identifies the associated control. */
export function FieldLabel({ className, ...props }: ComponentProps<"label">) {
  return <label className={cn("text-sm font-medium", className)} {...props} />;
}
