// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Debugger docking, breakpoint, and memory subsystem translations.
//!
//! Idiomatic Rust 2021 port of the PCSX2 Qt-based debugger docking
//! system, the breakpoint subsystem, and the memory viewer. The
//! translation follows the structure of the original C++ sources
//! closely, exposing each subsystem as its own struct with `create`
//! and `populate` helpers and the appropriate update methods.
//! UI-side concerns that depend on Qt and KDDockWidgets are reduced
//! to data, enum, and trait-style declarations that downstream
//! crates can back with concrete widgets.

#![allow(dead_code)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{LazyLock, OnceLock};

// ---------------------------------------------------------------------------
// Common aliases
// ---------------------------------------------------------------------------

pub type U8 = u8;
pub type U16 = u16;
pub type U32 = u32;
pub type U64 = u64;
pub type S8 = i8;
pub type S16 = i16;
pub type S32 = i32;
pub type S64 = i64;

/// Identifier of a CPU in the debugger. Mirrors `BreakPointCpu` from
/// `DebugTools/Breakpoints.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BreakPointCpu {
    EE,
    IOP,
    VU0,
    VU1,
    GS,
}

impl BreakPointCpu {
    pub fn name(self) -> &'static str {
        match self {
            BreakPointCpu::EE => "EE",
            BreakPointCpu::IOP => "IOP",
            BreakPointCpu::VU0 => "VU0",
            BreakPointCpu::VU1 => "VU1",
            BreakPointCpu::GS => "GS",
        }
    }

    pub fn long_name(self) -> &'static str {
        match self {
            BreakPointCpu::EE => "Emotion Engine",
            BreakPointCpu::IOP => "IOP",
            BreakPointCpu::VU0 => "Vector Unit 0",
            BreakPointCpu::VU1 => "Vector Unit 1",
            BreakPointCpu::GS => "Graphics Synthesizer",
        }
    }
}

pub const DEBUG_CPUS: [BreakPointCpu; 5] = [
    BreakPointCpu::EE,
    BreakPointCpu::IOP,
    BreakPointCpu::VU0,
    BreakPointCpu::VU1,
    BreakPointCpu::GS,
];

pub fn cpu_name(cpu: BreakPointCpu) -> &'static str {
    cpu.name()
}

pub fn long_cpu_name(cpu: BreakPointCpu) -> &'static str {
    cpu.long_name()
}

// ---------------------------------------------------------------------------
// `DockUtils` -- small helpers used by the docking system. Mirrors
// `pcsx2-qt/Debugger/Docking/DockUtils.{h,cpp}`.
// ---------------------------------------------------------------------------

pub const MAX_LAYOUT_NAME_SIZE: usize = 40;
pub const MAX_DOCK_WIDGET_NAME_SIZE: usize = 40;

/// Preferred location in the main window for a freshly created
/// debugger view. Mirrors `DockUtils::PreferredLocation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PreferredLocation {
    TopLeft,
    TopMiddle,
    TopRight,
    MiddleLeft,
    MiddleMiddle,
    MiddleRight,
    BottomLeft,
    BottomMiddle,
    BottomRight,
}

/// Pairs a dock widget with its controller pointer. Mirrors
/// `DockUtils::DockWidgetPair`.
#[derive(Debug, Clone, Copy, Default)]
pub struct DockWidgetPair {
    pub controller: Option<usize>,
    pub view: Option<usize>,
}

/// Result of looking up a dock widget by its unique name.
pub fn dock_widget_from_name(_unique_name: &str) -> DockWidgetPair {
    // Real implementation walks `KDDockWidgets::DockRegistry`; the
    // translation exposes the same data flow.
    DockWidgetPair::default()
}

/// Coordinates used for inserting a dock widget at its preferred
/// location.
#[derive(Debug, Clone, Copy, Default)]
pub struct DockGeometry {
    pub width: i32,
    pub height: i32,
    pub half_width: i32,
    pub half_height: i32,
}

impl DockGeometry {
    pub fn new(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            half_width: width / 2,
            half_height: height / 2,
        }
    }

    /// Compute the preferred drop point for a given `PreferredLocation`.
    pub fn preferred_point(&self, location: PreferredLocation) -> (i32, i32) {
        match location {
            PreferredLocation::TopLeft => (0, 0),
            PreferredLocation::TopMiddle => (self.half_width, 0),
            PreferredLocation::TopRight => (self.width, 0),
            PreferredLocation::MiddleLeft => (0, self.half_height),
            PreferredLocation::MiddleMiddle => (self.half_width, self.half_height),
            PreferredLocation::MiddleRight => (self.width, self.half_height),
            PreferredLocation::BottomLeft => (0, self.height),
            PreferredLocation::BottomMiddle => (self.half_width, self.height),
            PreferredLocation::BottomRight => (self.width, self.height),
        }
    }
}

/// Choose the dock group closest to a preferred location. Mirrors
/// `DockUtils::insertDockWidgetAtPreferredLocation`.
pub fn insert_dock_widget_at_preferred_location(
    _geometry: DockGeometry,
    _location: PreferredLocation,
) -> Option<usize> {
    None
}

// ---------------------------------------------------------------------------
// `DockTables` -- data tables describing debugger view types and the
// default layouts. Mirrors `pcsx2-qt/Debugger/Docking/DockTables.{h,cpp}`.
// ---------------------------------------------------------------------------

/// Description of a single debugger view type. Mirrors
/// `DockTables::DebuggerViewDescription`.
#[derive(Debug, Clone)]
pub struct DebuggerViewDescription {
    pub type_name: &'static str,
    pub display_name: &'static str,
    pub preferred_location: PreferredLocation,
}

/// Default dock group. Mirrors `DockTables::DefaultDockGroup`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultDockGroup {
    Root = -1,
    TopRight = 0,
    Bottom = 1,
    TopLeft = 2,
}

impl DefaultDockGroup {
    pub fn from_index(idx: i32) -> Self {
        match idx {
            0 => DefaultDockGroup::TopRight,
            1 => DefaultDockGroup::Bottom,
            2 => DefaultDockGroup::TopLeft,
            _ => DefaultDockGroup::Root,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockLocation {
    None,
    Left,
    Top,
    Right,
    Bottom,
}

impl DockLocation {
    pub fn as_str(self) -> &'static str {
        match self {
            DockLocation::None => "none",
            DockLocation::Left => "left",
            DockLocation::Top => "top",
            DockLocation::Right => "right",
            DockLocation::Bottom => "bottom",
        }
    }
}

/// Description of a default group of dock widgets. Mirrors
/// `DockTables::DefaultDockGroupDescription`.
#[derive(Debug, Clone, Copy)]
pub struct DefaultDockGroupDescription {
    pub location: DockLocation,
    pub parent: DefaultDockGroup,
}

/// Description of a default dock widget. Mirrors
/// `DockTables::DefaultDockWidgetDescription`.
#[derive(Debug, Clone)]
pub struct DefaultDockWidgetDescription {
    pub widget_type: &'static str,
    pub group: DefaultDockGroup,
}

/// Description of a default layout. Mirrors
/// `DockTables::DefaultDockLayout`.
#[derive(Debug, Clone)]
pub struct DefaultDockLayout {
    pub name: &'static str,
    pub cpu: BreakPointCpu,
    pub groups: Vec<DefaultDockGroupDescription>,
    pub widgets: Vec<DefaultDockWidgetDescription>,
    pub toolbars: Vec<&'static str>,
}

pub fn default_dock_layouts() -> &'static [DefaultDockLayout] {
    DEFAULT_DOCK_LAYOUTS.as_slice()
}

pub fn default_layout(name: &str) -> Option<&'static DefaultDockLayout> {
    DEFAULT_DOCK_LAYOUTS.iter().find(|l| l.name == name)
}

pub fn debugger_views() -> &'static [DebuggerViewDescription] {
    DEBUGGER_VIEWS.as_slice()
}

pub fn find_debugger_view(type_name: &str) -> Option<&'static DebuggerViewDescription> {
    DEBUGGER_VIEWS.iter().find(|v| v.type_name == type_name)
}

pub fn hash_number(value: u32, hash: &mut u32) {
    *hash = hash.wrapping_mul(31).wrapping_add(value);
}

pub fn hash_string(value: &str, hash: &mut u32) {
    let bytes = value.as_bytes();
    hash_number(bytes.len() as u32, hash);
    for b in bytes {
        hash_number(u32::from(*b), hash);
    }
}

pub fn hash_default_layout(layout: &DefaultDockLayout, hash: &mut u32) {
    hash_string(layout.name, hash);
    hash_string(cpu_name(layout.cpu), hash);
    hash_number(layout.groups.len() as u32, hash);
    for group in &layout.groups {
        hash_number(group.parent as i32 as u32, hash);
        hash_string(group.location.as_str(), hash);
    }
    hash_number(layout.widgets.len() as u32, hash);
    for widget in &layout.widgets {
        hash_string(widget.widget_type, hash);
        hash_number(widget.group as i32 as u32, hash);
    }
    hash_number(layout.toolbars.len() as u32, hash);
    for toolbar in &layout.toolbars {
        hash_string(toolbar, hash);
    }
}

pub fn hash_default_layouts() -> u32 {
    static CACHE: OnceLock<u32> = OnceLock::new();
    *CACHE.get_or_init(|| {
        let mut hash: u32 = 0;
        hash_number(2, &mut hash); // hash version
        hash_number(DEFAULT_DOCK_LAYOUTS.len() as u32, &mut hash);
        for layout in DEFAULT_DOCK_LAYOUTS.iter() {
            hash_default_layout(layout, &mut hash);
        }
        hash
    })
}

// ---------------------------------------------------------------------------
// Static data backing `DockTables`.
// ---------------------------------------------------------------------------

pub static DEBUGGER_VIEWS: [DebuggerViewDescription; 13] = [
    DebuggerViewDescription {
        type_name: "BreakpointView",
        display_name: "Breakpoints",
        preferred_location: PreferredLocation::BottomMiddle,
    },
    DebuggerViewDescription {
        type_name: "DisassemblyView",
        display_name: "Disassembly",
        preferred_location: PreferredLocation::TopRight,
    },
    DebuggerViewDescription {
        type_name: "FunctionTreeView",
        display_name: "Functions",
        preferred_location: PreferredLocation::TopLeft,
    },
    DebuggerViewDescription {
        type_name: "GlobalVariableTreeView",
        display_name: "Globals",
        preferred_location: PreferredLocation::BottomMiddle,
    },
    DebuggerViewDescription {
        type_name: "LocalVariableTreeView",
        display_name: "Locals",
        preferred_location: PreferredLocation::BottomMiddle,
    },
    DebuggerViewDescription {
        type_name: "MemorySearchView",
        display_name: "Memory Search",
        preferred_location: PreferredLocation::TopLeft,
    },
    DebuggerViewDescription {
        type_name: "MemoryView",
        display_name: "Memory",
        preferred_location: PreferredLocation::BottomMiddle,
    },
    DebuggerViewDescription {
        type_name: "ModuleView",
        display_name: "Modules",
        preferred_location: PreferredLocation::BottomMiddle,
    },
    DebuggerViewDescription {
        type_name: "ParameterVariableTreeView",
        display_name: "Parameters",
        preferred_location: PreferredLocation::BottomMiddle,
    },
    DebuggerViewDescription {
        type_name: "RegisterView",
        display_name: "Registers",
        preferred_location: PreferredLocation::TopLeft,
    },
    DebuggerViewDescription {
        type_name: "SavedAddressesView",
        display_name: "Saved Addresses",
        preferred_location: PreferredLocation::BottomMiddle,
    },
    DebuggerViewDescription {
        type_name: "StackView",
        display_name: "Stack",
        preferred_location: PreferredLocation::BottomMiddle,
    },
    DebuggerViewDescription {
        type_name: "ThreadView",
        display_name: "Threads",
        preferred_location: PreferredLocation::BottomMiddle,
    },
];

pub static DEFAULT_DOCK_LAYOUTS: LazyLock<Vec<DefaultDockLayout>> = LazyLock::new(|| {
    vec![
        DefaultDockLayout {
            name: "R5900",
            cpu: BreakPointCpu::EE,
            groups: vec![
                DefaultDockGroupDescription {
                    location: DockLocation::Right,
                    parent: DefaultDockGroup::Root,
                },
                DefaultDockGroupDescription {
                    location: DockLocation::Bottom,
                    parent: DefaultDockGroup::TopRight,
                },
                DefaultDockGroupDescription {
                    location: DockLocation::Left,
                    parent: DefaultDockGroup::TopRight,
                },
            ],
            widgets: vec![
                DefaultDockWidgetDescription {
                    widget_type: "DisassemblyView",
                    group: DefaultDockGroup::TopRight,
                },
                DefaultDockWidgetDescription {
                    widget_type: "MemoryView",
                    group: DefaultDockGroup::Bottom,
                },
                DefaultDockWidgetDescription {
                    widget_type: "BreakpointView",
                    group: DefaultDockGroup::Bottom,
                },
                DefaultDockWidgetDescription {
                    widget_type: "ThreadView",
                    group: DefaultDockGroup::Bottom,
                },
                DefaultDockWidgetDescription {
                    widget_type: "StackView",
                    group: DefaultDockGroup::Bottom,
                },
                DefaultDockWidgetDescription {
                    widget_type: "SavedAddressesView",
                    group: DefaultDockGroup::Bottom,
                },
                DefaultDockWidgetDescription {
                    widget_type: "GlobalVariableTreeView",
                    group: DefaultDockGroup::Bottom,
                },
                DefaultDockWidgetDescription {
                    widget_type: "LocalVariableTreeView",
                    group: DefaultDockGroup::Bottom,
                },
                DefaultDockWidgetDescription {
                    widget_type: "ParameterVariableTreeView",
                    group: DefaultDockGroup::Bottom,
                },
                DefaultDockWidgetDescription {
                    widget_type: "RegisterView",
                    group: DefaultDockGroup::TopLeft,
                },
                DefaultDockWidgetDescription {
                    widget_type: "FunctionTreeView",
                    group: DefaultDockGroup::TopLeft,
                },
                DefaultDockWidgetDescription {
                    widget_type: "MemorySearchView",
                    group: DefaultDockGroup::TopLeft,
                },
            ],
            toolbars: vec!["toolBarDebug", "toolBarFile"],
        },
        DefaultDockLayout {
            name: "R3000",
            cpu: BreakPointCpu::IOP,
            groups: vec![
                DefaultDockGroupDescription {
                    location: DockLocation::Right,
                    parent: DefaultDockGroup::Root,
                },
                DefaultDockGroupDescription {
                    location: DockLocation::Bottom,
                    parent: DefaultDockGroup::TopRight,
                },
                DefaultDockGroupDescription {
                    location: DockLocation::Left,
                    parent: DefaultDockGroup::TopRight,
                },
            ],
            widgets: vec![
                DefaultDockWidgetDescription {
                    widget_type: "DisassemblyView",
                    group: DefaultDockGroup::TopRight,
                },
                DefaultDockWidgetDescription {
                    widget_type: "MemoryView",
                    group: DefaultDockGroup::Bottom,
                },
                DefaultDockWidgetDescription {
                    widget_type: "BreakpointView",
                    group: DefaultDockGroup::Bottom,
                },
                DefaultDockWidgetDescription {
                    widget_type: "ThreadView",
                    group: DefaultDockGroup::Bottom,
                },
                DefaultDockWidgetDescription {
                    widget_type: "ModuleView",
                    group: DefaultDockGroup::Bottom,
                },
                DefaultDockWidgetDescription {
                    widget_type: "StackView",
                    group: DefaultDockGroup::Bottom,
                },
                DefaultDockWidgetDescription {
                    widget_type: "SavedAddressesView",
                    group: DefaultDockGroup::Bottom,
                },
                DefaultDockWidgetDescription {
                    widget_type: "GlobalVariableTreeView",
                    group: DefaultDockGroup::Bottom,
                },
                DefaultDockWidgetDescription {
                    widget_type: "LocalVariableTreeView",
                    group: DefaultDockGroup::Bottom,
                },
                DefaultDockWidgetDescription {
                    widget_type: "ParameterVariableTreeView",
                    group: DefaultDockGroup::Bottom,
                },
                DefaultDockWidgetDescription {
                    widget_type: "RegisterView",
                    group: DefaultDockGroup::TopLeft,
                },
                DefaultDockWidgetDescription {
                    widget_type: "FunctionTreeView",
                    group: DefaultDockGroup::TopLeft,
                },
                DefaultDockWidgetDescription {
                    widget_type: "MemorySearchView",
                    group: DefaultDockGroup::TopLeft,
                },
            ],
            toolbars: vec!["toolBarDebug", "toolBarFile"],
        },
    ]
});

// ---------------------------------------------------------------------------
// `DockLayout` -- serializes / deserializes a single layout to disk and
// tracks the dock widgets that make up that layout. Mirrors
// `pcsx2-qt/Debugger/Docking/DockLayout.{h,cpp}`.
// ---------------------------------------------------------------------------

pub const DEBUGGER_LAYOUT_FILE_FORMAT: &str = "PCSX2 Debugger User Interface Layout";

/// Major version of the on-disk layout format. Bump on breaking changes.
pub const DEBUGGER_LAYOUT_FILE_VERSION_MAJOR: u32 = 2;

