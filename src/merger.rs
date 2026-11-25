//! Parallel execution of Local filesystem search + QNAP Qsirch 7 search,
//! with live progress reporting, instant ESC cancellation, canonical deduplication,
//! and unified result tagging (`[L]`, `[Q]`, `[L+Q]`).

use crate::config::Config;
use crate::far_ffi::{poll_escape_pressed, show_progress_box, PluginStartupInfo};
use crate::indexd;
use crate::local_search::{resolve_local_roots, search_local, LocalItem, SearchArea};
use crate::path_map::PathMapper;
use crate::qsirch::{QsirchClient, QsirchSearchResponse};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultSource {
    Local,
    Qsirch,
    Both,
}

impl ResultSource {
    pub fn tag(self) -> &'static str {
        match self {
            ResultSource::Local => "[L]  ",
            ResultSource::Qsirch => "[Q]  ",
            ResultSource::Both => "[L+Q]",
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct UnifiedResult {
    pub windows_path: String,
    pub size: u64,
    pub modified_epoch: i64,
    pub is_dir: bool,
    pub attributes: u32,
    pub source: ResultSource,
    pub snippet: String,
}

#[derive(Debug, Clone)]
pub struct SearchParams {
    pub current_dir: String,
    pub file_mask: String,
    pub containing_text: String,
    pub include_local: bool,
    pub include_qsirch: bool,
    pub search_area: SearchArea,
    /// When true and searching from current folder/drive on a NAS path, restrict Qsirch to that NAS folder/share.
    pub scope_qsirch_to_current_folder: bool,
    pub case_sensitive: bool,
    pub advanced_mode: u8,
}

#[derive(Debug)]
pub struct SearchOutcome {
    pub results: Vec<UnifiedResult>,
    pub local_count: usize,
    pub qsirch_count: usize,
    pub qsirch_total_reported: usize,
    pub qsirch_fetched_raw: usize,
    pub qsirch_effective_query: String,
    pub qsirch_exhausted: bool,
    pub merged_both_count: usize,
    pub warnings: Vec<String>,
    pub elapsed_ms: u128,
}

impl SearchOutcome {
    pub fn qsirch_has_more(&self) -> bool {
        !self.qsirch_exhausted
            && !self.qsirch_effective_query.is_empty()
            && self.qsirch_fetched_raw < self.qsirch_total_reported
    }
}

pub fn run_unified_search(
    psi: &PluginStartupInfo,
    cfg: &Config,
    params: &SearchParams,
    session_cache_path: Option<PathBuf>,
) -> SearchOutcome {
    let started = Instant::now();
    let mapper = PathMapper::from_config(cfg);
    let cancel = Arc::new(AtomicBool::new(false));
    let local_scanned = Arc::new(AtomicUsize::new(0));
    let local_found = Arc::new(AtomicUsize::new(0));
    let qsirch_live_status = Arc::new(Mutex::new(format!("querying {}...", cfg.nas_host)));

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

    let run_local = params.include_local && !local_roots.is_empty();
    let run_qsirch = params.include_qsirch;

    let (local_tx, local_rx) = mpsc::channel::<Result<Vec<LocalItem>, String>>();
    let (qsirch_tx, qsirch_rx) = mpsc::channel::<Result<QsirchSearchResponse, String>>();

    if run_local {
        let roots = local_roots.clone();
        let mask = params.file_mask.clone();
        let text = params.containing_text.clone();
        let case_s = params.case_sensitive;
        let max_local = cfg.max_local_results;
        let max_bytes = cfg.local_max_content_mb.saturating_mul(1024 * 1024);
        let cancel_cl = Arc::clone(&cancel);
        let scanned_cl = Arc::clone(&local_scanned);
        let found_cl = Arc::clone(&local_found);
        std::thread::spawn(move || {
            let res = search_local(
                &roots,
                &mask,
                &text,
                case_s,
                max_local,
                max_bytes,
                &cancel_cl,
                &scanned_cl,
                &found_cl,
            );
            let _ = local_tx.send(res);
        });
    }

    if run_qsirch {
        let cfg_clone = cfg.clone();
        let mask = params.file_mask.clone();
        let text = params.containing_text.clone();
        let scope = nas_scope.clone();
        let adv_mode = params.advanced_mode;
        let cancel_cl = Arc::clone(&cancel);
        let status_cl = Arc::clone(&qsirch_live_status);
        std::thread::spawn(move || {
            // Fast path: when searching by filename/path mask only (no containing_text)
            // and `nas_indexd` is configured, query `nas_indexd` first (~5-20 ms for 10,000+ hits),
            // falling back to Qsirch 7 automatically if `nas_indexd` is unreachable.
            let res = if text.trim().is_empty() && cfg_clone.is_indexd_configured() {
                if let Ok(mut s) = status_cl.lock() {
                    *s = format!("nas_indexd ({}:{})...", cfg_clone.nas_host, cfg_clone.indexd_port);
                }
                match indexd::search(&cfg_clone, &mask, scope.as_deref()) {
                    Ok(resp) => Ok(resp),
                    Err(indexd_err) => {
                        if cfg_clone.is_qsirch_configured() {
                            QsirchClient::from_config(&cfg_clone, session_cache_path).and_then(
                                |mut client| {
                                    client.search(
                                        &mask,
                                        &text,
                                        scope.as_deref(),
                                        cfg_clone.max_qsirch_results,
                                        adv_mode,
                                        &cancel_cl,
                                        &status_cl,
                                    )
                                },
                            )
                        } else {
                            Err(indexd_err)
                        }
                    }
                }
            } else if !cfg_clone.is_qsirch_configured() {
                Err("Qsirch/nas_indexd not configured (open Settings to set NAS host)".into())
            } else {
                QsirchClient::from_config(&cfg_clone, session_cache_path).and_then(|mut client| {
                    client.search(
                        &mask,
                        &text,
                        scope.as_deref(),
                        cfg_clone.max_qsirch_results,
                        adv_mode,
                        &cancel_cl,
                        &status_cl,
                    )
                })
            };
            let _ = qsirch_tx.send(res);
        });
    }

    // Save FAR screen and show a live non-blocking progress box with instant ESC cancellation.
    let h_screen = unsafe { psi.SaveScreen.map(|f| f(0, 0, -1, -1)) };

    let mut local_res: Option<Result<Vec<LocalItem>, String>> = None;
    let mut qsirch_res: Option<Result<QsirchSearchResponse, String>> = None;
    let mut cancelled = false;
    let mut last_draw = Instant::now() - Duration::from_secs(1);

    loop {
        if run_local && local_res.is_none() {
            if let Ok(r) = local_rx.try_recv() {
                local_res = Some(r);
            }
        }
        if run_qsirch && qsirch_res.is_none() {
            if let Ok(r) = qsirch_rx.try_recv() {
                qsirch_res = Some(r);
            }
        }

        let local_done = !run_local || local_res.is_some();
        let qsirch_done = !run_qsirch || qsirch_res.is_some();
        if local_done && qsirch_done {
            break;
        }

        if unsafe { poll_escape_pressed() } {
            cancel.store(true, Ordering::Relaxed);
            cancelled = true;
            // Give local thread a brief moment to send its partial results.
            if run_local && local_res.is_none() {
                if let Ok(r) = local_rx.recv_timeout(Duration::from_millis(250)) {
                    local_res = Some(r);
                }
            }
            if run_qsirch && qsirch_res.is_none() {
                if let Ok(r) = qsirch_rx.try_recv() {
                    qsirch_res = Some(r);
                }
            }
            break;
        }

        if last_draw.elapsed() >= Duration::from_millis(80) {
            let secs = started.elapsed().as_secs_f64();
            let local_line = if !run_local {
                "Local:  disabled".to_string()
            } else if local_res.is_some() {
                format!(
                    "Local:  done ({} scanned, {} found)",
                    local_scanned.load(Ordering::Relaxed),
                    local_found.load(Ordering::Relaxed)
                )
            } else {
                format!(
                    "Local:  scanning... {} files checked, {} matched",
                    local_scanned.load(Ordering::Relaxed),
                    local_found.load(Ordering::Relaxed)
                )
            };
            let qsirch_line = if !run_qsirch {
                "Qsirch: disabled".to_string()
            } else if let Some(ref r) = qsirch_res {
                match r {
                    Ok(resp) => format!("Qsirch: done ({} matched / {} total)", resp.items.len(), resp.total),
                    Err(e) => format!("Qsirch: error ({})", e.chars().take(48).collect::<String>()),
                }
            } else {
                let st = qsirch_live_status
                    .lock()
                    .map(|s| s.clone())
                    .unwrap_or_else(|_| format!("querying {}...", cfg.nas_host));
                format!("Qsirch: {st}")
            };

            let lines = vec![
                format!("{:<64}", format!("Mask:    {}", params.file_mask)),
                format!("{:<64}", local_line),
                format!("{:<64}", qsirch_line),
                format!("{:<64}", format!("Elapsed: {:.1} s   (Press ESC to stop)", secs)),
            ];
            unsafe {
                show_progress_box(psi, " Unified Search ", &lines);
            }
            last_draw = Instant::now();
        }

        std::thread::sleep(Duration::from_millis(25));
    }

    if let (Some(restore), Some(hs)) = (psi.RestoreScreen, h_screen) {
        if !hs.is_null() {
            unsafe { restore(hs) };
        }
    }

    let mut warnings = Vec::new();
    if cancelled {
        warnings.push("Cancelled by user (showing partial results)".into());
    }

    let mut results: Vec<UnifiedResult> = Vec::new();
    let mut index_by_key: HashMap<String, usize> = HashMap::new();
    let mut local_count = 0usize;
    let mut qsirch_count = 0usize;
    let mut qsirch_total_reported = 0usize;
    let mut qsirch_fetched_raw = 0usize;
    let mut qsirch_effective_query = String::new();
    let mut merged_both_count = 0usize;

    if let Some(res) = local_res {
        match res {
            Ok(items) => {
                local_count = items.len();
                for item in items {
                    let key = mapper.dedup_key(&item.path);
                    let idx = results.len();
                    index_by_key.insert(key, idx);
                    results.push(UnifiedResult {
                        windows_path: item.path,
                        size: item.size,
                        modified_epoch: item.modified_epoch,
                        is_dir: item.is_dir,
                        attributes: item.attributes,
                        source: ResultSource::Local,
                        snippet: String::new(),
                    });
                }
            }
            Err(e) => warnings.push(format!("Local: {e}")),
        }
    }

    if let Some(res) = qsirch_res {
        match res {
            Ok(resp) => {
                qsirch_total_reported = resp.total;
                qsirch_fetched_raw = resp.fetched_raw;
                qsirch_effective_query = resp.effective_query;
                qsirch_count = resp.items.len();
                for item in resp.items {
                    let win_path = mapper.nas_to_windows(&item.nas_path);
                    let key = mapper.dedup_key(&win_path);
                    if let Some(&existing_idx) = index_by_key.get(&key) {
                        let existing = &mut results[existing_idx];
                        if existing.source == ResultSource::Local {
                            existing.source = ResultSource::Both;
                            merged_both_count += 1;
                        }
                        if existing.snippet.is_empty() && !item.snippet.is_empty() {
                            existing.snippet = item.snippet;
                        }
                    } else {
                        let idx = results.len();
                        index_by_key.insert(key, idx);
                        results.push(UnifiedResult {
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
            }
            Err(e) => warnings.push(format!("Qsirch: {e}")),
        }
    }

    SearchOutcome {
        results,
        local_count,
        qsirch_count,
        qsirch_total_reported,
        qsirch_fetched_raw,
        qsirch_effective_query,
        qsirch_exhausted: qsirch_fetched_raw >= qsirch_total_reported,
        merged_both_count,
        warnings,
        elapsed_ms: started.elapsed().as_millis(),
    }
}
