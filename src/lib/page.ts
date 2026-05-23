import { writable } from "svelte/store";

export type Page = "tools" | "presets" | "fixes" | "about" | "logs";

export const page = writable<Page>("tools");

export function navigate(p: Page) {
  page.set(p);
}