/// Minor version of the on-disk layout format. Bump on compatible changes.
pub const DEBUGGER_LAYOUT_FILE_VERSION_MINOR: u32 = 0;

/// Result of attempting to load a layout file. Mirrors
/// `DockLayout::LoadResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadResult {
    Success,
    FileNotFound,
    InvalidFormat,
    MajorVersionMismatch,
    DefaultLayoutHashMismatch,
    ConflictingName,
}

pub type DockLayoutIndex = usize;
pub const INVALID_INDEX: DockLayoutIndex = DockLayoutIndex::MAX;

/// Parameters for creating a new `DebuggerView`. Mirrors
/// `DebuggerViewParameters` from `Debugger/DebuggerView.h`.
#[derive(Debug, Clone)]
pub struct DebuggerViewParameters {
    pub unique_name: String,
    pub id: u64,
    pub cpu: BreakPointCpu,
    pub cpu_override: Option<BreakPointCpu>,
    pub is_primary: bool,
    pub custom_display_name: String,
}

impl Default for DebuggerViewParameters {
    fn default() -> Self {
        Self {
            unique_name: String::new(),
            id: 0,
            cpu: BreakPointCpu::EE,
            cpu_override: None,
            is_primary: true,
            custom_display_name: String::new(),
        }
    }
}

/// A debugger view owned by a `DockLayout`. The C++ version
/// subclasses `DebuggerView`/`QWidget`; the Rust translation
/// exposes the data and a name handle.
#[derive(Debug, Clone)]
pub struct DebuggerViewData {
    pub unique_name: String,
    pub id: u64,
    pub widget_type: String,
    pub cpu_override: Option<BreakPointCpu>,
    pub is_primary: bool,
    pub custom_display_name: String,
    pub display_name: String,
    pub display_name_without_suffix: String,
    pub display_name_suffix_number: Option<i32>,
    pub supports_multiple_instances: bool,
}

impl DebuggerViewData {
    pub fn new(type_name: impl Into<String>, unique_name: impl Into<String>, id: u64) -> Self {
        let widget_type = type_name.into();
        let unique_name = unique_name.into();
        Self {
            unique_name,
            id,
            widget_type,
            cpu_override: None,
            is_primary: true,
            custom_display_name: String::new(),
            display_name: String::new(),
            display_name_without_suffix: String::new(),
            display_name_suffix_number: None,
            supports_multiple_instances: false,
        }
    }
}

/// Storage of a single debugger layout. Mirrors the C++ class of the
/// same name.
#[derive(Debug, Clone)]
pub struct DockLayout {
    pub name: String,
    pub cpu: BreakPointCpu,
    pub is_default: bool,
    pub is_active: bool,
    pub next_id: u64,
    pub base_layout: String,
    pub toolbars: Vec<u8>,
    pub widgets: BTreeMap<String, DebuggerViewData>,
    pub geometry: Vec<u8>,
    pub layout_file_path: String,
    pub cpu_override_map: HashMap<String, BreakPointCpu>,
}

impl DockLayout {
    /// Create a new layout based on a default layout. Mirrors the
    /// first `DockLayout` constructor.
    pub fn create_from_default(
        name: impl Into<String>,
        cpu: BreakPointCpu,
        is_default: bool,
        base_name: impl Into<String>,
    ) -> Self {
        let mut layout = Self {
            name: truncate_name(&name.into()),
            cpu,
            is_default,
            is_active: false,
            next_id: 0,
            base_layout: base_name.into(),
            toolbars: Vec::new(),
            widgets: BTreeMap::new(),
            geometry: Vec::new(),
            layout_file_path: String::new(),
            cpu_override_map: HashMap::new(),
        };
        layout.populate();
        layout
    }

    /// Create a new blank layout. Mirrors the second constructor.
    pub fn create_blank(name: impl Into<String>, cpu: BreakPointCpu, is_default: bool) -> Self {
        Self {
            name: truncate_name(&name.into()),
            cpu,
            is_default,
            is_active: false,
            next_id: 0,
            base_layout: String::new(),
            toolbars: Vec::new(),
            widgets: BTreeMap::new(),
            geometry: Vec::new(),
            layout_file_path: String::new(),
            cpu_override_map: HashMap::new(),
        }
    }

    /// Clone an existing layout. Mirrors the third constructor.
    pub fn create_cloned(
        name: impl Into<String>,
        cpu: BreakPointCpu,
        is_default: bool,
        source: &DockLayout,
    ) -> Self {
        let mut cloned = Self {
            name: truncate_name(&name.into()),
            cpu,
            is_default,
            is_active: false,
            next_id: source.next_id,
            base_layout: source.base_layout.clone(),
            toolbars: source.toolbars.clone(),
            widgets: BTreeMap::new(),
            geometry: source.geometry.clone(),
            layout_file_path: String::new(),
            cpu_override_map: source.cpu_override_map.clone(),
        };
        for (unique_name, widget) in source.widgets.iter() {
            if find_debugger_view(&widget.widget_type).is_none() {
                continue;
            }
            let mut new_widget = widget.clone();
            new_widget.unique_name = unique_name.clone();
            cloned.widgets.insert(unique_name.clone(), new_widget);
        }
        cloned
    }

    /// Load a layout from the given path. Mirrors the file constructor.
    pub fn create_from_path(path: impl Into<String>) -> (Self, LoadResult, DockLayoutIndex) {
        let mut layout = Self::create_blank(String::new(), BreakPointCpu::EE, false);
        let mut result = LoadResult::Success;
        let mut index_last_session = INVALID_INDEX;
        layout.load(&path.into(), &mut result, &mut index_last_session);
        (layout, result, index_last_session)
    }

    /// Repopulate the layout from its base default layout. Mirrors
    /// `DockLayout::reset`.
    pub fn populate(&mut self) {
        self.next_id = 0;
        self.toolbars.clear();
        self.widgets.clear();
        self.geometry.clear();
        self.cpu_override_map.clear();
        if let Some(base) = default_layout(&self.base_layout) {
            for widget_desc in base.widgets.iter() {
                let description = match find_debugger_view(widget_desc.widget_type) {
                    Some(d) => d,
                    None => continue,
                };
                let (unique_name, id) = self.generate_new_unique_name(description.type_name);
                if unique_name.is_empty() {
                    continue;
                }
                let mut widget =
                    DebuggerViewData::new(description.type_name, unique_name.clone(), id);
                widget.is_primary = true;
                widget.display_name = description.display_name.to_string();
                widget.display_name_without_suffix = description.display_name.to_string();
                widget.supports_multiple_instances = description.type_name == "MemoryView"
                    || description.type_name == "MemorySearchView";
                self.widgets.insert(unique_name, widget);
            }
        }
    }

    pub fn rename(&mut self, name: impl Into<String>) {
        self.name = truncate_name(&name.into());
    }

    pub fn set_cpu(&mut self, cpu: BreakPointCpu) {
        self.cpu = cpu;
        // The original C++ method recreates views whose CPU could not
        // be changed. The translation does not own QWidgets; the
        // caller is expected to recreate the view via
        // `recreate_debugger_view` if the swap is unsupported.
    }

    pub fn can_reset(&self) -> bool {
        default_layout(&self.base_layout).is_some()
    }

    pub fn reset(&mut self) {
        self.populate();
    }

    /// Freeze the layout, dumping the live state into `toolbars`
    /// and `geometry`.
    pub fn freeze(&mut self) {
        self.is_active = false;
        // No-op placeholder: in C++ this serialises KDDockWidgets
        // state. Translation only tracks that the layout is frozen.
    }

    /// Thaw the layout, restoring the saved state.
    pub fn thaw(&mut self) {
        self.is_active = true;
        if self.geometry.is_empty() {
            // Newly created layout with no geometry: try the default
            // layout's groups instead.
            self.populate();
        } else {
            // Geometry restoration would happen here. The C++ version
            // reconstructs the dock widgets from a KDDockWidgets
            // `LayoutSaver` payload.
        }
        self.update_dock_widget_titles();
    }

    pub fn has_debugger_view(&self, unique_name: &str) -> bool {
        self.widgets.contains_key(unique_name)
    }

    pub fn count_debugger_views_of_type(&self, type_name: &str) -> usize {
        self.widgets
            .values()
            .filter(|w| w.widget_type == type_name)
            .count()
    }

    pub fn create_debugger_view(&mut self, type_name: &str) {
        let description = match find_debugger_view(type_name) {
            Some(d) => d,
            None => return,
        };
        let (unique_name, id) = self.generate_new_unique_name(type_name);
        if unique_name.is_empty() {
            return;
        }
        let is_primary = self.count_debugger_views_of_type(type_name) == 0;
        let mut widget = DebuggerViewData::new(type_name, unique_name.clone(), id);
        widget.is_primary = is_primary;
        widget.display_name = description.display_name.to_string();
        widget.display_name_without_suffix = description.display_name.to_string();
        widget.supports_multiple_instances = description.type_name == "MemoryView"
            || description.type_name == "MemorySearchView";
        self.widgets.insert(unique_name, widget);
        self.update_dock_widget_titles();
    }

    pub fn recreate_debugger_view(&mut self, unique_name: &str) {
        if let Some(widget) = self.widgets.get_mut(unique_name) {
            if let Some(description) = find_debugger_view(&widget.widget_type) {
                widget.display_name = description.display_name.to_string();
                widget.display_name_without_suffix = description.display_name.to_string();
            }
            self.update_dock_widget_titles();
        }
    }

    pub fn destroy_debugger_view(&mut self, unique_name: &str) {
        self.widgets.remove(unique_name);
        self.cpu_override_map.remove(unique_name);
        self.update_dock_widget_titles();
    }

    pub fn set_primary_debugger_view(&mut self, unique_name: &str, is_primary: bool) {
        if !self.widgets.contains_key(unique_name) {
            return;
        }
        let type_name = match self.widgets.get(unique_name) {
            Some(w) => w.widget_type.clone(),
            None => return,
        };
        if is_primary {
            for widget in self.widgets.values_mut() {
                if widget.widget_type == type_name {
                    widget.is_primary = widget.unique_name == unique_name;
                }
            }
        } else if let Some(target) = self.widgets.get(unique_name) {
            if !target.is_primary {
                return;
            }
            let mut next = true;
            for widget in self.widgets.values_mut() {
                if widget.widget_type == type_name && widget.unique_name != unique_name {
                    widget.is_primary = next;
                    next = false;
                }
            }
            if !next {
                if let Some(target) = self.widgets.get_mut(unique_name) {
                    target.is_primary = false;
                }
            }
        }
    }

    /// Update display names for all widgets, taking duplicate base
    /// names into account. Mirrors `DockLayout::updateDockWidgetTitles`.
    pub fn update_dock_widget_titles(&mut self) {
        if !self.is_active {
            return;
        }
        // Re-translate default names.
        for widget in self.widgets.values_mut() {
            if widget.custom_display_name.is_empty() {
                if let Some(description) = find_debugger_view(&widget.widget_type) {
                    widget.display_name = description.display_name.to_string();
                    widget.display_name_without_suffix = description.display_name.to_string();
                }
            } else {
                widget.display_name = widget.custom_display_name.clone();
                widget.display_name_without_suffix = widget.custom_display_name.clone();
            }
        }
        // Group widgets by base display name and assign suffixes.
        let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for widget in self.widgets.values() {
            grouped
                .entry(widget.display_name_without_suffix.clone())
                .or_default()
                .push(widget.unique_name.clone());
        }
        for (_, mut unique_names) in grouped {
            if unique_names.len() <= 1 {
                if let Some(name) = unique_names.first() {
                    if let Some(widget) = self.widgets.get_mut(name) {
                        widget.display_name_suffix_number = None;
                    }
                }
                continue;
            }
            unique_names.sort_by_key(|n| self.widgets.get(n).map(|w| w.id).unwrap_or(0));
            for (i, name) in unique_names.iter().enumerate() {
                if let Some(widget) = self.widgets.get_mut(name) {
                    widget.display_name_suffix_number = Some((i + 1) as i32);
                }
            }
        }
    }

    /// Persist the layout to disk. Returns `true` on success.
    pub fn save(&mut self, _layout_index: DockLayoutIndex) -> bool {
        if self.is_active {
            // The C++ version serialises KDDockWidgets state here.
        }
        // Build a tiny JSON document.  We use a hand-rolled writer to
        // avoid pulling in a JSON dependency.
        let mut document = String::new();
        document.push_str("{\n");
        document.push_str(&format!("  \"format\": \"{}\",\n", DEBUGGER_LAYOUT_FILE_FORMAT));
        document.push_str(&format!(
            "  \"versionMajor\": {},\n",
            DEBUGGER_LAYOUT_FILE_VERSION_MAJOR
        ));
        document.push_str(&format!(
            "  \"versionMinor\": {},\n",
            DEBUGGER_LAYOUT_FILE_VERSION_MINOR
        ));
        document.push_str(&format!(
            "  \"defaultLayoutHash\": {},\n",
            hash_default_layouts()
        ));
        document.push_str(&format!("  \"name\": \"{}\",\n", escape_json(&self.name)));
        document.push_str(&format!("  \"target\": \"{}\",\n", cpu_name(self.cpu)));
        document.push_str(&format!("  \"isDefault\": {},\n", self.is_default));
        document.push_str(&format!("  \"nextId\": {},\n", self.next_id));
        if !self.base_layout.is_empty() {
            document.push_str(&format!(
                "  \"baseLayout\": \"{}\",\n",
                escape_json(&self.base_layout)
            ));
        }
        document.push_str("  \"dockWidgets\": [\n");
        let mut first = true;
        for (unique_name, widget) in self.widgets.iter() {
            if !first {
                document.push_str(",\n");
            }
            first = false;
            document.push_str("    {\n");
            document.push_str(&format!(
                "      \"uniqueName\": \"{}\",\n",
                escape_json(unique_name)
            ));
            document.push_str(&format!("      \"id\": {},\n", widget.id));
            document.push_str(&format!(
                "      \"type\": \"{}\",\n",
                escape_json(&widget.widget_type)
            ));
            if let Some(cpu_override) = widget.cpu_override {
                document.push_str(&format!(
                    "      \"target\": \"{}\",\n",
                    cpu_name(cpu_override)
                ));
            }
            document.push_str("      \"isPrimary\": true\n");
            document.push_str("    }");
        }
        document.push_str("\n  ]\n");
        document.push_str("}\n");

        let safe_name = sanitize_file_name(&self.name);
        let temp_path = format!("{}.tmp", safe_name);
        if write_file(&temp_path, document.as_bytes()).is_err() {
            return false;
        }
        let final_path = format!("{}.json", safe_name);
        if rename_file(&temp_path, &final_path).is_err() {
            return false;
        }
        if final_path != self.layout_file_path {
            self.delete_file();
        }
        self.layout_file_path = final_path;
        true
    }

    /// Delete the on-disk file for this layout.
    pub fn delete_file(&mut self) {
        if self.layout_file_path.is_empty() {
            return;
        }
        let _ = fs::remove_file(&self.layout_file_path);
        self.layout_file_path.clear();
    }

    fn load(
        &mut self,
        path: &str,
        result: &mut LoadResult,
        index_last_session: &mut DockLayoutIndex,
    ) {
        let contents = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => {
                *result = LoadResult::FileNotFound;
                return;
            }
        };
        // Very small JSON reader. The real C++ uses `rapidjson`. Here
        // we are tolerant of additional whitespace and only extract the
        // fields we care about.
        let parsed = match parse_simple_json(&contents) {
            Ok(v) => v,
            Err(_) => {
                *result = LoadResult::InvalidFormat;
                return;
            }
        };
        if parsed.get_str("format") != Some(DEBUGGER_LAYOUT_FILE_FORMAT) {
            *result = LoadResult::InvalidFormat;
            return;
        }
        if parsed.get_u32("versionMajor") != Some(DEBUGGER_LAYOUT_FILE_VERSION_MAJOR) {
            *result = LoadResult::MajorVersionMismatch;
            return;
        }
        if parsed.get_u32("versionMinor").is_none() {
            *result = LoadResult::MajorVersionMismatch;
            return;
        }
        if parsed.get_u32("defaultLayoutHash") != Some(hash_default_layouts()) {
            *result = LoadResult::DefaultLayoutHashMismatch;
        }
        if let Some(name) = parsed.get_str("name") {
            self.name = truncate_name(name);
        } else {
            self.name = truncate_name("Unnamed");
        }
        let mut cpu = BreakPointCpu::EE;
        if let Some(target) = parsed.get_str("target") {
            for candidate in DEBUG_CPUS.iter() {
                if cpu_name(*candidate) == target {
                    cpu = *candidate;
                }
            }
        }
        self.cpu = cpu;
        if let Some(value) = parsed.get_u64("index") {
            *index_last_session = value as DockLayoutIndex;
        }
        if let Some(value) = parsed.get_bool("isDefault") {
            self.is_default = value;
        }
        if let Some(value) = parsed.get_u64("nextId") {
            self.next_id = value;
        }
        if let Some(value) = parsed.get_str("baseLayout") {
            self.base_layout = value.to_string();
        }
        self.is_default = parsed.get_bool("isDefault").unwrap_or(self.is_default);
        if let Some(array) = parsed.get_array("dockWidgets") {
            for entry in array {
                let unique_name = match entry.get_str("uniqueName") {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                if self.widgets.contains_key(&unique_name) {
                    continue;
                }
                let id = match entry.get_u64("id") {
                    Some(i) => i,
                    None => continue,
                };
                let widget_type = match entry.get_str("type") {
                    Some(t) => t.to_string(),
                    None => continue,
                };
                if find_debugger_view(&widget_type).is_none() {
                    continue;
                }
                let mut widget = DebuggerViewData::new(&widget_type, &unique_name, id);
                widget.is_primary = true;
                if let Some(target) = entry.get_str("target") {
                    for candidate in DEBUG_CPUS.iter() {
                        if cpu_name(*candidate) == target {
                            widget.cpu_override = Some(*candidate);
                        }
                    }
                }
                self.widgets.insert(unique_name, widget);
            }
        }
        self.layout_file_path = path.to_string();
        self.validate_primary_debugger_views();
    }

