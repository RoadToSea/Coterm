import { forwardRef, type ComponentProps } from "react";
import { cn } from "../../lib/utils";

/** Native input attributes and ref; browser validation and input events remain available. */
export const Input = forwardRef<HTMLInputElement, ComponentProps<"input">>(({ className, ...props }, ref) => (
  <input ref={ref} className={cn("flex h-9 w-full min-w-0 rounded-sm border border-input bg-background px-3 text-sm text-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 aria-[invalid=true]:border-destructive", className)} {...props} />
));
Input.displayName = "Input";

/** Native select attributes and ref; keeps operating-system keyboard and picker behavior. */
export const Select = forwardRef<HTMLSelectElement, ComponentProps<"select">>(({ className, ...props }, ref) => (
  <select ref={ref} className={cn("h-9 w-full min-w-0 rounded-sm border border-input bg-background px-2 text-sm text-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50", className)} {...props} />
));
Select.displayName = "Select";

/** Native textarea attributes and ref; resizing is controlled by the caller. */
export const Textarea = forwardRef<HTMLTextAreaElement, ComponentProps<"textarea">>(({ className, ...props }, ref) => (
  <textarea ref={ref} className={cn("w-full min-w-0 rounded-sm border border-input bg-background p-2 font-mono text-sm text-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50", className)} {...props} />
));
Textarea.displayName = "Textarea";
