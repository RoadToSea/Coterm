/** Return a stable field error, or null for a valid explicit name / an omitted default. */
export function validateInstanceName(value: string, existingNames: readonly string[]): string | null {
  const name = value.trim();
  if (!name) return null;
  if (new TextEncoder().encode(name).length > 256 || /[\u0000-\u001f\u007f-\u009f]/u.test(name)) {
    return "instance_name_invalid";
  }
  return existingNames.some(existing => existing.trim() === name) ? "instance_name_exists" : null;
}

/** Recognize only instance-name validation errors; unrelated host errors return null. */
export function instanceNameErrorCode(reason: unknown): string | null {
  const code = typeof reason === "object" && reason !== null && "code" in reason ? String(reason.code) : String(reason);
  return code.match(/\binstance_name_(?:exists|invalid)\b/u)?.[0] ?? null;
}
