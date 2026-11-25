//! FAR Manager 3 native dialogs:
//! - Unified Search Dialog (`Alt+F7`-style)
//! - Results Dialog (with live snippet status, Go to, Panel, View, Edit, New Search)
//! - QNAP Qsirch Settings Dialog (with live Connection Test)

use crate::config::Config;
use crate::far_ffi::*;
use crate::local_search::{resolve_local_roots, SearchArea};
use crate::merger::{ResultSource, SearchOutcome, SearchParams, UnifiedResult};
use crate::path_map::PathMapper;
use crate::qsirch::{QsirchClient, QsirchItem};
use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use windows_sys::Win32::Foundation::HANDLE;

pub enum SearchDialogAction {
    Search(SearchParams),
    OpenSettings(SearchParams),
    Cancel,
}

pub enum ResultsAction {
    GoTo(String),
    SendToPanel,
    NewSearch,
    Close,
}

// ─── Unified Search Dialog ────────────────────────────────────────────────────

pub unsafe fn show_search_dialog(
    psi: &PluginStartupInfo,
    initial: &SearchParams,
) -> SearchDialogAction {
    let (Some(dlg_init), Some(dlg_run), Some(dlg_free)) =
        (psi.DialogInit, psi.DialogRun, psi.DialogFree)
    else {
        return SearchDialogAction::Cancel;
    };

    const W: isize = 80;
    const H: isize = 23;

    // Keep all WideStrings alive for the duration of the dialog
    let s_title = WideString::new(" Unified Search (Local + QNAP Qsirch 7) ");
    let s_mask_lbl = WideString::new("File / path &mask(s) (e.g. *.pdf; */part1/part2/*.xsl):");
    let s_mask_val = WideString::new(&initial.file_mask);
    let s_mask_hist = WideString::new("FarQsirchMask");
    let s_text_lbl = WideString::new("Containing &text (local + Qsirch full-text):");
    let s_text_val = WideString::new(&initial.containing_text);
    let s_text_hist = WideString::new("FarQsirchText");
    let s_sep1 = WideString::new("Search Area");
    let s_area_cur = WideString::new("From the &current folder");
    let s_area_root = WideString::new("&Root of current drive");
    let s_area_fixed = WideString::new("All fixed &drives");
    let s_area_all = WideString::new("All dri&ves (local + net)");
    let s_sep2 = WideString::new("Sources && Qsirch Options");
    let s_chk_local = WideString::new("Include &Local / SMB walk");
    let s_chk_qsirch = WideString::new("Include &QNAP Qsirch 7");
    let s_chk_case = WideString::new("C&ase sensitive (local)");
    let s_chk_nas_scope = WideString::new("Current NAS &folder only");
    let s_rad_m0 = WideString::new("Mode: Te&xt");
    let s_rad_m1 = WideString::new("Image &OCR");
    let s_rad_m2 = WideString::new("Com&bined");
    let s_sep3 = WideString::new("");
    let s_btn_search = WideString::new("&Search");
    let s_btn_settings = WideString::new("Settin&gs");
    let s_btn_cancel = WideString::new("Ca&ncel");

    let mut items: Vec<FarDialogItem> = Vec::with_capacity(24);

    // 0: Frame
    let mut frame = FarDialogItem::empty(DI_DOUBLEBOX, 3, 1, W - 4, H - 2);
    frame.Data = s_title.as_ptr();
    items.push(frame);

    // 1: File mask label
    let mut lbl_mask = FarDialogItem::empty(DI_TEXT, 5, 2, W - 6, 2);
    lbl_mask.Data = s_mask_lbl.as_ptr();
    items.push(lbl_mask);

    // 2: File mask edit
    let mut ed_mask = FarDialogItem::empty(DI_EDIT, 5, 3, W - 6, 3);
    ed_mask.Flags = DIF_HISTORY | DIF_SELECTONENTRY | DIF_FOCUS;
    ed_mask.History = s_mask_hist.as_ptr();
    ed_mask.Data = s_mask_val.as_ptr();
    items.push(ed_mask);

    // 3: Containing text label
    let mut lbl_text = FarDialogItem::empty(DI_TEXT, 5, 4, W - 6, 4);
    lbl_text.Data = s_text_lbl.as_ptr();
    items.push(lbl_text);

    // 4: Containing text edit
    let mut ed_text = FarDialogItem::empty(DI_EDIT, 5, 5, W - 6, 5);
    ed_text.Flags = DIF_HISTORY;
    ed_text.History = s_text_hist.as_ptr();
    ed_text.Data = s_text_val.as_ptr();
    items.push(ed_text);

    // 5: Separator 1 (Search Area)
    let mut sep1 = FarDialogItem::empty(DI_TEXT, -1, 7, 0, 7);
    sep1.Flags = DIF_SEPARATOR;
    sep1.Data = s_sep1.as_ptr();
    items.push(sep1);

    // 6: Radio CurrentDir
    let mut r_cur = FarDialogItem::empty(DI_RADIOBUTTON, 5, 8, 38, 8);
    r_cur.Flags = DIF_GROUP;
    r_cur.Param.Selected = if initial.search_area == SearchArea::CurrentDir { 1 } else { 0 };
    r_cur.Data = s_area_cur.as_ptr();
    items.push(r_cur);

    // 7: Radio CurrentDriveRoot
    let mut r_root = FarDialogItem::empty(DI_RADIOBUTTON, 41, 8, W - 6, 8);
    r_root.Param.Selected = if initial.search_area == SearchArea::CurrentDriveRoot { 1 } else { 0 };
    r_root.Data = s_area_root.as_ptr();
    items.push(r_root);

    // 8: Radio AllFixedDrives
    let mut r_fixed = FarDialogItem::empty(DI_RADIOBUTTON, 5, 9, 38, 9);
    r_fixed.Param.Selected = if initial.search_area == SearchArea::AllFixedDrives { 1 } else { 0 };
    r_fixed.Data = s_area_fixed.as_ptr();
    items.push(r_fixed);

    // 9: Radio AllDrives
    let mut r_all = FarDialogItem::empty(DI_RADIOBUTTON, 41, 9, W - 6, 9);
    r_all.Param.Selected = if initial.search_area == SearchArea::AllDrives { 1 } else { 0 };
    r_all.Data = s_area_all.as_ptr();
    items.push(r_all);

    // 10: Separator 2 (Sources & Qsirch Options)
    let mut sep2 = FarDialogItem::empty(DI_TEXT, -1, 11, 0, 11);
    sep2.Flags = DIF_SEPARATOR;
    sep2.Data = s_sep2.as_ptr();
    items.push(sep2);

    // 11: Checkbox Local
    let mut chk_local = FarDialogItem::empty(DI_CHECKBOX, 5, 12, 38, 12);
    chk_local.Param.Selected = if initial.include_local { 1 } else { 0 };
    chk_local.Data = s_chk_local.as_ptr();
    items.push(chk_local);

    // 12: Checkbox Qsirch
    let mut chk_qsirch = FarDialogItem::empty(DI_CHECKBOX, 41, 12, W - 6, 12);
    chk_qsirch.Param.Selected = if initial.include_qsirch { 1 } else { 0 };
    chk_qsirch.Data = s_chk_qsirch.as_ptr();
    items.push(chk_qsirch);

    // 13: Checkbox Case sensitive
    let mut chk_case = FarDialogItem::empty(DI_CHECKBOX, 5, 13, 38, 13);
    chk_case.Param.Selected = if initial.case_sensitive { 1 } else { 0 };
    chk_case.Data = s_chk_case.as_ptr();
    items.push(chk_case);

    // 14: Checkbox Limit Qsirch to current NAS folder
    let mut chk_nas_scope = FarDialogItem::empty(DI_CHECKBOX, 41, 13, W - 6, 13);
    chk_nas_scope.Param.Selected = if initial.scope_qsirch_to_current_folder { 1 } else { 0 };
    chk_nas_scope.Data = s_chk_nas_scope.as_ptr();
    items.push(chk_nas_scope);

    // 15: Radio Mode 0 (Text)
    let mut rad_m0 = FarDialogItem::empty(DI_RADIOBUTTON, 5, 15, 24, 15);
    rad_m0.Flags = DIF_GROUP;
    rad_m0.Param.Selected = if initial.advanced_mode == 0 { 1 } else { 0 };
    rad_m0.Data = s_rad_m0.as_ptr();
    items.push(rad_m0);

    // 16: Radio Mode 1 (OCR)
    let mut rad_m1 = FarDialogItem::empty(DI_RADIOBUTTON, 28, 15, 46, 15);
    rad_m1.Param.Selected = if initial.advanced_mode == 1 { 1 } else { 0 };
    rad_m1.Data = s_rad_m1.as_ptr();
    items.push(rad_m1);

    // 17: Radio Mode 2 (Combined)
    let mut rad_m2 = FarDialogItem::empty(DI_RADIOBUTTON, 50, 15, W - 6, 15);
    rad_m2.Param.Selected = if initial.advanced_mode == 2 { 1 } else { 0 };
    rad_m2.Data = s_rad_m2.as_ptr();
    items.push(rad_m2);

    // 18: Separator 3
    let mut sep3 = FarDialogItem::empty(DI_TEXT, -1, 17, 0, 17);
    sep3.Flags = DIF_SEPARATOR;
    sep3.Data = s_sep3.as_ptr();
    items.push(sep3);

    // 19: Search button
    let mut btn_search = FarDialogItem::empty(DI_BUTTON, 0, 19, 0, 19);
    btn_search.Flags = DIF_CENTERGROUP | DIF_DEFAULTBUTTON;
    btn_search.Data = s_btn_search.as_ptr();
    items.push(btn_search);

    // 20: Settings button
    let mut btn_settings = FarDialogItem::empty(DI_BUTTON, 0, 19, 0, 19);
    btn_settings.Flags = DIF_CENTERGROUP;
    btn_settings.Data = s_btn_settings.as_ptr();
    items.push(btn_settings);

    // 21: Cancel button
    let mut btn_cancel = FarDialogItem::empty(DI_BUTTON, 0, 19, 0, 19);
    btn_cancel.Flags = DIF_CENTERGROUP;
    btn_cancel.Data = s_btn_cancel.as_ptr();
    items.push(btn_cancel);

    const ID_MASK: isize = 2;
    const ID_TEXT: isize = 4;
    const ID_AREA_ROOT: isize = 7;
    const ID_AREA_FIXED: isize = 8;
    const ID_AREA_ALL: isize = 9;
    const ID_LOCAL: isize = 11;
    const ID_QSIRCH: isize = 12;
    const ID_CASE: isize = 13;
    const ID_NAS_SCOPE: isize = 14;
    const ID_M1: isize = 16;
    const ID_M2: isize = 17;
    const ID_BTN_SEARCH: isize = 19;
    const ID_BTN_SETTINGS: isize = 20;

    let h_dlg = dlg_init(
        &PLUGIN_GUID,
        &SEARCH_DLG_GUID,
        -1,
        -1,
        W,
        H,
        std::ptr::null(),
        items.as_ptr(),
        items.len(),
        0,
        FDLG_NONE,
        None,
        std::ptr::null_mut(),
    );
    if h_dlg.is_null() || h_dlg == (-1isize as HANDLE) {
        return SearchDialogAction::Cancel;
    }

    let exit_code = dlg_run(h_dlg);
    let read_params = || {
        let mut mask = get_dlg_text(psi, h_dlg, ID_MASK);
        if mask.trim().is_empty() {
            mask = "*".to_string();
        }
        let search_area = if get_dlg_check(psi, h_dlg, ID_AREA_ROOT) {
            SearchArea::CurrentDriveRoot
        } else if get_dlg_check(psi, h_dlg, ID_AREA_FIXED) {
            SearchArea::AllFixedDrives
        } else if get_dlg_check(psi, h_dlg, ID_AREA_ALL) {
            SearchArea::AllDrives
        } else {
            SearchArea::CurrentDir
        };
        let adv = if get_dlg_check(psi, h_dlg, ID_M1) {
            1
        } else if get_dlg_check(psi, h_dlg, ID_M2) {
            2
        } else {
            0
        };
        SearchParams {
            current_dir: initial.current_dir.clone(),
            file_mask: mask,
            containing_text: get_dlg_text(psi, h_dlg, ID_TEXT),
            include_local: get_dlg_check(psi, h_dlg, ID_LOCAL),
            include_qsirch: get_dlg_check(psi, h_dlg, ID_QSIRCH),
            search_area,
            scope_qsirch_to_current_folder: get_dlg_check(psi, h_dlg, ID_NAS_SCOPE),
            case_sensitive: get_dlg_check(psi, h_dlg, ID_CASE),
            advanced_mode: adv,
        }
    };

    let action = if exit_code == ID_BTN_SEARCH {
        SearchDialogAction::Search(read_params())
    } else if exit_code == ID_BTN_SETTINGS {
        SearchDialogAction::OpenSettings(read_params())
    } else {
        SearchDialogAction::Cancel
    };

    dlg_free(h_dlg);
    action
}

