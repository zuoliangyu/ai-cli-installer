import { writable } from "svelte/store";
import type { ToolDescriptor, MirrorProbe } from "./types";

export const tools = writable<ToolDescriptor[]>([]);
export const mirrorProbes = writable<MirrorProbe[]>([]);
