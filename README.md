# ChromaPick - Screen Color Picker

A screen color picker built with **Rust** and native Windows APIs. It features a modern dark UI, precision pixel loupe/magnifier, instant HEX color sampling, auto-clipboard copy, and an embedded 3D color palette icon.

---

## ✨ Features

- **🎯 Precision Screen Eyedropper**: Click *"Pick Color From Screen"* (or press <kbd>Space</kbd> / <kbd>Enter</kbd>) to freeze a snapshot of your screens and sample any pixel with an 11×11 magnified loupe and live crosshair.
- **⚡ HEX Format First**: Displays clean, large monospace HEX values (e.g. `#4F46E5`).
- **📋 Copy Button & Feedback**: 
  - Dedicated **"📋 Copy HEX"** button on the interface that dynamically flashes to **"✓ Copied!"** upon selection or click.
  - Quick **RGB** and **HSL** copy buttons.
- **🔄 Auto-Copy on Click**: Enabled by default—simply click any pixel on your screen to sample it and immediately copy the `#HEX` code to your clipboard.
- **🎨 Recent Palette History**: Stores your 10 most recent picks in clickable swatches to restore and copy previous colors anytime.
- **🖼️ 3D Color Palette App Icon**: Features a custom 3D isometric artist palette icon embedded directly into the Windows executable and window title bar.
- **🚀 Native & Ultra-Fast**: Built with Rust and native Win32/GDI double-buffered rendering—no heavy browser or runtime overhead.

---

## 🎮 How to Use

1. Run [`color_picker.exe`](file:///c:/Users/ZM/Desktop/ColorPicker/color_picker.exe) (double-click to launch).
2. Click **"Pick Color From Screen"** or press <kbd>Space</kbd>.
3. Move your mouse across any window or monitor. A magnifying loupe will show individual pixels and the live HEX value.
4. **Left-click** on any area to pick the color.
   - The color is displayed on the main card.
   - If auto-copy is on, the HEX code is copied directly to your clipboard!
5. Click **"Copy HEX"** anytime to re-copy.
6. Press <kbd>Esc</kbd> or right-click to cancel picking mode.

---

## 🛠️ Building From Source

Prerequisites installed:
- **Rust toolchain** (`stable-x86_64-pc-windows-gnu`)
- **MinGW-w64** (`C:\w64devkit\bin`)

To rebuild the project:
```powershell
cargo build --release
```
The optimized executable will be generated at `target/release/color_picker.exe`.