// ─── Results Dialog ───────────────────────────────────────────────────────────

// ─── Live Background Pagination State ─────────────────────────────────────────

struct LiveResultsStream {
    h_dlg: HANDLE,
    psi: PluginStartupInfo,
    mapper: PathMapper,
    results: Vec<UnifiedResult>,
    index_by_key: HashMap<String, usize>,
    pending: VecDeque<QsirchItem>,
    local_count: usize,
    qsirch_count: usize,
    qsirch_total: usize,
    qsirch_fetched_raw: usize,
    merged_both_count: usize,
    elapsed_ms: u128,
    bg_running: bool,
    exhausted: bool,
    bg_note: String,
    list_item_id: isize,
    frame_item_id: isize,
    warn_item_id: isize,
}

// Safety: `h_dlg` and `psi` are only ever touched on FAR's main UI thread
// inside `show_results_dialog`, `results_dlg_proc`, and `ProcessSynchroEventW`.
unsafe impl Send for LiveResultsStream {}

static ACTIVE_STREAM: OnceLock<Mutex<Option<Arc<Mutex<LiveResultsStream>>>>> = OnceLock::new();

fn active_stream() -> &'static Mutex<Option<Arc<Mutex<LiveResultsStream>>>> {
    ACTIVE_STREAM.get_or_init(|| Mutex::new(None))
}

