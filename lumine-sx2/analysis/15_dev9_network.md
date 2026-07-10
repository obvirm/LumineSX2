# DEV9 / Network Settings — Deep Analysis

## Files Analyzed
- `pcsx2-qt/Settings/DEV9SettingsWidget.h`
- `pcsx2-qt/Settings/DEV9SettingsWidget.cpp`

## Dependencies
- `DEV9UiCommon.h` — HostEntryUi, AdapterEntry, AdapterOptions, IPValidator, IPItemDelegate
- `DEV9DnsHostDialog.h` — DNS host export/import dialog
- `DEV9/net.h` — Pcsx2Config::DEV9Options
- `DEV9/pcap_io.h` — PCAPAdapter
- `DEV9/Win32/tap.h` — TAPAdapter (Windows only)
- `DEV9/sockets.h` — SocketAdapter
- `HddCreateQt.h` — HDD image creation
- `QStandardItemModel`, `QSortFilterProxyModel` — host table

---

## 1. Ethernet (Network Adapter) Settings

### Enable/Disable
- Setting: `DEV9/Eth` → `EthEnable` (bool, default: false)
- Widget: `m_ui.ethEnabled` (QCheckBox)
- Signal: `onEthEnabledChanged(Qt::CheckState)` — toggles all ethernet UI

### Network API Selection
- Widget: `m_ui.ethDevType` (QComboBox)
- API types (enum `NetApi`):
  | Index | Name | Description |
  |-------|------|-------------|
  | 0 | Unset | Global default |
  | 1 | PCAP Bridged | pcap bridged mode |
  | 2 | PCAP Switched | pcap switched mode |
  | 3 | TAP | TAP adapter (Windows only) |
  | 4 | Sockets | Socket-based networking |

### Adapter Selection
- Widget: `m_ui.ethDev` (QComboBox)
- Setting: `DEV9/Eth` → `EthDevice` (string, GUID)
- Setting: `DEV9/Eth` → `EthApi` (string, API name)
- Adapters loaded lazily on first show via `LoadAdapters()`
- Per-game: "Use Global Setting [adapter_name]" option
- Adapters sorted alphabetically within each API group

### Adapter Options (per API)
| API | AdapterOptions flags |
|-----|---------------------|
| TAP (Win32) | `TAPAdapter::GetAdapterOptions()` |
| PCAP Bridged/Switched | `PCAPAdapter::GetAdapterOptions()` |
| Sockets | `SocketAdapter::GetAdapterOptions()` |

Flags:
- `AdapterOptions::None`
- `AdapterOptions::DHCP_ForcedOn` — forces DHCP intercept on, disables toggle
- `AdapterOptions::DHCP_OverrideIP` — disables PS2 IP field
- `AdapterOptions::DHCP_OverideSubnet` — disables subnet mask + auto toggle
- `AdapterOptions::DHCP_OverideGateway` — disables gateway + auto toggle

---

## 2. DHCP Settings

### DHCP Intercept
- Setting: `DEV9/Eth` → `InterceptDHCP` (bool, default: false)
- Widget: `m_ui.ethInterceptDHCP` (QCheckBox)
- When enabled: shows IP configuration fields
- When `DHCP_ForcedOn`: checkbox disabled but treated as enabled

### IP Configuration (5 fields)
| Widget | Setting Key | Default |
|--------|-------------|---------|
| `ethPS2Addr` | `PS2IP` | `0.0.0.0` |
| `ethNetMask` | `Mask` | `0.0.0.0` |
| `ethGatewayAddr` | `Gateway` | `0.0.0.0` |
| `ethDNS1Addr` | `DNS1` | `0.0.0.0` |
| `ethDNS2Addr` | `DNS2` | `0.0.0.0` |

All fields have `IPValidator`. Per-game: empty string = use global (placeholder shown).

