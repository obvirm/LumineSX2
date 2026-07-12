#!/usr/bin/env python3
# Master generator for settings wiring:
#  1) parse each settings_*.slint -> local prop names + types -> short (prefix+local)
#  2) write settings_map.json
#  3) add 274 short props to main.slint MainWindow (dedup vs existing)
#  4) add two-way bindings at each SettingsView instantiation in main.slint
#  5) write src/settings_bindings.rs (apply + load) from settings_map.json
import os, re, glob, json

UI = r'E:\project\pcsx2\lumine-sx2\ui'
MAIN = os.path.join(UI, 'main.slint')
MAP = r'E:\project\pcsx2\settings_map.json'
RUST = r'E:\project\pcsx2\lumine-sx2\src\settings_bindings.rs'

prefix = {
    'settings_graphics': 'graphics-', 'settings_audio': 'audio-', 'settings_emulation': 'emu-',
    'settings_bios': 'bios-', 'settings_folders': 'folders-', 'settings_controller': 'controller-',
    'settings_interface': 'interface-', 'settings_osd': 'osd-', 'settings_memorycards': 'memcard-',
    'settings_achievements': 'ra-', 'settings_advanced': 'adv-', 'settings_patches': 'patch-',
    'settings_cheats': 'cheats-', 'settings_gamefixes': 'gf-', 'settings_gamelist': 'gamelist-',
}

prop_re = re.compile(r'^(\s*)in-out property <(bool|int|float|string)>\s+([A-Za-z0-9_-]+)\s*(?:<=>[^;]*)?:(.*)$')
skip_model_re = re.compile(r'in-out property <\[')

COMP_RE = re.compile(r'^export\s+component\s+([A-Za-z0-9_]+)\s+inherits')

# Map base -> exact component name to parse (must match what main.slint instantiates)
COMP_NAME = {
    'settings_graphics': 'GraphicsSettingsView',
    'settings_audio': 'AudioSettingsView',
    'settings_emulation': 'EmulationSettingsView',
    'settings_bios': 'BiosSettingsView',
    'settings_folders': 'FoldersSettingsView',
    'settings_controller': 'ControllerSettingsView',
    'settings_interface': 'InterfaceSettingsView',
    'settings_osd': 'OsdSettingsView',
    'settings_memorycards': 'MemoryCardSettingsView',
    'settings_achievements': 'AchievementSettingsView',
    'settings_advanced': 'AdvancedSettingsView',
    'settings_patches': 'PatchesSettingsView',
    'settings_cheats': 'CheatsSettingsView',
    'settings_gamefixes': 'GameFixesSettingsView',
    'settings_gamelist': 'GameListSettingsView',
}
# ---------- 1) parse ----------
data = {}
# BIOS view locals map to EXISTING MainWindow prop names (already wired by main.slint)
BIOS_MAP = {
    'bios-dir': 'bios-dir-path',
    'fast-boot': 'fast-boot',
    'fast-forward-boot': 'fast-forward-boot',
    'selected-bios-idx': 'selected-bios',
    'bios-entries': 'bios-entries',  # model, not scalar
}
for fp in glob.glob(os.path.join(UI, 'settings_*.slint')):
    base = os.path.splitext(os.path.basename(fp))[0]
    pre = prefix.get(base)
    if not pre:
        continue
    src = open(fp, encoding='utf-8').read().split('\n')
    # find the exact `export component <NAME> inherits` block for this view
    comp_name = COMP_NAME.get(base)
    if not comp_name:
        continue
    start = None
    for i, ln in enumerate(src):
        m = re.match(r'^export\s+component\s+' + re.escape(comp_name) + r'\s+inherits', ln.strip())
        if m:
            start = i
            break
    if start is None:
        continue
    # track brace depth from the `{` after the component header
    depth = 0
    in_block = False
    entries = []
    for ln in src[start:]:
        for ch in ln:
            if ch == '{':
                depth += 1
                in_block = True
            elif ch == '}':
                depth -= 1
        if in_block:
            mm = prop_re.match(ln)
            if mm and not skip_model_re.search(ln):
                typ, name = mm.group(2), mm.group(3)
                entries.append((name, typ, name if base == 'settings_bios' else pre + name))
        if in_block and depth <= 0:
            break
    # For BIOS map to existing MainWindow prop names
    if base == 'settings_bios':
        entries = [(n, t, BIOS_MAP.get(n, n)) for (n, t, _s) in entries]
    data[base] = entries

