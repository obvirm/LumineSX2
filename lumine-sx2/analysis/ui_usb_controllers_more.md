# Additional USB/Controller Binding UI Analysis — Previously Unanalyzed .ui Files

Files analyzed: 6 new .ui files not covered by previous batch.

---

## 1. USBBindingWidget_Buzz.ui — Buzz! Controller

**Class**: `USBBindingWidget_Buzz`
**Size**: 1100×500
**Layout**: QGridLayout with 4 player columns + spacers

### Structure:
- **Player 1–4** (4 columns via `verticalLayout_p1` through `verticalLayout_p4`)
  - Each player has a QGroupBox titled "Player 1" etc.
  - Inside: 5 colored button bindings:

| Binding Name | GroupBox Title | Widget Type |
|-------------|---------------|-------------|
| `Red` | "Red" | InputBindingWidget |
| `Blue` | "Blue" | InputBindingWidget |
| `Orange` | "Orange" | InputBindingWidget |
| `Green` | "Green" | InputBindingWidget |
| `Yellow` | "Yellow" | InputBindingWidget |

- **Spacer** `horizontalSpacer0` (left) and `horizontalSpacer5` (right)
- **Vertical spacer** at bottom

### Special Features:
- **4 players simultaneously** (unique — only Buzz! supports 4-player binding)
- **5 colored buttons per player** (Red, Blue, Orange, Green, Yellow)
- **No D-Pad, no face buttons, no shoulders** — just 5 colored buttons
- **No vibration** — pure button input
- All buttons are InputBindingWidget (100×... pushbutton)
- Tab order: P1 Red→Blue→Orange→Green→Yellow → P2 same → P3 → P4

---

## 2. USBBindingWidget_Gametrak.ui — Gametrak Position Tracker

**Class**: `USBBindingWidget_Gametrak`
**Size**: 1100×500
**Layout**: QGridLayout with 3 vertical groups + spacers

### Structure:
- **Left Hand** group (`groupBox1`)
  - X Axis: `LeftX` (InputBindingWidget) + label "Left=0 / Right=0x3ff"
  - Y Axis: `LeftY` (InputBindingWidget) + label "Back=0 / Front=0x3ff"
  - Z Axis: `LeftZ` (InputBindingWidget) + label "Top=0 / Bottom=0xfff"
- **Foot Pedal** group (`groupBox2`)
  - `FootPedal` (InputBindingWidget) — single binding
- **Right Hand** group (`groupBox3`)
  - X Axis: `RightX` + label "Left=0 / Right=0x3ff"
  - Y Axis: `RightY` + label "Back=0 / Front=0x3ff"
  - Z Axis: `RightZ` + label "Top=0 / Bottom=0xfff"

### Special Features:
- **Full 3D position tracking**: Left X, Left Y, Left Z + Right X, Right Y, Right Z
- **Foot pedal**: binary pedal input
- **Range labels** on each axis show the expected value ranges (0 to 0x3ff or 0xfff)
- **No buttons, no D-Pad** — pure analog position tracking
- Uses InputBindingWidget for all bindings (130px wide)
- Tab order: LeftX→LeftY→LeftZ→RightX→RightY→RightZ→FootPedal

---

## 3. USBBindingWidget_RealPlay.ui — Real Play Motion Controller

**Class**: `USBBindingWidget_RealPlay`
**Size**: 1100×500
**Layout**: QGridLayout with D-Pad, colored buttons, accelerometer

### Structure:
- **D-Pad** (center layout, arranged Up/Down/Left/Right):
  - `DPadUp`, `DPadDown`, `DPadLeft`, `DPadRight`
- **Colored Buttons** (right side, vertical column):
  - `Red`, `Green`, `Yellow`, `Blue`
- **Accelerometer** (bottom row):
  - `AccelX`, `AccelY`, `AccelZ`

### Special Features:
- **Motion control**: 3-axis accelerometer (Accel X/Y/Z)
- **D-Pad**: 4-directional digital pad
- **4 colored buttons** (Red/Green/Yellow/Blue — not Triangle/Cross/Square/Circle)
- **No shoulder buttons, no Start/Select** — minimal button layout
- **No vibration**
- Tab order: DPadUp→DPadDown→DPadLeft→DPadRight→Red→Green→Yellow→Blue→AccelX→AccelY→AccelZ

---

## 4. USBBindingWidget_TranceVibrator.ui — Sony Trance Vibrator

**Class**: `USBBindingWidget_TranceVibrator`
**Size**: 1100×500 (minimum enforced)
**Layout**: QGridLayout with single group + spacers

### Structure:
- **Motor** group (`groupBox`)
  - `Motor` binding — **InputVibrationBindingWidget** (not InputBindingWidget!)

### Special Features:
- **Simplest device**: only 1 binding
- **Vibration-only**: uses `InputVibrationBindingWidget` (unique — no other device uses this for its primary binding)
- No buttons, no D-Pad, no axes
- Tab order: just `Motor`

---

## 5. ControllerBindingWidget_Jogcon.ui — Namco Jogcon

**Class**: `ControllerBindingWidget_Jogcon`
**Size**: 1232×644
**Layout**: QGridLayout with 3 major zones + center image