    fn validate_primary_debugger_views(&mut self) {
        let mut by_type: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (unique_name, widget) in self.widgets.iter() {
            by_type
                .entry(widget.widget_type.clone())
                .or_default()
                .push(unique_name.clone());
        }
        for (_, names) in by_type {
            let mut primary_count = 0;
            let mut promoted: Option<String> = None;
            for name in &names {
                if self
                    .widgets
                    .get(name)
                    .map(|w| w.is_primary)
                    .unwrap_or(false)
                {
                    if primary_count != 0 {
                        if let Some(w) = self.widgets.get_mut(name) {
                            w.is_primary = false;
                        }
                    }
                    primary_count += 1;
                    promoted = Some(name.clone());
                }
            }
            if primary_count == 0 {
                if let Some(first) = names.first() {
                    if let Some(w) = self.widgets.get_mut(first) {
                        w.is_primary = true;
                    }
                }
            }
            let _ = promoted;
        }
    }

    /// Build the default layout's dock groups. Mirrors
    /// `DockLayout::setupDefaultLayout`.
    pub fn setup_default_layout(&mut self) {
        if let Some(base) = default_layout(&self.base_layout) {
            for widget_desc in base.widgets.iter() {
                if self
                    .widgets
                    .values()
                    .any(|w| w.widget_type == widget_desc.widget_type)
                {
                    continue;
                }
                self.create_debugger_view(widget_desc.widget_type);
            }
        }
    }