fn format_results_title(st: &LiveResultsStream) -> String {
    let suffix = if st.bg_running {
        format!(" [loading {}/{}...]", st.qsirch_fetched_raw, st.qsirch_total)
    } else if !st.bg_note.is_empty() {
        format!(" [{}]", st.bg_note)
    } else {
        String::new()
    };
    format!(
        " Search Results: {} found (Local: {}, Qsirch: {}/{}, Both: {}) [{} ms]{} ",
        st.results.len(),
        st.local_count,
        st.qsirch_count,
        st.qsirch_total,
        st.merged_both_count,
        st.elapsed_ms,
        suffix
    )
}

/// Called by FAR Manager on the main UI thread via `ProcessSynchroEventW` whenever
/// the background pagination thread finishes a page and signals `ACTL_SYNCHRO`.
pub unsafe fn handle_synchro_event() {
    let stream_arc = match active_stream().lock() {
        Ok(guard) => guard.clone(),
        Err(_) => return,
    };
    let Some(stream_arc) = stream_arc else {
        return;
    };
    let Ok(mut st) = stream_arc.lock() else {
        return;
    };
    if st.h_dlg.is_null() {
        return;
    }
    let Some(send) = st.psi.SendDlgMessage else {
        return;
    };

    let mut new_lines: Vec<WideString> = Vec::new();
    while let Some(item) = st.pending.pop_front() {
        let win_path = st.mapper.nas_to_windows(&item.nas_path);
        let key = st.mapper.dedup_key(&win_path);
        if let Some(&existing_idx) = st.index_by_key.get(&key) {
            let mut upgraded = false;
            {
                let existing = &mut st.results[existing_idx];
                if existing.source == ResultSource::Local {
                    existing.source = ResultSource::Both;
                    upgraded = true;
                }
                if existing.snippet.is_empty() && !item.snippet.is_empty() {
                    existing.snippet = item.snippet;
                }
            }
            if upgraded {
                st.merged_both_count += 1;
                st.qsirch_count += 1;
            }
        } else {
            let idx = st.results.len();
            st.index_by_key.insert(key, idx);
            st.qsirch_count += 1;
            new_lines.push(WideString::new(&format!(
                "{} {}",
                ResultSource::Qsirch.tag(),
                win_path
            )));
            st.results.push(UnifiedResult {
                windows_path: win_path,
                size: item.size,
                modified_epoch: item.modified_epoch,
                is_dir: false,
                attributes: 0x20,
                source: ResultSource::Qsirch,
                snippet: item.snippet,
            });
        }
    }

    let h_dlg = st.h_dlg;
    let list_id = st.list_item_id;
    let frame_id = st.frame_item_id;
    let warn_id = st.warn_item_id;
    let title = WideString::new(&format_results_title(&st));
    let hint = if st.bg_running {
        Some(WideString::new(&format!(
            "Background loading Qsirch {}/{}... (Enter=Go to, F3=View, F4=Edit, Alt+P=Panel, Esc=Close)",
            st.qsirch_fetched_raw, st.qsirch_total
        )))
    } else {
        Some(WideString::new(
            "Keys: Enter=Go to, F3=View, F4=Edit, Alt+P=Panel, Alt+N=New Search, Esc=Close",
        ))
    };
    drop(st);

    for ws in &new_lines {
        send(h_dlg, DM_LISTADDSTR, list_id, ws.as_ptr() as *mut c_void);
    }
    send(h_dlg, DM_SETTEXTPTR, frame_id, title.as_ptr() as *mut c_void);
    if let Some(h) = hint {
        send(h_dlg, DM_SETTEXTPTR, warn_id, h.as_ptr() as *mut c_void);
    }
}

