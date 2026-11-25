//! FAR Manager 3.x (x64) Plugin API FFI definitions.
//!
//! Layout verified directly against official `FarGroup/FarManager/far/plugin.hpp`.

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use std::ffi::{c_void, OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use windows_sys::Win32::Foundation::{FILETIME, HANDLE, RECT};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GUID {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

pub const ZERO_GUID: GUID = GUID {
    data1: 0,
    data2: 0,
    data3: 0,
    data4: [0; 8],
};

// Plugin GUIDs (unique for FarQsirch)
pub const PLUGIN_GUID: GUID = GUID {
    data1: 0x6F4A9C12,
    data2: 0x8E31,
    data3: 0x4D7B,
    data4: [0x9A, 0x24, 0x5C, 0x1E, 0x7B, 0x3D, 0x8F, 0x01],
};

pub const MENU_GUID: GUID = GUID {
    data1: 0x6F4A9C12,
    data2: 0x8E31,
    data3: 0x4D7B,
    data4: [0x9A, 0x24, 0x5C, 0x1E, 0x7B, 0x3D, 0x8F, 0x02],
};

pub const CONFIG_MENU_GUID: GUID = GUID {
    data1: 0x6F4A9C12,
    data2: 0x8E31,
    data3: 0x4D7B,
    data4: [0x9A, 0x24, 0x5C, 0x1E, 0x7B, 0x3D, 0x8F, 0x03],
};

pub const SEARCH_DLG_GUID: GUID = GUID {
    data1: 0x6F4A9C12,
    data2: 0x8E31,
    data3: 0x4D7B,
    data4: [0x9A, 0x24, 0x5C, 0x1E, 0x7B, 0x3D, 0x8F, 0x04],
};

pub const RESULTS_DLG_GUID: GUID = GUID {
    data1: 0x6F4A9C12,
    data2: 0x8E31,
    data3: 0x4D7B,
    data4: [0x9A, 0x24, 0x5C, 0x1E, 0x7B, 0x3D, 0x8F, 0x05],
};

pub const SETTINGS_DLG_GUID: GUID = GUID {
    data1: 0x6F4A9C12,
    data2: 0x8E31,
    data3: 0x4D7B,
    data4: [0x9A, 0x24, 0x5C, 0x1E, 0x7B, 0x3D, 0x8F, 0x06],
};

pub const MSG_GUID: GUID = GUID {
    data1: 0x6F4A9C12,
    data2: 0x8E31,
    data3: 0x4D7B,
    data4: [0x9A, 0x24, 0x5C, 0x1E, 0x7B, 0x3D, 0x8F, 0x07],
};

// ─── Constants ────────────────────────────────────────────────────────────────

pub const PANEL_ACTIVE: HANDLE = -1isize as HANDLE;
pub const PANEL_PASSIVE: HANDLE = -2isize as HANDLE;
pub const PANEL_STOP: HANDLE = -1isize as HANDLE;

pub const CP_DEFAULT: usize = usize::MAX;

// Version stages
pub const VS_RELEASE: i32 = 0;

// Far message flags
pub const FMSG_NONE: u64 = 0;
pub const FMSG_WARNING: u64 = 0x0000_0000_0000_0001;
pub const FMSG_LEFTALIGN: u64 = 0x0000_0000_0000_0008;
pub const FMSG_MB_OK: u64 = 0x0000_0000_0001_0000;

// Dialog item types (enum FARDIALOGITEMTYPES)
pub const DI_TEXT: i32 = 0;
pub const DI_VTEXT: i32 = 1;
pub const DI_SINGLEBOX: i32 = 2;
pub const DI_DOUBLEBOX: i32 = 3;
pub const DI_EDIT: i32 = 4;
pub const DI_PSWEDIT: i32 = 5;
pub const DI_FIXEDIT: i32 = 6;
pub const DI_BUTTON: i32 = 7;
pub const DI_CHECKBOX: i32 = 8;
pub const DI_RADIOBUTTON: i32 = 9;
pub const DI_COMBOBOX: i32 = 10;
pub const DI_LISTBOX: i32 = 11;

// Dialog item flags (FARDIALOGITEMFLAGS)
pub const DIF_NONE: u64 = 0;
pub const DIF_BOXCOLOR: u64 = 0x0000_0000_0000_0200;
pub const DIF_GROUP: u64 = 0x0000_0000_0000_0400;
pub const DIF_CENTERGROUP: u64 = 0x0000_0000_0000_4000;
pub const DIF_SEPARATOR: u64 = 0x0000_0000_0001_0000;
pub const DIF_LISTNOAMPERSAND: u64 = 0x0000_0000_0002_0000;
pub const DIF_LISTNOBOX: u64 = 0x0000_0000_0004_0000;
pub const DIF_HISTORY: u64 = 0x0000_0000_0004_0000;
pub const DIF_BTNNOCLOSE: u64 = 0x0000_0000_0004_0000;
pub const DIF_DROPDOWNLIST: u64 = 0x0000_0000_0010_0000;
pub const DIF_USELASTHISTORY: u64 = 0x0000_0000_0020_0000;
pub const DIF_SELECTONENTRY: u64 = 0x0000_0000_0080_0000;
pub const DIF_LISTWRAPMODE: u64 = 0x0000_0000_0100_0000;
pub const DIF_LISTNOCLOSE: u64 = 0x0000_0000_0400_0000;
pub const DIF_READONLY: u64 = 0x0000_0000_2000_0000;
pub const DIF_DISABLE: u64 = 0x0000_0000_8000_0000;
pub const DIF_DEFAULTBUTTON: u64 = 0x0000_0001_0000_0000;
pub const DIF_FOCUS: u64 = 0x0000_0002_0000_0000;

// Dialog messages (enum FARMESSAGE)
pub const DM_CLOSE: isize = 1;
pub const DM_ENABLE: isize = 2;
pub const DM_GETTEXT: isize = 7;
pub const DM_SETTEXTPTR: isize = 22;
pub const DM_GETCHECK: isize = 25;
pub const DM_LISTGETCURPOS: isize = 30;
pub const DM_LISTSETCURPOS: isize = 31;
pub const DM_LISTADDSTR: isize = 34;
pub const DM_GETCONSTTEXTPTR: isize = 63;

pub const DN_BTNCLICK: isize = 4097;
pub const DN_INITDIALOG: isize = 4108;
pub const DN_LISTCHANGE: isize = 4110;
pub const DN_CONTROLINPUT: isize = 4116;

// Checked states
pub const BSTATE_UNCHECKED: isize = 0;
pub const BSTATE_CHECKED: isize = 1;

// Dialog flags
pub const FDLG_NONE: u64 = 0;
pub const FDLG_SMALLDIALOG: u64 = 0x0000_0000_0000_0002;

// Advanced control commands
pub const ACTL_SYNCHRO: i32 = 20;
pub const ACTL_GETFARRECT: i32 = 24;

#[repr(C)]
pub struct ProcessSynchroEventInfo {
    pub StructSize: usize,
    pub Event: isize,
    pub Param: *mut c_void,
    pub Instance: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SMALL_RECT {
    pub Left: i16,
    pub Top: i16,
    pub Right: i16,
    pub Bottom: i16,
}

// List item flags
pub const LIF_NONE: u64 = 0;
pub const LIF_SELECTED: u64 = 0x0000_0000_0001_0000;

// OpenPanelInfo flags
pub const OPIF_NONE: u64 = 0;
pub const OPIF_ADDDOTS: u64 = 0x0000_0000_0000_0008;
pub const OPIF_REALNAMES: u64 = 0x0000_0000_0000_0020;

// Panel control commands
pub const FCTL_GETPANELINFO: i32 = 1;
pub const FCTL_UPDATEPANEL: i32 = 2;
pub const FCTL_REDRAWPANEL: i32 = 3;
pub const FCTL_SETPANELDIRECTORY: i32 = 10;
pub const FCTL_GETPANELITEM: i32 = 21;
pub const FCTL_GETPANELDIRECTORY: i32 = 24;

// Viewer / Editor flags
pub const VF_NONE: u64 = 0;
pub const VF_ENABLE_F6: u64 = 0x0000_0000_0000_0004;
pub const EF_NONE: u64 = 0;
pub const EF_ENABLE_F6: u64 = 0x0000_0000_0000_0004;

// OpenFrom values
pub const OPEN_PLUGINSMENU: i32 = 1;
pub const OPEN_COMMANDLINE: i32 = 4;
pub const OPEN_FROMMACRO: i32 = 11;

// Macro value types
pub const FMVT_PANEL: i32 = 9;

#[repr(C)]
#[derive(Clone, Copy)]
pub union FarMacroValueUnion {
    pub Integer: i64,
    pub Pointer: *mut c_void,
    pub Binary: (*mut c_void, usize),
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FarMacroValue {
    pub Type: i32,
    pub Value: FarMacroValueUnion,
}

pub type FARMACROCALLCALLBACK = unsafe extern "system" fn(
    CallbackData: *mut c_void,
    Values: *mut FarMacroValue,
    Count: usize,
);

#[repr(C)]
pub struct FarMacroCall {
    pub StructSize: usize,
    pub Count: usize,
    pub Values: *mut FarMacroValue,
    pub Callback: Option<FARMACROCALLCALLBACK>,
    pub CallbackData: *mut c_void,
}

/// Boxed `FarMacroCall` + single `FMVT_PANEL` value for safely returning a panel
/// handle when `OpenW` was invoked via `Plugin.Call(...)` (`OPEN_FROMMACRO`).
struct MacroPanelReturn {
    call: FarMacroCall,
    value: FarMacroValue,
}

unsafe extern "system" fn free_macro_panel_return(
    callback_data: *mut c_void,
    _values: *mut FarMacroValue,
    _count: usize,
) {
    if !callback_data.is_null() {
        let _ = Box::from_raw(callback_data as *mut MacroPanelReturn);
    }
}

pub fn wrap_panel_for_macro(panel_handle: HANDLE) -> HANDLE {
    let boxed = Box::new(MacroPanelReturn {
        call: FarMacroCall {
            StructSize: std::mem::size_of::<FarMacroCall>(),
            Count: 1,
            Values: std::ptr::null_mut(),
            Callback: Some(free_macro_panel_return),
            CallbackData: std::ptr::null_mut(),
        },
        value: FarMacroValue {
            Type: FMVT_PANEL,
            Value: FarMacroValueUnion {
                Pointer: panel_handle as *mut c_void,
            },
        },
    });
    let raw = Box::into_raw(boxed);
    unsafe {
        (*raw).call.Values = &mut (*raw).value as *mut FarMacroValue;
        (*raw).call.CallbackData = raw.cast::<c_void>();
        (&mut (*raw).call as *mut FarMacroCall) as HANDLE
    }
}

// Sort modes
pub const SM_DEFAULT: i32 = 0;
pub const SM_UNSORTED: i32 = 1;

// Plugin flags
pub const PF_NONE: u64 = 0;

// ─── Structs ──────────────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VersionInfo {
    pub Major: u32,
    pub Minor: u32,
    pub Revision: u32,
    pub Build: u32,
    pub Stage: i32,
}

#[repr(C)]
pub struct GlobalInfo {
    pub StructSize: usize,
    pub MinFarVersion: VersionInfo,
    pub Version: VersionInfo,
    pub Guid: GUID,
    pub Title: *const u16,
    pub Description: *const u16,
    pub Author: *const u16,
    pub Instance: *mut c_void,
}

#[repr(C)]
pub struct PluginMenuItem {
    pub Guids: *const GUID,
    pub Strings: *const *const u16,
    pub Count: usize,
}

#[repr(C)]
pub struct PluginInfo {
    pub StructSize: usize,
    pub Flags: u64,
    pub DiskMenu: PluginMenuItem,
    pub PluginMenu: PluginMenuItem,
    pub PluginConfig: PluginMenuItem,
    pub CommandPrefix: *const u16,
    pub Instance: *mut c_void,
}

#[repr(C)]
pub struct OpenInfo {
    pub StructSize: usize,
    pub OpenFrom: i32,
    pub Guid: *const GUID,
    pub Data: isize,
    pub Instance: *mut c_void,
}

#[repr(C)]
pub struct OpenCommandLineInfo {
    pub StructSize: usize,
    pub CommandLine: *const u16,
}

#[repr(C)]
pub struct ConfigureInfo {
    pub StructSize: usize,
    pub Guid: *const GUID,
    pub Instance: *mut c_void,
}

#[repr(C)]
pub struct ExitInfo {
    pub StructSize: usize,
    pub Instance: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FarListItem {
    pub Flags: u64,
    pub Text: *const u16,
    pub UserData: isize,
    pub Reserved: isize,
}

#[repr(C)]
pub struct FarList {
    pub StructSize: usize,
    pub ItemsNumber: usize,
    pub Items: *mut FarListItem,
}

#[repr(C)]
pub struct FarListPos {
    pub StructSize: usize,
    pub SelectPos: isize,
    pub TopPos: isize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union FarDialogItemParam {
    pub Selected: isize,
    pub ListItems: *mut FarList,
    pub VBuf: *mut c_void,
    pub Reserved0: isize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FarDialogItem {
    pub Type: i32,
    pub X1: isize,
    pub Y1: isize,
    pub X2: isize,
    pub Y2: isize,
    pub Param: FarDialogItemParam,
    pub History: *const u16,
    pub Mask: *const u16,
    pub Flags: u64,
    pub Data: *const u16,
    pub MaxLength: usize,
    pub UserData: isize,
    pub Reserved: [isize; 2],
}

impl FarDialogItem {
    pub fn empty(item_type: i32, x1: isize, y1: isize, x2: isize, y2: isize) -> Self {
        Self {
            Type: item_type,
            X1: x1,
            Y1: y1,
            X2: x2,
            Y2: y2,
            Param: FarDialogItemParam { Selected: 0 },
            History: std::ptr::null(),
            Mask: std::ptr::null(),
            Flags: DIF_NONE,
            Data: std::ptr::null(),
            MaxLength: 0,
            UserData: 0,
            Reserved: [0; 2],
        }
    }
}

#[repr(C)]
pub struct FarDialogItemData {
    pub StructSize: usize,
    pub PtrLength: usize,
    pub PtrData: *mut u16,
}

#[repr(C)]
pub struct FarPanelDirectory {
    pub StructSize: usize,
    pub Name: *const u16,
    pub Param: *const u16,
    pub PluginId: GUID,
    pub File: *const u16,
}

#[repr(C)]
pub struct PanelInfo {
    pub StructSize: usize,
    pub PluginHandle: HANDLE,
    pub OwnerGuid: GUID,
    pub Flags: u64,
    pub ItemsNumber: usize,
    pub SelectedItemsNumber: usize,
    pub PanelRect: RECT,
    pub CurrentItem: usize,
    pub TopPanelItem: usize,
    pub ViewMode: isize,
    pub PanelType: i32,
    pub SortMode: i32,
}

#[repr(C)]
pub struct PanelRedrawInfo {
    pub StructSize: usize,
    pub CurrentItem: usize,
    pub TopPanelItem: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct UserDataItem {
    pub Data: *mut c_void,
    pub FreeData: Option<unsafe extern "system" fn(*mut c_void, *const c_void)>,
}

impl Default for UserDataItem {
    fn default() -> Self {
        Self {
            Data: std::ptr::null_mut(),
            FreeData: None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PluginPanelItem {
    pub CreationTime: FILETIME,
    pub LastAccessTime: FILETIME,
    pub LastWriteTime: FILETIME,
    pub ChangeTime: FILETIME,
    pub FileSize: u64,
    pub AllocationSize: u64,
    pub FileName: *const u16,
    pub AlternateFileName: *const u16,
    pub Description: *const u16,
    pub Owner: *const u16,
    pub CustomColumnData: *const *const u16,
    pub CustomColumnNumber: usize,
    pub Flags: u64,
    pub UserData: UserDataItem,
    pub FileAttributes: usize,
    pub NumberOfLinks: usize,
    pub CRC32: usize,
    pub Reserved: [isize; 2],
}

#[repr(C)]
pub struct FarGetPluginPanelItem {
    pub StructSize: usize,
    pub Size: usize,
    pub Item: *mut PluginPanelItem,
}

#[repr(C)]
pub struct OpenPanelInfo {
    pub StructSize: usize,
    pub hPanel: HANDLE,
    pub Flags: u64,
    pub HostFile: *const u16,
    pub CurDir: *const u16,
    pub Format: *const u16,
    pub PanelTitle: *const u16,
    pub InfoLines: *const c_void,
    pub InfoLinesNumber: usize,
    pub DescrFiles: *const *const u16,
    pub DescrFilesNumber: usize,
    pub PanelModesArray: *const c_void,
    pub PanelModesNumber: usize,
    pub StartPanelMode: isize,
    pub StartSortMode: i32,
    pub StartSortOrder: isize,
    pub KeyBar: *const c_void,
    pub ShortcutData: *const u16,
    pub FreeSize: u64,
    pub UserData: UserDataItem,
    pub Instance: *mut c_void,
}

#[repr(C)]
pub struct GetFindDataInfo {
    pub StructSize: usize,
    pub hPanel: HANDLE,
    pub PanelItem: *mut PluginPanelItem,
    pub ItemsNumber: usize,
    pub OpMode: u64,
    pub Instance: *mut c_void,
}

#[repr(C)]
pub struct FreeFindDataInfo {
    pub StructSize: usize,
    pub hPanel: HANDLE,
    pub PanelItem: *mut PluginPanelItem,
    pub ItemsNumber: usize,
    pub Instance: *mut c_void,
}

#[repr(C)]
pub struct ClosePanelInfo {
    pub StructSize: usize,
    pub hPanel: HANDLE,
    pub Instance: *mut c_void,
}

// ─── Callback Function Pointer Types ──────────────────────────────────────────

pub type FARWINDOWPROC = unsafe extern "system" fn(
    hDlg: HANDLE,
    Msg: isize,
    Param1: isize,
    Param2: *mut c_void,
) -> isize;

pub type FARAPIMESSAGE = unsafe extern "system" fn(
    PluginId: *const GUID,
    Id: *const GUID,
    Flags: u64,
    HelpTopic: *const u16,
    Items: *const *const u16,
    ItemsNumber: usize,
    ButtonsNumber: isize,
) -> isize;

pub type FARAPIPANELCONTROL = unsafe extern "system" fn(
    hPanel: HANDLE,
    Command: i32,
    Param1: isize,
    Param2: *mut c_void,
) -> isize;

pub type FARAPIVIEWER = unsafe extern "system" fn(
    FileName: *const u16,
    Title: *const u16,
    X1: isize,
    Y1: isize,
    X2: isize,
    Y2: isize,
    Flags: u64,
    CodePage: usize,
) -> isize;

pub type FARAPIEDITOR = unsafe extern "system" fn(
    FileName: *const u16,
    Title: *const u16,
    X1: isize,
    Y1: isize,
    X2: isize,
    Y2: isize,
    Flags: u64,
    StartLine: isize,
    StartChar: isize,
    CodePage: usize,
) -> isize;

pub type FARAPIDIALOGINIT = unsafe extern "system" fn(
    PluginId: *const GUID,
    Id: *const GUID,
    X1: isize,
    Y1: isize,
    X2: isize,
    Y2: isize,
    HelpTopic: *const u16,
    Item: *const FarDialogItem,
    ItemsNumber: usize,
    Reserved: isize,
    Flags: u64,
    DlgProc: Option<FARWINDOWPROC>,
    Param: *mut c_void,
) -> HANDLE;

pub type FARAPIDIALOGRUN = unsafe extern "system" fn(hDlg: HANDLE) -> isize;
pub type FARAPIDIALOGFREE = unsafe extern "system" fn(hDlg: HANDLE);

pub type FARAPISENDDLGMESSAGE = unsafe extern "system" fn(
    hDlg: HANDLE,
    Msg: isize,
    Param1: isize,
    Param2: *mut c_void,
) -> isize;

pub type FARAPIDEFDLGPROC = unsafe extern "system" fn(
    hDlg: HANDLE,
    Msg: isize,
    Param1: isize,
    Param2: *mut c_void,
) -> isize;

pub type FARAPIADVCONTROL = unsafe extern "system" fn(
    PluginId: *const GUID,
    Command: i32,
    Param1: isize,
    Param2: *mut c_void,
) -> isize;

pub type FARAPISAVESCREEN =
    unsafe extern "system" fn(X1: isize, Y1: isize, X2: isize, Y2: isize) -> HANDLE;
pub type FARAPIRESTORESCREEN = unsafe extern "system" fn(hScreen: HANDLE);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PluginStartupInfo {
    pub StructSize: usize,
    pub ModuleName: *const u16,
    pub Menu: *const c_void,
    pub Message: Option<FARAPIMESSAGE>,
    pub GetMsg: *const c_void,
    pub PanelControl: Option<FARAPIPANELCONTROL>,
    pub SaveScreen: Option<FARAPISAVESCREEN>,
    pub RestoreScreen: Option<FARAPIRESTORESCREEN>,
    pub GetDirList: *const c_void,
    pub GetPluginDirList: *const c_void,
    pub FreeDirList: *const c_void,
    pub FreePluginDirList: *const c_void,
    pub Viewer: Option<FARAPIVIEWER>,
    pub Editor: Option<FARAPIEDITOR>,
    pub Text: *const c_void,
    pub EditorControl: *const c_void,
    pub FSF: *const c_void,
    pub ShowHelp: *const c_void,
    pub AdvControl: Option<FARAPIADVCONTROL>,
    pub InputBox: *const c_void,
    pub ColorDialog: *const c_void,
    pub DialogInit: Option<FARAPIDIALOGINIT>,
    pub DialogRun: Option<FARAPIDIALOGRUN>,
    pub DialogFree: Option<FARAPIDIALOGFREE>,
    pub SendDlgMessage: Option<FARAPISENDDLGMESSAGE>,
    pub DefDlgProc: Option<FARAPIDEFDLGPROC>,
    pub ViewerControl: *const c_void,
    pub PluginsControl: *const c_void,
    pub FileFilterControl: *const c_void,
    pub RegExpControl: *const c_void,
    pub MacroControl: *const c_void,
    pub SettingsControl: *const c_void,
    pub Private: *const c_void,
    pub Instance: *mut c_void,
    pub FreeScreen: *const c_void,
}

// ─── UTF-16 Helpers ───────────────────────────────────────────────────────────

/// Null-terminated UTF-16 buffer for passing strings to Far Manager.
#[derive(Clone)]
pub struct WideString(pub Vec<u16>);

impl WideString {
    pub fn new(s: &str) -> Self {
        let mut buf: Vec<u16> = OsStr::new(s).encode_wide().collect();
        buf.push(0);
        Self(buf)
    }

    pub fn as_ptr(&self) -> *const u16 {
        self.0.as_ptr()
    }
}

pub unsafe fn from_wide_ptr(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    let slice = std::slice::from_raw_parts(ptr, len);
    OsString::from_wide(slice).to_string_lossy().into_owned()
}

/// Convert Unix epoch seconds to Windows FILETIME (100-ns intervals since 1601-01-01).
pub fn unix_secs_to_filetime(secs: i64) -> FILETIME {
    if secs <= 0 {
        return FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
    }
    const EPOCH_DIFF_SECS: u64 = 11_644_473_600;
    let ticks = (secs as u64).saturating_add(EPOCH_DIFF_SECS).saturating_mul(10_000_000);
    FILETIME {
        dwLowDateTime: (ticks & 0xFFFF_FFFF) as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    }
}

/// Helper to read a dialog edit/combo item's text via `DM_GETCONSTTEXTPTR`.
pub unsafe fn get_dlg_text(psi: &PluginStartupInfo, h_dlg: HANDLE, item_id: isize) -> String {
    let Some(send) = psi.SendDlgMessage else {
        return String::new();
    };
    let ptr = send(h_dlg, DM_GETCONSTTEXTPTR, item_id, std::ptr::null_mut()) as *const u16;
    from_wide_ptr(ptr)
}

/// Helper to read a checkbox/radiobutton checked state via `DM_GETCHECK`.
pub unsafe fn get_dlg_check(psi: &PluginStartupInfo, h_dlg: HANDLE, item_id: isize) -> bool {
    let Some(send) = psi.SendDlgMessage else {
        return false;
    };
    send(h_dlg, DM_GETCHECK, item_id, std::ptr::null_mut()) == BSTATE_CHECKED
}

/// Helper to show a Far Manager message box.
pub unsafe fn show_message(psi: &PluginStartupInfo, title: &str, lines: &[&str], warning: bool) {
    let Some(msg_fn) = psi.Message else {
        return;
    };
    let mut wide_lines: Vec<WideString> = Vec::with_capacity(lines.len() + 1);
    wide_lines.push(WideString::new(title));
    for line in lines {
        wide_lines.push(WideString::new(line));
    }
    let ptrs: Vec<*const u16> = wide_lines.iter().map(|w| w.as_ptr()).collect();
    let flags = FMSG_MB_OK | FMSG_LEFTALIGN | if warning { FMSG_WARNING } else { FMSG_NONE };
    msg_fn(
        &PLUGIN_GUID,
        &MSG_GUID,
        flags,
        std::ptr::null(),
        ptrs.as_ptr(),
        ptrs.len(),
        0,
    );
}

/// Draw a non-blocking FAR progress box (does not wait for a key press).
pub unsafe fn show_progress_box(psi: &PluginStartupInfo, title: &str, lines: &[String]) {
    let Some(msg_fn) = psi.Message else {
        return;
    };
    let mut wide_lines: Vec<WideString> = Vec::with_capacity(lines.len() + 1);
    wide_lines.push(WideString::new(title));
    for line in lines {
        wide_lines.push(WideString::new(line));
    }
    let ptrs: Vec<*const u16> = wide_lines.iter().map(|w| w.as_ptr()).collect();
    msg_fn(
        &PLUGIN_GUID,
        &MSG_GUID,
        FMSG_LEFTALIGN,
        std::ptr::null(),
        ptrs.as_ptr(),
        ptrs.len(),
        0,
    );
}

/// Non-blocking check for ESC key press in FAR's console input buffer.
/// Drains queued mouse/focus events so ESC is never buried behind them,
/// and also checks `GetAsyncKeyState(VK_ESCAPE)` when FAR is the foreground window.
pub unsafe fn poll_escape_pressed() -> bool {
    use windows_sys::Win32::System::Console::{
        GetConsoleWindow, GetStdHandle, PeekConsoleInputW, ReadConsoleInputW, INPUT_RECORD,
        KEY_EVENT, STD_INPUT_HANDLE,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    const VK_ESCAPE: i32 = 0x1B;

    let mut esc = false;

    let h_in = GetStdHandle(STD_INPUT_HANDLE);
    if !h_in.is_null() && h_in != (-1isize as HANDLE) {
        let mut rec: [INPUT_RECORD; 64] = std::mem::zeroed();
        loop {
            let mut avail: u32 = 0;
            if PeekConsoleInputW(h_in, rec.as_mut_ptr(), rec.len() as u32, &mut avail) == 0
                || avail == 0
            {
                break;
            }
            let mut read_count: u32 = 0;
            if ReadConsoleInputW(h_in, rec.as_mut_ptr(), avail, &mut read_count) == 0
                || read_count == 0
            {
                break;
            }
            for r in &rec[..read_count as usize] {
                if r.EventType as u32 == KEY_EVENT {
                    let ke = r.Event.KeyEvent;
                    if ke.bKeyDown != 0 && ke.wVirtualKeyCode == VK_ESCAPE as u16 {
                        esc = true;
                    }
                }
            }
            if (read_count as usize) < rec.len() {
                break;
            }
        }
    }

    if !esc {
        let con_hwnd = GetConsoleWindow();
        let fg_hwnd = GetForegroundWindow();
        if (con_hwnd.is_null() || con_hwnd == fg_hwnd) && (GetAsyncKeyState(VK_ESCAPE) as u16 & 0x8000) != 0 {
            esc = true;
        }
    }

    esc
}

/// Query FAR Manager's console dimensions `(width, height)` via `ACTL_GETFARRECT`.
pub unsafe fn get_far_console_size(psi: &PluginStartupInfo) -> (isize, isize) {
    if let Some(adv) = psi.AdvControl {
        let mut rect = SMALL_RECT::default();
        let ok = adv(
            &PLUGIN_GUID,
            ACTL_GETFARRECT,
            0,
            (&mut rect as *mut SMALL_RECT).cast::<c_void>(),
        );
        if ok != 0 {
            let w = (rect.Right - rect.Left + 1) as isize;
            let h = (rect.Bottom - rect.Top + 1) as isize;
            if w >= 40 && h >= 12 {
                return (w, h);
            }
        }
    }
    (100, 30)
}

/// Get the current directory of the active Far panel.
pub unsafe fn get_active_panel_dir(psi: &PluginStartupInfo) -> String {
    let Some(panel_ctrl) = psi.PanelControl else {
        return String::new();
    };
    let needed = panel_ctrl(PANEL_ACTIVE, FCTL_GETPANELDIRECTORY, 0, std::ptr::null_mut());
    if needed <= 0 {
        return String::new();
    }
    let mut buf = vec![0u8; needed as usize];
    let dir_info = buf.as_mut_ptr() as *mut FarPanelDirectory;
    (*dir_info).StructSize = std::mem::size_of::<FarPanelDirectory>();
    let res = panel_ctrl(
        PANEL_ACTIVE,
        FCTL_GETPANELDIRECTORY,
        needed,
        dir_info as *mut c_void,
    );
    if res == 0 {
        return String::new();
    }
    from_wide_ptr((*dir_info).Name)
}

/// Navigate the active Far panel to `dir` and position the cursor on `file_name`
/// (equivalent to FAR's `goto:"<dir>\<file_name>"` command).
pub unsafe fn goto_panel_file(psi: &PluginStartupInfo, dir: &str, file_name: &str) {
    let Some(panel_ctrl) = psi.PanelControl else {
        return;
    };
    let w_dir = WideString::new(dir);
    let mut fpd = FarPanelDirectory {
        StructSize: std::mem::size_of::<FarPanelDirectory>(),
        Name: w_dir.as_ptr(),
        Param: std::ptr::null(),
        PluginId: ZERO_GUID,
        File: std::ptr::null(),
    };
    panel_ctrl(
        PANEL_ACTIVE,
        FCTL_SETPANELDIRECTORY,
        0,
        (&mut fpd as *mut FarPanelDirectory).cast::<c_void>(),
    );
    panel_ctrl(PANEL_ACTIVE, FCTL_UPDATEPANEL, 1, std::ptr::null_mut());

    if !file_name.is_empty() {
        let mut pinfo: PanelInfo = std::mem::zeroed();
        pinfo.StructSize = std::mem::size_of::<PanelInfo>();
        if panel_ctrl(
            PANEL_ACTIVE,
            FCTL_GETPANELINFO,
            0,
            (&mut pinfo as *mut PanelInfo).cast::<c_void>(),
        ) != 0
        {
            let target_lower = file_name.to_lowercase();
            let mut found_idx: Option<usize> = None;
            let mut buf: Vec<u8> = Vec::new();

            for i in 0..pinfo.ItemsNumber {
                let mut gpi = FarGetPluginPanelItem {
                    StructSize: std::mem::size_of::<FarGetPluginPanelItem>(),
                    Size: 0,
                    Item: std::ptr::null_mut(),
                };
                let needed = panel_ctrl(
                    PANEL_ACTIVE,
                    FCTL_GETPANELITEM,
                    i as isize,
                    (&mut gpi as *mut FarGetPluginPanelItem).cast::<c_void>(),
                );
                if needed <= 0 {
                    continue;
                }
                let needed_usize = needed as usize;
                if buf.len() < needed_usize {
                    buf.resize(needed_usize, 0);
                }
                gpi.Size = needed_usize;
                gpi.Item = buf.as_mut_ptr() as *mut PluginPanelItem;
                if panel_ctrl(
                    PANEL_ACTIVE,
                    FCTL_GETPANELITEM,
                    i as isize,
                    (&mut gpi as *mut FarGetPluginPanelItem).cast::<c_void>(),
                ) != 0
                {
                    let name = from_wide_ptr((*gpi.Item).FileName);
                    if name.to_lowercase() == target_lower {
                        found_idx = Some(i);
                        break;
                    }
                }
            }

            if let Some(idx) = found_idx {
                let mut pri = PanelRedrawInfo {
                    StructSize: std::mem::size_of::<PanelRedrawInfo>(),
                    CurrentItem: idx,
                    TopPanelItem: 0,
                };
                panel_ctrl(
                    PANEL_ACTIVE,
                    FCTL_REDRAWPANEL,
                    0,
                    (&mut pri as *mut PanelRedrawInfo).cast::<c_void>(),
                );
                return;
            }
        }
    }

    panel_ctrl(PANEL_ACTIVE, FCTL_REDRAWPANEL, 0, std::ptr::null_mut());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn ffi_struct_sizes_x64() {
        assert_eq!(size_of::<GUID>(), 16);
        assert_eq!(size_of::<VersionInfo>(), 20);
        // PluginStartupInfo has 35 pointer-sized fields on x64
        assert_eq!(size_of::<PluginStartupInfo>(), 35 * 8);
        assert_eq!(size_of::<FarList>(), 24);
        assert_eq!(size_of::<FarListItem>(), 32);
        assert_eq!(size_of::<FarMacroValue>(), 24);
        assert_eq!(size_of::<FarMacroCall>(), 40);
    }

    #[test]
    fn filetime_conversion() {
        let ft = unix_secs_to_filetime(1_700_000_000);
        assert!(ft.dwHighDateTime > 0);
        let zero = unix_secs_to_filetime(0);
        assert_eq!(zero.dwLowDateTime, 0);
        assert_eq!(zero.dwHighDateTime, 0);
    }
}
