# Achievement Settings — PCSX2 Qt Deep Analysis

## Files Analyzed
- `pcsx2-qt/Settings/AchievementSettingsWidget.h`
- `pcsx2-qt/Settings/AchievementSettingsWidget.cpp`
- `pcsx2-qt/Settings/AchievementSettingsWidget.ui`
- `pcsx2-qt/Settings/AchievementLoginDialog.h`
- `pcsx2-qt/Settings/AchievementLoginDialog.cpp`
- `pcsx2/Achievements.h` (LoginRequestReason enum)
- `pcsx2/Config.h` (AchievementsOptions, AchievementOverlayPosition)

---

## UI Structure (from .ui file)

### Layout: QVBoxLayout (single-column, top-to-bottom)
1. **rcheevosDisclaimer** (QLabel) — HTML link to retroachievements.org + instructions to access in-game via Pause Menu → Achievements
2. **loginBox** (QGroupBox "Account") — QHBoxLayout
   - loginStatus (QLabel) — "Username: ... Login token generated on ..."
   - viewProfile (QPushButton "View Profile...")
   - loginButton (QPushButton "Login...")
3. **gameInfoBox** (QGroupBox "Game Info") — min-height 75px
   - gameInfo (QLabel) — achievement/game info text
4. **settingsBox** (QGroupBox "Settings") — QGridLayout 2-col
   - Row 0: enable, hardcoreMode
   - Row 2: spectatorMode, encoreMode
   - Row 3: unofficialAchievements
5. **notificationBox** (QGroupBox "Notifications") — QGridLayout 2-col
   - achievementNotifications + duration slider (3–30s, default 5)
   - leaderboardNotifications + duration slider (3–30s, default 5)
   - soundEffects
   - notificationPosition (QComboBox, 10 positions: None/TL/TC/TR/CL/C/CR/BL/BC/BR)
6. **overlaySettingsBox** (QGroupBox "Overlay Settings") — QGridLayout 2-col
   - overlays (Enable In-Game Overlays)
   - leaderboardOverlays (Enable In-Game Leaderboard Overlays)
   - overlayPosition (QComboBox, 9 positions: TL/TC/TR/CL/C/CR/BL/BC/BR — default BottomRight)
7. **soundEffectsBox** (QGroupBox "Sound Effects") — QGridLayout
   - notificationSound (checkbox) + notificationSoundPath + Browse + Preview + Reset
   - unlockSound (checkbox) + unlockSoundPath + Browse + Preview + Reset
   - lbSound (checkbox) + lbSoundPath + Browse + Preview + Reset

---

## Settings Keys (INI [Achievements] section)

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| Enabled | bool | false | Enable achievements tracking |
| ChallengeMode | bool | false | Hardcore mode |
| Notifications | bool | true | Show achievement unlock notifications |
| LeaderboardNotifications | bool | true | Show leaderboard notifications |
| SoundEffects | bool | true | Enable sound effects globally |
| InfoSound | bool | true | Notification sound enabled |
| UnlockSound | bool | true | Achievement unlock sound enabled |
| LBSubmitSound | bool | true | Leaderboard submit sound enabled |
| Overlays | bool | true | In-game overlay icons |
| LBOverlays | bool | true | Leaderboard overlay icons |
| OverlayPosition | int (enum) | BottomRight | 9-position grid |
| NotificationPosition | int (enum) | TopLeft | 10-position grid (includes "None") |
| EncoreMode | bool | false | Each session as-if no achievements unlocked |
| SpectatorMode | bool | false | Don't send unlocks to server |
| UnofficialTestMode | bool | false | Show unofficial achievement sets |
| NotificationsDuration | int | 5 | Seconds (3–30) |
| LeaderboardsDuration | int | 10 | Seconds (3–30) |
| InfoSoundName | string | "sounds/achievements/message.wav" | Custom notification sound |
| UnlockSoundName | string | "sounds/achievements/unlock.wav" | Custom unlock sound |
| LBSubmitSoundName | string | "sounds/achievements/lbsubmit.wav" | Custom LB submit sound |
| Username | string | — | Stored username |
| LoginTimestamp | string (u64) | — | Unix timestamp of login |

---

## Login Flow (AchievementLoginDialog)

### LoginRequestReason enum
- **UserInitiated** — user clicked Login button
- **TokenInvalid** — stored token expired, re-auth required

### Login Dialog UI
- RA icon (svg, 50x50)
- instructionText (QLabel) — changes text if TokenInvalid
- userName (QLineEdit)
- password (QLineEdit)
- status (QLabel) — shows "Logging in..." / success / error
- Buttons: Login (disabled until both fields filled), Cancel

### Post-login behavior
- If UserInitiated: prompts to **Enable Achievements** if not enabled
- If UserInitiated: prompts to **Enable Hardcore Mode** if not enabled
  - If game active: prompts to **Reset System** for hardcore to take effect
- If TokenInvalid + Cancel: disables hardcore mode if no active game

---

## Enable State Cascade (updateEnableState)

```
enabled ← "Achievements/Enabled"
  ├── hardcoreMode ← enabled
  ├── achievementNotifications ← enabled
  ├── leaderboardNotifications ← enabled
  ├── achievementNotificationsDuration ← (enabled && notifications)
  ├── leaderboardNotificationsDuration ← (enabled && lb_notifications)
  ├── soundEffects ← enabled
  ├── overlays ← enabled
  ├── leaderboardOverlays ← enabled
  ├── overlaySettingsBox ← enabled && (overlays || lb_overlays)
  ├── overlayPosition ← overlays_enabled
  ├── notificationPosition ← notifications_enabled
  ├── encoreMode ← enabled
  ├── spectatorMode ← enabled
  └── unofficialAchievements ← enabled

sound ← "Achievements/SoundEffects"
  ├── notificationSound ← enabled
  ├── notificationSoundPath/Browse/Open/Reset ← (enabled && sound && info)
  ├── unlockSound ← enabled
  ├── unlockSoundPath/Browse/Open/Reset ← (enabled && sound && unlock)
  ├── lbSound ← enabled
  └── lbSoundPath/Browse/Open/Reset ← (enabled && sound && lbsound)
```

---

## Hidden/Non-Obvious Features

1. **Per-game settings mode**: loginBox, gameInfoBox, and soundEffectsBox are HIDDEN in per-game context
2. **Hardcore mode prompts**: Changing hardcore mode while game running prompts for system reset
3. **Token invalidation flow**: When token expires, dialog appears with different message + Cancel disables hardcore
4. **Notification duration sliders**: 3–30 second range, with label showing current value
5. **Sound customization**: Each of 3 sound types (info/unlock/lbsubmit) has independent enable + custom WAV path + Browse + Preview + Reset buttons
6. **Notification position "None"**: Overlay position has 9 options, but notification position has 10 (includes "None")
7. **View Profile button**: Opens `https://retroachievements.org/user/{encoded_username}` in browser
8. **Encore mode**: Resets achievement state each session (replayability)
9. **Spectator mode**: Tracks locally but never sends to server
10. **Game Info box**: Shows live achievement info for current game (refreshed on `onAchievementsRefreshed` signal)
11. **Default sound paths**: Resource-relative (`sounds/achievements/message.wav`, `unlock.wav`, `lbsubmit.wav`)
12. **Notification sound file filter**: WAV only (`WAV Audio Files (*.wav)`)
