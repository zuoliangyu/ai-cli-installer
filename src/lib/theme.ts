import { writable } from "svelte/store";

export type Theme = "light" | "dark" | "system";

const KEY = "aci_theme";

function getStored(): Theme {
  if (typeof localStorage === "undefined") return "system";
  const v = localStorage.getItem(KEY);
  return v === "light" || v === "dark" || v === "system" ? v : "system";
}

function systemPrefersDark(): boolean {
  return (
    typeof matchMedia !== "undefined" &&
    matchMedia("(prefers-color-scheme: dark)").matches
  );
}

function applyTheme(t: Theme) {
  if (typeof document === "undefined") return;
  const dark = t === "dark" || (t === "system" && systemPrefersDark());
  document.documentElement.classList.toggle("dark", dark);
}

export const theme = writable<Theme>(getStored());

theme.subscribe((t) => {
  if (typeof localStorage !== "undefined") localStorage.setItem(KEY, t);
  applyTheme(t);
});

if (typeof matchMedia !== "undefined") {
  matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    let current: Theme = "system";
    theme.subscribe((v) => (current = v))();
    if (current === "system") applyTheme("system");
  });
}

export function setTheme(t: Theme) {
  theme.set(t);
}