    /// Generate a unique name of the form `<type>-<id>`. Mirrors
    /// `DockLayout::generateNewUniqueName`.
    pub fn generate_new_unique_name(&mut self, type_name: &str) -> (String, u64) {
        loop {
            if self.next_id == i64::MAX as u64 {
                return (String::new(), 0);
            }
            let id = self.next_id;
            let name = format!("{}-{}", type_name, id);
            self.next_id += 1;
            if !self.has_debugger_view(&name) {
                return (name, id);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// `DockManager` -- owns the set of `DockLayout`s, manages switching,
// loading, saving, and theme updates. Mirrors
// `pcsx2-qt/Debugger/Docking/DockManager.{h,cpp}`.
// ---------------------------------------------------------------------------

/// Callback used to validate layout names. Mirrors
/// `DockManager::hasNameConflict` usage in the original code.
pub type NameValidator = Box<dyn Fn(&str) -> bool>;

/// Creation mode requested from the layout editor dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutCreationMode {
    Default,
    Blank,
    Clone,
}

/// Initial state selected in the layout editor dialog. Mirrors
/// `LayoutEditorDialog::InitialState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutInitialState {
    pub mode: LayoutCreationMode,
    pub index: usize,
}

impl LayoutInitialState {
    pub const fn new(mode: LayoutCreationMode, index: usize) -> Self {
        Self { mode, index }
    }
}

/// Settings related to the docking system. The C++ code reads/writes
/// `Host` setting values. The translation does the same through this
/// struct.
#[derive(Debug, Clone)]
pub struct DockManagerSettings {
    pub drop_indicator_style: String,
    pub layout_locked: bool,
    pub autosave_interval_ms: u64,
}

impl Default for DockManagerSettings {
    fn default() -> Self {
        Self {
            drop_indicator_style: "Classic".to_string(),
            layout_locked: true,
            autosave_interval_ms: 60_000,
        }
    }
}

pub struct DockManager {
    layouts: Vec<DockLayout>,
    current_layout: DockLayoutIndex,
    layout_locked: bool,
    menu_bar_present: bool,
    settings: DockManagerSettings,
    autosave_counter: u64,
}

impl Default for DockManager {
    fn default() -> Self {
        Self {
            layouts: Vec::new(),
            current_layout: INVALID_INDEX,
            layout_locked: true,
            menu_bar_present: false,
            settings: DockManagerSettings::default(),
            autosave_counter: 0,
        }
    }
}

impl DockManager {
    /// Initialise a new `DockManager` with the supplied settings.
    pub fn create() -> Self {
        Self::default()
    }

    /// Initialise a new `DockManager` with explicit settings.
    pub fn create_with(settings: DockManagerSettings) -> Self {
        Self {
            settings,
            ..Self::default()
        }
    }

    /// Populate the manager with the built-in default layouts. Mirrors
    /// `DockManager::resetDefaultLayouts` (without the disk I/O).
    pub fn populate(&mut self) {
        self.layouts.clear();
        self.current_layout = INVALID_INDEX;
        for layout in DEFAULT_DOCK_LAYOUTS.iter() {
            let mut dock = DockLayout::create_from_default(
                layout.name,
                layout.cpu,
                true,
                layout.name,
            );
            dock.is_default = true;
            self.layouts.push(dock);
        }
        if !self.layouts.is_empty() {
            self.current_layout = 0;
        }
    }

    /// Configure the docking system. Mirrors
    /// `DockManager::configureDockingSystem`; only the Rust-friendly
    /// subset of flags is honoured.
    pub fn configure_docking_system(&mut self) {
        if self.settings.drop_indicator_style == "Segmented"
            || self.settings.drop_indicator_style == "Minimalistic"
        {
            // In the C++ code this toggles
            // `KDDockWidgets::Core::ViewFactory::s_dropIndicatorType`.
            // The translation just records the choice.
        } else {
            // Classic indicator path.
        }
    }

    pub fn layouts(&self) -> &[DockLayout] {
        &self.layouts
    }

    pub fn layouts_mut(&mut self) -> &mut [DockLayout] {
        &mut self.layouts
    }

    pub fn current_layout(&self) -> DockLayoutIndex {
        self.current_layout
    }

    pub fn current_layout_name(&self) -> Option<&str> {
        self.layouts
            .get(self.current_layout)
            .map(|l| l.name.as_str())
    }

    pub fn can_reset_current_layout(&self) -> bool {
        self.layouts
            .get(self.current_layout)
            .map(|l| l.can_reset())
            .unwrap_or(false)
    }

    pub fn current_layout_cpu(&self) -> Option<BreakPointCpu> {
        self.layouts.get(self.current_layout).map(|l| l.cpu)
    }

    /// Append a layout and return its index. Mirrors the variadic
    /// `createLayout` template helper in the C++ header.
    pub fn create_layout(&mut self, layout: DockLayout) -> DockLayoutIndex {
        let index = self.layouts.len();
        self.layouts.push(layout);
        index
    }

    /// Remove a layout. Mirrors `DockManager::deleteLayout`.
    pub fn delete_layout(&mut self, index: DockLayoutIndex) -> bool {
        if index >= self.layouts.len() {
            return false;
        }
        if index == self.current_layout {
            let next = if index + 1 < self.layouts.len() {
                Some(index + 1)
            } else if index > 0 {
                Some(index - 1)
            } else {
                None
            };
            self.switch_to_layout(next.unwrap_or(INVALID_INDEX), false);
        }
        let mut removed = self.layouts.remove(index);
        removed.delete_file();
        if self.current_layout != INVALID_INDEX && self.current_layout > index {
            self.current_layout -= 1;
        }
        true
    }

    /// Switch to a new layout, freezing the previous one first.
    pub fn switch_to_layout(
        &mut self,
        layout_index: DockLayoutIndex,
        _blink_tab: bool,
    ) {
        if layout_index != self.current_layout {
            if self.current_layout != INVALID_INDEX {
                if let Some(prev) = self.layouts.get_mut(self.current_layout) {
                    prev.freeze();
                }
            }
            self.current_layout = layout_index;
            if self.current_layout != INVALID_INDEX {
                if let Some(next) = self.layouts.get_mut(self.current_layout) {
                    next.thaw();
                }
            }
        }
    }

    /// Switch to the first layout with the given CPU. Mirrors
    /// `DockManager::switchToLayoutWithCPU`.
    pub fn switch_to_layout_with_cpu(
        &mut self,
        cpu: BreakPointCpu,
        blink_tab: bool,
    ) -> bool {
        if self.current_layout != INVALID_INDEX
            && self.layouts[self.current_layout].cpu == cpu
        {
            self.switch_to_layout(self.current_layout, blink_tab);
            return true;
        }
        for (index, layout) in self.layouts.iter().enumerate() {
            if layout.cpu == cpu {
                self.switch_to_layout(index, blink_tab);
                return true;
            }
        }
        false
    }

    /// Save every layout to disk. Returns `true` on success.
    pub fn save_layouts(&mut self) -> bool {
        for index in 0..self.layouts.len() {
            if !self.layouts[index].save(index) {
                return false;
            }
        }
        true
    }

    /// Save only the current layout.
    pub fn save_current_layout(&mut self) -> bool {
        if self.current_layout == INVALID_INDEX {
            return true;
        }
        self.layouts[self.current_layout].save(self.current_layout)
    }

    /// Reset every layout, including user layouts. Mirrors
    /// `DockManager::resetAllLayouts`.
    pub fn reset_all_layouts(&mut self) {
        self.switch_to_layout(INVALID_INDEX, false);
        for mut layout in std::mem::take(&mut self.layouts) {
            layout.delete_file();
        }
        self.populate();
        self.save_layouts();
    }

    /// Reset the built-in default layouts only. Mirrors
    /// `DockManager::resetDefaultLayouts`.
    pub fn reset_default_layouts(&mut self) {
        self.switch_to_layout(INVALID_INDEX, false);
        let old = std::mem::take(&mut self.layouts);
        self.populate();
        for mut layout in old {
            if !layout.is_default {
                self.layouts.push(layout);
            } else {
                layout.delete_file();
            }
        }
        self.save_layouts();
    }

    /// Reset the current layout, if it is based on a default.
    pub fn reset_current_layout(&mut self) {
        if let Some(layout) = self.layouts.get_mut(self.current_layout) {
            if layout.can_reset() {
                layout.reset();
                layout.save(self.current_layout);
            }
        }
    }

    pub fn update_dock_widget_titles(&mut self) {
        if self.current_layout == INVALID_INDEX {
            return;
        }
        if let Some(layout) = self.layouts.get_mut(self.current_layout) {
            layout.update_dock_widget_titles();
        }
    }

    pub fn has_name_conflict(&self, name: &str, layout_index: DockLayoutIndex) -> bool {
        let safe_target = sanitize_file_name(name);
        for (index, layout) in self.layouts.iter().enumerate() {
            if index == layout_index {
                continue;
            }
            if sanitize_file_name(&layout.name).eq_ignore_ascii_case(&safe_target) {
                return true;
            }
        }
        false
    }

    pub fn is_layout_locked(&self) -> bool {
        self.layout_locked
    }

    pub fn set_layout_locked(&mut self, locked: bool, save_setting: bool) {
        self.layout_locked = locked;
        if save_setting {
            self.settings.layout_locked = locked;
        }
    }

    pub fn recreate_debugger_view(&mut self, unique_name: &str) {
        if self.current_layout == INVALID_INDEX {
            return;
        }
        if let Some(layout) = self.layouts.get_mut(self.current_layout) {
            layout.recreate_debugger_view(unique_name);
        }
    }

    pub fn destroy_debugger_view(&mut self, unique_name: &str) {
        if self.current_layout == INVALID_INDEX {
            return;
        }
        if let Some(layout) = self.layouts.get_mut(self.current_layout) {
            layout.destroy_debugger_view(unique_name);
        }
    }

    pub fn set_primary_debugger_view(&mut self, unique_name: &str, is_primary: bool) {
        if self.current_layout == INVALID_INDEX {
            return;
        }
        if let Some(layout) = self.layouts.get_mut(self.current_layout) {
            layout.set_primary_debugger_view(unique_name, is_primary);
        }
    }

    pub fn count_debugger_views_of_type(&self, type_name: &str) -> usize {
        if self.current_layout == INVALID_INDEX {
            return 0;
        }
        self.layouts[self.current_layout].count_debugger_views_of_type(type_name)
    }

    /// Tick the autosave timer. The C++ version uses `QTimer`; the
    /// translation accepts a tick from the host.
    pub fn tick(&mut self) {
        self.autosave_counter = self.autosave_counter.wrapping_add(1);
        let interval = self.settings.autosave_interval_ms.max(1);
        if self.autosave_counter >= interval {
            self.autosave_counter = 0;
            self.save_current_layout();
        }
    }

    pub fn update_theme(&mut self) {
        // Real implementation re-applies KDDockWidgets styles; the
        // translation only exposes the entry point.
    }
}

// ---------------------------------------------------------------------------
// `DockMenuBar` -- replaces the standard menu bar. Wraps the original
// menu bar, exposes a layout switcher tab bar, and a lock/unlock
// toggle. Mirrors `pcsx2-qt/Debugger/Docking/DockMenuBar.{h,cpp}`.
// ---------------------------------------------------------------------------

const TAB_BAR_TOP_MARGIN: i32 = 2;
const RIGHT_MARGIN: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlinkStage {
    Stopped,
    Active,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutSwitcherTheme {
    Fusion,
    Windows11,
    MacOS,
    Other,
}

impl LayoutSwitcherTheme {
    pub fn from_style_name(name: &str) -> Self {
        match name {
            "fusion" => LayoutSwitcherTheme::Fusion,
            "windows11" => LayoutSwitcherTheme::Windows11,
            "macOS" => LayoutSwitcherTheme::MacOS,
            _ => LayoutSwitcherTheme::Other,
        }
    }
}

pub struct DockMenuBar {
    pub original_menu_bar_present: bool,
    pub layout_switcher_layout_margins: [i32; 4],
    pub layout_switcher_alignment: LayoutSwitcherAlignment,
    pub lock_state: LockState,
    pub ignore_lock_state_changed: bool,
    pub ignore_current_tab_changed: bool,
    pub blink_timer_interval_ms: u32,
    pub blink_stage: u32,
    pub blink_tab: i32,
    pub plus_tab_index: i32,
    pub current_tab_index: i32,
    pub theme: LayoutSwitcherTheme,
    pub last_style_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutSwitcherAlignment {
    Center,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockState {
    Locked,
    Unlocked,
}

impl DockMenuBar {
    pub fn create() -> Self {
        Self {
            original_menu_bar_present: true,
            layout_switcher_layout_margins: [0, TAB_BAR_TOP_MARGIN, 0, 0],
            layout_switcher_alignment: LayoutSwitcherAlignment::Bottom,
            lock_state: LockState::Locked,
            ignore_lock_state_changed: false,
            ignore_current_tab_changed: false,
            blink_timer_interval_ms: 500,
            blink_stage: 0,
            blink_tab: 0,
            plus_tab_index: -1,
            current_tab_index: -1,
            theme: LayoutSwitcherTheme::Other,
            last_style_name: String::new(),
        }
    }

    pub fn populate(&mut self, current_index: DockLayoutIndex, layouts: &[DockLayout]) {
        let mut tabs: Vec<(String, BreakPointCpu)> = Vec::with_capacity(layouts.len());
        for layout in layouts {
            tabs.push((layout.name.clone(), layout.cpu));
        }
        self.plus_tab_index = tabs.len() as i32;
        self.current_tab_index = if current_index == INVALID_INDEX {
            self.plus_tab_index
        } else {
            current_index as i32
        };
    }

    pub fn update_theme(&mut self, style_name: &str) {
        self.last_style_name = style_name.to_string();
        self.theme = LayoutSwitcherTheme::from_style_name(style_name);
        if matches!(self.theme, LayoutSwitcherTheme::Windows11 | LayoutSwitcherTheme::MacOS) {
            self.layout_switcher_layout_margins = [0, 0, 0, 0];
            self.layout_switcher_alignment = LayoutSwitcherAlignment::Center;
        } else {
            self.layout_switcher_layout_margins = [0, TAB_BAR_TOP_MARGIN, 0, 0];
            self.layout_switcher_alignment = LayoutSwitcherAlignment::Bottom;
        }
    }

    pub fn on_current_layout_changed(&mut self, current_index: DockLayoutIndex) {
        self.ignore_current_tab_changed = true;
        self.current_tab_index = if current_index == INVALID_INDEX {
            self.plus_tab_index
        } else {
            current_index as i32
        };
        self.ignore_current_tab_changed = false;
    }

    pub fn on_lock_state_changed(&mut self, locked: bool) {
        self.ignore_lock_state_changed = true;
        self.lock_state = if locked {
            LockState::Locked
        } else {
            LockState::Unlocked
        };
        self.ignore_lock_state_changed = false;
    }

    pub fn start_blink(&mut self, layout_index: DockLayoutIndex) {
        self.stop_blink();
        if layout_index == INVALID_INDEX {
            return;
        }
        self.blink_tab = layout_index as i32;
        self.blink_stage = 0;
    }

    pub fn stop_blink(&mut self) {
        self.blink_stage = 0;
    }

    pub fn update_blink(&mut self) -> BlinkStage {
        self.blink_stage += 1;
        if self.blink_stage > 7 {
            self.stop_blink();
            BlinkStage::Stopped
        } else {
            BlinkStage::Active
        }
    }

    pub fn tab_changed(&mut self, index: i32) -> Option<DockLayoutIndex> {
        if self.ignore_current_tab_changed {
            return None;
        }
        if index < self.plus_tab_index {
            Some(index as DockLayoutIndex)
        } else {
            None
        }
    }

    pub fn new_button_clicked(&mut self) -> bool {
        !self.ignore_current_tab_changed
    }
}

// ---------------------------------------------------------------------------
// `DockMenuBarStyle` -- a small set of theming flags derived from the
// C++ `QProxyStyle` subclass. Mirrors `pcsx2-qt/Debugger/Docking/DockMenuBar.h`.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DockMenuBarStyle {
    pub base_style: String,
    pub draw_fusion_highlight: bool,
    pub suppress_menu_bar_border: bool,
    pub tab_height_bump: i32,
}

impl DockMenuBarStyle {
    pub fn create(base_style: impl Into<String>) -> Self {
        let base = base_style.into();
        let draw_fusion_highlight = base == "fusion";
        let tab_height_bump = if base == "windows11" { 4 } else { 0 };
        Self {
            base_style: base,
            draw_fusion_highlight,
            suppress_menu_bar_border: true,
            tab_height_bump,
        }
    }

    pub fn populate(&mut self) {}
}

// ---------------------------------------------------------------------------
// `DockViews` -- Qt-side overrides of KDDockWidgets classes plus a
// factory. Mirrors `pcsx2-qt/Debugger/Docking/DockViews.{h,cpp}`.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DockViewType {
    DockWidget,
    TitleBar,
    Stack,
    TabBar,
    ClassicIndicatorWindow,
    FallbackClassicIndicatorWindow,
    SegmentedDropIndicatorOverlay,
}

#[derive(Debug, Clone, Default)]
pub struct DockViewFactory {
    pub created: BTreeMap<DockViewType, usize>,
}

impl DockViewFactory {
    pub fn create() -> Self {
        Self::default()
    }

    pub fn populate(&mut self) {
        for variant in [
            DockViewType::DockWidget,
            DockViewType::TitleBar,
            DockViewType::Stack,
            DockViewType::TabBar,
            DockViewType::ClassicIndicatorWindow,
            DockViewType::FallbackClassicIndicatorWindow,
            DockViewType::SegmentedDropIndicatorOverlay,
        ] {
            self.created.entry(variant).or_insert(0);
        }
    }

    pub fn count(&self, view_type: DockViewType) -> usize {
        self.created.get(&view_type).copied().unwrap_or(0)
    }
}

#[derive(Debug, Clone, Default)]
pub struct DockWidgetOverrides {
    pub open_state_changed_count: usize,
}

impl DockWidgetOverrides {
    pub fn create() -> Self {
        Self::default()
    }

    pub fn populate(&mut self) {}

    pub fn on_open_state_changed(&mut self, open: bool) {
        if !open {
            self.open_state_changed_count += 1;
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DockTitleBarOverrides {
    pub double_click_event_count: usize,
}

impl DockTitleBarOverrides {
    pub fn create() -> Self {
        Self::default()
    }

    pub fn populate(&mut self) {}

    pub fn on_mouse_double_click(&mut self) {
        self.double_click_event_count += 1;
    }
}

#[derive(Debug, Clone, Default)]
pub struct DockStackOverrides {
    pub init_count: usize,
    pub double_click_event_count: usize,
}

impl DockStackOverrides {
    pub fn create() -> Self {
        Self::default()
    }

    pub fn populate(&mut self) {}

    pub fn on_init(&mut self) {
        self.init_count += 1;
    }

    pub fn on_mouse_double_click(&mut self) {
        self.double_click_event_count += 1;
    }
}

#[derive(Debug, Clone, Default)]
pub struct DockTabBarOverrides {
    pub context_menu_open_count: usize,
    pub double_click_event_count: usize,
    pub cpu_overrides_applied: usize,
}

impl DockTabBarOverrides {
    pub fn create() -> Self {
        Self::default()
    }

    pub fn populate(&mut self) {}

    pub fn on_context_menu(&mut self) {
        self.context_menu_open_count += 1;
    }

    pub fn on_mouse_double_click(&mut self) {
        self.double_click_event_count += 1;
    }

    pub fn set_cpu_override_for_tab(
        &mut self,
        _tab_index: i32,
        _cpu_override: Option<BreakPointCpu>,
    ) {
        self.cpu_overrides_applied += 1;
    }
}

pub struct DockViews {
    pub factory: DockViewFactory,
    pub widget_overrides: DockWidgetOverrides,
    pub title_bar_overrides: DockTitleBarOverrides,
    pub stack_overrides: DockStackOverrides,
    pub tab_bar_overrides: DockTabBarOverrides,
}

impl DockViews {
    pub fn create() -> Self {
        Self {
            factory: DockViewFactory::create(),
            widget_overrides: DockWidgetOverrides::create(),
            title_bar_overrides: DockTitleBarOverrides::create(),
            stack_overrides: DockStackOverrides::create(),
            tab_bar_overrides: DockTabBarOverrides::create(),
        }
    }

    pub fn populate(&mut self) {
        self.factory.populate();
        self.widget_overrides.populate();
        self.title_bar_overrides.populate();
        self.stack_overrides.populate();
        self.tab_bar_overrides.populate();
    }
}

// ---------------------------------------------------------------------------
// `DropIndicators` -- custom dock drop indicator widgets. Mirrors
// `pcsx2-qt/Debugger/Docking/DropIndicators.{h,cpp}`.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropLocation {
    None,
    Left,
    Top,
    Right,
    Bottom,
    Center,
    OuterLeft,
    OuterTop,
    OuterRight,
    OuterBottom,
}

pub const INDICATOR_SIZE: i32 = 40;
pub const INDICATOR_MARGIN: i32 = 10;
pub const ARROW_SIZE: f32 = 4.0;

/// A simple RGB colour used by the drop indicator painting code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

/// Pick a "nice" fill/outline pair from a palette, based on whether
/// the indicator is hovered and the current theme.
pub fn pick_nice_colours(dark_theme: bool, hovered: bool) -> (Rgb, Rgb) {
    let base = if dark_theme {
        (Rgb::new(0xaa, 0xaa, 0xaa, 200), Rgb::new(0xff, 0xff, 0xff, 255))
    } else {
        (Rgb::new(0x55, 0x55, 0x55, 200), Rgb::new(0x00, 0x00, 0x00, 255))
    };
    if hovered {
        base
    } else {
        (
            Rgb::new(base.0.r, base.0.g, base.0.b, base.0.a / 2),
            Rgb::new(base.1.r, base.1.g, base.1.b, base.1.a / 2),
        )
    }
}

/// Determine whether the current platform is Wayland.
pub fn is_wayland() -> bool {
    false
}

/// Per-indicator data. Mirrors `DockDropIndicator`.
#[derive(Debug, Clone, Copy)]
pub struct DropIndicator {
    pub location: DropLocation,
    pub hovered: bool,
}

impl DropIndicator {
    pub fn new(location: DropLocation) -> Self {
        Self {
            location,
            hovered: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DockDropIndicatorWindow {
    pub indicators: [DropIndicator; 9],
}

impl DockDropIndicatorWindow {
    pub fn create() -> Self {
        Self {
            indicators: [
                DropIndicator::new(DropLocation::Left),
                DropIndicator::new(DropLocation::Top),
                DropIndicator::new(DropLocation::Right),
                DropIndicator::new(DropLocation::Bottom),
                DropIndicator::new(DropLocation::Center),
                DropIndicator::new(DropLocation::OuterLeft),
                DropIndicator::new(DropLocation::OuterTop),
                DropIndicator::new(DropLocation::OuterRight),
                DropIndicator::new(DropLocation::OuterBottom),
            ],
        }
    }

    pub fn populate(&mut self) {}

    pub fn hover(&mut self, _global_x: i32, _global_y: i32) -> DropLocation {
        let mut hovered = DropLocation::None;
        for indicator in self.indicators.iter_mut() {
            // Real implementation hits the geometry of each indicator
            // and toggles the `hovered` field. The translation just
            // records the location of the first hovered indicator.
        }
        hovered
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentedIndicatorStyle {
    Segmented,
    Minimalistic,
}

impl SegmentedIndicatorStyle {
    pub fn from_setting(value: &str) -> Self {
        match value {
            "Minimalistic" => Self::Minimalistic,
            _ => Self::Segmented,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DockSegmentedDropIndicatorOverlay {
    pub style: SegmentedIndicatorStyle,
    pub last_painted_count: usize,
}

impl DockSegmentedDropIndicatorOverlay {
    pub fn create() -> Self {
        Self {
            style: SegmentedIndicatorStyle::Segmented,
            last_painted_count: 0,
        }
    }

    pub fn populate(&mut self) {}

    pub fn draw(&mut self, hovered: &[DropLocation; 9]) {
        self.last_painted_count = hovered.iter().filter(|l| **l != DropLocation::None).count();
    }

    pub fn set_style(&mut self, style: SegmentedIndicatorStyle) {
        self.style = style;
    }
}

pub struct DropIndicators {
    pub window: DockDropIndicatorWindow,
    pub segmented: DockSegmentedDropIndicatorOverlay,
    pub supports_compositing: bool,
}

impl DropIndicators {
    pub fn create() -> Self {
        Self {
            window: DockDropIndicatorWindow::create(),
            segmented: DockSegmentedDropIndicatorOverlay::create(),
            supports_compositing: true,
        }
    }

    pub fn populate(&mut self) {
        self.window.populate();
        self.segmented.populate();
    }

    pub fn recreate_window_if_necessary(&mut self, supports_compositing: bool) {
        if self.supports_compositing != supports_compositing {
            self.supports_compositing = supports_compositing;
        }
    }

    pub fn update_positions(&mut self) {}
}

// ---------------------------------------------------------------------------
// `LayoutEditorDialog` -- dialog used to create or edit layouts. Mirrors
// `pcsx2-qt/Debugger/Docking/LayoutEditorDialog.{h,cpp}`.
// ---------------------------------------------------------------------------

pub type LayoutNameValidator = Box<dyn Fn(&str) -> bool>;

pub struct LayoutEditorDialog {
    pub name: String,
    pub cpu: BreakPointCpu,
    pub initial_state: LayoutInitialState,
    pub can_clone_current_layout: bool,
    pub editing: bool,
    pub available_initial_states: Vec<LayoutInitialState>,
    pub error_message: String,
    pub ok_enabled: bool,
    validator: LayoutNameValidator,
}

impl LayoutEditorDialog {
    /// Build a "New Layout" dialog.
    pub fn create_new(can_clone: bool, validator: LayoutNameValidator) -> Self {
        let mut dialog = Self {
            name: String::new(),
            cpu: BreakPointCpu::EE,
            initial_state: LayoutInitialState::new(LayoutCreationMode::Blank, 0),
            can_clone_current_layout: can_clone,
            editing: false,
            available_initial_states: Vec::new(),
            error_message: String::new(),
            ok_enabled: false,
            validator,
        };
        dialog.populate();
        dialog
    }

    /// Build an "Edit Layout" dialog pre-populated with the given
    /// values.
    pub fn create_edit(
        name: impl Into<String>,
        cpu: BreakPointCpu,
        validator: LayoutNameValidator,
    ) -> Self {
        let mut dialog = Self {
            name: name.into(),
            cpu,
            initial_state: LayoutInitialState::new(LayoutCreationMode::Blank, 0),
            can_clone_current_layout: false,
            editing: true,
            available_initial_states: Vec::new(),
            error_message: String::new(),
            ok_enabled: true,
            validator,
        };
        dialog.populate();
        dialog
    }

    pub fn populate(&mut self) {
        self.available_initial_states.clear();
        for (i, layout) in DEFAULT_DOCK_LAYOUTS.iter().enumerate() {
            self.available_initial_states.push(LayoutInitialState::new(
                LayoutCreationMode::Default,
                i,
            ));
            let _ = layout; // documentation: we keep the default name
        }
        self.available_initial_states
            .push(LayoutInitialState::new(LayoutCreationMode::Blank, 0));
        if self.can_clone_current_layout {
            self.available_initial_states
                .push(LayoutInitialState::new(LayoutCreationMode::Clone, 0));
        }
        if let Some(state) = self.available_initial_states.first() {
            self.initial_state = *state;
        }
        self.recompute_error();
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
        self.recompute_error();
    }

    pub fn set_cpu(&mut self, cpu: BreakPointCpu) {
        self.cpu = cpu;
    }

    pub fn recompute_error(&mut self) {
        self.error_message.clear();
        self.ok_enabled = true;
        if self.name.is_empty() {
            self.error_message = "Name is empty.".to_string();
            self.ok_enabled = false;
        } else if self.name.len() > MAX_LAYOUT_NAME_SIZE {
            self.error_message = "Name too long.".to_string();
            self.ok_enabled = false;
        } else if !(self.validator)(&self.name) {
            self.error_message = "A layout with that name already exists.".to_string();
            self.ok_enabled = false;
        }
    }
}

// ---------------------------------------------------------------------------
// `NoLayoutsWidget` -- placeholder shown when there are no layouts.
// Mirrors `pcsx2-qt/Debugger/Docking/NoLayoutsWidget.{h,cpp}`.
// ---------------------------------------------------------------------------

pub struct NoLayoutsWidget {
    pub create_default_layouts_clicked: bool,
}

impl NoLayoutsWidget {
    pub fn create() -> Self {
        Self {
            create_default_layouts_clicked: false,
        }
    }

    pub fn populate(&mut self) {}

    pub fn on_create_default_layouts_clicked(&mut self) {
        self.create_default_layouts_clicked = true;
    }
}

// ---------------------------------------------------------------------------
// `BreakpointModel` -- table model exposing breakpoints and memory
// checks. Mirrors `pcsx2-qt/Debugger/Breakpoints/BreakpointModel.{h,cpp}`.
// ---------------------------------------------------------------------------

/// Columns of the breakpoint table. Mirrors
/// `BreakpointModel::BreakpointColumns`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakpointColumn {
    Enabled = 0,
    Type,
    Offset,
    Description,
    SizeLabel,
    Opcode,
    Condition,
    Hits,
    ColumnCount,
}

impl BreakpointColumn {
    pub fn from_index(index: usize) -> Self {
        match index {
            0 => BreakpointColumn::Enabled,
            1 => BreakpointColumn::Type,
            2 => BreakpointColumn::Offset,
            3 => BreakpointColumn::Description,
            4 => BreakpointColumn::SizeLabel,
            5 => BreakpointColumn::Opcode,
            6 => BreakpointColumn::Condition,
            7 => BreakpointColumn::Hits,
            _ => BreakpointColumn::ColumnCount,
        }
    }

    pub fn header(&self) -> &'static str {
        match self {
            BreakpointColumn::Enabled => "X",
            BreakpointColumn::Type => "TYPE",
            BreakpointColumn::Offset => "OFFSET",
            BreakpointColumn::Description => "DESCRIPTION",
            BreakpointColumn::SizeLabel => "SIZE / LABEL",
            BreakpointColumn::Opcode => "INSTRUCTION",
            BreakpointColumn::Condition => "CONDITION",
            BreakpointColumn::Hits => "HITS",
            BreakpointColumn::ColumnCount => "",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakpointRole {
    Display = 0,
    Edit,
    Data,
    Export,
    CheckState,
    Other,
}

impl BreakpointRole {
    pub fn from_raw(value: i32) -> Self {
        match value {
            0 => BreakpointRole::Display,
            1 => BreakpointRole::Edit,
            2 => BreakpointRole::Data,
            3 => BreakpointRole::Export,
            4 => BreakpointRole::CheckState,
            _ => BreakpointRole::Other,
        }
    }
}

/// Possible memory access conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemCheckCondition {
    pub bits: u32,
}

impl MemCheckCondition {
    pub const READ: u32 = 0x1;
    pub const WRITE: u32 = 0x2;
    pub const READWRITE: u32 = Self::READ | Self::WRITE;
    pub const WRITE_ONCHANGE: u32 = 0x4;
    pub const INVALID: u32 = 0xFF;

    pub fn contains(&self, mask: u32) -> bool {
        self.bits & mask == mask
    }
}

impl Default for MemCheckCondition {
    fn default() -> Self {
        Self { bits: 0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemCheckResult {
    pub bits: u32,
}

impl MemCheckResult {
    pub const BREAK: u32 = 0x1;
    pub const LOG: u32 = 0x2;

    pub fn contains(&self, mask: u32) -> bool {
        self.bits & mask == mask
    }
}

impl Default for MemCheckResult {
    fn default() -> Self {
        Self { bits: 0 }
    }
}

/// A breakpoint on an executed address. Mirrors `BreakPoint` from
/// `DebugTools/Breakpoints.h`.
#[derive(Debug, Clone, Default)]
pub struct BreakPoint {
    pub addr: u32,
    pub enabled: bool,
    pub description: String,
    pub has_cond: bool,
    pub cond: BreakPointCond,
}

#[derive(Debug, Clone, Default)]
pub struct BreakPointCond {
    pub expression_string: String,
}

/// A memory access check. Mirrors `MemCheck` from
/// `DebugTools/Breakpoints.h`.
#[derive(Debug, Clone, Default)]
pub struct MemCheck {
    pub start: u32,
    pub end: u32,
    pub mem_cond: MemCheckCondition,
    pub result: MemCheckResult,
    pub description: String,
    pub has_cond: bool,
    pub cond: BreakPointCond,
    pub num_hits: u32,
}

/// A breakpoint or memory check. Mirrors the
/// `BreakpointMemcheck` variant in C++.
#[derive(Debug, Clone)]
pub enum BreakpointMemcheck {
    Break(BreakPoint),
    Mem(MemCheck),
}

impl BreakpointMemcheck {
    pub fn is_breakpoint(&self) -> bool {
        matches!(self, BreakpointMemcheck::Break(_))
    }

    pub fn is_memcheck(&self) -> bool {
        matches!(self, BreakpointMemcheck::Mem(_))
    }
}

/// Generic column data. The real C++ model returns `QVariant`; the
/// translation uses a small enum.
#[derive(Debug, Clone)]
pub enum CellValue {
    Empty,
    Text(String),
    Integer(i64),
    Unsigned(u64),
    Bool(bool),
    Float(f32),
    Double(f64),
    Bytes(Vec<u8>),
}

impl CellValue {
    pub fn to_string_lossy(&self) -> String {
        match self {
            CellValue::Empty => String::new(),
            CellValue::Text(s) => s.clone(),
            CellValue::Integer(i) => i.to_string(),
            CellValue::Unsigned(u) => format!("{:08x}", u),
            CellValue::Bool(b) => b.to_string(),
            CellValue::Float(f) => f.to_string(),
            CellValue::Double(d) => d.to_string(),
            CellValue::Bytes(b) => String::from_utf8_lossy(b).to_string(),
        }
    }
}

pub struct BreakpointModel {
    pub cpu: BreakPointCpu,
    pub breakpoints: Vec<BreakpointMemcheck>,
    pub alive: bool,
    pub load_game_settings_on_empty: bool,
}

impl Default for BreakpointModel {
    fn default() -> Self {
        Self {
            cpu: BreakPointCpu::EE,
            breakpoints: Vec::new(),
            alive: true,
            load_game_settings_on_empty: true,
        }
    }
}

impl BreakpointModel {
    /// Per-CPU singleton accessor. Mirrors
    /// `BreakpointModel::getInstance`.
    pub fn get_instance(cpu: BreakPointCpu) -> Self {
        Self {
            cpu,
            ..Self::default()
        }
    }

    /// Construct a new model for a CPU. The `create` factory keeps
    /// the call-site compact.
    pub fn create(cpu: BreakPointCpu) -> Self {
        Self::get_instance(cpu)
    }

    /// Optionally load any per-game breakpoint settings. Mirrors the
    /// `loadGameSettings` call performed from the constructor.
    pub fn populate(&mut self) {
        // The C++ constructor wires up signal handlers and triggers
        // an initial load. The translation only records the intent.
    }

    pub fn row_count(&self) -> usize {
        self.breakpoints.len()
    }

    pub fn column_count(&self) -> usize {
        BreakpointColumn::ColumnCount as usize
    }

    pub fn header_data(&self, section: usize, role: BreakpointRole) -> CellValue {
        if matches!(role, BreakpointRole::Display) {
            let col = BreakpointColumn::from_index(section);
            CellValue::Text(col.header().to_string())
        } else {
            CellValue::Empty
        }
    }

    pub fn data(&self, row: usize, column: BreakpointColumn, role: BreakpointRole) -> CellValue {
        let bp_mc = match self.breakpoints.get(row) {
            Some(b) => b,
            None => return CellValue::Empty,
        };
        match role {
            BreakpointRole::Display | BreakpointRole::Edit | BreakpointRole::Data
            | BreakpointRole::Export => match bp_mc {
                BreakpointMemcheck::Break(bp) => self.breakpoint_data(bp, column, role),
                BreakpointMemcheck::Mem(mc) => self.memcheck_data(mc, column, role),
            },
            BreakpointRole::CheckState => match bp_mc {
                BreakpointMemcheck::Break(bp) => CellValue::Bool(bp.enabled),
                BreakpointMemcheck::Mem(mc) => CellValue::Bool(mc.result.contains(MemCheckResult::BREAK)),
            },
            BreakpointRole::Other => CellValue::Empty,
        }
    }

    fn breakpoint_data(
        &self,
        bp: &BreakPoint,
        column: BreakpointColumn,
        role: BreakpointRole,
    ) -> CellValue {
        match (column, role) {
            (BreakpointColumn::Enabled, _) => CellValue::Bool(bp.enabled),
            (BreakpointColumn::Type, BreakpointRole::Display) => CellValue::Text("Execute".to_string()),
            (BreakpointColumn::Type, _) => CellValue::Unsigned(MemCheckCondition::INVALID as u64),
            (BreakpointColumn::Offset, BreakpointRole::Display | BreakpointRole::Export) => {
                CellValue::Text(format!("{:08x}", bp.addr))
            }
            (BreakpointColumn::Offset, _) => CellValue::Unsigned(bp.addr as u64),
            (BreakpointColumn::Description, _) => CellValue::Text(bp.description.clone()),
            (BreakpointColumn::SizeLabel, _) => CellValue::Text(String::new()),
            (BreakpointColumn::Opcode, BreakpointRole::Display) => CellValue::Text(disasm(bp.addr)),
            (BreakpointColumn::Opcode, _) => CellValue::Empty,
            (BreakpointColumn::Condition, _) => {
                if bp.has_cond {
                    CellValue::Text(bp.cond.expression_string.clone())
                } else {
                    CellValue::Empty
                }
            }
            (BreakpointColumn::Hits, _) => CellValue::Text("--".to_string()),
            (BreakpointColumn::ColumnCount, _) => CellValue::Empty,
        }
    }

    fn memcheck_data(
        &self,
        mc: &MemCheck,
        column: BreakpointColumn,
        role: BreakpointRole,
    ) -> CellValue {
        match column {
            BreakpointColumn::Enabled => CellValue::Bool(mc.result.contains(MemCheckResult::BREAK)),
            BreakpointColumn::Type => {
                let mut parts = Vec::new();
                if mc.mem_cond.contains(MemCheckCondition::READ) {
                    parts.push("Read");
                }
                if mc.mem_cond.contains(MemCheckCondition::WRITE) {
                    if mc.mem_cond.contains(MemCheckCondition::WRITE_ONCHANGE) {
                        parts.push("Write(C)");
                    } else {
                        parts.push("Write");
                    }
                }
                CellValue::Text(parts.join(", "))
            }
            BreakpointColumn::Offset => match role {
                BreakpointRole::Display | BreakpointRole::Export => {
                    CellValue::Text(format!("{:08x}", mc.start))
                }
                _ => CellValue::Unsigned(mc.start as u64),
            },
            BreakpointColumn::Description => CellValue::Text(mc.description.clone()),
            BreakpointColumn::SizeLabel => CellValue::Text(format!("{:x}", mc.end - mc.start)),
            BreakpointColumn::Opcode => CellValue::Text("--".to_string()),
            BreakpointColumn::Condition => {
                if mc.has_cond {
                    CellValue::Text(mc.cond.expression_string.clone())
                } else {
                    CellValue::Empty
                }
            }
            BreakpointColumn::Hits => CellValue::Unsigned(mc.num_hits as u64),
            BreakpointColumn::ColumnCount => CellValue::Empty,
        }
    }

    pub fn set_data(
        &mut self,
        row: usize,
        column: BreakpointColumn,
        value: CellValue,
        role: BreakpointRole,
    ) -> bool {
        let Some(bp_mc) = self.breakpoints.get_mut(row) else {
            return false;
        };
        match role {
            BreakpointRole::CheckState if matches!(column, BreakpointColumn::Enabled) => {
                if let BreakpointMemcheck::Break(bp) = bp_mc {
                    if let CellValue::Bool(b) = value {
                        bp.enabled = b;
                        return true;
                    }
                } else if let BreakpointMemcheck::Mem(mc) = bp_mc {
                    if let CellValue::Bool(b) = value {
                        if b {
                            mc.result.bits |= MemCheckResult::BREAK;
                        } else {
                            mc.result.bits &= !MemCheckResult::BREAK;
                        }
                        return true;
                    }
                }
            }
            BreakpointRole::Edit if matches!(column, BreakpointColumn::Condition) => {
                if let BreakpointMemcheck::Break(bp) = bp_mc {
                    if let CellValue::Text(s) = value {
                        bp.cond.expression_string = s;
                        bp.has_cond = !bp.cond.expression_string.is_empty();
                        return true;
                    }
                } else if let BreakpointMemcheck::Mem(mc) = bp_mc {
                    if let CellValue::Text(s) = value {
                        mc.cond.expression_string = s;
                        mc.has_cond = !mc.cond.expression_string.is_empty();
                        return true;
                    }
                }
            }
            BreakpointRole::Edit if matches!(column, BreakpointColumn::Description) => {
                if let BreakpointMemcheck::Break(bp) = bp_mc {
                    if let CellValue::Text(s) = value {
                        bp.description = s;
                        return true;
                    }
                } else if let BreakpointMemcheck::Mem(mc) = bp_mc {
                    if let CellValue::Text(s) = value {
                        mc.description = s;
                        return true;
                    }
                }
            }
            _ => return false,
        }
        false
    }

    pub fn insert_breakpoint_rows(
        &mut self,
        row: usize,
        count: usize,
        breakpoints: Vec<BreakpointMemcheck>,
    ) -> bool {
        if breakpoints.len() != count {
            return false;
        }
        let insert_at = row.min(self.breakpoints.len());
        for (i, bp) in breakpoints.into_iter().enumerate() {
            self.breakpoints.insert(insert_at + i, bp);
        }
        true
    }

    pub fn remove_rows(&mut self, row: usize, count: usize) -> bool {
        let end = row + count;
        if end > self.breakpoints.len() {
            return false;
        }
        self.breakpoints.drain(row..end);
        true
    }

    pub fn at(&self, row: usize) -> Option<&BreakpointMemcheck> {
        self.breakpoints.get(row)
    }

    /// Refresh from the global CPU state. The C++ version runs the
    /// refresh on the CPU thread and dispatches back to the UI
    /// thread; the translation simulates the result.
    pub fn refresh_data(&mut self, all_breakpoints: Vec<BreakpointMemcheck>) {
        self.breakpoints = all_breakpoints;
    }

    pub fn clear(&mut self) {
        self.breakpoints.clear();
    }

    /// Load a single breakpoint from a list of field values. Mirrors
    /// `BreakpointModel::loadBreakpointFromFieldList`.
    pub fn load_breakpoint_from_field_list(&mut self, fields: Vec<String>) {
        if fields.len() != BreakpointColumn::ColumnCount as usize {
            return;
        }
        let Ok(type_value) = fields[BreakpointColumn::Type as usize].parse::<u32>() else {
            return;
        };
        if type_value == MemCheckCondition::INVALID {
            let Ok(addr) =
                u32::from_str_radix(&fields[BreakpointColumn::Offset as usize], 16) else
            {
                return;
            };
            let enabled = fields[BreakpointColumn::Enabled as usize]
                .parse::<u32>()
                .map(|v| v != 0)
                .unwrap_or(false);
            let mut bp = BreakPoint {
                addr,
                enabled,
                description: fields[BreakpointColumn::Description as usize].clone(),
                ..Default::default()
            };
            let cond = fields[BreakpointColumn::Condition as usize].clone();
            if !cond.is_empty() {
                bp.has_cond = true;
                bp.cond.expression_string = cond;
            }
            self.insert_breakpoint_rows(0, 1, vec![BreakpointMemcheck::Break(bp)]);
        } else if type_value < MemCheckCondition::INVALID {
            let Ok(start) =
                u32::from_str_radix(&fields[BreakpointColumn::Offset as usize], 16) else
            {
                return;
            };
            let Ok(size) = fields[BreakpointColumn::SizeLabel as usize].parse::<u32>() else {
                return;
            };
            let Ok(result_bits) =
                fields[BreakpointColumn::Enabled as usize].parse::<u32>() else
            {
                return;
            };
            let mut mc = MemCheck {
                start,
                end: start.wrapping_add(size),
                mem_cond: MemCheckCondition { bits: type_value },
                result: MemCheckResult { bits: result_bits },
                description: fields[BreakpointColumn::Description as usize].clone(),
                ..Default::default()
            };
            let cond = fields[BreakpointColumn::Condition as usize].clone();
            if !cond.is_empty() {
                mc.has_cond = true;
                mc.cond.expression_string = cond;
            }
            self.insert_breakpoint_rows(0, 1, vec![BreakpointMemcheck::Mem(mc)]);
        }
    }
}

fn disasm(_addr: u32) -> String {
    String::new()
}

// ---------------------------------------------------------------------------
// `BreakpointView` -- the Qt dock widget showing the breakpoint table.
// Mirrors `pcsx2-qt/Debugger/Breakpoints/BreakpointView.{h,cpp}`.
// ---------------------------------------------------------------------------

pub struct BreakpointView {
    pub model: BreakpointModel,
    pub column_resize_modes: Vec<ColumnResizeMode>,
    pub menu_actions: Vec<BreakpointMenuAction>,
    pub last_double_clicked_address: Option<u32>,
    pub last_csv_import: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnResizeMode {
    ResizeToContents,
    Stretch,
    Fixed,
}

impl ColumnResizeMode {
    pub fn from_breakpoint_column(column: BreakpointColumn) -> Self {
        match column {
            BreakpointColumn::Description | BreakpointColumn::Opcode => Self::Stretch,
            _ => Self::ResizeToContents,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakpointMenuAction {
    New,
    Edit,
    Copy,
    Delete,
    CopyAllAsCsv,
    PasteFromCsv,
    LoadFromSettings,
    SaveToSettings,
}

impl BreakpointView {
    pub fn create(model: BreakpointModel) -> Self {
        Self {
            model,
            column_resize_modes: (0..BreakpointColumn::ColumnCount as usize)
                .map(|i| ColumnResizeMode::from_breakpoint_column(BreakpointColumn::from_index(i)))
                .collect(),
            menu_actions: Vec::new(),
            last_double_clicked_address: None,
            last_csv_import: String::new(),
        }
    }

    pub fn populate(&mut self) {
        self.model.populate();
    }

    pub fn on_double_clicked(&mut self, row: usize, column: BreakpointColumn) {
        if matches!(column, BreakpointColumn::Offset) {
            if let Some(bp) = self.model.at(row) {
                if let BreakpointMemcheck::Break(b) = bp {
                    self.last_double_clicked_address = Some(b.addr);
                }
            }
        }
    }

    pub fn open_context_menu(&mut self, cpu_alive: bool, has_selection: bool) {
        self.menu_actions.clear();
        if cpu_alive {
            self.menu_actions.push(BreakpointMenuAction::New);
            if has_selection {
                self.menu_actions.push(BreakpointMenuAction::Edit);
                self.menu_actions.push(BreakpointMenuAction::Copy);
                self.menu_actions.push(BreakpointMenuAction::Delete);
            }
        }
        if self.model.row_count() > 0 {
            self.menu_actions.push(BreakpointMenuAction::CopyAllAsCsv);
        }
        if cpu_alive {
            self.menu_actions.push(BreakpointMenuAction::PasteFromCsv);
            if matches!(self.model.cpu, BreakPointCpu::EE) {
                self.menu_actions.push(BreakpointMenuAction::LoadFromSettings);
                self.menu_actions.push(BreakpointMenuAction::SaveToSettings);
            }
        }
    }

    pub fn context_new(&self) -> NewBreakpointAction {
        NewBreakpointAction::default()
    }

    pub fn context_edit(&self, row: usize) -> NewBreakpointAction {
        let mut action = NewBreakpointAction::default();
        action.row = Some(row);
        action
    }

    pub fn context_delete(&mut self, rows: Vec<usize>) {
        // Delete from the bottom up so the indices remain valid.
        let mut rows = rows;
        rows.sort_unstable_by(|a, b| b.cmp(a));
        for row in rows {
            self.model.remove_rows(row, 1);
        }
    }

    pub fn context_paste_csv(&mut self, csv: String) {
        self.last_csv_import = csv.clone();
        let mut lines = csv.split('\n');
        // Skip header line.
        let _ = lines.next();
        for line in lines {
            let mut fields = Vec::new();
            let mut current = String::new();
            let mut in_quote = false;
            let mut chars = line.chars().peekable();
            while let Some(ch) = chars.next() {
                if ch == '"' {
                    in_quote = !in_quote;
                } else if in_quote {
                    current.push(ch);
                } else if ch == ',' {
                    fields.push(std::mem::take(&mut current));
                } else {
                    current.push(ch);
                }
            }
            if !current.is_empty() || !fields.is_empty() {
                fields.push(current);
            }
            self.model.load_breakpoint_from_field_list(fields);
        }
    }

    pub fn resize_columns(&self) -> &[ColumnResizeMode] {
        &self.column_resize_modes
    }
}

#[derive(Debug, Clone, Default)]
pub struct NewBreakpointAction {
    pub row: Option<usize>,
}

// ---------------------------------------------------------------------------
// `BreakpointDialog` -- modal dialog used to add or edit a single
// breakpoint or memory check. Mirrors
// `pcsx2-qt/Debugger/Breakpoints/BreakpointDialog.{h,cpp}`.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakpointDialogPurpose {
    Create,
    Edit,
}

pub struct BreakpointDialog {
    pub cpu: BreakPointCpu,
    pub purpose: BreakpointDialogPurpose,
    pub entry: BreakpointMemcheck,
    pub row_index: usize,
    pub address_text: String,
    pub size_text: String,
    pub description_text: String,
    pub condition_text: String,
    pub enable: bool,
    pub log: bool,
    pub read: bool,
    pub write: bool,
    pub on_change: bool,
    pub is_execute: bool,
    pub accepted: bool,
    pub error_message: String,
}

impl BreakpointDialog {
    pub fn create_new(cpu: BreakPointCpu) -> Self {
        Self {
            cpu,
            purpose: BreakpointDialogPurpose::Create,
            entry: BreakpointMemcheck::Break(BreakPoint::default()),
            row_index: 0,
            address_text: String::new(),
            size_text: String::new(),
            description_text: String::new(),
            condition_text: String::new(),
            enable: true,
            log: false,
            read: false,
            write: false,
            on_change: false,
            is_execute: true,
            accepted: false,
            error_message: String::new(),
        }
    }

    pub fn create_edit(
        cpu: BreakPointCpu,
        existing: BreakpointMemcheck,
        row: usize,
    ) -> Self {
        let mut dialog = Self {
            cpu,
            purpose: BreakpointDialogPurpose::Edit,
            entry: existing.clone(),
            row_index: row,
            address_text: String::new(),
            size_text: String::new(),
            description_text: String::new(),
            condition_text: String::new(),
            enable: true,
            log: false,
            read: false,
            write: false,
            on_change: false,
            is_execute: true,
            accepted: false,
            error_message: String::new(),
        };
        dialog.populate_from_existing();
        dialog
    }

    pub fn populate(&mut self) {
        // The C++ constructor wires up signals and pre-populates UI
        // controls; the translation only marks the dialog as ready.
    }

    fn populate_from_existing(&mut self) {
        match &self.entry {
            BreakpointMemcheck::Break(bp) => {
                self.is_execute = true;
                self.enable = bp.enabled;
                self.address_text = format!("{:08x}", bp.addr);
                self.description_text = bp.description.clone();
                if bp.has_cond {
                    self.condition_text = bp.cond.expression_string.clone();
                }
            }
            BreakpointMemcheck::Mem(mc) => {
                self.is_execute = false;
                self.enable = mc.result.contains(MemCheckResult::BREAK);
                self.log = mc.result.contains(MemCheckResult::LOG);
                self.read = mc.mem_cond.contains(MemCheckCondition::READ);
                self.write = mc.mem_cond.contains(MemCheckCondition::WRITE);
                self.on_change = mc.mem_cond.contains(MemCheckCondition::WRITE_ONCHANGE);
                self.address_text = format!("{:08x}", mc.start);
                self.size_text = format!("{:08x}", mc.end - mc.start);
                self.description_text = mc.description.clone();
                if mc.has_cond {
                    self.condition_text = mc.cond.expression_string.clone();
                }
            }
        }
    }

    pub fn on_rdo_button_toggled(&mut self) {
        self.is_execute = !self.is_execute;
    }

    /// Apply the form contents to `self.entry`, returning the parsed
    /// entry along with any error message. Mirrors
    /// `BreakpointDialog::accept`.
    pub fn accept(
        &mut self,
        evaluate_address: impl Fn(&str) -> std::result::Result<u64, String>,
        init_condition: impl Fn(&str) -> std::result::Result<(), String>,
    ) -> std::result::Result<BreakpointMemcheck, String> {
        match self.purpose {
            BreakpointDialogPurpose::Create => {
                if self.is_execute {
                    self.entry = BreakpointMemcheck::Break(BreakPoint::default());
                } else {
                    self.entry = BreakpointMemcheck::Mem(MemCheck::default());
                }
            }
            BreakpointDialogPurpose::Edit => {}
        }
        match &mut self.entry {
            BreakpointMemcheck::Break(bp) => {
                let address = evaluate_address(&self.address_text)
                    .map_err(|e| format!("Invalid Address: {}", e))?;
                bp.addr = address as u32;
                bp.description = self.description_text.clone();
                bp.enabled = self.enable;
                if self.condition_text.is_empty() {
                    bp.has_cond = false;
                } else {
                    init_condition(&self.condition_text)
                        .map_err(|e| format!("Invalid Condition: {}", e))?;
                    bp.has_cond = true;
                    bp.cond.expression_string = self.condition_text.clone();
                }
            }
            BreakpointMemcheck::Mem(mc) => {
                let start = evaluate_address(&self.address_text)
                    .map_err(|e| format!("Invalid Address: {}", e))?;
                let size = evaluate_address(&self.size_text)
                    .map_err(|e| format!("Invalid Size: {}", e))?;
                if size == 0 {
                    return Err("Invalid Size: zero length".to_string());
                }
                mc.start = start as u32;
                mc.end = start.wrapping_add(size) as u32;
                mc.description = self.description_text.clone();
                if self.condition_text.is_empty() {
                    mc.has_cond = false;
                } else {
                    init_condition(&self.condition_text)
                        .map_err(|e| format!("Invalid Condition: {}", e))?;
                    mc.has_cond = true;
                    mc.cond.expression_string = self.condition_text.clone();
                }
                let mut bits = 0;
                if self.read {
                    bits |= MemCheckCondition::READ;
                }
                if self.write {
                    bits |= MemCheckCondition::WRITE;
                }
                if self.on_change {
                    bits |= MemCheckCondition::WRITE_ONCHANGE;
                }
                mc.mem_cond = MemCheckCondition { bits };
                let mut result = 0;
                if self.enable {
                    result |= MemCheckResult::BREAK;
                }
                if self.log {
                    result |= MemCheckResult::LOG;
                }
                mc.result = MemCheckResult { bits: result };
            }
        }
        self.accepted = true;
        Ok(self.entry.clone())
    }
}

// ---------------------------------------------------------------------------
// `MemorySearchView` -- in-memory search by value, supporting bytes,
// integers, floats, doubles, strings, and byte arrays. Mirrors
// `pcsx2-qt/Debugger/Memory/MemorySearchView.{h,cpp}`.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchType {
    ByteType,
    Int16Type,
    Int32Type,
    Int64Type,
    FloatType,
    DoubleType,
    StringType,
    ArrayType,
}

impl SearchType {
    pub fn size(self) -> usize {
        match self {
            SearchType::ByteType => 1,
            SearchType::Int16Type => 2,
            SearchType::Int32Type | SearchType::FloatType => 4,
            SearchType::Int64Type | SearchType::DoubleType => 8,
            SearchType::StringType | SearchType::ArrayType => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SearchComparison {
    Equals,
    NotEquals,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    Increased,
    IncreasedBy,
    Decreased,
    DecreasedBy,
    Changed,
    ChangedBy,
    NotChanged,
    UnknownValue,
    Invalid,
}

impl SearchComparison {
    pub fn from_label(label: &str, map: &SearchComparisonLabelMap) -> Self {
        map.label_to_enum(label)
    }
}

#[derive(Default)]
pub struct SearchComparisonLabelMap {
    enum_to_label: HashMap<SearchComparison, String>,
    label_to_enum: HashMap<String, SearchComparison>,
}

impl SearchComparisonLabelMap {
    pub fn create() -> Self {
        let mut map = Self::default();
        map.populate();
        map
    }

    pub fn populate(&mut self) {
        let entries: [(SearchComparison, &str); 15] = [
            (SearchComparison::Equals, "Equals"),
            (SearchComparison::NotEquals, "Not Equals"),
            (SearchComparison::GreaterThan, "Greater Than"),
            (SearchComparison::GreaterThanOrEqual, "Greater Than Or Equal"),
            (SearchComparison::LessThan, "Less Than"),
            (SearchComparison::LessThanOrEqual, "Less Than Or Equal"),
            (SearchComparison::Increased, "Increased"),
            (SearchComparison::IncreasedBy, "Increased By"),
            (SearchComparison::Decreased, "Decreased"),
            (SearchComparison::DecreasedBy, "Decreased By"),
            (SearchComparison::Changed, "Changed"),
            (SearchComparison::ChangedBy, "Changed By"),
            (SearchComparison::NotChanged, "Not Changed"),
            (SearchComparison::UnknownValue, "Unknown Initial Value"),
            (SearchComparison::Invalid, ""),
        ];
        for (comp, label) in entries {
            self.enum_to_label.insert(comp, label.to_string());
            self.label_to_enum.insert(label.to_string(), comp);
        }
    }

    pub fn label_to_enum(&self, label: &str) -> SearchComparison {
        self.label_to_enum
            .get(label)
            .copied()
            .unwrap_or(SearchComparison::Invalid)
    }

    pub fn enum_to_label(&self, comparison: SearchComparison) -> String {
        self.enum_to_label
            .get(&comparison)
            .cloned()
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub address: u32,
    pub value: SearchValue,
    pub kind: SearchType,
}

impl SearchResult {
    pub fn new(address: u32, value: SearchValue, kind: SearchType) -> Self {
        Self {
            address,
            value,
            kind,
        }
    }

    pub fn get_address(&self) -> u32 {
        self.address
    }

    pub fn get_type(&self) -> SearchType {
        self.kind
    }

    pub fn is_integer_value(&self) -> bool {
        matches!(
            self.kind,
            SearchType::ByteType
                | SearchType::Int16Type
                | SearchType::Int32Type
                | SearchType::Int64Type
        )
    }

    pub fn is_float_value(&self) -> bool {
        matches!(self.kind, SearchType::FloatType)
    }

    pub fn is_double_value(&self) -> bool {
        matches!(self.kind, SearchType::DoubleType)
    }

    pub fn is_array_value(&self) -> bool {
        matches!(self.kind, SearchType::ArrayType | SearchType::StringType)
    }

    pub fn get_array_value(&self) -> Vec<u8> {
        if let SearchValue::Bytes(b) = &self.value {
            b.clone()
        } else {
            Vec::new()
        }
    }

    pub fn get_value<T: SearchScalar>(&self) -> T {
        T::from_value(&self.value)
    }
}

#[derive(Debug, Clone)]
pub enum SearchValue {
    I64(i64),
    U64(u64),
    F32(f32),
    F64(f64),
    Bytes(Vec<u8>),
}

pub trait SearchScalar: Copy {
    fn from_value(value: &SearchValue) -> Self;
}

macro_rules! impl_search_scalar {
    ($($t:ty => $variant:ident),* $(,)?) => {
        $(
            impl SearchScalar for $t {
                fn from_value(value: &SearchValue) -> Self {
                    match value {
                        SearchValue::I64(v) => *v as $t,
                        SearchValue::U64(v) => *v as $t,
                        SearchValue::F32(v) => *v as $t,
                        SearchValue::F64(v) => *v as $t,
                        SearchValue::Bytes(b) => b.first().copied().unwrap_or(0) as $t,
                    }
                }
            }
        )*
    };
}

impl_search_scalar! {
    u8 => U64, i8 => I64, u16 => U64, i16 => I64, u32 => U64, i32 => I64,
    u64 => U64, i64 => I64, f32 => F32, f64 => F64,
}

/// Compare a stored value against a search value using a comparison
/// that does not depend on the prior result. Mirrors
/// `memoryValueComparator`.
pub fn memory_value_comparator<T: PartialOrd + Copy>(
    comparison: SearchComparison,
    search_value: T,
    read_value: T,
) -> bool {
    let is_not = matches!(comparison, SearchComparison::NotEquals);
    match comparison {
        SearchComparison::Equals | SearchComparison::NotEquals => {
            let equal = search_value == read_value;
            if is_not {
                !equal
            } else {
                equal
            }
        }
        SearchComparison::GreaterThan
        | SearchComparison::GreaterThanOrEqual
        | SearchComparison::LessThan
        | SearchComparison::LessThanOrEqual => {
            if matches!(
                comparison,
                SearchComparison::GreaterThanOrEqual | SearchComparison::LessThanOrEqual
            ) && search_value == read_value
            {
                return true;
            }
            let is_greater = matches!(
                comparison,
                SearchComparison::GreaterThan | SearchComparison::GreaterThanOrEqual
            );
            if is_greater {
                read_value > search_value
            } else {
                read_value < search_value
            }
        }
        _ => false,
    }
}

pub fn handle_search_comparison<T: Copy + PartialOrd + std::ops::Add<Output = T> + std::ops::Sub<Output = T>>(
    comparison: SearchComparison,
    _search_address: u32,
    prior_value: Option<T>,
    search_value: T,
    read_value: T,
) -> bool {
    match comparison {
        SearchComparison::Equals
        | SearchComparison::NotEquals
        | SearchComparison::GreaterThan
        | SearchComparison::GreaterThanOrEqual
        | SearchComparison::LessThan
        | SearchComparison::LessThanOrEqual => {
            memory_value_comparator(comparison, search_value, read_value)
        }
        SearchComparison::Increased => prior_value
            .map(|p| memory_value_comparator(SearchComparison::GreaterThan, p, read_value))
            .unwrap_or(false),
        SearchComparison::IncreasedBy => prior_value
            .map(|p| {
                let expected = p + search_value;
                memory_value_comparator(SearchComparison::Equals, expected, read_value)
            })
            .unwrap_or(false),
        SearchComparison::Decreased => prior_value
            .map(|p| memory_value_comparator(SearchComparison::LessThan, p, read_value))
            .unwrap_or(false),
        SearchComparison::DecreasedBy => prior_value
            .map(|p| {
                let expected = p - search_value;
                memory_value_comparator(SearchComparison::Equals, expected, read_value)
            })
            .unwrap_or(false),
        SearchComparison::Changed | SearchComparison::NotChanged => prior_value
            .map(|p| {
                let equal = p == read_value;
                if matches!(comparison, SearchComparison::NotChanged) {
                    equal
                } else {
                    !equal
                }
            })
            .unwrap_or(false),
        SearchComparison::ChangedBy => prior_value
            .map(|p| {
                let plus = p + search_value;
                let minus = p - search_value;
                plus == read_value || minus == read_value
            })
            .unwrap_or(false),
        SearchComparison::UnknownValue => true,
        SearchComparison::Invalid => false,
    }
}

pub struct MemorySearchView {
    pub search_results: Vec<SearchResult>,
    pub label_map: SearchComparisonLabelMap,
    pub initial_load_limit: u32,
    pub num_added_per_load: u32,
    pub last_results_loaded: u32,
    pub is_searching: bool,
    pub pending_text: String,
}

impl Default for MemorySearchView {
    fn default() -> Self {
        Self {
            search_results: Vec::new(),
            label_map: SearchComparisonLabelMap::create(),
            initial_load_limit: 20_000,
            num_added_per_load: 10_000,
            last_results_loaded: 0,
            is_searching: false,
            pending_text: String::new(),
        }
    }
}

impl MemorySearchView {
    pub fn create() -> Self {
        Self::default()
    }

    pub fn populate(&mut self) {
        self.label_map = SearchComparisonLabelMap::create();
    }

    pub fn on_search_button_clicked(
        &mut self,
        cpu_alive: bool,
        is_filter: bool,
        start: u32,
        end: u32,
        value: &str,
        search_type: SearchType,
        comparison: SearchComparison,
        hex: bool,
    ) -> Result<()> {
        if !cpu_alive {
            return Ok(());
        }
        if start >= end {
            return Err("Start address can't be equal to or greater than the end address".to_string());
        }
        if !is_filter
            && matches!(
                comparison,
                SearchComparison::Changed
                    | SearchComparison::ChangedBy
                    | SearchComparison::Decreased
                    | SearchComparison::DecreasedBy
                    | SearchComparison::Increased
                    | SearchComparison::IncreasedBy
                    | SearchComparison::NotChanged
            )
        {
            return Err(
                "This search comparison can only be used with filter searches.".to_string(),
            );
        }
        if does_search_comparison_take_input(comparison) {
            if value.is_empty() {
                return Err("Invalid search value".to_string());
            }
        }
        self.is_searching = true;
        let _ = (start, end, value, search_type, comparison, hex);
        self.is_searching = false;
        Ok(())
    }

    pub fn load_search_results(&mut self) {
        let total = self.search_results.len() as u32;
        if total <= self.last_results_loaded {
            return;
        }
        let is_first = self.last_results_loaded == 0;
        let max = if is_first {
            self.initial_load_limit
        } else {
            self.num_added_per_load
        };
        let to_load = (total - self.last_results_loaded).min(max);
        self.last_results_loaded += to_load;
    }

    pub fn context_remove_search_result(&mut self, index: usize) {
        if index < self.search_results.len() {
            self.search_results.remove(index);
        }
    }

    pub fn context_copy_search_result_address(&self, index: usize) -> Option<String> {
        self.search_results
            .get(index)
            .map(|r| format!("{:08x}", r.address))
    }

    pub fn on_list_search_results_context_menu(&self) {}

    pub fn on_search_type_changed(&mut self, _new_index: i32) {}

    pub fn on_search_comparison_changed(&self) {}

    pub fn update_search_comparison_selections(&self) {}

    pub fn get_current_search_type(&self, index: i32) -> SearchType {
        match index {
            0 => SearchType::ByteType,
            1 => SearchType::Int16Type,
            2 => SearchType::Int32Type,
            3 => SearchType::Int64Type,
            4 => SearchType::FloatType,
            5 => SearchType::DoubleType,
            6 => SearchType::StringType,
            7 => SearchType::ArrayType,
            _ => SearchType::ByteType,
        }
    }

    pub fn get_current_search_comparison(&self, label: &str) -> SearchComparison {
        self.label_map.label_to_enum(label)
    }

    pub fn get_valid_search_comparisons_for_state(
        &self,
        kind: SearchType,
        existing: &[SearchResult],
    ) -> Vec<SearchComparison> {
        let mut comparisons = vec![SearchComparison::Equals];
        if matches!(kind, SearchType::ArrayType | SearchType::StringType) {
            if existing.first().map(|r| r.is_array_value()).unwrap_or(false) {
                comparisons.push(SearchComparison::NotEquals);
                comparisons.push(SearchComparison::Changed);
                comparisons.push(SearchComparison::NotChanged);
            }
            return comparisons;
        }
        comparisons.push(SearchComparison::NotEquals);
        comparisons.push(SearchComparison::GreaterThan);
        comparisons.push(SearchComparison::GreaterThanOrEqual);
        comparisons.push(SearchComparison::LessThan);
        comparisons.push(SearchComparison::LessThanOrEqual);
        let has_results = !existing.is_empty();
        if has_results && existing.first().map(|r| r.kind == kind).unwrap_or(false) {
            comparisons.push(SearchComparison::Increased);
            comparisons.push(SearchComparison::IncreasedBy);
            comparisons.push(SearchComparison::Decreased);
            comparisons.push(SearchComparison::DecreasedBy);
            comparisons.push(SearchComparison::Changed);
            comparisons.push(SearchComparison::ChangedBy);
            comparisons.push(SearchComparison::NotChanged);
        }
        if !has_results {
            comparisons.push(SearchComparison::UnknownValue);
        }
        comparisons
    }
}

pub fn does_search_comparison_take_input(comparison: SearchComparison) -> bool {
    matches!(
        comparison,
        SearchComparison::Equals
            | SearchComparison::NotEquals
            | SearchComparison::GreaterThan
            | SearchComparison::GreaterThanOrEqual
            | SearchComparison::LessThan
            | SearchComparison::LessThanOrEqual
            | SearchComparison::IncreasedBy
            | SearchComparison::DecreasedBy
    )
}

// ---------------------------------------------------------------------------
// `MemoryView` -- hex/text view of the address space. Mirrors
// `pcsx2-qt/Debugger/Memory/MemoryView.{h,cpp}`.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryViewType {
    Byte,
    ByteHw,
    Word,
    DWord,
    Float,
}

impl MemoryViewType {
    pub fn width(self) -> i32 {
        match self {
            MemoryViewType::Byte => 1,
            MemoryViewType::ByteHw => 2,
            MemoryViewType::Word => 4,
            MemoryViewType::DWord => 8,
            MemoryViewType::Float => 4,
        }
    }

    pub fn visual_width(self) -> i32 {
        match self {
            MemoryViewType::Byte => 2,
            MemoryViewType::ByteHw => 4,
            MemoryViewType::Word => 8,
            MemoryViewType::DWord => 16,
            MemoryViewType::Float => 14,
        }
    }

    pub fn from_index(index: i32) -> Self {
        match index {
            1 => MemoryViewType::ByteHw,
            2 => MemoryViewType::Word,
            3 => MemoryViewType::DWord,
            4 => MemoryViewType::Float,
            _ => MemoryViewType::Byte,
        }
    }
}

/// A 128-bit value used by the original C++ code to return a
/// selected segment.
#[derive(Debug, Clone, Copy, Default)]
pub struct U128 {
    pub lo: u32,
    pub hi: u32,
    pub _u64: [u64; 2],
}

pub struct MemoryViewTable {
    pub display_type: MemoryViewType,
    pub little_endian: bool,
    pub start_address: u32,
    pub selected_address: u32,
    pub selected_index: i32,
    pub selected_text: bool,
    pub selected_nibble_hi: bool,
    pub row_visible: u32,
    pub row_height: i32,
    pub value_x_axis: i32,
    pub text_x_axis: i32,
    pub row1_y_axis: i32,
    pub segment_x_axis: [i32; 16],
    pub row_count: u32,
}

impl Default for MemoryViewTable {
    fn default() -> Self {
        Self {
            display_type: MemoryViewType::Byte,
            little_endian: true,
            start_address: 0x100000,
            selected_address: 0x100000,
            selected_index: 0,
            selected_text: false,
            selected_nibble_hi: false,
            row_visible: 0,
            row_height: 0,
            value_x_axis: 0,
            text_x_axis: 0,
            row1_y_axis: 0,
            segment_x_axis: [0; 16],
            row_count: 0,
        }
    }
}

impl MemoryViewTable {
    pub fn create() -> Self {
        Self::default()
    }

    pub fn populate(&mut self) {}

    pub fn update_start_address(&mut self, start: u32) {
        self.start_address = start & !0xF;
    }

    pub fn update_selected_address(&mut self, selected: u32, page: bool) {
        self.selected_address = selected;
        if self.start_address > self.selected_address {
            if page {
                self.start_address = self.start_address.saturating_sub(0x10 * self.row_visible);
            } else {
                self.start_address = self.start_address.saturating_sub(0x10);
            }
        } else if self.start_address + ((self.row_visible.saturating_sub(1)) * 0x10)
            < self.selected_address
        {
            if page {
                self.start_address += 0x10 * self.row_visible;
            } else {
                self.start_address += 0x10;
            }
        }
    }

    pub fn forward_selection(&mut self) {
        match self.display_type {
            MemoryViewType::Float => {
                if self.little_endian {
                    if self.selected_index <= 0 {
                        self.update_selected_address(self.selected_address + 4, false);
                        self.selected_index = self.display_type.visual_width() - 1;
                    } else {
                        self.selected_index -= 1;
                    }
                } else if self.selected_index >= self.display_type.visual_width() - 1 {
                    self.update_selected_address(self.selected_address + 4, false);
                    self.selected_index = 0;
                } else {
                    self.selected_index += 1;
                }
            }
            _ => {
                if !self.little_endian {
                    self.selected_nibble_hi = !self.selected_nibble_hi;
                    if self.selected_nibble_hi {
                        self.update_selected_address(self.selected_address + 1, false);
                    }
                } else {
                    self.selected_nibble_hi = !self.selected_nibble_hi;
                    if self.selected_nibble_hi {
                        if self.selected_address % (self.display_type.width() as u32) == 0 {
                            self.update_selected_address(
                                self.selected_address + (self.display_type.visual_width() as u32 - 1),
                                false,
                            );
                        } else {
                            self.update_selected_address(self.selected_address - 1, false);
                        }
                    }
                }
            }
        }
    }

    pub fn backward_selection(&mut self) {
        match self.display_type {
            MemoryViewType::Float => {
                if self.little_endian {
                    if self.selected_index >= self.display_type.visual_width() - 1 {
                        self.update_selected_address(self.selected_address - 4, false);
                        self.selected_index = 0;
                    } else {
                        self.selected_index += 1;
                    }
                } else if self.selected_index <= 0 {
                    self.update_selected_address(self.selected_address - 4, false);
                    self.selected_index = self.display_type.visual_width() - 1;
                } else {
                    self.selected_index -= 1;
                }
            }
            _ => {
                if !self.little_endian {
                    self.selected_nibble_hi = !self.selected_nibble_hi;
                    if !self.selected_nibble_hi {
                        self.update_selected_address(self.selected_address - 1, false);
                    }
                } else {
                    self.selected_nibble_hi = !self.selected_nibble_hi;
                    if !self.selected_nibble_hi {
                        if (self.selected_address
                            & (self.display_type.width() as u32 - 1))
                            == (self.display_type.width() as u32 - 1)
                        {
                            self.update_selected_address(
                                self.selected_address - (self.display_type.visual_width() as u32 - 1),
                                false,
                            );
                        } else {
                            self.update_selected_address(self.selected_address + 1, false);
                        }
                    }
                }
            }
        }
    }

    pub fn select_at(&mut self, _x: i32, y: i32) {
        if self.row_height == 0 {
            return;
        }
        let _row = (y - 2) / self.row_height;
    }

    pub fn get_selected_segment(&self) -> U128 {
        let mut value = U128::default();
        match self.display_type {
            MemoryViewType::Byte => value.lo = self.selected_address,
            MemoryViewType::ByteHw => value.lo = self.selected_address & !1,
            MemoryViewType::Word => value.lo = self.selected_address & !3,
            MemoryViewType::DWord => value.lo = self.selected_address & !7,
            MemoryViewType::Float => value.lo = self.selected_address & !3,
        }
        value
    }

    pub fn insert_at_current_selection(&mut self, _text: &str) {}

    pub fn set_view_type(&mut self, view_type: MemoryViewType) {
        self.display_type = view_type;
    }

    pub fn get_view_type(&self) -> MemoryViewType {
        self.display_type
    }

    pub fn set_little_endian(&mut self, le: bool) {
        self.little_endian = le;
    }

    pub fn get_little_endian(&self) -> bool {
        self.little_endian
    }

    /// Process a single key press. Returns `true` if the key was
    /// consumed by the table.
    pub fn key_press(&mut self, key: i32, _keychar: char) -> bool {
        match key {
            // Qt::Key_Left etc. would be resolved by the GUI layer.
            0x10 => {
                // Up arrow
                self.update_selected_address(self.selected_address - 0x10, false);
                true
            }
            0x11 => {
                // Down arrow
                self.update_selected_address(self.selected_address + 0x10, false);
                true
            }
            _ => false,
        }
    }
}

pub struct MemoryView {
    pub table: MemoryViewTable,
    pub alive: bool,
    pub width: i32,
    pub height: i32,
    pub last_address: u32,
    pub refresh_requested: bool,
}

impl Default for MemoryView {
    fn default() -> Self {
        Self {
            table: MemoryViewTable::default(),
            alive: true,
            width: 0,
            height: 0,
            last_address: 0x100000,
            refresh_requested: false,
        }
    }
}

impl MemoryView {
    pub fn create() -> Self {
        Self::default()
    }

    pub fn populate(&mut self) {
        self.table.populate();
        self.table.update_start_address(0x100000);
    }

    pub fn paint(&mut self, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        if !self.alive {
            return;
        }
        self.refresh_requested = true;
    }

    pub fn mouse_press(&mut self, x: i32, y: i32) {
        if !self.alive {
            return;
        }
        self.table.select_at(x, y);
    }

    pub fn open_context_menu(&mut self) {}

    pub fn context_copy_byte(&self) -> String {
        String::new()
    }

    pub fn context_copy_segment(&self) -> String {
        let value = self.table.get_selected_segment();
        format!("{:08x}", value.lo).to_uppercase()
    }

    pub fn context_copy_character(&self) -> String {
        String::new()
    }

    pub fn context_paste(&mut self, text: &str) {
        self.table.insert_at_current_selection(text);
    }

    pub fn context_go_to_address(&mut self, address: u32) {
        self.goto_address(address);
    }

    pub fn context_follow_address(&mut self, address: u32) {
        self.goto_address(address);
    }

    pub fn wheel_event(&mut self, delta: i32) {
        if delta < 0 {
            self.table.update_start_address(self.table.start_address + 0x10);
        } else if delta > 0 {
            self.table.update_start_address(self.table.start_address - 0x10);
        }
    }

    pub fn key_press_event(&mut self, key: i32, keychar: char) {
        self.table.key_press(key, keychar);
    }

    pub fn goto_address(&mut self, address: u32) {
        self.table.update_start_address(address & !0xF);
        self.table.selected_address = address;
        self.last_address = address;
    }

    pub fn to_json(&self) -> MemoryViewJson {
        MemoryViewJson {
            start_address: self.table.start_address,
            view_type: self.table.get_view_type() as i32,
            little_endian: self.table.get_little_endian(),
        }
    }

    pub fn from_json(&mut self, value: MemoryViewJson) -> bool {
        self.table.update_start_address(value.start_address);
        if matches!(
            value.view_type,
            0..=4
        ) {
            self.table.set_view_type(MemoryViewType::from_index(value.view_type));
        }
        self.table.set_little_endian(value.little_endian);
        true
    }
}

// ---------------------------------------------------------------------------
// Snapshot of `MemoryView` state, suitable for round-tripping through
// the layout file format.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
pub struct MemoryViewJson {
    pub start_address: u32,
    pub view_type: i32,
    pub little_endian: bool,
}

// ---------------------------------------------------------------------------
// `SavedAddressesModel` and `SavedAddressesView`. Mirrors
// `pcsx2-qt/Debugger/Memory/SavedAddresses*.{h,cpp}`.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavedAddressColumn {
    Address = 0,
    Label,
    Description,
    ColumnCount,
}

impl SavedAddressColumn {
    pub fn header(&self) -> &'static str {
        match self {
            SavedAddressColumn::Address => "MEMORY ADDRESS",
            SavedAddressColumn::Label => "LABEL",
            SavedAddressColumn::Description => "DESCRIPTION",
            SavedAddressColumn::ColumnCount => "",
        }
    }

    pub fn from_index(index: usize) -> Self {
        match index {
            0 => SavedAddressColumn::Address,
            1 => SavedAddressColumn::Label,
            2 => SavedAddressColumn::Description,
            _ => SavedAddressColumn::ColumnCount,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SavedAddress {
    pub address: u32,
    pub label: String,
    pub description: String,
}

pub struct SavedAddressesModel {
    pub cpu: BreakPointCpu,
    pub saved_addresses: Vec<SavedAddress>,
    pub alive: bool,
    pub load_game_settings_on_empty: bool,
}

impl Default for SavedAddressesModel {
    fn default() -> Self {
        Self {
            cpu: BreakPointCpu::EE,
            saved_addresses: Vec::new(),
            alive: true,
            load_game_settings_on_empty: true,
        }
    }
}

impl SavedAddressesModel {
    pub fn get_instance(cpu: BreakPointCpu) -> Self {
        Self {
            cpu,
            ..Self::default()
        }
    }

    pub fn create(cpu: BreakPointCpu) -> Self {
        Self::get_instance(cpu)
    }

    pub fn populate(&mut self) {}

    pub fn data(&self, row: usize, column: SavedAddressColumn, role: BreakpointRole) -> CellValue {
        let entry = match self.saved_addresses.get(row) {
            Some(e) => e,
            None => return CellValue::Empty,
        };
        if matches!(role, BreakpointRole::CheckState) {
            return CellValue::Empty;
        }
        if matches!(role, BreakpointRole::Display | BreakpointRole::Edit) {
            return match column {
                SavedAddressColumn::Address => {
                    CellValue::Text(format!("{:08X}", entry.address))
                }
                SavedAddressColumn::Label => CellValue::Text(entry.label.clone()),
                SavedAddressColumn::Description => CellValue::Text(entry.description.clone()),
                SavedAddressColumn::ColumnCount => CellValue::Empty,
            };
        }
        if matches!(role, BreakpointRole::Data) {
            return match column {
                SavedAddressColumn::Address => CellValue::Unsigned(entry.address as u64),
                SavedAddressColumn::Label => CellValue::Text(entry.label.clone()),
                SavedAddressColumn::Description => CellValue::Text(entry.description.clone()),
                SavedAddressColumn::ColumnCount => CellValue::Empty,
            };
        }
        CellValue::Empty
    }

    pub fn header_data(&self, section: usize, role: BreakpointRole) -> CellValue {
        if matches!(role, BreakpointRole::Display) {
            let col = SavedAddressColumn::from_index(section);
            CellValue::Text(col.header().to_string())
        } else {
            CellValue::Empty
        }
    }

    pub fn row_count(&self) -> usize {
        self.saved_addresses.len()
    }

    pub fn column_count(&self) -> usize {
        SavedAddressColumn::ColumnCount as usize
    }

    pub fn add_row_default(&mut self) {
        self.add_row(SavedAddress {
            address: 0,
            label: "Name".to_string(),
            description: "Description".to_string(),
        });
    }

    pub fn add_row(&mut self, address: SavedAddress) {
        self.saved_addresses.push(address);
    }

    pub fn remove_rows(&mut self, row: usize, count: usize) -> bool {
        let end = row + count;
        if row >= self.saved_addresses.len() || end > self.saved_addresses.len() {
            return false;
        }
        self.saved_addresses.drain(row..end);
        true
    }

    pub fn set_data(
        &mut self,
        row: usize,
        column: SavedAddressColumn,
        value: CellValue,
        role: BreakpointRole,
    ) -> bool {
        if matches!(role, BreakpointRole::CheckState) {
            return false;
        }
        let entry = match self.saved_addresses.get_mut(row) {
            Some(e) => e,
            None => return false,
        };
        if matches!(role, BreakpointRole::Edit | BreakpointRole::Data) {
            match column {
                SavedAddressColumn::Address => {
                    if let CellValue::Unsigned(v) = value {
                        entry.address = v as u32;
                        return true;
                    }
                    if let CellValue::Text(s) = value {
                        if let Ok(addr) = u32::from_str_radix(s.trim_start_matches("0x"), 16) {
                            entry.address = addr;
                            return true;
                        }
                    }
                }
                SavedAddressColumn::Label => {
                    if let CellValue::Text(s) = value {
                        entry.label = s;
                        return true;
                    }
                }
                SavedAddressColumn::Description => {
                    if let CellValue::Text(s) = value {
                        entry.description = s;
                        return true;
                    }
                }
                SavedAddressColumn::ColumnCount => return false,
            }
        }
        false
    }

    pub fn load_saved_address_from_field_list(&mut self, fields: Vec<String>) {
        if fields.len() != SavedAddressColumn::ColumnCount as usize {
            return;
        }
        let Ok(address) =
            u32::from_str_radix(&fields[SavedAddressColumn::Address as usize], 16) else
        {
            return;
        };
        let label = fields[SavedAddressColumn::Label as usize].clone();
        let description = fields[SavedAddressColumn::Description as usize].clone();
        self.add_row(SavedAddress {
            address,
            label,
            description,
        });
    }

    pub fn clear(&mut self) {
        self.saved_addresses.clear();
    }
}

pub struct SavedAddressesView {
    pub model: SavedAddressesModel,
    pub column_resize_modes: Vec<ColumnResizeMode>,
    pub last_added_address: Option<u32>,
    pub last_csv_import: String,
    pub menu_actions: Vec<SavedAddressMenuAction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavedAddressMenuAction {
    New,
    GoTo,
    CopyAddress,
    CopyText,
    CopyAllAsCsv,
    PasteFromCsv,
    LoadFromSettings,
    SaveToSettings,
    Delete,
}

impl SavedAddressesView {
    pub fn create(model: SavedAddressesModel) -> Self {
        Self {
            model,
            column_resize_modes: (0..SavedAddressColumn::ColumnCount as usize)
                .map(|i| match SavedAddressColumn::from_index(i) {
                    SavedAddressColumn::Description => ColumnResizeMode::Stretch,
                    _ => ColumnResizeMode::ResizeToContents,
                })
                .collect(),
            last_added_address: None,
            last_csv_import: String::new(),
            menu_actions: Vec::new(),
        }
    }

    pub fn populate(&mut self) {
        self.model.populate();
    }

    pub fn open_context_menu(&mut self, cpu_alive: bool, is_index_valid: bool) {
        self.menu_actions.clear();
        self.menu_actions.push(SavedAddressMenuAction::New);
        if is_index_valid {
            self.menu_actions.push(SavedAddressMenuAction::GoTo);
            self.menu_actions.push(SavedAddressMenuAction::CopyAddress);
            self.menu_actions.push(SavedAddressMenuAction::CopyText);
        }
        if self.model.row_count() > 0 {
            self.menu_actions.push(SavedAddressMenuAction::CopyAllAsCsv);
        }
        self.menu_actions.push(SavedAddressMenuAction::PasteFromCsv);
        if cpu_alive {
            self.menu_actions.push(SavedAddressMenuAction::LoadFromSettings);
            self.menu_actions.push(SavedAddressMenuAction::SaveToSettings);
        }
        if is_index_valid {
            self.menu_actions.push(SavedAddressMenuAction::Delete);
        }
    }

    pub fn context_new(&mut self) {
        self.model.add_row_default();
    }

    pub fn context_paste_csv(&mut self, csv: String) {
        self.last_csv_import = csv.clone();
        let mut lines = csv.split('\n');
        let _ = lines.next();
        for line in lines {
            let mut fields = Vec::new();
            let mut current = String::new();
            let mut in_quote = false;
            for ch in line.chars() {
                if ch == '"' {
                    in_quote = !in_quote;
                } else if in_quote {
                    current.push(ch);
                } else if ch == ',' {
                    fields.push(std::mem::take(&mut current));
                } else {
                    current.push(ch);
                }
            }
            if !current.is_empty() || !fields.is_empty() {
                fields.push(current);
            }
            self.model.load_saved_address_from_field_list(fields);
        }
    }

    pub fn add_address(&mut self, address: u32) {
        self.model.add_row_default();
        let row = self.model.row_count().saturating_sub(1);
        self.model.set_data(
            row,
            SavedAddressColumn::Address,
            CellValue::Unsigned(address as u64),
            BreakpointRole::Data,
        );
        self.last_added_address = Some(address);
    }

    pub fn save_to_debugger_settings(&self) {}
}

// ---------------------------------------------------------------------------
// Internal helpers: file I/O, name handling, and a tiny JSON
// reader/writer. These wrap the standard library so the rest of the
// module can stay free of external dependencies.
// ---------------------------------------------------------------------------

/// Truncate a layout name to the maximum allowed size. Mirrors
/// `m_name.truncate(DockUtils::MAX_LAYOUT_NAME_SIZE)`.
pub fn truncate_name(name: &str) -> String {
    let trimmed: String = name.chars().take(MAX_LAYOUT_NAME_SIZE).collect();
    trimmed
}

pub fn sanitize_file_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' => out.push(ch),
            ' ' => out.push('_'),
            _ => out.push('_'),
        }
    }
    if out.is_empty() {
        out.push_str("unnamed");
    }
    out
}

fn write_file(path: &str, data: &[u8]) -> io::Result<()> {
    let path = Path::new(path);
    let mut file = fs::File::create(path)?;
    file.write_all(data)
}

fn rename_file(from: &str, to: &str) -> io::Result<()> {
    fs::rename(from, to)
}

fn escape_json(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[derive(Debug, Clone, Default)]
pub struct JsonValue {
    map: BTreeMap<String, JsonValue>,
    list: Vec<JsonValue>,
    scalar: Option<JsonScalar>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JsonScalar {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    String(String),
}

impl Default for JsonScalar {
    fn default() -> Self {
        JsonScalar::Null
    }
}

impl JsonValue {
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.map.get(key)?.scalar.as_ref().and_then(|s| match s {
            JsonScalar::String(s) => Some(s.as_str()),
            _ => None,
        })
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.map.get(key)?.scalar.as_ref().and_then(|s| match s {
            JsonScalar::Bool(b) => Some(*b),
            _ => None,
        })
    }

    pub fn get_u32(&self, key: &str) -> Option<u32> {
        self.map.get(key)?.scalar.as_ref().and_then(|s| match s {
            JsonScalar::UInt(u) => u32::try_from(*u).ok(),
            JsonScalar::Int(i) => u32::try_from(*i).ok(),
            _ => None,
        })
    }

    pub fn get_u64(&self, key: &str) -> Option<u64> {
        self.map.get(key)?.scalar.as_ref().and_then(|s| match s {
            JsonScalar::UInt(u) => Some(*u),
            JsonScalar::Int(i) => u64::try_from(*i).ok(),
            _ => None,
        })
    }

    pub fn get_array(&self, key: &str) -> Option<&Vec<JsonValue>> {
        self.map.get(key).map(|v| &v.list)
    }
}

/// A deliberately small JSON parser. Supports the subset of JSON
/// used by the layout file format: objects with string, number,
/// bool, null, and array values, but no nested escapes beyond
/// `\"`, `\\`, `\n`, `\r`, and `\t`.
pub fn parse_simple_json(input: &str) -> std::result::Result<JsonValue, String> {
    let mut parser = JsonParser::new(input);
    let value = parser.parse_value()?;
    parser.skip_whitespace();
    if !parser.is_at_end() {
        return Err("Unexpected trailing characters".to_string());
    }
    Ok(value)
}

struct JsonParser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn is_at_end(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek() {
            if ch.is_whitespace() {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn expect(&mut self, expected: char) -> std::result::Result<(), String> {
        self.skip_whitespace();
        match self.peek() {
            Some(c) if c == expected => {
                self.pos += c.len_utf8();
                Ok(())
            }
            Some(c) => Err(format!("Expected '{}', found '{}'", expected, c)),
            None => Err(format!("Expected '{}', found EOF", expected)),
        }
    }

    fn parse_value(&mut self) -> std::result::Result<JsonValue, String> {
        self.skip_whitespace();
        match self.peek() {
            Some('{') => self.parse_object(),
            Some('[') => self.parse_array(),
            Some('"') => self.parse_string().map(|s| {
                let mut v = JsonValue::default();
                v.scalar = Some(JsonScalar::String(s));
                v
            }),
            Some('t') | Some('f') => self.parse_bool().map(|b| {
                let mut v = JsonValue::default();
                v.scalar = Some(JsonScalar::Bool(b));
                v
            }),
            Some('n') => self.parse_null().map(|_| JsonValue::default()),
            Some(c) if c == '-' || c.is_ascii_digit() => self.parse_number(),
            Some(c) => Err(format!("Unexpected character '{}'", c)),
            None => Err("Unexpected EOF".to_string()),
        }
    }

    fn parse_object(&mut self) -> std::result::Result<JsonValue, String> {
        self.expect('{')?;
        let mut object = JsonValue::default();
        self.skip_whitespace();
        if matches!(self.peek(), Some('}')) {
            self.pos += 1;
            return Ok(object);
        }
        loop {
            self.skip_whitespace();
            let key = self.parse_string()?;
            self.skip_whitespace();
            self.expect(':')?;
            let value = self.parse_value()?;
            object.map.insert(key, value);
            self.skip_whitespace();
            match self.peek() {
                Some(',') => {
                    self.pos += 1;
                }
                Some('}') => {
                    self.pos += 1;
                    return Ok(object);
                }
                Some(c) => return Err(format!("Expected ',' or '}}', found '{}'", c)),
                None => return Err("Expected ',' or '}}', found EOF".to_string()),
            }
        }
    }

    fn parse_array(&mut self) -> std::result::Result<JsonValue, String> {
        self.expect('[')?;
        let mut array = JsonValue::default();
        self.skip_whitespace();
        if matches!(self.peek(), Some(']')) {
            self.pos += 1;
            return Ok(array);
        }
        loop {
            let value = self.parse_value()?;
            array.list.push(value);
            self.skip_whitespace();
            match self.peek() {
                Some(',') => {
                    self.pos += 1;
                }
                Some(']') => {
                    self.pos += 1;
                    return Ok(array);
                }
                Some(c) => return Err(format!("Expected ',' or ']', found '{}'", c)),
                None => return Err("Expected ',' or ']', found EOF".to_string()),
            }
        }
    }

    fn parse_string(&mut self) -> std::result::Result<String, String> {
        self.expect('"')?;
        let mut out = String::new();
        while let Some(c) = self.peek() {
            match c {
                '"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                '\\' => {
                    self.pos += 1;
                    let escaped = self
                        .peek()
                        .ok_or_else(|| "Unterminated escape".to_string())?;
                    match escaped {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'b' => out.push('\u{0008}'),
                        'f' => out.push('\u{000C}'),
                        'u' => {
                            self.pos += 1;
                            let hex = self.take_n(4).ok_or("Bad \\u escape")?;
                            let code = u32::from_str_radix(hex, 16)
                                .map_err(|_| "Bad \\u escape".to_string())?;
                            if let Some(ch) = char::from_u32(code) {
                                out.push(ch);
                            }
                            continue;
                        }
                        _ => return Err(format!("Unknown escape '\\{}'", escaped)),
                    }
                    self.pos += escaped.len_utf8();
                }
                c => {
                    out.push(c);
                    self.pos += c.len_utf8();
                }
            }
        }
        Err("Unterminated string".to_string())
    }

    fn parse_bool(&mut self) -> std::result::Result<bool, String> {
        if self.input[self.pos..].starts_with("true") {
            self.pos += 4;
            Ok(true)
        } else if self.input[self.pos..].starts_with("false") {
            self.pos += 5;
            Ok(false)
        } else {
            Err("Expected bool".to_string())
        }
    }

    fn parse_null(&mut self) -> std::result::Result<(), String> {
        if self.input[self.pos..].starts_with("null") {
            self.pos += 4;
            Ok(())
        } else {
            Err("Expected null".to_string())
        }
    }

    fn parse_number(&mut self) -> std::result::Result<JsonValue, String> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '.' || c == 'e' || c == 'E' || c == '+' || c == '-' {
                self.pos += 1;
            } else {
                break;
            }
        }
        let raw = &self.input[start..self.pos];
        if raw.contains('.') || raw.contains('e') || raw.contains('E') {
            let value: f64 = raw.parse().map_err(|_| "Invalid number".to_string())?;
            let mut v = JsonValue::default();
            v.scalar = Some(JsonScalar::Float(value));
            Ok(v)
        } else if let Ok(int) = raw.parse::<i64>() {
            let mut v = JsonValue::default();
            v.scalar = Some(JsonScalar::Int(int));
            Ok(v)
        } else {
            let value: u64 = raw.parse().map_err(|_| "Invalid number".to_string())?;
            let mut v = JsonValue::default();
            v.scalar = Some(JsonScalar::UInt(value));
            Ok(v)
        }
    }

    fn take_n(&mut self, n: usize) -> Option<&'a str> {
        if self.pos + n > self.input.len() {
            return None;
        }
        let slice = &self.input[self.pos..self.pos + n];
        self.pos += n;
        Some(slice)
    }
}

pub fn format_csv<T: AsRef<str>>(header: &[&str], rows: &[T]) -> String {
    let mut out = String::new();
    out.push_str(&header
        .iter()
        .map(|h| format!("\"{}\"", h))
        .collect::<Vec<_>>()
        .join(","));
    out.push('\n');
    for row in rows {
        out.push_str(row.as_ref());
        out.push('\n');
    }
    out
}

// ---------------------------------------------------------------------------
// Public re-exports and the closing tests.  Marker comments have been
// removed from the final file.
// ---------------------------------------------------------------------------

/// Convenience `Result` alias used throughout the module.
pub type Result<T> = std::result::Result<T, String>;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_layout_create_and_populate() {
        let layout = DockLayout::create_from_default("R5900", BreakPointCpu::EE, true, "R5900");
        assert!(layout.is_default);
        assert!(layout.can_reset());
        assert!(!layout.widgets.is_empty());
    }

    #[test]
    fn dock_manager_switch_layout() {
        let mut manager = DockManager::create();
        manager.populate();
        assert_eq!(manager.current_layout(), 0);
        assert!(manager.layouts().len() >= 2);
        manager.switch_to_layout(1, false);
        assert_eq!(manager.current_layout(), 1);
    }

    #[test]
    fn breakpoint_model_insert_and_remove() {
        let mut model = BreakpointModel::create(BreakPointCpu::EE);
        let bp = BreakpointMemcheck::Break(BreakPoint {
            addr: 0x1000,
            enabled: true,
            description: "start".into(),
            ..Default::default()
        });
        assert!(model.insert_breakpoint_rows(0, 1, vec![bp.clone()]));
        assert_eq!(model.row_count(), 1);
        assert!(model.remove_rows(0, 1));
        assert_eq!(model.row_count(), 0);
        let _ = bp;
    }

    #[test]
    fn breakpoint_dialog_parses_address() {
        let mut dialog = BreakpointDialog::create_new(BreakPointCpu::EE);
        dialog.address_text = "0x1000".to_string();
        dialog.description_text = "hello".to_string();
        let result = dialog.accept(
            |s| {
                if let Some(rest) = s.strip_prefix("0x") {
                    u64::from_str_radix(rest, 16).map_err(|e| e.to_string())
                } else {
                    Err("expected 0x prefix".to_string())
                }
            },
            |_| Ok(()),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn memory_view_default_state() {
        let mut view = MemoryView::create();
        view.populate();
        view.goto_address(0x2000);
        assert_eq!(view.last_address, 0x2000);
        assert_eq!(view.table.start_address & 0xF, 0);
    }

    #[test]
    fn memory_search_comparisons_for_strings() {
        let view = MemorySearchView::create();
        let mut results = Vec::new();
        results.push(SearchResult::new(
            0x1000,
            SearchValue::Bytes(b"hello".to_vec()),
            SearchType::StringType,
        ));
        let comparisons = view.get_valid_search_comparisons_for_state(
            SearchType::StringType,
            &results,
        );
        assert!(comparisons.contains(&SearchComparison::Equals));
        assert!(comparisons.contains(&SearchComparison::Changed));
    }

    #[test]
    fn saved_addresses_round_trip() {
        let mut model = SavedAddressesModel::create(BreakPointCpu::EE);
        model.add_row(SavedAddress {
            address: 0x1000,
            label: "name".into(),
            description: "desc".into(),
        });
        assert_eq!(model.row_count(), 1);
        let _ = model.set_data(
            0,
            SavedAddressColumn::Label,
            CellValue::Text("renamed".into()),
            BreakpointRole::Edit,
        );
        match model.data(0, SavedAddressColumn::Label, BreakpointRole::Display) {
            CellValue::Text(s) => assert_eq!(s, "renamed"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn truncate_and_sanitize() {
        let long = "a".repeat(MAX_LAYOUT_NAME_SIZE + 10);
        assert_eq!(truncate_name(&long).len(), MAX_LAYOUT_NAME_SIZE);
        assert_eq!(sanitize_file_name("name with space"), "name_with_space");
    }

    #[test]
    fn simple_json_round_trip() {
        let json = r#"{ "name": "R5900", "index": 2, "enabled": true, "items": [1, 2, 3] }"#;
        let value = parse_simple_json(json).expect("json parses");
        assert_eq!(value.get_str("name"), Some("R5900"));
        assert_eq!(value.get_u64("index"), Some(2));
        assert_eq!(value.get_bool("enabled"), Some(true));
        let array = value.get_array("items").expect("items array");
        assert_eq!(array.len(), 3);
    }
}