struct ResultsDlgState {
    psi: PluginStartupInfo,
    stream: Arc<Mutex<LiveResultsStream>>,
    status_item_id: isize,
    list_item_id: isize,
    btn_goto_id: isize,
    btn_panel_id: isize,
    btn_view_id: isize,
    btn_edit_id: isize,
    btn_new_id: isize,
    btn_close_id: isize,
}

unsafe fn open_selected_in_viewer_or_editor(
    st: &ResultsDlgState,
    h_dlg: HANDLE,
    send: FARAPISENDDLGMESSAGE,
    is_editor: bool,
) {
    let pos = send(h_dlg, DM_LISTGETCURPOS, st.list_item_id, std::ptr::null_mut());
    let path_opt = st.stream.lock().ok().and_then(|s| {
        if pos >= 0 && (pos as usize) < s.results.len() {
            Some(s.results[pos as usize].windows_path.clone())
        } else {
            None
        }
    });
    if let Some(win_path) = path_opt {
        let w_path = WideString::new(&win_path);
        if !is_editor {
            if let Some(viewer) = st.psi.Viewer {
                viewer(
                    w_path.as_ptr(),
                    std::ptr::null(),
                    0,
                    0,
                    -1,
                    -1,
                    VF_ENABLE_F6,
                    CP_DEFAULT,
                );
            }
        } else if let Some(editor) = st.psi.Editor {
            editor(
                w_path.as_ptr(),
                std::ptr::null(),
                0,
                0,
                -1,
                -1,
                EF_ENABLE_F6,
                1,
                1,
                CP_DEFAULT,
            );
        }
    }
}

