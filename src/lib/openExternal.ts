export async function openExternalUrl(url: string): Promise<void> {
  const protocol = new URL(url).protocol;
  if (!["http:", "https:", "mailto:"].includes(protocol)) {
    throw new Error(`不允许打开 ${protocol} 链接`);
  }
  if (!__IS_TAURI__) {
    window.open(url, "_blank", "noopener,noreferrer");
    return;
  }
  const { open } = await import("@tauri-apps/plugin-shell");
  await open(url);
}
