// 在首帧渲染前同步应用深色主题，避免深色模式启动时闪白。
// 必须是独立的非 module 脚本（CSP 为 script-src 'self'，不能内联）；
// 存储 key 与跟随系统的逻辑需与 src/lib/theme.ts 保持一致。
(function () {
  var dark = false;
  try {
    var t = localStorage.getItem("aci_theme");
    if (t !== "light" && t !== "dark") t = "system";
    dark =
      t === "dark" ||
      (t === "system" &&
        typeof window.matchMedia === "function" &&
        window.matchMedia("(prefers-color-scheme: dark)").matches);
  } catch (e) {
    // 存储不可用时按浅色处理，theme.ts 加载后会再校正一次
  }
  document.documentElement.classList.toggle("dark", dark);
})();
