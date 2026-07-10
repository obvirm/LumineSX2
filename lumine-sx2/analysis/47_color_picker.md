# 47 — ColorPickerButton

**Source files:** `pcsx2-qt/ColorPickerButton.h`, `pcsx2-qt/ColorPickerButton.cpp`

## Purpose
A QPushButton subclass that provides a simple color picker. Clicking opens a `QColorDialog` to pick a color; the button's background reflects the current color.

## Class: `ColorPickerButton` (extends `QPushButton`)

### State
| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `m_color` | `u32` | `0` (black) | Current color packed as `0xRRGGBB` (24-bit RGB, no alpha) |

### Signals
| Signal | Payload | When |
|--------|---------|------|
| `colorChanged(quint32 new_color)` | RGB value | After user picks a new color from the dialog |

### Public Slots
| Slot | Returns | Description |
|------|---------|-------------|
| `color()` | `quint32` | Returns current `m_color` |
| `setColor(quint32 rgb)` | void | Sets color (skip if unchanged), updates button background |

### Private Slots
| Slot | Description |
|------|-------------|
| `onClicked()` | Opens `QColorDialog`, emits `colorChanged` if valid new color selected |

### Private Methods
| Method | Description |
|--------|-------------|
| `updateBackgroundColor()` | Sets QSS `background-color: #RRGGBB` on the button |

## UI Elements
- **Button appearance:** Solid colored rectangle via QSS `background-color`
- **Dialog title:** "Select LED Color" (translatable)
- **Dialog type:** `QColorDialog::getColor()` — native OS color picker

## Key Behaviors
1. **RGB only, no alpha** — only 24-bit RGB (`0xRRGGBB`), no transparency support
2. **Cancel-safe** — if user cancels or picks same color, no signal emitted
3. **No preset colors** — uses default `QColorDialog` (which has its own built-in palette of 48 basic + 16 custom slots, but PCSX2 doesn't customize them)
4. **No HSV mode API** — HSV is available inside QColorDialog itself, but PCSX2 doesn't expose it separately
5. **No color swatch/text** — the button itself is just a colored rectangle
6. **Pack format:** RGB packed as `(R << 16) | (G << 8) | B`

## Where Used
- LED color picker in controller settings (DualSense / DualShock 4 LED color)
- Any settings that need a simple single-color selection

## Features Summary for Slint Implementation
| Feature | PCSX2 Qt | Notes for Slint |
|---------|----------|-----------------|
| Color preview button | ✅ QSS background | Use Rectangle with fill color |
| Open OS color dialog | ✅ QColorDialog | No native Slint equivalent — need a custom HSV picker or use a Rust crate |
| RGB value storage | ✅ u32 0xRRGGBB | Same format works in Slint |
| Alpha support | ❌ | Not needed |
| Preset palette | ❌ (QColorDialog defaults) | Could add Material You presets |
| Color change signal | ✅ `colorChanged` | Use Slint callback |
| HSV picker | ⚠️ Only inside QColorDialog | Need custom if implementing natively |
