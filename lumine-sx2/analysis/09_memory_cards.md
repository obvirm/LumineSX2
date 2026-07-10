# Memory Card Settings — Deep Analysis

Source files analyzed:
- `MemoryCardSettingsWidget.h` / `.cpp`
- `MemoryCardCreateDialog.h` / `.cpp`
- `MemoryCardConvertDialog.h` (referenced)

---

## 1. Card Types

| Type | Enum Value | Extension | Description |
|------|-----------|-----------|-------------|
| File | `MemoryCardType::File` | `.ps2` / `.mcr` | Standard file-based memory card |
| Folder | `MemoryCardType::Folder` | directory | Folder-based card (each save = individual file) |
| None | (eject) | — | Slot disabled / empty |

## 2. Card Sizes (File Type)

| Size | Enum | Notes |
|------|------|-------|
| PS2 8MB | `MemoryCardFileType::PS2_8MB` | Default, most compatible |
| PS2 16MB | `MemoryCardFileType::PS2_16MB` | May have compatibility issues |
| PS2 32MB | `MemoryCardFileType::PS2_32MB` | May have compatibility issues |
| PS2 64MB | `MemoryCardFileType::PS2_64MB` | May have compatibility issues |
| PS1 128KB | `MemoryCardFileType::PS1` | Uses `.mcr` extension |

Capacity bytes (from `CardCapacity` namespace):
- 8MB: `0x1f40 * 512 * 2` = 8,028,160 bytes
- 16MB: `0x3e80 * 512 * 2` = 16,056,320 bytes
- 32MB: `0x7d00 * 512 * 2` = 32,112,640 bytes
- 64MB: `0xfde8 * 512 * 2` = 64,507,904 bytes

## 3. Slot System

- **MAX_SLOTS = 2** (Port 1 and Port 2)
- Each slot has: `Slot{n}_Enable` (bool), `Slot{n}_Filename` (string)
- Config section: `[MemoryCards]`
- Default folder: `data_dir/memcards`

### Slot Widget Structure
```
SlotGroup {
    root: QWidget
    enable: QCheckBox ("Slot 1" / "Slot 2")
    eject: QToolButton (eject icon, or delete-back icon for per-game)
    slot: MemoryCardSlotWidget (drag-drop target)
}
```

## 4. Card List Widget

A `QTreeWidget` showing all available cards with columns:
1. **Card Name** (text)
2. **Size/Type** (PS2 8MB, PS2 16MB, PS2 Folder, PS1 128KB, etc.)
3. **Formatted** (Yes/No)
4. **Modified Date** (short datetime format)

Card icons: `memcard-line` for file cards, `folder-open-line` for folder cards.
Disabled (greyed out) if currently assigned to a slot.

## 5. Features

### 5.1 Create Card
- Dialog: `MemoryCardCreateDialog`
- Fields: Name (LineEdit, dots auto-stripped), Type radio buttons (8/16/32/64MB, 128KB, Folder)
- On Windows: NTFS compression checkbox (only for File type)
- Validation: name must be non-empty, valid filename chars, no duplicate names
- Auto-appends `.ps2` or `.mcr` extension
- `FileMcd_CreateNewCard()` backend call
- RestoreDefaults button resets to 8MB File type

### 5.2 Delete Card
- Confirmation dialog: "This action cannot be reversed"
- `FileMcd_DeleteCard()` backend call
- Only available when card is selected

### 5.3 Rename Card
- `QInputDialog::getText()` for new name
- Must end with `.ps2`
- Must not conflict with existing card names
- `FileMcd_RenameCard()` backend call

### 5.4 Convert Card
- Opens `MemoryCardConvertDialog`
- Target types: 8MB, 16MB, 32MB, 64MB, Folder
- Only works on formatted cards (error if unformatted)
- Cannot convert PS1 cards (button disabled)
- Threaded conversion with progress bar
- `MemoryCardConvertWorker` runs in background thread
- Status updates and progress callbacks
- Cancel support

### 5.5 Swap Cards
- Swaps Port 1 ↔ Port 2 assignments
- Both slots must have a card (error if either empty)
- Swaps `Slot1_Filename` ↔ `Slot2_Filename` in config

### 5.6 Eject / Reset
- **Normal mode**: Ejects card → sets slot filename to `""`
- **Per-game mode**: "Reset" icon → removes per-game override (sets to `nullopt`, inheriting global)
- Eject button icon changes: `eject-line` (normal) vs `delete-back-2-line` (per-game)

### 5.7 Drag & Drop
- `MemoryCardListWidget` supports drag (mouse press + move → `QDrag` with card name as text)
- `MemoryCardSlotWidget` accepts drops (`setAcceptDrops(true)`)
- On drop: validates card exists in available list, assigns to slot
- Error dialog if card not recognized

### 5.8 Right-Click Context Menu
- "Use for Slot 1" / "Use for Slot 2" (assign selected card)
- "Rename"
- "Convert"
- "Delete"
- Separator
- "Create"

### 5.9 Per-Game Overrides
- In per-game mode, slot shows inherited cards in **italic + greyed text**
- `inherited` flag from `dialog()->containsSettingValue()`
- Eject button becomes "Reset" (removes per-game override)

### 5.10 Memory Card Directory
- Folder setting: `[Folders] MemoryCards = <path>`
- Browse / Open / Reset buttons
- Default: `data_dir/memcards`
- Changing directory triggers `refresh()`

### 5.11 Card List Display
- `refresh()` calls `FileMcd_GetAvailableCards(true)`
- Shows all `.ps2` and folder cards in the memory cards directory
- Columns auto-sized: [-1, 100, 80, 150]
- Cards currently in use are **disabled** (greyed out, not selectable)

## 6. Hidden / Advanced Features

### 6.1 NTFS Compression (Windows only)
- In `MemoryCardCreateDialog`: checkbox for NTFS compression
- Applied after card creation via `FileSystem::SetPathCompression()`
- Only available for File type cards
- Removed entirely on non-Windows builds

### 6.2 Missing Card Detection
- If a slot references a card file that doesn't exist → shows "[Missing]" with close-line icon
- `FileMcd_GetCardInfo()` returns nullopt for missing cards

### 6.3 Formatted Status
- Cards track whether they are formatted
- Cannot convert unformatted cards
- Stored in `AvailableMcdInfo.formatted`

### 6.4 PS1 Card Support
- PS1 cards use `.mcr` extension
- 128KB size
- Cannot be converted to other formats

## 7. Config Keys

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `[MemoryCards] Slot1_Enable` | bool | true | Enable Port 1 |
| `[MemoryCards] Slot2_Enable` | bool | true | Enable Port 2 |
| `[MemoryCards] Slot1_Filename` | string | "Mcd001.ps2" | Port 1 card file |
| `[MemoryCards] Slot2_Filename` | string | "Mcd002.ps2" | Port 2 card file |
| `[Folders] MemoryCards` | string | "data_dir/memcards" | Card directory |

## 8. Missing Features (Not in LumineSX2 yet)

- [ ] Card list with 4 columns (Name, Size, Formatted, Modified Date)
- [ ] Drag from card list to slot
- [ ] Right-click context menu (Use for Slot, Rename, Convert, Delete, Create)
- [ ] NTFS compression option (Windows-only)
- [ ] Missing card detection with "[Missing]" label
- [ ] Inherited card display in per-game mode (italic + grey)
- [ ] Per-game eject vs global eject (different icons)
- [ ] Card convert dialog with threaded progress
- [ ] PS1 (.mcr) card support
- [ ] Card formatted status tracking
- [ ] Folder-based memory cards
