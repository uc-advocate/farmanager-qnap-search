//! FAR Manager 3 (x64) native Rust plugin: Unified Search (Local + QNAP Qsirch 7.x).

mod config;
mod dialogs;
mod far_ffi;
mod indexd;
mod local_search;
mod merger;
mod mini_json;
mod panel;
mod path_map;
mod qsirch;

use config::Config;
use dialogs::{show_results_dialog, show_search_dialog, show_settings_dialog, ResultsAction, SearchDialogAction};
use far_ffi::*;
use local_search::SearchArea;
use merger::{run_unified_search, SearchParams};
use panel::SearchPanel;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::Foundation::HANDLE;

struct StaticStrings {
    title: WideString,
    description: WideString,
    author: WideString,
    _menu_item: WideString,
    _config_item: WideString,
    cmd_prefix: WideString,
    menu_ptrs: [*const u16; 1],
    config_ptrs: [*const u16; 1],
}

// Safety: StaticStrings contains immutable read-only UTF-16 buffers initialized once.
unsafe impl Sync for StaticStrings {}
unsafe impl Send for StaticStrings {}

struct PluginState {
    psi: Option<PluginStartupInfo>,
    module_path: Option<PathBuf>,
    last_mask: String,
    last_text: String,
    last_area: SearchArea,
}

// Safety: FAR Manager invokes plugin exports on the main UI thread.
unsafe impl Send for PluginState {}

static STRINGS: OnceLock<StaticStrings> = OnceLock::new();
static STATE: OnceLock<Mutex<PluginState>> = OnceLock::new();

fn strings() -> &'static StaticStrings {
    STRINGS.get_or_init(|| {
        let title = WideString::new("FarQsirch");
        let description = WideString::new("Unified Local + QNAP Qsirch 7 File Search");
        let author = WideString::new("FarQsirch Plugin");
        let menu_item = WideString::new("Unified Search (Local + Qsirch)");
        let config_item = WideString::new("FarQsirch Settings");
        let cmd_prefix = WideString::new("qs");
        let menu_ptrs = [menu_item.as_ptr()];
        let config_ptrs = [config_item.as_ptr()];
        StaticStrings {
            title,
            description,
            author,
            _menu_item: menu_item,
            _config_item: config_item,
            cmd_prefix,
            menu_ptrs,
            config_ptrs,
        }
    })
}

fn state() -> &'static Mutex<PluginState> {
    STATE.get_or_init(|| {
        Mutex::new(PluginState {
            psi: None,
            module_path: None,
            last_mask: "*".to_string(),
            last_text: String::new(),
            last_area: SearchArea::CurrentDir,
        })
    })
}

pub(crate) fn get_psi() -> Option<PluginStartupInfo> {
    state().lock().ok().and_then(|s| s.psi)
}

fn get_paths() -> (PathBuf, PathBuf) {
    let mod_path = state().lock().ok().and_then(|s| s.module_path.clone());
    let cfg_path = Config::config_path(mod_path.as_deref());
    let cache_path = Config::session_cache_path(mod_path.as_deref());
    (cfg_path, cache_path)
}

// ─── FAR Manager 3 Exported Entry Points ──────────────────────────────────────

#[no_mangle]
pub unsafe extern "system" fn GetGlobalInfoW(info: *mut GlobalInfo) {
    let _ = std::panic::catch_unwind(|| {
        if info.is_null() {
            return;
        }
        let s = strings();
        let g = &mut *info;
        g.StructSize = std::mem::size_of::<GlobalInfo>();
        g.MinFarVersion = VersionInfo {
            Major: 3,
            Minor: 0,
            Revision: 0,
            Build: 1905,
            Stage: VS_RELEASE,
        };
        g.Version = VersionInfo {
            Major: 0,
            Minor: 1,
            Revision: 0,
            Build: 1,
            Stage: VS_RELEASE,
        };
        g.Guid = PLUGIN_GUID;
        g.Title = s.title.as_ptr();
        g.Description = s.description.as_ptr();
        g.Author = s.author.as_ptr();
    });
}

