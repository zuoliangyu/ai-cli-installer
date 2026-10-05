import { writable } from "svelte/store";

export type Theme = "light" | "dark" | "system";

// 与 public/theme-init.js 使用同一个 key（首帧前同步应用主题，避免闪白）
const KEY = "aci_theme";

function getStored(): Theme {
  try {
    const v = localStorage.getItem(KEY);
    return v === "light" || v === "dark" || v === "system" ? v : "system";
  } catch {
    return "system";
  }
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
  try {
    localStorage.setItem(KEY, t);
  } catch {
    // 存储不可用时仅本次会话生效
  }
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
