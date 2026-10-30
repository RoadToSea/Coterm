import { useSyncExternalStore } from "react";
import { Monitor, Moon, Sun } from "lucide-react";
import { useI18n } from "../use-i18n";
import { getThemePreference, setThemePreference, subscribeTheme } from "../theme";
import { ToggleGroup, ToggleGroupItem } from "./ui/toggle-group";

export function ThemeSwitcher() {
  const { t } = useI18n();
  const preference = useSyncExternalStore(subscribeTheme, getThemePreference, getThemePreference);
  return (
    <ToggleGroup id="theme-switcher" type="single" size="sm" value={preference}
      aria-label={t("Appearance")} className="rounded-lg border border-border p-0.5"
      onValueChange={(value) => {
        if (value === "light" || value === "dark" || value === "system") setThemePreference(value);
      }}>
      <ToggleGroupItem value="light" aria-label={t("Light mode")} title={t("Light mode")}><Sun data-icon="inline-start" /></ToggleGroupItem>
      <ToggleGroupItem value="dark" aria-label={t("Dark mode")} title={t("Dark mode")}><Moon data-icon="inline-start" /></ToggleGroupItem>
      <ToggleGroupItem value="system" aria-label={t("System theme")} title={t("System theme")}><Monitor data-icon="inline-start" /></ToggleGroupItem>
    </ToggleGroup>
  );
}
