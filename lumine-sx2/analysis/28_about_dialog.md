# 28 — AboutDialog

## Source Files
- `pcsx2-qt/AboutDialog.h`
- `pcsx2-qt/AboutDialog.cpp`
- `pcsx2-qt/AboutDialog.ui`
- `pcsx2/SupportURLs.h`

## Purpose
Modal "About PCSX2" dialog showing version info, description, legal disclaimer, and hyperlinks to project resources.

## UI Layout (from .ui file)
Fixed size: 580×320 pixels.

| Element | Widget | Name | Content |
|---------|--------|------|---------|
| PCSX2 Logo | QLabel | `icon` | `:/icons/PCSX2logo.svg` (from QRC) |
| Version String | QLabel | `scmversion` | Set at runtime → `QtHost::GetAppNameAndVersion()` |
| Description | QLabel | `description` | HTML: "PCSX2 is a free and open-source PlayStation 2 (PS2) emulator..." |
| Legal Disclaimer | QLabel | `disclaimer` | **Bold**: "PlayStation 2 and PS2 are registered trademarks of Sony Interactive Entertainment. This application is not affiliated in any way with Sony Interactive Entertainment." |
| Resource Links | QLabel (QTextBrowser interaction) | `links` | 5 clickable links (see below) |
| Close Button | QDialogButtonBox | `buttonBox` | Standard Close button, centered |

## Features

### 1. Version Display
- `scmversion` label: `Qt::TextSelectableByMouse` — user can select and copy version text
- Value: `QtHost::GetAppNameAndVersion()` — returns app name + version string (e.g. "PCSX2 v2.0.0")

### 2. Hyperlinks (5 links)
All rendered as a single HTML string with pipe separators:

| Link | URL Source | Static Method |
|------|-----------|---------------|
| Website | `PCSX2_WEBSITE_URL` = `https://pcsx2.net/` | `getWebsiteUrl()` |
| Support Forums | `PCSX2_FORUMS_URL` = `https://forums.pcsx2.net/` | `getSupportForumsUrl()` |
| GitHub Repository | `PCSX2_GITHUB_URL` = `https://github.com/PCSX2/pcsx2` | `getGitHubRepositoryUrl()` |
| License | Local file `docs/GPL.html` | `getLicenseUrl()` |
| Third-Party Licenses | Local file `docs/ThirdPartyLicenses.html` | `getThirdPartyLicensesUrl()` |

### 3. Link Handling (`linksLinkActivated`)
- External URLs → `QDesktopServices::openUrl()` (opens in default browser)
- Local file URLs → `showHTMLDialog()` (in-app HTML viewer)

### 4. In-App HTML Viewer (`showHTMLDialog`)
- Static method, reusable
- Creates a modal QDialog (min 700×400)
- Contains `QTextBrowser` (rich text, read-only, external links enabled)
- Loads local HTML files (GPL.html, ThirdPartyLicenses.html)
- Shows "File not found" error if file missing
- Close button via `QDialogButtonBox`
- Used by: License link, Third-Party Licenses link

### 5. Additional URL Methods (not shown in dialog but available)
These static methods exist for use elsewhere in the app:

| Method | URL |
|--------|-----|
| `getWikiUrl()` | `https://wiki.pcsx2.net/Main_Page` |
| `getDocumentationUrl()` | `https://pcsx2.net/docs` |
| `getDiscordServerUrl()` | `https://pcsx2.net/discord` |

### 6. Legal Disclaimer
Always displayed (not conditional):
> "PlayStation 2 and PS2 are registered trademarks of Sony Interactive Entertainment. This application is not affiliated in any way with Sony Interactive Entertainment."

## Signals/Callbacks
| Signal | Trigger | Action |
|--------|---------|--------|
| `linksLinkActivated(QString)` | Click any link label | Open URL (browser or in-app HTML dialog) |
| `buttonBox.rejected` | Click Close | `QDialog::close()` |

## Constructor Behavior
1. `setupUi(this)` — load .ui layout
2. Remove `Qt::WindowContextHelpButtonHint` (no ? button)
3. `setFixedSize()` — lock dialog size to layout geometry
4. Set version text (selectable by mouse)
5. Set links HTML with 5 hyperlinks
6. Connect link activation signal
7. Connect close button

## LumineSX2 Mapping Notes
For Slint UI, this is a simple "About" page/dialog:
- Show logo/image
- Show version string (selectable/copyable)
- Show description paragraph
- Show legal disclaimer (bold)
- 5 clickable links (open in browser)
- For License/ThirdPartyLicenses: could open in-app web view or just link to GitHub
- Close/Back button
- Fixed dialog or full-screen page on mobile

Hidden features: The `showHTMLDialog` is a **reusable utility** — any part of the app can show local HTML docs in a dialog. The Discord/Wiki/Documentation URLs are exposed as static methods even though they're not shown in the About dialog UI itself.