### Auto-Settings
| Widget | Setting Key | Default | Controls |
|--------|-------------|---------|----------|
| `ethNetMaskAuto` | `AutoMask` | true | Enables/disables `ethNetMask` |
| `ethGatewayAuto` | `AutoGateway` | true | Enables/disables `ethGatewayAddr` |

### DNS Mode (per DNS server)
| Widget | Setting Key | Modes |
|--------|-------------|-------|
| `ethDNS1Mode` | `ModeDNS1` | Manual / Auto / Internal |
| `ethDNS2Mode` | `ModeDNS2` | Manual / Auto / Internal |

- Default: `Auto`
- `Manual` → enables address field
- `Auto` / `Internal` → disables address field
- Per-game: index 0 = "Use Global Setting"

---

## 3. DNS Host Table

### Table Model
- Columns: Name, Hostname, Address, Enabled
- Model: `QStandardItemModel` (4 columns)
- Proxy: `QSortFilterProxyModel` for sorting/filtering
- Column 2 uses `IPItemDelegate` for IP validation
- Column widths: {-1, 170, 90, 80} (auto-resized on show/resize)

### Host Entry Structure (`HostEntryUi`)
```cpp
struct HostEntryUi {
    std::string Desc;     // Display name
    std::string Url;      // Hostname to intercept
    std::string Address;  // IP to redirect to
    bool Enabled;         // Active flag
};
```

### Config Storage
- Section: `DEV9/Eth/Hosts`
- Key: `Count` (int)
- Per host: `DEV9/Eth/Hosts/Host{N}` with keys `Url`, `Desc`, `Address`, `Enabled`

### Actions
| Button | Slot | Function |
|--------|------|----------|
| `ethHostAdd` | `onEthHostAdd()` | Creates new host entry (Desc="New Host", Enabled=false), selects it |
| `ethHostDel` | `onEthHostDel()` | Deletes selected host, shuffles remaining entries down |
| `ethHostExport` | `onEthHostExport()` | Exports selected hosts to `hosts.ini` via `DEV9DnsHostDialog` |
| `ethHostImport` | `onEthHostImport()` | Imports hosts from `hosts.ini`, shows selection dialog |
| `ethHostPerGame` | `onEthHostPerGame()` | Toggles per-game host list (copy global or start empty) |

### Inline Editing
- `onEthHostEdit(QStandardItem*)` — saves changes on edit
  - Column 0 → `Desc`
  - Column 1 → `Url`
  - Column 2 → `Address`
  - Column 3 → `Enabled` (checkbox)

### Per-Game Host Logic
- If no per-game hosts: shows "Override" button, displays global hosts (read-only)
- If per-game hosts exist: shows "Use Global" button, allows editing
- Per-game host list is completely independent from global

### Import/Export Format
- File: `hosts.ini` (INI format)
- Sections: `Host0`, `Host1`, ...
- Keys: `Url`, `Desc`, `Address`, `Enabled`
- Export: prompts user to select which hosts via `DEV9DnsHostDialog`
- Import: reads all hosts, prompts user to select which to add

---

## 4. HDD (Hard Disk Drive) Settings

### Enable/Disable
- Setting: `DEV9/Hdd` → `HddEnable` (bool, default: false)
- Widget: `m_ui.hddEnabled` (QCheckBox)

### HDD File
- Setting: `DEV9/Hdd` → `HddFile` (string, default: `"DEV9hdd.raw"`)
- Widget: `m_ui.hddFile` (QLineEdit)
- Browse button: `m_ui.hddBrowseFile` → `QFileDialog::getSaveFileName` (*.raw)
- Per-game: empty = use global path

### LBA48 Support
- Widget: `m_ui.hddLBA48` (QCheckBox)
- When checked: max size = 2000 GB, min = 100 GB, tick interval = 100
- When unchecked: max size = 120 GB, min = 40 GB, tick interval = 5
- Auto-detected from existing file: if file > 120GB, auto-checks