unsafe extern "system" fn results_dlg_proc(
    h_dlg: HANDLE,
    msg: isize,
    param1: isize,
    param2: *mut c_void,
) -> isize {
    use windows_sys::Win32::System::Console::{INPUT_RECORD, KEY_EVENT};
    const VK_RETURN: u16 = 0x0D;
    const VK_F3: u16 = 0x72;
    const VK_F4: u16 = 0x73;
    const RIGHT_ALT_PRESSED: u32 = 0x0001;
    const LEFT_ALT_PRESSED: u32 = 0x0002;
    const ALT_MASK: u32 = LEFT_ALT_PRESSED | RIGHT_ALT_PRESSED;

    let Some(psi) = crate::get_psi() else {
        return 0;
    };
    let Some(send) = psi.SendDlgMessage else {
        return 0;
    };
    let Some(def_proc) = psi.DefDlgProc else {
        return 0;
    };

    let dlg_data = send(h_dlg, 4 /* DM_GETDLGDATA */, 0, std::ptr::null_mut()) as *mut ResultsDlgState;
    if !dlg_data.is_null() {
        let st = &*dlg_data;
        if msg == DN_INITDIALOG || (msg == DN_LISTCHANGE && param1 == st.list_item_id) {
            let idx = if msg == DN_LISTCHANGE {
                param2 as isize
            } else {
                0
            };
            let status_opt = st.stream.lock().ok().and_then(|s| {
                if idx >= 0 && (idx as usize) < s.results.len() {
                    Some(format_result_status(&s.results[idx as usize]))
                } else {
                    None
                }
            });
            if let Some(info_line) = status_opt {
                let w = WideString::new(&info_line);
                send(
                    h_dlg,
                    DM_SETTEXTPTR,
                    st.status_item_id,
                    w.as_ptr() as *mut c_void,
                );
            }
        } else if msg == DN_BTNCLICK
            && (param1 == st.btn_view_id || param1 == st.btn_edit_id)
        {
            open_selected_in_viewer_or_editor(st, h_dlg, send, param1 == st.btn_edit_id);
            return 1;
        } else if msg == DN_CONTROLINPUT && !param2.is_null() {
            let rec = &*(param2 as *const INPUT_RECORD);
            if rec.EventType as u32 == KEY_EVENT {
                let ke = rec.Event.KeyEvent;
                if ke.bKeyDown != 0 {
                    let alt = (ke.dwControlKeyState & ALT_MASK) != 0;
                    match (alt, ke.wVirtualKeyCode) {
                        (false, VK_F3) | (true, 0x56 /* V */) => {
                            open_selected_in_viewer_or_editor(st, h_dlg, send, false);
                            return 1;
                        }
                        (false, VK_F4) | (true, 0x45 /* E */) => {
                            open_selected_in_viewer_or_editor(st, h_dlg, send, true);
                            return 1;
                        }
                        (false, VK_RETURN) if param1 == st.list_item_id => {
                            send(h_dlg, DM_CLOSE, st.btn_goto_id, std::ptr::null_mut());
                            return 1;
                        }
                        (true, 0x47 /* G */) => {
                            send(h_dlg, DM_CLOSE, st.btn_goto_id, std::ptr::null_mut());
                            return 1;
                        }
                        (true, 0x50 /* P */) => {
                            send(h_dlg, DM_CLOSE, st.btn_panel_id, std::ptr::null_mut());
                            return 1;
                        }
                        (true, 0x4E /* N */) => {
                            send(h_dlg, DM_CLOSE, st.btn_new_id, std::ptr::null_mut());
                            return 1;
                        }
                        (true, 0x43 /* C */) => {
                            send(h_dlg, DM_CLOSE, st.btn_close_id, std::ptr::null_mut());
                            return 1;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    def_proc(h_dlg, msg, param1, param2)
}

fn format_result_status(r: &UnifiedResult) -> String {
    let size_str = format_size(r.size);
    if r.snippet.is_empty() {
        format!("{} | {} | {}", r.source.tag().trim(), size_str, r.windows_path)
    } else {
        format!("{} | {} | {}", r.source.tag().trim(), size_str, r.snippet)
    }
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GiB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MiB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KiB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

pub unsafe fn show_results_dialog(
    psi: &PluginStartupInfo,
    outcome: &mut SearchOutcome,
    cfg: &Config,
    params: &SearchParams,
    session_cache_path: Option<PathBuf>,
) -> ResultsAction {
    let (Some(dlg_init), Some(dlg_run), Some(dlg_free), Some(send)) =
        (psi.DialogInit, psi.DialogRun, psi.DialogFree, psi.SendDlgMessage)
    else {
        return ResultsAction::Close;
    };

    let (w, h) = get_far_console_size(psi);
    let has_more = outcome.qsirch_has_more();

    let mapper = PathMapper::from_config(cfg);
    let index_by_key: HashMap<String, usize> = outcome
        .results
        .iter()
        .enumerate()
        .map(|(i, r)| (mapper.dedup_key(&r.windows_path), i))
        .collect();

    const ID_FRAME: isize = 0;
    const ID_WARN: isize = 1;
    const ID_LIST: isize = 2;
    const ID_STATUS: isize = 4;
    const ID_BTN_GOTO: isize = 5;
    const ID_BTN_PANEL: isize = 6;
    const ID_BTN_VIEW: isize = 7;
    const ID_BTN_EDIT: isize = 8;
    const ID_BTN_NEW: isize = 9;
    const ID_BTN_CLOSE: isize = 10;

    let stream = Arc::new(Mutex::new(LiveResultsStream {
        h_dlg: std::ptr::null_mut(),
        psi: *psi,
        mapper: mapper.clone(),
        results: outcome.results.clone(),
        index_by_key,
        pending: VecDeque::new(),
        local_count: outcome.local_count,
        qsirch_count: outcome.qsirch_count,
        qsirch_total: outcome.qsirch_total_reported,
        qsirch_fetched_raw: outcome.qsirch_fetched_raw,
        merged_both_count: outcome.merged_both_count,
        elapsed_ms: outcome.elapsed_ms,
        bg_running: has_more,
        exhausted: outcome.qsirch_exhausted,
        bg_note: String::new(),
        list_item_id: ID_LIST,
        frame_item_id: ID_FRAME,
        warn_item_id: ID_WARN,
    }));

    let title_str = stream
        .lock()
        .map(|s| format_results_title(&s))
        .unwrap_or_default();
    let warn_str = if !outcome.warnings.is_empty() {
        format!("Note: {}", outcome.warnings.join(" | "))
    } else if has_more {
        format!(
            "Background loading Qsirch {}/{}... (Enter=Go to, F3=View, F4=Edit, Alt+P=Panel, Esc=Close)",
            outcome.qsirch_fetched_raw, outcome.qsirch_total_reported
        )
    } else {
        "Keys: Enter=Go to, F3=View, F4=Edit, Alt+P=Panel, Alt+N=New Search, Esc=Close".to_string()
    };

    let s_title = WideString::new(&title_str);
    let s_warn = WideString::new(&warn_str);
    let s_status = WideString::new(if outcome.results.is_empty() {
        "No matching files found."
    } else {
        ""
    });
    let s_sep = WideString::new("");
    let s_btn_goto = WideString::new("&Go to");
    let s_btn_panel = WideString::new("&Panel");
    let s_btn_view = WideString::new("&View");
    let s_btn_edit = WideString::new("&Edit");
    let s_btn_new = WideString::new("&New Search");
    let s_btn_close = WideString::new("&Close");

    let list_strings: Vec<WideString> = outcome
        .results
        .iter()
        .map(|r| WideString::new(&format!("{} {}", r.source.tag(), r.windows_path)))
        .collect();

    let mut list_items: Vec<FarListItem> = list_strings
        .iter()
        .enumerate()
        .map(|(i, ws)| FarListItem {
            Flags: if i == 0 { LIF_SELECTED } else { LIF_NONE },
            Text: ws.as_ptr(),
            UserData: i as isize,
            Reserved: 0,
        })
        .collect();

    let mut far_list = FarList {
        StructSize: std::mem::size_of::<FarList>(),
        ItemsNumber: list_items.len(),
        Items: if list_items.is_empty() {
            std::ptr::null_mut()
        } else {
            list_items.as_mut_ptr()
        },
    };

    let mut items: Vec<FarDialogItem> = Vec::with_capacity(11);

    // 0: Full-screen frame
    let mut frame = FarDialogItem::empty(DI_DOUBLEBOX, 0, 0, w - 1, h - 1);
    frame.Data = s_title.as_ptr();
    items.push(frame);

    // 1: Warning / shortcut hint line
    let mut lbl_warn = FarDialogItem::empty(DI_TEXT, 2, 1, w - 3, 1);
    lbl_warn.Data = s_warn.as_ptr();
    items.push(lbl_warn);

    // 2: Borderless listbox (no redundant double border at bottom)
    let mut listbox = FarDialogItem::empty(DI_LISTBOX, 1, 2, w - 2, h - 5);
    listbox.Param.ListItems = &mut far_list;
    listbox.Flags = DIF_LISTNOAMPERSAND | DIF_LISTNOBOX | DIF_LISTWRAPMODE | DIF_FOCUS;
    items.push(listbox);

    // 3: Single separator
    let mut sep = FarDialogItem::empty(DI_TEXT, -1, h - 4, 0, h - 4);
    sep.Flags = DIF_SEPARATOR;
    sep.Data = s_sep.as_ptr();
    items.push(sep);

    // 4: Status / snippet line
    let mut lbl_status = FarDialogItem::empty(DI_TEXT, 2, h - 3, w - 3, h - 3);
    lbl_status.Data = s_status.as_ptr();
    items.push(lbl_status);

    // 5: Go to
    let mut btn_goto = FarDialogItem::empty(DI_BUTTON, 0, h - 2, 0, h - 2);
    btn_goto.Flags = DIF_CENTERGROUP | DIF_DEFAULTBUTTON;
    btn_goto.Data = s_btn_goto.as_ptr();
    items.push(btn_goto);

    // 6: Panel
    let mut btn_panel = FarDialogItem::empty(DI_BUTTON, 0, h - 2, 0, h - 2);
    btn_panel.Flags = DIF_CENTERGROUP;
    btn_panel.Data = s_btn_panel.as_ptr();
    items.push(btn_panel);

    // 7: View (does not close dialog)
    let mut btn_view = FarDialogItem::empty(DI_BUTTON, 0, h - 2, 0, h - 2);
    btn_view.Flags = DIF_CENTERGROUP | DIF_BTNNOCLOSE;
    btn_view.Data = s_btn_view.as_ptr();
    items.push(btn_view);

    // 8: Edit (does not close dialog)
    let mut btn_edit = FarDialogItem::empty(DI_BUTTON, 0, h - 2, 0, h - 2);
    btn_edit.Flags = DIF_CENTERGROUP | DIF_BTNNOCLOSE;
    btn_edit.Data = s_btn_edit.as_ptr();
    items.push(btn_edit);

    // 9: New Search
    let mut btn_new = FarDialogItem::empty(DI_BUTTON, 0, h - 2, 0, h - 2);
    btn_new.Flags = DIF_CENTERGROUP;
    btn_new.Data = s_btn_new.as_ptr();
    items.push(btn_new);

    // 10: Close
    let mut btn_close = FarDialogItem::empty(DI_BUTTON, 0, h - 2, 0, h - 2);
    btn_close.Flags = DIF_CENTERGROUP;
    btn_close.Data = s_btn_close.as_ptr();
    items.push(btn_close);

    let mut state = ResultsDlgState {
        psi: *psi,
        stream: Arc::clone(&stream),
        status_item_id: ID_STATUS,
        list_item_id: ID_LIST,
        btn_goto_id: ID_BTN_GOTO,
        btn_panel_id: ID_BTN_PANEL,
        btn_view_id: ID_BTN_VIEW,
        btn_edit_id: ID_BTN_EDIT,
        btn_new_id: ID_BTN_NEW,
        btn_close_id: ID_BTN_CLOSE,
    };

    let h_dlg = dlg_init(
        &PLUGIN_GUID,
        &RESULTS_DLG_GUID,
        0,
        0,
        w - 1,
        h - 1,
        std::ptr::null(),
        items.as_ptr(),
        items.len(),
        0,
        FDLG_SMALLDIALOG,
        Some(results_dlg_proc),
        (&mut state as *mut ResultsDlgState).cast::<c_void>(),
    );
    if h_dlg.is_null() || h_dlg == (-1isize as HANDLE) {
        return ResultsAction::Close;
    }

    // Register active dialog handle and spawn background pagination thread if more Qsirch pages exist.
    let bg_cancel = Arc::new(AtomicBool::new(false));
    if has_more {
        if let Ok(mut s) = stream.lock() {
            s.h_dlg = h_dlg;
        }
        if let Ok(mut guard) = active_stream().lock() {
            *guard = Some(Arc::clone(&stream));
        }

        let cfg_clone = cfg.clone();
        let q = outcome.qsirch_effective_query.clone();
        let start_offset = outcome.qsirch_fetched_raw;
        let total_hint = outcome.qsirch_total_reported;
        let mask = params.file_mask.clone();
        let want_hl = !params.containing_text.trim().is_empty();
        let adv_mode = params.advanced_mode;
        let local_roots = resolve_local_roots(&params.current_dir, params.search_area);
        let nas_scope = match params.search_area {
            SearchArea::CurrentDir if params.scope_qsirch_to_current_folder => {
                mapper.windows_to_nas_prefix(&params.current_dir)
            }
            SearchArea::CurrentDriveRoot if params.scope_qsirch_to_current_folder => local_roots
                .first()
                .and_then(|r| mapper.windows_to_nas_prefix(r)),
            _ => None,
        };
        let stream_bg = Arc::clone(&stream);
        let cancel_bg = Arc::clone(&bg_cancel);
        let adv_ctrl = psi.AdvControl;

        std::thread::spawn(move || {
            let signal_ui = || {
                if let Some(adv) = adv_ctrl {
                    unsafe {
                        adv(&PLUGIN_GUID, ACTL_SYNCHRO, 0, std::ptr::null_mut());
                    }
                }
            };

            if let Ok(mut client) = QsirchClient::from_config(&cfg_clone, session_cache_path) {
                let mut offset = start_offset;
                let mut total = total_hint;
                let dummy_status = Mutex::new(String::new());

                while offset < total && !cancel_bg.load(Ordering::Relaxed) {
                    match client.fetch_one_page(
                        &q,
                        offset,
                        100,
                        &mask,
                        nas_scope.as_deref(),
                        adv_mode,
                        want_hl,
                    ) {
                        Ok((new_total, fetched_count, matched_items)) => {
                            total = new_total;
                            if fetched_count == 0 {
                                break;
                            }
                            offset += fetched_count;
                            let got_matches = !matched_items.is_empty();
                            if let Ok(mut s) = stream_bg.lock() {
                                s.qsirch_total = total;
                                s.qsirch_fetched_raw = offset;
                                for it in matched_items {
                                    s.pending.push_back(it);
                                }
                            }
                            signal_ui();
                            // Stop early on 0-match page (relevance-ordered Qsirch results).
                            if !got_matches {
                                if let Ok(mut s) = stream_bg.lock() {
                                    s.exhausted = true;
                                    s.bg_note = "done".to_string();
                                }
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                    let _ = &dummy_status;
                }

                if !cancel_bg.load(Ordering::Relaxed) && offset >= total {
                    if let Ok(mut s) = stream_bg.lock() {
                        s.exhausted = true;
                        s.bg_note = "all loaded".to_string();
                    }
                }
            }

            if let Ok(mut s) = stream_bg.lock() {
                s.bg_running = false;
            }
            signal_ui();
        });
    }

    let exit_code = dlg_run(h_dlg);
    let selected_idx = send(h_dlg, DM_LISTGETCURPOS, ID_LIST, std::ptr::null_mut());

    // Stop background thread and detach dialog handle BEFORE dlg_free!
    bg_cancel.store(true, Ordering::Relaxed);
    if let Ok(mut guard) = active_stream().lock() {
        *guard = None;
    }
    if let Ok(mut s) = stream.lock() {
        s.h_dlg = std::ptr::null_mut();
        // Sync any streamed results back into `outcome` so GoTo / Panel see everything!
        outcome.results = s.results.clone();
        outcome.qsirch_count = s.qsirch_count;
        outcome.qsirch_total_reported = s.qsirch_total;
        outcome.qsirch_fetched_raw = s.qsirch_fetched_raw;
        outcome.qsirch_exhausted = s.exhausted;
        outcome.merged_both_count = s.merged_both_count;
    }

    dlg_free(h_dlg);

    if exit_code == ID_BTN_GOTO {
        if selected_idx >= 0 && (selected_idx as usize) < outcome.results.len() {
            return ResultsAction::GoTo(outcome.results[selected_idx as usize].windows_path.clone());
        }
    } else if exit_code == ID_BTN_PANEL && !outcome.results.is_empty() {
        return ResultsAction::SendToPanel;
    } else if exit_code == ID_BTN_NEW {
        return ResultsAction::NewSearch;
    }

    ResultsAction::Close
}

// ─── Settings Dialog ──────────────────────────────────────────────────────────

pub unsafe fn show_settings_dialog(
    psi: &PluginStartupInfo,
    cfg: &mut Config,
    cfg_path: &Path,
    session_cache_path: Option<PathBuf>,
) -> bool {
    let (Some(dlg_init), Some(dlg_run), Some(dlg_free)) =
        (psi.DialogInit, psi.DialogRun, psi.DialogFree)
    else {
        return false;
    };

    let mut draft = cfg.clone();

    loop {
        const W: isize = 72;
        const H: isize = 22;

        let s_title = WideString::new(" QNAP Qsirch 7 & Fast Indexer Settings ");
        let s_host_lbl = WideString::new("NAS &Host / IP:");
        let s_host_val = WideString::new(&draft.nas_host);
        let s_port_lbl = WideString::new("&Port:");
        let s_port_val = WideString::new(&draft.nas_port.to_string());
        let s_ssl = WideString::new("Use SS&L (HTTPS)");
        let s_insecure = WideString::new("Accept self-signed &cert");
        let s_user_lbl = WideString::new("&Username:");
        let s_user_val = WideString::new(&draft.username);
        let s_pass_lbl = WideString::new("Pass&word (or set QSIRCH_PASS env):");
        let s_pass_val = WideString::new(&draft.password);
        let s_unc_lbl = WideString::new("SMB / U&NC host override (optional):");
        let s_unc_val = WideString::new(&draft.unc_host);
        let s_max_lbl = WideString::new("&Max Qsirch results (1..1000):");
        let s_max_val = WideString::new(&draft.max_qsirch_results.to_string());
        let s_idx_lbl = WideString::new("Fast nas_&indexd port (0=off, e.g. 9876):");
        let s_idx_val = WideString::new(&draft.indexd_port.to_string());
        let s_cfg_note = WideString::new(&format!("Config: {}", cfg_path.display()));
        let s_sep = WideString::new("");
        let s_btn_save = WideString::new("&Save");
        let s_btn_test = WideString::new("&Test Connection");
        let s_btn_cancel = WideString::new("Canc&el");

        let mut items: Vec<FarDialogItem> = Vec::with_capacity(22);

        let mut frame = FarDialogItem::empty(DI_DOUBLEBOX, 3, 1, W - 4, H - 2);
        frame.Data = s_title.as_ptr();
        items.push(frame);

        let mut l_host = FarDialogItem::empty(DI_TEXT, 5, 2, 30, 2);
        l_host.Data = s_host_lbl.as_ptr();
        items.push(l_host);

        let mut e_host = FarDialogItem::empty(DI_EDIT, 5, 3, 48, 3);
        e_host.Flags = DIF_FOCUS | DIF_SELECTONENTRY;
        e_host.Data = s_host_val.as_ptr();
        items.push(e_host);

        let mut l_port = FarDialogItem::empty(DI_TEXT, 51, 2, W - 6, 2);
        l_port.Data = s_port_lbl.as_ptr();
        items.push(l_port);

        let mut e_port = FarDialogItem::empty(DI_EDIT, 51, 3, W - 6, 3);
        e_port.Data = s_port_val.as_ptr();
        items.push(e_port);

        let mut c_ssl = FarDialogItem::empty(DI_CHECKBOX, 5, 5, 28, 5);
        c_ssl.Param.Selected = if draft.use_ssl { 1 } else { 0 };
        c_ssl.Data = s_ssl.as_ptr();
        items.push(c_ssl);

        let mut c_insec = FarDialogItem::empty(DI_CHECKBOX, 30, 5, W - 6, 5);
        c_insec.Param.Selected = if draft.accept_invalid_certs { 1 } else { 0 };
        c_insec.Data = s_insecure.as_ptr();
        items.push(c_insec);

        let mut l_user = FarDialogItem::empty(DI_TEXT, 5, 7, 34, 7);
        l_user.Data = s_user_lbl.as_ptr();
        items.push(l_user);

        let mut e_user = FarDialogItem::empty(DI_EDIT, 5, 8, 34, 8);
        e_user.Data = s_user_val.as_ptr();
        items.push(e_user);

        let mut l_pass = FarDialogItem::empty(DI_TEXT, 37, 7, W - 6, 7);
        l_pass.Data = s_pass_lbl.as_ptr();
        items.push(l_pass);

        let mut e_pass = FarDialogItem::empty(DI_PSWEDIT, 37, 8, W - 6, 8);
        e_pass.Data = s_pass_val.as_ptr();
        items.push(e_pass);

        let mut l_unc = FarDialogItem::empty(DI_TEXT, 5, 10, 42, 10);
        l_unc.Data = s_unc_lbl.as_ptr();
        items.push(l_unc);

        let mut e_unc = FarDialogItem::empty(DI_EDIT, 5, 11, 42, 11);
        e_unc.Data = s_unc_val.as_ptr();
        items.push(e_unc);

        let mut l_max = FarDialogItem::empty(DI_TEXT, 45, 10, W - 6, 10);
        l_max.Data = s_max_lbl.as_ptr();
        items.push(l_max);

        let mut e_max = FarDialogItem::empty(DI_EDIT, 45, 11, W - 6, 11);
        e_max.Data = s_max_val.as_ptr();
        items.push(e_max);

        let mut l_idx = FarDialogItem::empty(DI_TEXT, 5, 13, 46, 13);
        l_idx.Data = s_idx_lbl.as_ptr();
        items.push(l_idx);

        let mut e_idx = FarDialogItem::empty(DI_EDIT, 48, 13, W - 6, 13);
        e_idx.Data = s_idx_val.as_ptr();
        items.push(e_idx);

        let mut l_note = FarDialogItem::empty(DI_TEXT, 5, 15, W - 6, 15);
        l_note.Data = s_cfg_note.as_ptr();
        items.push(l_note);

        let mut sep = FarDialogItem::empty(DI_TEXT, -1, 17, 0, 17);
        sep.Flags = DIF_SEPARATOR;
        sep.Data = s_sep.as_ptr();
        items.push(sep);

        let mut b_save = FarDialogItem::empty(DI_BUTTON, 0, 18, 0, 18);
        b_save.Flags = DIF_CENTERGROUP | DIF_DEFAULTBUTTON;
        b_save.Data = s_btn_save.as_ptr();
        items.push(b_save);

        let mut b_test = FarDialogItem::empty(DI_BUTTON, 0, 18, 0, 18);
        b_test.Flags = DIF_CENTERGROUP;
        b_test.Data = s_btn_test.as_ptr();
        items.push(b_test);

        let mut b_cancel = FarDialogItem::empty(DI_BUTTON, 0, 18, 0, 18);
        b_cancel.Flags = DIF_CENTERGROUP;
        b_cancel.Data = s_btn_cancel.as_ptr();
        items.push(b_cancel);

        const ID_HOST: isize = 2;
        const ID_PORT: isize = 4;
        const ID_SSL: isize = 5;
        const ID_INSEC: isize = 6;
        const ID_USER: isize = 8;
        const ID_PASS: isize = 10;
        const ID_UNC: isize = 12;
        const ID_MAX: isize = 14;
        const ID_IDX: isize = 16;
        const ID_SAVE: isize = 19;
        const ID_TEST: isize = 20;

        let h_dlg = dlg_init(
            &PLUGIN_GUID,
            &SETTINGS_DLG_GUID,
            -1,
            -1,
            W,
            H,
            std::ptr::null(),
            items.as_ptr(),
            items.len(),
            0,
            FDLG_NONE,
            None,
            std::ptr::null_mut(),
        );
        if h_dlg.is_null() || h_dlg == (-1isize as HANDLE) {
            return false;
        }

        let exit_code = dlg_run(h_dlg);
        if exit_code == ID_SAVE || exit_code == ID_TEST {
            draft.nas_host = get_dlg_text(psi, h_dlg, ID_HOST).trim().to_string();
            draft.nas_port = get_dlg_text(psi, h_dlg, ID_PORT)
                .trim()
                .parse()
                .unwrap_or(8080);
            draft.use_ssl = get_dlg_check(psi, h_dlg, ID_SSL);
            draft.accept_invalid_certs = get_dlg_check(psi, h_dlg, ID_INSEC);
            draft.username = get_dlg_text(psi, h_dlg, ID_USER).trim().to_string();
            draft.password = get_dlg_text(psi, h_dlg, ID_PASS);
            draft.unc_host = get_dlg_text(psi, h_dlg, ID_UNC).trim().to_string();
            draft.max_qsirch_results = get_dlg_text(psi, h_dlg, ID_MAX)
                .trim()
                .parse::<usize>()
                .unwrap_or(200)
                .clamp(1, 1000);
            draft.indexd_port = get_dlg_text(psi, h_dlg, ID_IDX)
                .trim()
                .parse::<u16>()
                .unwrap_or(0);
        }
        dlg_free(h_dlg);

        if exit_code == ID_TEST {
            let mut lines: Vec<String> = Vec::new();
            let mut any_err = false;
            if draft.is_indexd_configured() {
                match crate::indexd::test_connection(&draft) {
                    Ok(msg) => lines.push(msg),
                    Err(err) => {
                        any_err = true;
                        lines.push(format!("nas_indexd ERROR: {err}"));
                    }
                }
            }
            if draft.is_qsirch_configured() || !draft.is_indexd_configured() {
                match QsirchClient::from_config(&draft, session_cache_path.clone())
                    .and_then(|mut c| c.test_connection())
                {
                    Ok(msg) => lines.push(format!("Qsirch: {msg}")),
                    Err(err) => {
                        any_err = true;
                        lines.push(format!("Qsirch ERROR: {err}"));
                    }
                }
            }
            let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
            show_message(psi, "NAS Connection Test", &refs, any_err);
            continue;
        }

        if exit_code == ID_SAVE {
            crate::qsirch::clear_offline_cooldown();
            *cfg = draft;
            if let Err(err) = cfg.save(cfg_path) {
                show_message(psi, "Failed to Save Config", &[&err], true);
            }
            return true;
        }

        return false;
    }
}
