// Runs before the application stylesheet to avoid a flash of the wrong palette.
(() => {
  let preference = "system";
  try {
    const saved = localStorage.getItem("coterm.theme");
    if (saved === "light" || saved === "dark") preference = saved;
  } catch { /* Storage can be disabled; system appearance still works. */ }
  const dark = preference === "dark" || (preference === "system" && matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.classList.toggle("dark", dark);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
  document.documentElement.style.colorScheme = dark ? "dark" : "light";
})();