### HDD Size
- Widget: `m_ui.hddSizeSlider` (QSlider) + `m_ui.hddSizeSpinBox` (QSpinBox)
- Size in GB
- Bidirectional sync between slider and spinbox (with signal blockers)
- If file exists: reads actual file size and sets UI accordingly

### HDD Creator
- Button: `m_ui.hddCreate` → `onHddCreateClicked()`
- Uses `HddCreateQt` class
- Validates: non-empty path, non-zero size
- Converts relative paths to absolute (based on `EmuFolders::Settings`)
- Prompts for overwrite if file exists
- Shows success/error message

### UI State Logic
- `UpdateHddSizeUIEnabled()`: disables size controls if per-game + empty path
- `UpdateHddSizeUIValues()`: reads existing file size, sets slider/spinbox/LBA48

---

## 5. Event Handling

### Lazy Loading
- Adapters loaded on first `showEvent()` via `onEthEnabledChanged()`
- `m_adaptersLoaded` flag prevents re-loading
- On re-show: reverts API dropdown to saved `EthApi` value

### Event Filter
- Installed on `m_ui.ethHosts` table
- `QEvent::Resize` and `QEvent::Show` → `QtUtils::ResizeColumnsForTableView()`
- Ensures column widths are correct even when widget is nested in hidden tabs

### Per-Game Settings Support
- All IP fields show placeholder text from global settings
- Empty field = use global setting
- Device dropdown: "Use Global Setting [adapter_name]"
- API dropdown: "Use Global Setting [api_name]"
- Host list: separate "Override"/"Use Global" toggle

---

## 6. Settings Keys Summary

### Section: `DEV9/Eth`
| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `EthEnable` | bool | false | Enable ethernet |
| `EthApi` | string | Unset | Network API |
| `EthDevice` | string | "" | Adapter GUID |
| `InterceptDHCP` | bool | false | DHCP interception |
| `PS2IP` | string | 0.0.0.0 | PS2 IP address |
| `Mask` | string | 0.0.0.0 | Subnet mask |
| `Gateway` | string | 0.0.0.0 | Gateway IP |
| `DNS1` | string | 0.0.0.0 | Primary DNS |
| `DNS2` | string | 0.0.0.0 | Secondary DNS |
| `AutoMask` | bool | true | Auto subnet mask |
| `AutoGateway` | bool | true | Auto gateway |
| `ModeDNS1` | enum | Auto | DNS1 mode |
| `ModeDNS2` | enum | Auto | DNS2 mode |

### Section: `DEV9/Eth/Hosts`
| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `Count` | int | 0 | Number of host entries |

### Section: `DEV9/Eth/Hosts/Host{N}`
| Key | Type | Description |
|-----|------|-------------|
| `Url` | string | Hostname to intercept |
| `Desc` | string | Display name |
| `Address` | string | Redirect IP |
| `Enabled` | bool | Active flag |

### Section: `DEV9/Hdd`
| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `HddEnable` | bool | false | Enable HDD |
| `HddFile` | string | DEV9hdd.raw | HDD image path |
| `LBA48` | bool | false | LBA48 support |

---

## 7. Hidden/Advanced Features

1. **Adapter Options Flags** — each API reports which DHCP/IP fields it forces on/off
2. **DHCP_ForcedOn** — some adapters force DHCP intercept (TAP does this)
3. **Per-Game Host List** — completely independent DNS host table per game
4. **Host Import/Export** — portable `hosts.ini` format, with selection dialog
5. **HDD Auto-Detection** — reads existing file size and sets LBA48 + slider automatically
6. **Relative-to-Absolute Path** — HDD paths resolved against `EmuFolders::Settings`
7. **IP Validator** — custom validator for IP fields (supports per-game empty-to-clear)
8. **Lazy Adapter Loading** — adapters enumerated only when ethernet is enabled
9. **Column Auto-Resize** — event filter ensures table columns resize on tab switch
10. **Signal Blocking** — prevents feedback loops when programmatically updating UI