### Structure:
- **Left column**: D-Pad (Up/Down/Left/Right) + Large Motor
- **Middle top**: Shoulder row (L1, L2, R1, R2, Select, Start)
- **Middle center**: Jogcon.svg image (400×266, scaled contents)
- **Middle bottom**: Jog Dial (Dial Left, Dial Right)
- **Right column**: Face Buttons (Triangle, Cross, Square, Circle) + Small Motor

### Complete Binding List:

| Group | Bindings |
|-------|----------|
| D-Pad | Up, Down, Left, Right |
| Face | Triangle, Cross, Square, Circle |
| Shoulders | L1, L2, R1, R2 |
| System | Select, Start |
| Jog Dial | DialLeft, DialRight |
| Vibration | LargeMotor, SmallMotor |

### Special Features:
- **Rotary jog dial** (DialLeft/DialRight) — unique to Jogcon
- **Standard DS face buttons** (Triangle/Cross/Square/Circle)
- **Full shoulder set** (L1/L2/R1/R2) — unlike Negcon
- **Dual vibration motors** (LargeMotor + SmallMotor via InputVibrationBindingWidget)
- **Controller image**: Shows Jogcon.svg for visual reference
- Tab order: Up→Down→Left→Right→Triangle→Cross→Square→Circle→L1→L2→R1→R2→DialLeft→DialRight→Select→Start→LargeMotor→SmallMotor

---

## 6. ControllerBindingWidget_Negcon.ui — Namco Negcon

**Class**: `ControllerBindingWidget_Negcon`
**Size**: 1232×644
**Layout**: QGridLayout with 3 major zones + center image

### Structure:
- **Left column**: D-Pad (Up/Down/Left/Right) + Large Motor
- **Middle top**: Shoulder row (L, Start, R)
- **Middle center**: Negcon.svg image (400×266, scaled contents)
- **Middle bottom**: Twist (Twist Left, Twist Right)
- **Right column**: Face Buttons (I, II, A, B) + Small Motor

### Complete Binding List:

| Group | Bindings |
|-------|----------|
| D-Pad | Up, Down, Left, Right |
| Face | **I, II, A, B** (Roman numerals, NOT standard) |
| Shoulders | **L, R** (only 1 each side, no L2/R2) |
| System | Start (no Select) |
| Twist | TwistLeft, TwistRight |
| Vibration | LargeMotor, SmallMotor |

### Special Features:
- **Twist motion** (TwistLeft/TwistRight) — unique twisting grip
- **Roman numeral face buttons** (I, II, A, B) — different from standard naming
- **Single shoulder buttons** (L, R instead of L1/L2/R1/R2)
- **No Select button** — only Start (unusual)
- **Dual vibration motors**
- **Controller image**: Shows Negcon.svg
- Tab order: Up→Down→Left→Right→I→II→A→B→L→R→TwistLeft→TwistRight→Start→LargeMotor→SmallMotor

---

## Summary: All USB/Controller Types in PCSX2 (Complete)

| # | Device | .ui File | Inputs | Vibration | Unique Feature |
|---|--------|----------|--------|-----------|----------------|
| 1 | DualShock 2 | ControllerBindingWidget_DualShock2.ui | Standard | ✓ | Full DS2 layout |
| 2 | Guitar Hero | ControllerBindingWidget_Guitar.ui | 5 frets+strum | ✗ | Frets, whammy bar |
| 3 | Jogcon | ControllerBindingWidget_Jogcon.ui | D-Pad+Face+Shoulders+Dial | ✓ | Rotary jog dial |
| 4 | Negcon | ControllerBindingWidget_Negcon.ui | D-Pad+I/II/A/B+Twist | ✓ | Twist motion, Roman face |
| 5 | Pop'n | ControllerBindingWidget_Popn.ui | 9 buttons | ✗ | 9-button layout |
| 6 | Buzz | USBBindingWidget_Buzz.ui | 5 colored×4 players | ✗ | 4 players, colored buttons |
| 7 | DenshaCon | USBBindingWidget_DenshaCon.ui | Train controls | ✗ | Train controller |
| 8 | Driving Force | USBBindingWidget_DrivingForce.ui | Wheel+pedals | ✓ | Force feedback wheel |
| 9 | GT Force | USBBindingWidget_GTForce.ui | Wheel+pedals | ✓ | GT Force wheel |
| 10 | Gametrak | USBBindingWidget_Gametrak.ui | 6 axes+pedal | ✗ | 3D position tracking |
| 11 | GunCon2 | USBBindingWidget_GunCon2.ui | Trigger+Aim | ✗ | Light gun |
| 12 | RealPlay | USBBindingWidget_RealPlay.ui | D-Pad+4btns+Accel | ✗ | Motion accelerometer |
| 13 | RyojouhenCon | USBBindingWidget_RyojouhenCon.ui | Special | ✗ | Ryojouhen controller |
| 14 | ShinkansenCon | USBBindingWidget_ShinkansenCon.ui | Train controls | ✗ | Shinkansen controller |
| 15 | TranceVibrator | USBBindingWidget_TranceVibrator.ui | 1 motor | ✓ | Vibration-only device |

Note: DJ Hero, Drum, Keyboard Mania, SingStar, Sea Mic, EyeToy, MSD — these exist as icons but may not have dedicated binding UIs or use standard bindings.
