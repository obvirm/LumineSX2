# PCSX2 Qt Deep Analysis: Themes & Translations System

## Files Analyzed
- `pcsx2-qt/Themes.cpp` — Theme/style engine
- `pcsx2-qt/Translations.cpp` — i18n, locale, font system

---

## 1. THEMES SYSTEM

### Config Key
- `UI/Theme` (string), default: `"darkfusionblue"` (non-Apple), `""` (Apple/macOS)

### All 18 Themes

| # | Theme ID | Display Name | Color Scheme | Primary Author | Notes |
|---|----------|-------------|--------------|----------------|-------|
| 1 | `"fusion"` | Fusion | Light (Unknown) | Qt built-in | Standard Fusion, no custom palette |
| 2 | `"windowsvista"` | Windows Vista | Unknown | Qt built-in | **Windows only**, native OS style |
| 3 | `"darkfusion"` | Dark Fusion | Dark | Community | Gray highlights, blue links |
| 4 | `"darkfusionblue"` | Dark Fusion Blue | **Dark (DEFAULT)** | Community | Blue accent (#0058D0), blue highlights |
| 5 | `"GreyMatter"` | Grey Matter | Dark | KamFretoZ | Sleek gray, easy on eyes |
| 6 | `"UntouchedLagoon"` | Untouched Lagoon | **Light** | RedDevilus | Washed green/teal |
| 7 | `"BabyPastel"` | Baby Pastel | **Light** | RedDevilus | Pink/pastel |
| 8 | `"PizzaBrown"` | Pizza Brown | **Light** | KamFretoZ | Latte/brown, Pizza Tower ref |
| 9 | `"PCSX2Blue"` | PCSX2 Blue | **Light** | RedDevilus | White + blue |
| 10 | `"ScarletDevilRed"` | Scarlet Devil Red | Dark | RedDevilus | Red/purple |
| 11 | `"VioletAngelPurple"` | Violet Angel Purple | Dark | RedDevilus | Deep purple |
| 12 | `"CobaltSky"` | Cobalt Sky | Dark | KamFretoZ | Deep royal blue |
| 13 | `"AMOLED"` | AMOLED | Dark | KamFretoZ | Pure black (#050505), OLED optimized |
| 14 | `"Ruby"` | Ruby | Dark | Daisouji | Black + red (#AC151F) |
| 15 | `"Sapphire"` | Sapphire | Dark | RedDevilus | Black + Persian blue (#2023CC) |
| 16 | `"Emerald"` | Emerald | Dark | RedDevilus | Black + evergreen (#0F513B) |
| 17 | `"Custom"` | Custom QSS | Unknown (user-defined) | User | Loads `custom.qss` from DataRoot |
| 18 | (empty/fallback) | System Default | Unknown | OS | Restores unthemed OS palette |

### Dark Themes (10): darkfusion, darkfusionblue, GreyMatter, ScarletDevilRed, VioletAngelPurple, CobaltSky, AMOLED, Ruby, Sapphire, Emerald
### Light Themes (5): fusion, UntouchedLagoon, BabyPastel, PizzaBrown, PCSX2Blue
### Neutral/OS-dependent (3): windowsvista, Custom, system default

### Theme Color Palettes (Key Colors)

#### darkfusionblue (DEFAULT)
| Role | Color |
|------|-------|
| Window | #353535 (dark gray) |
| WindowText | #FFFFFF |
| Base | #191919 (near black) |
| Button | #353535 |
| Highlight | **#0058D0** (blue) |
| HighlightedText | #FFFFFF |
| Link | #C6EEFF (light blue) |
| Disabled Text | #808080 |

#### darkfusion
| Role | Color |
|------|-------|
| Window | #353535 |
| Base | #191919 |
| Highlight | #4B4B4B (lighter gray) |
| Link | #C6EEFF |

#### GreyMatter
| Role | Color |
|------|-------|
| Window | #2E3440 |
| Base | #3B4252 |
| Highlight | lighter(#3B4252) |
| Link | #C6EEFF |

#### AMOLED
| Role | Color |
|------|-------|
| Window | #050505 (near-black) |
| Base | #16161D |
| Button | #16161D |
| Highlight | #414F62 |
| Link | #C6EEFF |

#### Ruby
| Role | Color |
|------|-------|
| Window/Base | #121212 (slate) |
| Highlight | #AC151F (ruby red) |
| Link | #FFFFFF |

#### Sapphire
| Role | Color |
|------|-------|
| Window/Base | #121212 |
| Highlight | #2023CC (Persian blue) |
| Link | #FFFFFF |

#### Emerald
| Role | Color |
|------|-------|
| Window/Base | #121212 |
| Highlight | #0F513B (evergreen) |
| Link | #FFFFFF |

#### CobaltSky
| Role | Color |
|------|-------|
| Window | #273A72 |
| Base | #1D2951 |
| Button | #192082 |
| Highlight | lighter(#273A72) |

### Theme Engine Internals

**Key Functions:**
- `QtHost::UpdateApplicationTheme()` — Entry point, saves unthemed state on first call, then applies theme
- `QtHost::SetStyleFromSettings()` — Main theme switch (if/else chain)
- `QtHost::SetColorScheme(Qt::ColorScheme)` — Sets OS color scheme hint
- `QtHost::IsDarkApplicationTheme()` — Returns true if current theme is dark
- `QtHost::SetIconThemeFromStyle()` — Sets "white" or "black" icon theme
- `QtHost::GetDefaultThemeName()` — Returns "darkfusionblue" (non-Apple), "" (Apple)

**Hidden Features:**
1. **QPixmapCache.clear()** — Clears Qt's icon tinting cache after theme change. Qt caches tinted icons with keys that don't include the theme, causing wrong-tinted icons to persist. This is a deliberate workaround.
2. **Unthemed state saving** — First call to `UpdateApplicationTheme()` saves the original OS style name and palette so they can be restored for non-custom themes.
3. **Custom QSS loading** — Reads `custom.qss` from `EmuFolders::DataRoot`. Falls back to plain Fusion if file not found.
4. **High contrast awareness** — `windowsvista` theme explicitly avoids setting `Qt::ColorScheme::Light` because it breaks Windows high contrast mode.
5. **Palette heuristic for dark detection** — If theme doesn't set explicit color scheme, `IsDarkApplicationTheme()` compares `windowText.value() > window.value()` (text lighter than background = dark theme).
6. **All custom themes use Fusion base** — Every non-system theme calls `QStyleFactory::create("Fusion")` then overrides palette. No custom QStyle subclasses.
7. **Stylesheet cleared on every theme change** — `qApp->setStyleSheet(QString())` is called for every theme except Custom, preventing style leak.

### Slint/Material You Mapping Notes
- Slint doesn't have QPalette concept — use Material You color tokens instead
- Dark/light mode maps directly to Slint's color scheme
- 18 themes → can be reduced to 4-5 Material You schemes (dark, light, AMOLED, accent-color)
- Custom QSS support has no Slint equivalent (would need runtime style injection)

---

## 2. TRANSLATIONS / i18n SYSTEM

### Config Key
- `UI/Language` (string), default: `"system"` (use OS locale)

### Language Detection Flow
1. Read `UI/Language` setting
2. If `"system"`, call `getSystemLanguage()`
3. `getSystemLanguage()`: Exact locale match → partial language match → fallback to `"en-US"`
4. Load Qt base translation (`qt_XX.qm`) then PCSX2 translation (`pcsx2-qt_XX.qm`)
5. Update glyph ranges for OSD/ImGui font rendering
6. Notify FullscreenUI of locale change

### All 39 Available Languages

| # | Locale Code | Display Name |
|---|-------------|-------------|
| 0 | `"system"` | System Language [Default] |
| 1 | `af-ZA` | Afrikaans |
| 2 | `ar-SA` | عربي (Arabic) |
| 3 | `az-AZ` | Azərbaycanca |
| 4 | `ca-ES` | Català |
| 5 | `cs-CZ` | Čeština |
| 6 | `da-DK` | Dansk |
| 7 | `de-DE` | Deutsch |
| 8 | `el-GR` | Ελληνικά (Greek) |
| 9 | `en-US` | English (en) |
| 10 | `es-419` | Español (Hispanoamérica) |
| 11 | `es-ES` | Español (España) |
| 12 | `fa-IR` | فارسی (Persian) |
| 13 | `fi-FI` | Suomi |
| 14 | `fr-FR` | Français |
| 15 | `he-IL` | עִבְרִית (Hebrew) |
| 16 | `hi-IN` | मानक हिन्दी (Hindi) |
| 17 | `hu-HU` | Magyar |
| 18 | `hr-HR` | hrvatski |
| 19 | `id-ID` | Bahasa Indonesia |
| 20 | `it-IT` | Italiano |
| 21 | `ja-JP` | 日本語 |
| 22 | `ko-KR` | 한국어 |
| 23 | `lv-LV` | Latvija |
| 24 | `lt-LT` | Lietuvių |
| 25 | `nl-NL` | Nederlands |
| 26 | `no-NO` | Norsk |
| 27 | `pl-PL` | Polski |
| 28 | `pt-BR` | Português (Brasil) |
| 29 | `pt-PT` | Português (Portugal) |
| 30 | `ro-RO` | Limba română |
| 31 | `ru-RU` | Русский |
| 32 | `sr-SP` | Српски (Serbian) |
| 33 | `sv-SE` | Svenska |
| 34 | `tr-TR` | Türkçe |
| 35 | `uk-UA` | Українська |
| 36 | `vi-VN` | Tiếng Việt |
| 37 | `zh-CN` | 简体中文 (Simplified Chinese) |
| 38 | `zh-TW` | 繁體中文 (Traditional Chinese) |

### Translation File Locations
- Qt base: `{app_dir}/translations/qt_{lang}.qm`
- PCSX2: `{app_dir}/translations/pcsx2-qt_{lang}.qm`
- macOS: `{app_dir}/../Resources/translations/`
- Linux w/ datadir: `{app_dir}/{PCSX2_APP_DATADIR}/translations/`

### Runtime Language Switching
- `QtHost::InstallTranslator()` — Removes old translators, loads new ones, updates collator
- Updates `s_current_locale` and `s_current_collator` for locale-sensitive sorting
- Calls `UpdateGlyphRangesAndClearCache()` to reload OSD fonts
- Notifies `FullscreenUI::LocaleChanged()` via GS thread

### Hidden Features in Translations
1. **`LocaleCircleConfirm()`** — Returns true for Japanese, Chinese, Korean locales. These cultures use O (circle) as confirm button instead of X. Affects controller button mapping in UI.
2. **`LocaleSensitiveCompare()`** — Locale-aware string sorting using QCollator (used for game list sorting)
3. **Plural forms** — `Host::TranslatePluralToString()` supports Qt's plural translation mechanism
4. **Translation cache** — `Host::ClearTranslationCache()` called after font changes
5. **Missing font auto-download** — Prompts user to download font files if not found locally
6. **Secondary script loading** — Chinese Simplified automatically loads Traditional as fallback (and vice versa)

---

## 3. FONT SYSTEM (for OSD / Big Picture / ImGui)

### 9 Font Scripts

| Script | Enum Value | Primary Font File | System Fallbacks |
|--------|-----------|-------------------|-----------------|
| Arabic | 0 | NotoSansArabic-Regular.ttf | Segoeui.ttf, Geeza Pro |
| ChineseSimplified | 1 | NotoSansSC-Regular.ttf | Msyh.ttc, Heiti SC |
| ChineseTraditional | 2 | NotoSansTC-Regular.ttf | Msjh.ttc, Heiti TC |
| Devanagari | 3 | NotoSansDevanagari-Regular.ttf | Nirmala.ttf, Kohinoor Devanagari |
| Emoji | 4 | Twemoji.Mozilla.ttf | Seguiemj.ttf, Twemoji Mozilla |
| Hebrew | 5 | NotoSansHebrew-Regular.ttf | Segoeui.ttf, Arial Hebrew |
| Japanese | 6 | NotoSansJP-Regular.ttf | YuGothR.ttc, Meiryo.ttc, Hiragino Sans |
| Korean | 7 | NotoSansKR-Regular.ttf | Malgun.ttf, Apple SD Gothic Neo |
| Latin | 8 | Roboto-Regular.ttf | (none, shipped with app) |

### Font Loading Priority
1. User resources: `{EmuFolders::UserResources}/fonts/{filename}`
2. Shipped: `{EmuFolders::GetOverridableResourcePath}/fonts/{filename}`
3. System (Windows): `C:\Windows\Fonts\{filename}`
4. System (macOS): CoreText face name lookup
5. System (Linux): Fontconfig face name lookup

### Language-to-FontScript Mapping
| Language | Font Script |
|----------|------------|
| ar-SA | Arabic |
| fa-IR | Arabic |
| hi-IN | Devanagari |
| he-IL | Hebrew |
| ja-JP | Japanese |
| ko-KR | Korean |
| zh-CN | ChineseSimplified |
| zh-TW | ChineseTraditional |
| All others | Latin |

### Font Features
- **Auto-download**: If primary font missing, prompts user to download from runtime URL
- **Fallback chain**: Primary → secondary (CN↔TW only) → all other scripts
- **Latin always included**: Even for non-Latin languages
- **Ellipsis handling**: CJK languages use their own centered ellipsis; Latin ellipsis excluded for these
- **Emoji font**: Always loaded for all languages
- **Font validation**: `ValidateFont()` exists but currently returns true unconditionally

### Hidden Features in Font System
1. **Cross-script fallback**: If you load Japanese, you also get Chinese Simplified, Traditional, Korean, Arabic, Hebrew, Devanagari, and Emoji fonts as fallbacks
2. **Font file override**: Users can place fonts in `{UserResources}/fonts/` to override shipped ones
3. **Face name vs file name**: FontLoadInfo has both — file_name for bundled/downloaded, face_name for system lookup
4. **GS thread font updates**: Font changes are dispatched to the GS thread if emu_thread is running
5. **Collator mutex**: Locale-sensitive string comparison is mutex-protected for thread safety

---

## Summary for LumineSX2 UI Implementation

### Must-Have Features
- [ ] 18 theme options with proper dark/light classification
- [ ] Theme switching with palette application
- [ ] Icon theme switching (dark=white icons, light=black icons)
- [ ] Custom QSS loading support
- [ ] 39 language options with system default
- [ ] Runtime language switching
- [ ] Locale-aware string sorting
- [ ] Circle-confirm for CJK locales
- [ ] Font script detection and fallback
- [ ] Missing font download prompt

### Slint-Specific Considerations
- No QPalette — use Slint's color scheme globals or Material You tokens
- No QSS — use Slint's native styling
- No QTranslator — need custom i18n (e.g., fluent, rust-i18n, or custom string table)
- Font loading: Slint uses its own font system, not Qt's
- 18 themes → can simplify to dark/light/AMOLED + accent color picker