#[no_mangle]
pub unsafe extern "system" fn SetStartupInfoW(info: *const PluginStartupInfo) {
    let _ = std::panic::catch_unwind(|| {
        if info.is_null() {
            return;
        }
        let psi = *info;
        let mod_path = from_wide_ptr(psi.ModuleName);
        if let Ok(mut st) = state().lock() {
            st.psi = Some(psi);
            if !mod_path.is_empty() {
                st.module_path = Some(PathBuf::from(mod_path));
            }
        }
    });
}

#[no_mangle]
pub unsafe extern "system" fn GetPluginInfoW(info: *mut PluginInfo) {
    let _ = std::panic::catch_unwind(|| {
        if info.is_null() {
            return;
        }
        let s = strings();
        let p = &mut *info;
        p.StructSize = std::mem::size_of::<PluginInfo>();
        p.Flags = PF_NONE;
        p.DiskMenu = PluginMenuItem {
            Guids: std::ptr::null(),
            Strings: std::ptr::null(),
            Count: 0,
        };
        p.PluginMenu = PluginMenuItem {
            Guids: &MENU_GUID,
            Strings: s.menu_ptrs.as_ptr(),
            Count: 1,
        };
        p.PluginConfig = PluginMenuItem {
            Guids: &CONFIG_MENU_GUID,
            Strings: s.config_ptrs.as_ptr(),
            Count: 1,
        };
        p.CommandPrefix = s.cmd_prefix.as_ptr();
    });
}