with open(MAP, 'w', encoding='utf-8') as fh:
    json.dump(data, fh, indent=2)
total = sum(len(v) for v in data.values())
print(f"[1] parsed {total} settings props -> {MAP}")
# ---------- 2) add props to main.slint (dedup) ----------
main_lines = open(MAIN, encoding='utf-8').read().split('\n')
existing = set(re.findall(r'in-out property <(?:bool|int|float|string)>\s+([A-Za-z0-9_-]+)\s*[;:]', '\n'.join(main_lines)))
props_to_add = []
seen = set()
for base, entries in data.items():
    for name, typ, short in entries:
        if short in seen:
            continue
        seen.add(short)
        if short in existing:
            continue
        props_to_add.append((typ, short))

anchor = 'in-out property <bool> show-quick-menu: false;'
out = []
inserted = False
for ln in main_lines:
    out.append(ln)
    if not inserted and anchor in ln:
        out.append('')
        out.append('    // === Auto-generated settings properties (bound two-way from settings views) ===')
        for typ, short in props_to_add:
            out.append(f'    in-out property <{typ}> {short};')
        inserted = True
open(MAIN, 'w', encoding='utf-8').write('\n'.join(out))
print(f"[2] added {len(props_to_add)} new props to MainWindow ({len(existing)} already present / skipped)")

# ---------- 3) two-way bindings at instantiation ----------
# Map view base -> instantiation regex
instantiation = {
    'settings_interface': 'InterfaceSettingsView',
    'settings_controller': 'ControllerSettingsView',
    'settings_graphics': 'GraphicsSettingsView',
    'settings_emulation': 'EmulationSettingsView',
    'settings_bios': 'BiosSettingsView',
    'settings_audio': 'AudioSettingsView',
    'settings_osd': 'OsdSettingsView',
    'settings_memorycards': 'MemoryCardSettingsView',
    'settings_achievements': 'AchievementSettingsView',
    'settings_patches': 'PatchesSettingsView',
    'settings_cheats': 'CheatsSettingsView',
    'settings_gamefixes': 'GameFixesSettingsView',
    'settings_folders': 'FoldersSettingsView',
    'settings_advanced': 'AdvancedSettingsView',
}

main_text = open(MAIN, encoding='utf-8').read()
bounds = []
for base, comp in instantiation.items():
    for m in re.finditer(r'(' + re.escape(comp) + r'\s*\{)', main_text):
        start = m.end()
        depth = 1
        i = start
        while i < len(main_text) and depth > 0:
            c = main_text[i]
            if c == '{': depth += 1
            elif c == '}': depth -= 1
            i += 1
        bounds.append((base, comp, m.start(), m.end(), i))
bounds.sort(key=lambda x: x[3])

# For each, find existing assigned locals (name: or name <=>) to skip
new_text = main_text
added_total = 0
# process from end to preserve offsets
for base, comp, blk_start, blk_open, blk_close in sorted(bounds, key=lambda x: x[2], reverse=True):
    block = main_text[blk_open:blk_close]
    assigned = set(re.findall(r'([A-Za-z0-9_-]+)\s*(?::|<=>)', block))
    entries = data.get(base, [])
    binds = []
    for name, typ, short in entries:
        if name in assigned:
            continue
        binds.append(f'                {name} <=> root.{short};')
    if binds:
        # insert before the closing brace
        insert_at = blk_close - 1
        new_text = new_text[:insert_at] + '\n'.join(binds) + '\n' + new_text[insert_at:]
        added_total += len(binds)
