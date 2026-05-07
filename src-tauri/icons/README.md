# Icons

Tauri 构建需要这里有真实图标。占位文件不会让构建通过。

最快生成方式：准备一张 1024×1024 的 PNG（透明背景），然后跑：

```sh
cargo tauri icon path/to/logo.png
```

会自动生成：
- `32x32.png` / `128x128.png` / `128x128@2x.png`
- `icon.icns` (macOS)
- `icon.ico` (Windows)
- 各种 Android/iOS 资源（不影响桌面构建）

如果暂时没有 logo，可以从任何一个开源项目扒一份 placeholder 临时用，等正式发版前换。