#[no_mangle]
pub unsafe extern "system" fn ConfigureW(_info: *const ConfigureInfo) -> isize {
    std::panic::catch_unwind(|| {
        let Some(psi) = get_psi() else {
            return 0;
        };
        let (cfg_path, cache_path) = get_paths();
        let mut cfg = Config::load(&cfg_path);
        if show_settings_dialog(&psi, &mut cfg, &cfg_path, Some(cache_path)) {
            1
        } else {
            0
        }
    })
    .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "system" fn OpenW(info: *const OpenInfo) -> HANDLE {
    std::panic::catch_unwind(|| {
        if info.is_null() {
            return std::ptr::null_mut();
        }
        let Some(psi) = get_psi() else {
            return std::ptr::null_mut();
        };

        let open_info = &*info;
        let current_dir = get_active_panel_dir(&psi);
        let (cfg_path, cache_path) = get_paths();
        let mut cfg = Config::load(&cfg_path);

        // Support `qs:<query>` directly from FAR command line
        let cmd_query = if open_info.OpenFrom == OPEN_COMMANDLINE && open_info.Data != 0 {
            let cli = &*(open_info.Data as *const OpenCommandLineInfo);
            Some(from_wide_ptr(cli.CommandLine).trim().to_string())
        } else {
            None
        };

        let (last_mask, last_text, last_area) = state()
            .lock()
            .map(|s| (s.last_mask.clone(), s.last_text.clone(), s.last_area))
            .unwrap_or_else(|_| ("*".to_string(), String::new(), SearchArea::CurrentDir));

        // Automatically disable slow local/SMB walk when browsing inside a configured QNAP NAS share.
        let on_nas_share = (cfg.is_qsirch_configured() || cfg.is_indexd_configured())
            && path_map::PathMapper::from_config(&cfg)
                .windows_to_nas_prefix(&current_dir)
                .is_some();

        let mut params = SearchParams {
            current_dir,
            file_mask: last_mask,
            containing_text: last_text,
            include_local: if on_nas_share {
                false
            } else {
                cfg.default_include_local
            },
            include_qsirch: cfg.default_include_qsirch,
            search_area: last_area,
            scope_qsirch_to_current_folder: on_nas_share,
            case_sensitive: false,
            advanced_mode: cfg.default_advanced_mode,
        };

        let mut skip_first_dialog = false;
        if let Some(q) = cmd_query {
            if !q.is_empty() {
                if q.contains('*') || q.contains('?') {
                    params.file_mask = q;
                    params.containing_text.clear();
                } else {
                    params.file_mask = "*".to_string();
                    params.containing_text = q;
                }
                skip_first_dialog = true;
            }
        }

        loop {
            if !skip_first_dialog {
                match show_search_dialog(&psi, &params) {
                    SearchDialogAction::Search(p) => {
                        params = p;
                    }
                    SearchDialogAction::OpenSettings(p) => {
                        params = p;
                        show_settings_dialog(&psi, &mut cfg, &cfg_path, Some(cache_path.clone()));
                        continue;
                    }
                    SearchDialogAction::Cancel => return std::ptr::null_mut(),
                }
            }
            skip_first_dialog = false;

            if let Ok(mut st) = state().lock() {
                st.last_mask = params.file_mask.clone();
                st.last_text = params.containing_text.clone();
                st.last_area = params.search_area;
            }

            let mut outcome = run_unified_search(&psi, &cfg, &params, Some(cache_path.clone()));

            match show_results_dialog(&psi, &mut outcome, &cfg, &params, Some(cache_path.clone())) {
                ResultsAction::GoTo(full_path) => {
                    let p = Path::new(&full_path);
                    let dir = p
                        .parent()
                        .map(|d| d.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let file = p
                        .file_name()
                        .map(|f| f.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if !dir.is_empty() {
                        goto_panel_file(&psi, &dir, &file);
                    }
                    return std::ptr::null_mut();
                }
                ResultsAction::SendToPanel => {
                    let summary = if !params.containing_text.is_empty() {
                        format!("{} [{}]", params.file_mask, params.containing_text)
                    } else {
                        params.file_mask.clone()
                    };
                    let panel = SearchPanel::new(&summary, outcome.results);
                    let handle = panel.into_handle();
                    if open_info.OpenFrom == OPEN_FROMMACRO {
                        return wrap_panel_for_macro(handle);
                    }
                    return handle;
                }
                ResultsAction::NewSearch => continue,
                ResultsAction::Close => return std::ptr::null_mut(),
            }
        }
    })
    .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub unsafe extern "system" fn GetOpenPanelInfoW(info: *mut OpenPanelInfo) {
    let _ = std::panic::catch_unwind(|| {
        if info.is_null() {
            return;
        }
        let opi = &mut *info;
        if let Some(panel) = SearchPanel::from_handle_mut(opi.hPanel) {
            panel.fill_open_panel_info(opi);
        }
    });
}

#[no_mangle]
pub unsafe extern "system" fn GetFindDataW(info: *mut GetFindDataInfo) -> isize {
    std::panic::catch_unwind(|| {
        if info.is_null() {
            return 0;
        }
        let fdi = &mut *info;
        if let Some(panel) = SearchPanel::from_handle_mut(fdi.hPanel) {
            panel.fill_find_data(fdi)
        } else {
            0
        }
    })
    .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "system" fn FreeFindDataW(info: *const FreeFindDataInfo) {
    let _ = std::panic::catch_unwind(|| {
        if info.is_null() {
            return;
        }
        let fdi = &*info;
        if let Some(panel) = SearchPanel::from_handle_mut(fdi.hPanel) {
            panel.free_find_data(fdi.PanelItem);
        }
    });
}

#[no_mangle]
pub unsafe extern "system" fn ClosePanelW(info: *const ClosePanelInfo) {
    let _ = std::panic::catch_unwind(|| {
        if info.is_null() {
            return;
        }
        SearchPanel::drop_handle((*info).hPanel);
    });
}

#[no_mangle]
pub unsafe extern "system" fn ProcessSynchroEventW(_info: *const ProcessSynchroEventInfo) -> isize {
    let _ = std::panic::catch_unwind(|| {
        dialogs::handle_synchro_event();
    });
    0
}

#[no_mangle]
pub unsafe extern "system" fn ExitFARW(_info: *const ExitInfo) {}