open(MAIN, 'w', encoding='utf-8').write(new_text)
print(f"[3] added {added_total} two-way bindings at instantiation sites")

# ---------- 4) write settings_bindings.rs ----------
def section_for(short):
    p = short.split('-', 1)[0]
    return {'graphics':'EmuCore/GS','audio':'SPU2/Output','emu':'EmuCore','bios':'Filenames',
            'folders':'Folders','controller':'Input','interface':'UI','osd':'EmuCore/GS',
            'memcard':'MemoryCards','ra':'Achievements','adv':'EmuCore','patch':'Patches',
            'cheats':'Patches','gf':'EmuCore/GS','gamelist':'GameList'}.get(p, 'EmuCore')

OVERRIDE = {
    'bios-dir-path': ('Folders','Bios'), 'fast-boot': ('EmuCore','EnableFastBoot'),
    'fast-forward-boot': ('EmuCore','EnableFastBootFastForward'), 'selected-bios': (None,None),
    'emu-ee-cycle-rate': ('EmuCore/Speedhacks','EECycleRate'), 'emu-ee-cycle-skip': ('EmuCore/Speedhacks','EECycleSkip'),
    'emu-mtvu': ('EmuCore/Speedhacks','MTVU'), 'emu-fast-cdvd': ('EmuCore','FastCDVD'),
    'emu-cdvd-precache': ('EmuCore','CdvdPrecache'), 'emu-enable-cheats': ('EmuCore','EnableCheats'),
    'emu-apply-widescreen-patches': ('EmuCore/GS','WideScreenPatches'), 'emu-apply-no-interlace-patches': ('EmuCore/GS','NoInterlacePatches'),
    'emu-real-time-clock': ('EmuCore','RealTimeClock'), 'emu-sync-to-host': ('EmuCore/GS','SyncToHostRefreshRate'),
    'emu-frame-pacing': ('EmuCore/GS','FramePacing'), 'emu-vsync-for-timing': ('EmuCore/GS','VsyncForTiming'),
    'emu-skip-duplicate-frames': ('EmuCore/GS','SkipDuplicateFrames'), 'emu-max-frame-latency': ('EmuCore/GS','MaxFrameLatency'),
    'emu-normal-speed': ('EmuCore','NominalFrameRate'), 'emu-fast-speed': ('EmuCore','TurboFrameRate'),
    'emu-slow-speed': ('EmuCore','SlowmoFrameRate'), 'emu-thread-pinning': ('EmuCore','ThreadPinning'),
    'graphics-renderer': ('EmuCore/GS','Renderer'), 'graphics-adapter': ('EmuCore/GS','Adapter'),
    'graphics-resolution': ('EmuCore/GS','upscale_multiplier'), 'graphics-aspect-ratio': ('EmuCore/GS','AspectRatio'),
    'graphics-tex-filter': ('EmuCore/GS','Filter'), 'graphics-trilinear': ('EmuCore/GS','TrilinearFiltering'),
    'graphics-anisotropic': ('EmuCore/GS','AnisotropicFiltering'), 'graphics-auto-flush': ('EmuCore/GS','AutoFlushSw'),
    'graphics-mipmap': ('EmuCore/GS','Mipmap'), 'graphics-sw-threads': ('EmuCore/GS','SwThreadCount'),
    'graphics-accurate-blending': ('EmuCore/GS','accurate_blending_unit'), 'graphics-shade-boost': ('EmuCore/GS','ShadeBoost'),
    'graphics-preload-tex': ('EmuCore/GS','PreloadTexture'), 'graphics-dump-tex': ('EmuCore/GS','DumpTexture'),
    'graphics-load-replacements': ('EmuCore/GS','LoadTextureReplacements'), 'graphics-fxaa': ('EmuCore/GS','FXAA'),
    'graphics-shader-cache': ('EmuCore/GS','ShaderCache'), 'graphics-exclusive-fs': ('EmuCore/GS','ExclusiveFullscreen'),
    'graphics-deinterlace': ('EmuCore/GS','deinterlace'), 'graphics-integer-scaling': ('EmuCore/GS','IntegerScaling'),
    'graphics-anti-blur': ('EmuCore/GS','AntiBlur'), 'graphics-tv-shader': ('EmuCore/GS','TVShader'),
    'graphics-cas-sharpen': ('EmuCore/GS','CASMode'), 'graphics-cas-sharpness': ('EmuCore/GS','CASSharpening'),
    'graphics-vsync-queue-size': ('EmuCore/GS','VsyncQueueSize'), 'graphics-fullscreen-mode': ('EmuCore/GS','FullscreenMode'),
    'graphics-osd-font-path': ('EmuCore/GS','OsdFontPath'), 'graphics-capture-avi': ('EmuCore/GS','CaptureAVI'),
    'graphics-capture-png': ('EmuCore/GS','CapturePNG'), 'graphics-screenshot-format': ('EmuCore/GS','ScreenshotFormat'),
    'graphics-screenshot-quality': ('EmuCore/GS','ScreenshotQuality'), 'graphics-video-codec': ('EmuCore/GS','VideoCaptureCodec'),
    'graphics-video-bitrate': ('EmuCore/GS','VideoCaptureBitrate'), 'graphics-video-resolution': ('EmuCore/GS','VideoCaptureRes'),
    'audio-backend': ('SPU2/Output','Backend'), 'audio-driver-index': ('SPU2/Output','DriverIndex'),
    'audio-device-index': ('SPU2/Output','DeviceIndex'), 'audio-buffer-ms': ('SPU2/Output','BufferMs'),
    'audio-output-latency-ms': ('SPU2/Output','OutputLatencyMs'), 'audio-output-latency-minimal': ('SPU2/Output','OutputLatencyMinimal'),
    'audio-sync-mode': ('SPU2/Output','SyncMode'), 'audio-standard-volume': ('SPU2/Output','StandardVolume'),
    'audio-fast-forward-volume': ('SPU2/Output','FastForwardVolume'), 'audio-muted': ('SPU2/Output','OutputMuted'),
    'audio-expansion-mode': ('SPU2/Output','ExpansionMode'), 'audio-expand-block-size': ('SPU2/Output','ExpandBlockSize'),
    'audio-expand-circular-wrap': ('SPU2/Output','ExpandCircularWrap'), 'audio-expand-shift': ('SPU2/Output','ExpandShift'),
    'audio-expand-depth': ('SPU2/Output','ExpandDepth'), 'audio-expand-focus': ('SPU2/Output','ExpandFocus'),
    'audio-expand-center-image': ('SPU2/Output','ExpandCenterImage'), 'audio-expand-front-separation': ('SPU2/Output','ExpandFrontSeparation'),
    'audio-expand-rear-separation': ('SPU2/Output','ExpandRearSeparation'), 'audio-expand-low-cutoff': ('SPU2/Output','ExpandLowCutoff'),
    'audio-expand-high-cutoff': ('SPU2/Output','ExpandHighCutoff'),
    'audio-stretch-sequence-length': ('SPU2/Output','StretchSequenceLengthMS'), 'audio-stretch-seek-window': ('SPU2/Output','StretchSeekWindowMS'),
    'audio-stretch-overlap': ('SPU2/Output','StretchOverlapMS'), 'audio-stretch-quick-seek': ('SPU2/Output','StretchUseQuickSeek'),
    'audio-stretch-aa-filter': ('SPU2/Output','StretchUseAAFilter'),
    'adv-ee-clamp': ('EmuCore','eeMode'), 'adv-ee-round': ('EmuCore','eeRounding'), 'adv-ee-recompiler': ('EmuCore','EERecEnable'),
    'adv-ee-cache': ('EmuCore','EERecEnableCache'), 'adv-ee-div-round': ('EmuCore','eeDivRounding'),
    'adv-intc-spin': ('EmuCore','INTCSpinDetection'), 'adv-wait-loop': ('EmuCore','WaitLoopDetection'),
    'adv-fast-mem': ('EmuCore','FastMemoryAccess'), 'adv-pause-on-tlb': ('EmuCore','PauseOnTLBError'),
    'adv-extended-ram': ('EmuCore','ExtendedGSVRAM'), 'adv-vu0-clamp': ('EmuCore','vu0Mode'), 'adv-vu0-round': ('EmuCore','vu0Rounding'),
    'adv-vu1-clamp': ('EmuCore','vu1Mode'), 'adv-vu1-round': ('EmuCore','vu1Rounding'), 'adv-vu0-recompiler': ('EmuCore','VU0RecEnable'),
    'adv-vu1-recompiler': ('EmuCore','VU1RecEnable'), 'adv-mvu-flag': ('EmuCore','MVUFlagSpeedHack'), 'adv-instant-vu1': ('EmuCore','InstantVU1'),
    'adv-iop-recompiler': ('EmuCore','IOPRecEnable'), 'adv-compression-method': ('EmuCore','SavestateCompressionMethod'),
    'adv-compression-level': ('EmuCore','SavestateCompressionLevel'), 'adv-backup-savestates': ('EmuCore','BackupSavestates'),
    'adv-host-fs': ('EmuCore','HostFs'), 'adv-pine-enabled': ('EmuCore','PINEEnabled'), 'adv-pine-slot': ('EmuCore','PINEPort'),
    'interface-theme-index': ('UI','Theme'), 'interface-language-index': ('UI','Language'),
    'interface-render-separate': ('UI','RenderToSeparateWindow'), 'interface-hide-main-when-running': ('UI','HideMainWindowWhenRunning'),
    'interface-disable-window-resize': ('UI','DisableWindowResize'), 'interface-confirm-exit': ('UI','ConfirmShutdown'),
    'interface-save-on-shutdown': ('UI','SaveConfigOnShutdown'), 'interface-pause-focus-loss': ('UI','PauseOnFocusLoss'),
    'interface-pause-on-start': ('UI','StartPaused'), 'interface-pause-on-controller-disconnect': ('UI','PauseOnControllerDisconnection'),
    'interface-mouse-lock': ('UI','EnableMouseLock'), 'interface-inhibit-screensaver': ('UI','InhibitScreensaver'),
    'interface-start-fullscreen': ('UI','StartFullscreen'), 'interface-double-click-fullscreen': ('UI','DoubleClickTogglesFullscreen'),
    'interface-hide-cursor-fullscreen': ('UI','HideMouseCursor'), 'interface-prompt-state-failure': ('UI','PromptOnStateLoadSaveFailure'),
    'interface-use-savestate-selector': ('UI','UseSavestateSelector'), 'interface-discord-presence': ('UI','EnableDiscordPresence'),
    'interface-english-titles': ('UI','PreferEnglishGameList'), 'interface-bg-image-path': ('UI','GameListBackgroundPath'),
    'interface-bg-scale-index': ('UI','GameListBackgroundMode'), 'interface-bg-opacity': ('UI','GameListBackgroundOpacity'),
    'interface-start-big-picture': ('UI','StartBigPictureMode'), 'interface-show-advanced': ('UI','ShowAdvancedSettings'),
    'osd-show-perf': ('EmuCore/GS','OsdShowPerformanceMetrics'), 'osd-perf-pos': ('EmuCore/GS','OsdPerformanceMetricsPosition'),
    'osd-scale': ('EmuCore/GS','OsdScale'), 'osd-margin-x': ('EmuCore/GS','OsdMarginX'), 'osd-margin-y': ('EmuCore/GS','OsdMarginY'),
    'osd-bold': ('EmuCore/GS','OsdBold'), 'osd-font': ('EmuCore/GS','OsdFontPath'), 'osd-show-fps': ('EmuCore/GS','OsdShowFPS'),
    'osd-show-speed': ('EmuCore/GS','OsdShowSpeed'), 'osd-show-cpu': ('EmuCore/GS','OsdShowCPU'), 'osd-show-gpu': ('EmuCore/GS','OsdShowGPU'),
    'osd-show-frame-times': ('EmuCore/GS','OsdShowFrameTimes'), 'osd-show-hw-info': ('EmuCore/GS','OsdShowHardwareInfo'),
    'osd-show-version': ('EmuCore/GS','OsdShowVersion'), 'osd-show-settings-summary': ('EmuCore/GS','OsdShowSettingsSummary'),
    'osd-show-patches-list': ('EmuCore/GS','OsdShowPatchesList'), 'osd-show-inputs': ('EmuCore/GS','OsdShowInputs'),
    'osd-show-ee-perf': ('EmuCore/GS','OsdShowEEStats'), 'osd-show-vu-perf': ('EmuCore/GS','OsdShowVUStats'),
    'osd-show-gs-stats': ('EmuCore/GS','OsdShowGSStats'), 'osd-show-vram': ('EmuCore/GS','OsdShowVRAM'),
    'osd-show-internal-resolution': ('EmuCore/GS','OsdShowInternalResolution'), 'osd-show-game-info': ('EmuCore/GS','OsdShowGameInfo'),
    'osd-show-disc-info': ('EmuCore/GS','OsdShowDiscInfo'), 'osd-show-savestate-info': ('EmuCore/GS','OsdShowSavestateInfo'),
    'osd-show-audio-info': ('EmuCore/GS','OsdShowAudioInfo'), 'osd-show-controller-info': ('EmuCore/GS','OsdShowControllerInfo'),
    'osd-show-turbo-indicator': ('EmuCore/GS','OsdShowTurboIndicator'), 'osd-show-recording-indicator': ('EmuCore/GS','OsdShowRecordingIndicator'),
    'osd-show-log-screen': ('EmuCore/GS','OsdShowLogScreen'), 'osd-show-gs-window-title': ('EmuCore/GS','OsdShowGSWindowTitle'),
    'osd-show-notifications': ('EmuCore/GS','OsdShowNotifications'), 'osd-notif-duration': ('EmuCore/GS','OsdNotificationDuration'),
    'osd-warn-unsafe': ('EmuCore/GS','OsdWarnUnsafeSettings'),
    'folders-saves-dir': ('Folders','Saves'), 'folders-snapshots-dir': ('Folders','Snapshots'), 'folders-logs-dir': ('Folders','Logs'),
    'folders-cheats-dir': ('Folders','Cheats'), 'folders-patches-dir': ('Folders','Patches'), 'folders-textures-dir': ('Folders','Textures'),
    'folders-memcards-dir': ('Folders','MemoryCards'),
    'patch-widescreen': ('EmuCore/GS','WideScreenPatches'), 'patch-no-interlace': ('EmuCore/GS','NoInterlacePatches'),
    'patch-no-mpeg': ('EmuCore','NoMPEGPatch'), 'patch-texture-dump': ('EmuCore/GS','DumpTexture'),
    'gf-fpu-mul-hack': ('EmuCore/GS','FpuMulHack'), 'gf-fpu-full-mode': ('EmuCore/GS','FpuFullMode'), 'gf-xgkick-hack': ('EmuCore/GS','XGKickHack'),
    'gf-ee-timing-hack': ('EmuCore/GS','EETimingHack'), 'gf-skip-mpeg-hack': ('EmuCore/GS','SkipMPEGHack'), 'gf-oph-flag-hack': ('EmuCore/GS','OPHFlagHack'),
    'gf-blit-invalidate-hack': ('EmuCore/GS','BlitInvalidateHack'), 'gf-vu0-branch-hack': ('EmuCore/GS','VU0BranchHack'),
    'gf-vu-add-sub-hack': ('EmuCore/GS','VUAddSubHack'), 'gf-vu-compare-hack': ('EmuCore/GS','VUCompareHack'),
    'gf-vu-max-hack': ('EmuCore/GS','VUMaxHack'), 'gf-vu-min-hack': ('EmuCore/GS','VUMinHack'), 'gf-auto-flush-gs': ('EmuCore/GS','AutoFlush'),
    'gf-conservative-framebuffer': ('EmuCore/GS','ConservativeFramebuffer'), 'gf-texture-inside-rt': ('EmuCore/GS','TextureInsideRT'),
    'gf-dma-busy-hack': ('EmuCore/GS','DMABusyHack'), 'gf-gif-fifo-hack': ('EmuCore/GS','GIFxFIFOHack'), 'gf-goemon-tlb-hack': ('EmuCore/GS','GoemonTlbHack'),
    'gamelist-recursive-scan': ('GameList','RecursiveScan'),
    'ra-enable-ra': ('Achievements','Enabled'), 'ra-hardcore': ('Achievements','HardcoreMode'),
    'ra-show-notif': ('Achievements','Notifications'), 'ra-sound-fx': ('Achievements','SoundEffects'),
    'ra-leaderboard-notif': ('Achievements','LeaderboardNotifications'), 'ra-show-overlays': ('Achievements','Overlays'),
    'memcard-slot1-enabled': ('MemoryCards','Slot1_Enable'), 'memcard-slot1-card': ('MemoryCards','Slot1_Filename'),
    'memcard-slot1-type': ('MemoryCards','Slot1_Type'), 'memcard-slot2-enabled': ('MemoryCards','Slot2_Enable'),
    'memcard-slot2-card': ('MemoryCards','Slot2_Filename'), 'memcard-slot2-type': ('MemoryCards','Slot2_Type'),
    'memcard-folder': ('MemoryCards','Directory'),
}

