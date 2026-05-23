import { writable } from "svelte/store";

const KEY = "ai-cli-dev-mode";

function load(): boolean {
  try {
    return localStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
}

export const devMode = writable<boolean>(load());

export function toggleDevMode() {
  devMode.update((v) => {
    const next = !v;
    try {
      if (next) localStorage.setItem(KEY, "1");
      else localStorage.removeItem(KEY);
    } catch {
      // ignore
    }
    return next;
  });
}