# Build list of (short, typ) from data, dedup
all_props = []
seen = set()
for base, entries in data.items():
    for name, typ, short in entries:
        if short in seen:
            continue
        seen.add(short)
        all_props.append((short, typ))

apply = []
load = []
for short, typ in all_props:
    sec, key = OVERRIDE.get(short, (section_for(short), short))
    if sec is None:
        continue
    rid = short.replace('-', '_')
    if typ == 'string':
        apply.append(f'    Pcsx2Api::set_string_setting("{sec}", "{key}", window.get_{rid}().as_str());')
        load.append(f'    window.set_{rid}(Pcsx2Api::get_string_setting("{sec}", "{key}", &window.get_{rid}()).into());')
    elif typ == 'int':
        apply.append(f'    Pcsx2Api::set_int_setting("{sec}", "{key}", window.get_{rid}());')
        load.append(f'    window.set_{rid}(Pcsx2Api::get_int_setting("{sec}", "{key}", window.get_{rid}()));')
    elif typ == 'float':
        apply.append(f'    Pcsx2Api::set_float_setting("{sec}", "{key}", window.get_{rid}());')
        load.append(f'    window.set_{rid}(Pcsx2Api::get_float_setting("{sec}", "{key}", window.get_{rid}()));')
    else:
        apply.append(f'    Pcsx2Api::set_bool_setting("{sec}", "{key}", window.get_{rid}());')
        load.append(f'    window.set_{rid}(Pcsx2Api::get_bool_setting("{sec}", "{key}", window.get_{rid}()));')

rs = []
rs.append('// AUTO-GENERATED by gen_settings_all.py — do not edit by hand.')
rs.append('// Maps MainWindow settings properties to PCSX2 ini sections/keys.')
rs.append('use crate::pcsx2_capi::Pcsx2Api;')
rs.append('')
rs.append('pub fn apply_settings(window: &crate::MainWindow) {')
rs.extend(apply)
rs.append('    Pcsx2Api::commit_settings();')
rs.append('    let _ = Pcsx2Api::apply_settings();')
rs.append('}')
rs.append('')
rs.append('pub fn load_settings(window: &crate::MainWindow) {')
rs.extend(load)
rs.append('}')
open(RUST, 'w', encoding='utf-8').write('\n'.join(rs))
print(f"[4] wrote {RUST}: {len(apply)} apply + {len(load)} load entries")
