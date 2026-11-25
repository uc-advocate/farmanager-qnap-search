//! `nas_indexd` — Ultra-fast in-memory filename & path search daemon for QNAP NAS
//! with real-time filesystem event watching (`inotify` on Linux).
//!
//! Performs one initial scan on startup, then keeps the in-memory index up to date
//! incrementally via filesystem events (create, modify, rename, delete) with zero
//! background disk polling when idle.
//!
//! Environment variables:
//!   INDEX_ROOT     - Root directory to index (default: `/data`)
//!   LISTEN_ADDR    - Bind address (default: `0.0.0.0:9876`)
//!   REFRESH_SECS   - Optional periodic safety full-rescan interval in seconds (default: `0` = off)
//!   API_KEY        - Optional shared secret checked against `X-Api-Key` header
//!   INDEX_EXCLUDES - Comma-separated directory names to skip

use notify::{Config as NotifyConfig, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, RwLock};
use std::time::{Duration, Instant, UNIX_EPOCH};
use walkdir::WalkDir;

struct IndexedEntry {
    /// Normalized NAS path, e.g. `/Multimedia/2026/report.xlsx`.
    rel_path: String,
    /// Pre-lowercased `rel_path` for zero-allocation matching.
    path_lower: String,
    /// Pre-lowercased filename.
    name_lower: String,
    size: u64,
    mtime: i64,
    is_dir: bool,
}

struct LiveIndex {
    /// Keyed by `path_lower` for O(1) incremental upsert/delete on fs events.
    entries: HashMap<String, IndexedEntry>,
    last_scan_ms: u128,
}

struct AppState {
    index: RwLock<LiveIndex>,
    scanning: AtomicBool,
    watcher_status: RwLock<String>,
    watched_dirs: AtomicU64,
    watch_errors: AtomicU64,
    events_processed: AtomicU64,
    root: PathBuf,
    excludes: Vec<String>,
    api_key: String,
}

fn main() {
    let root_str = std::env::var("INDEX_ROOT").unwrap_or_else(|_| "/data".to_string());
    let listen_addr = std::env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:9876".to_string());
    let refresh_secs: u64 = std::env::var("REFRESH_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let api_key = std::env::var("API_KEY").unwrap_or_default();
    let excludes_raw = std::env::var("INDEX_EXCLUDES").unwrap_or_else(|_| {
        ".@__thumb,@Recycle,.streams,.qpkg,lost+found,.Trash-1000,.snapshot,@Recently-Snapshot"
            .to_string()
    });
    let excludes: Vec<String> = excludes_raw
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    let state = Arc::new(AppState {
        index: RwLock::new(LiveIndex {
            entries: HashMap::new(),
            last_scan_ms: 0,
        }),
        scanning: AtomicBool::new(false),
        watcher_status: RwLock::new("starting".to_string()),
        watched_dirs: AtomicU64::new(0),
        watch_errors: AtomicU64::new(0),
        events_processed: AtomicU64::new(0),
        root: PathBuf::from(&root_str),
        excludes,
        api_key,
    });

    // 1. Start real-time filesystem watcher (inotify on Linux) BEFORE initial scan
    //    so changes happening during the initial scan are never missed.
    {
        let st = Arc::clone(&state);
        std::thread::spawn(move || run_watcher(st));
    }

    // 2. Run initial scan (and optional periodic safety rescan if REFRESH_SECS > 0).
    {
        let st = Arc::clone(&state);
        std::thread::spawn(move || {
            run_full_scan(&st);
            if refresh_secs > 0 {
                let interval = Duration::from_secs(refresh_secs.max(60));
                loop {
                    std::thread::sleep(interval);
                    run_full_scan(&st);
                }
            }
        });
    }

    let listener = TcpListener::bind(&listen_addr)
        .unwrap_or_else(|e| panic!("Failed to bind {listen_addr}: {e}"));
    eprintln!(
        "[nas_indexd] Listening on {listen_addr}, watching '{root_str}' (safety rescan: {refresh_secs}s)"
    );

    for stream in listener.incoming().flatten() {
        let st = Arc::clone(&state);
        std::thread::spawn(move || {
            let _ = handle_client(stream, &st);
        });
    }
}

fn is_excluded(state: &AppState, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(&state.root) else {
        return true;
    };
    for comp in rel.components() {
        let name = comp.as_os_str().to_string_lossy().to_lowercase();
        if state.excludes.iter().any(|ex| ex == &name) {
            return true;
        }
    }
    false
}

fn build_entry(state: &AppState, path: &Path) -> Option<IndexedEntry> {
    let rel = path.strip_prefix(&state.root).ok()?;
    let rel_raw = rel.to_string_lossy().replace('\\', "/");
    let rel_path = normalize_qnap_rel_path(&rel_raw);
    if rel_path == "/" {
        return None;
    }
    let meta = std::fs::symlink_metadata(path).ok()?;
    let is_dir = meta.is_dir();
    let size = if is_dir { 0 } else { meta.len() };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let path_lower = rel_path.to_lowercase();
    let name_lower = path
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    Some(IndexedEntry {
        rel_path,
        path_lower,
        name_lower,
        size,
        mtime,
        is_dir,
    })
}

fn run_watcher(state: Arc<AppState>) {
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher = match RecommendedWatcher::new(tx, NotifyConfig::default()) {
        Ok(w) => w,
        Err(e) => {
            let msg = format!("init error: {e}");
            eprintln!("[nas_indexd] Watcher {msg}");
            if let Ok(mut ws) = state.watcher_status.write() {
                *ws = msg;
            }
            return;
        }
    };

    // Register NonRecursive watches only on non-excluded directories so `.@__thumb`,
    // `@Recycle`, `.qpkg`, and `.snapshot` never exhaust `fs.inotify.max_user_watches`.
    let mut watched: u64 = 0;
    let mut errors: u64 = 0;
    let mut last_err = String::new();

    let walker = WalkDir::new(&state.root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            if !e.file_type().is_dir() {
                return false;
            }
            let name = e.file_name().to_string_lossy().to_lowercase();
            !state.excludes.iter().any(|ex| ex == &name)
        });

    for entry in walker.flatten() {
        if !entry.file_type().is_dir() {
            continue;
        }
        match watcher.watch(entry.path(), RecursiveMode::NonRecursive) {
            Ok(()) => watched += 1,
            Err(e) => {
                errors += 1;
                if last_err.is_empty() {
                    last_err = e.to_string();
                }
            }
        }
    }

    state.watched_dirs.store(watched, Ordering::Relaxed);
    state.watch_errors.store(errors, Ordering::Relaxed);

    let status_msg = if errors == 0 {
        format!("active ({watched} dirs)")
    } else {
        format!("partial ({watched} dirs, {errors} errors: {last_err})")
    };
    eprintln!("[nas_indexd] Watcher {status_msg}");
    if errors > 0 {
        eprintln!(
            "[nas_indexd] Hint: if 'No space left on device' (os error 28), increase host limit: sysctl -w fs.inotify.max_user_watches=524288"
        );
    }
    if let Ok(mut ws) = state.watcher_status.write() {
        *ws = status_msg;
    }

    for res in rx {
        match res {
            Ok(event) => apply_fs_event(&state, &mut watcher, event),
            Err(e) => eprintln!("[nas_indexd] Watch event error: {e}"),
        }
    }
}

fn apply_fs_event(state: &AppState, watcher: &mut RecommendedWatcher, event: Event) {
    if matches!(event.kind, EventKind::Access(_)) {
        return;
    }

    state.events_processed.fetch_add(1, Ordering::Relaxed);

    for path in event.paths {
        if is_excluded(state, &path) {
            continue;
        }
        let Ok(rel) = path.strip_prefix(&state.root) else {
            continue;
        };
        let rel_path = normalize_qnap_rel_path(&rel.to_string_lossy().replace('\\', "/"));
        if rel_path == "/" {
            continue;
        }
        let key = rel_path.to_lowercase();

        if path.exists() {
            if let Some(entry) = build_entry(state, &path) {
                let is_dir = entry.is_dir;
                let mut subtree = Vec::new();
                if is_dir {
                    // Watch newly created/moved-in directory and any subdirectories.
                    if watcher.watch(&path, RecursiveMode::NonRecursive).is_ok() {
                        state.watched_dirs.fetch_add(1, Ordering::Relaxed);
                    }
                    for child in WalkDir::new(&path)
                        .follow_links(false)
                        .into_iter()
                        .filter_entry(|e| !is_excluded(state, e.path()))
                        .flatten()
                    {
                        if child.depth() == 0 {
                            continue;
                        }
                        if child.file_type().is_dir()
                            && watcher.watch(child.path(), RecursiveMode::NonRecursive).is_ok()
                        {
                            state.watched_dirs.fetch_add(1, Ordering::Relaxed);
                        }
                        if let Some(ce) = build_entry(state, child.path()) {
                            subtree.push(ce);
                        }
                    }
                }
                if let Ok(mut idx) = state.index.write() {
                    idx.entries.insert(entry.path_lower.clone(), entry);
                    for ce in subtree {
                        idx.entries.insert(ce.path_lower.clone(), ce);
                    }
                }
            }
        } else {
            let dir_prefix = format!("{key}/");
            if let Ok(mut idx) = state.index.write() {
                let removed = idx.entries.remove(&key);
                if removed.is_none_or(|r| r.is_dir) {
                    idx.entries.retain(|k, _| !k.starts_with(&dir_prefix));
                }
            }
        }
    }
}

fn run_full_scan(state: &AppState) {
    if state
        .scanning
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    let started = Instant::now();
    let mut entries = HashMap::with_capacity(65_536);

    let walker = WalkDir::new(&state.root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            let name = e.file_name().to_string_lossy().to_lowercase();
            !state.excludes.iter().any(|ex| ex == &name)
        });

    for entry in walker.flatten() {
        if entry.depth() == 0 {
            continue;
        }
        if let Some(item) = build_entry(state, entry.path()) {
            entries.insert(item.path_lower.clone(), item);
        }
    }

    let elapsed = started.elapsed().as_millis();
    let count = entries.len();
    if let Ok(mut guard) = state.index.write() {
        guard.entries = entries;
        guard.last_scan_ms = elapsed;
    }
    state.scanning.store(false, Ordering::SeqCst);
    eprintln!("[nas_indexd] Full scan indexed {count} items in {elapsed} ms");
}

/// Strip QNAP volume prefixes like `CACHEDEV1_DATA/`, `MD0_DATA/`, `ZFS530_DATA/`, or `share/`.
fn normalize_qnap_rel_path(raw: &str) -> String {
    let parts: Vec<&str> = raw.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return "/".to_string();
    }
    let mut start = 0usize;
    if parts[0].eq_ignore_ascii_case("share") {
        start = 1;
    }
    if start < parts.len() && is_qnap_volume_dir(parts[start]) {
        start += 1;
    }
    if start >= parts.len() {
        "/".to_string()
    } else {
        format!("/{}", parts[start..].join("/"))
    }
}

fn is_qnap_volume_dir(seg: &str) -> bool {
    let u = seg.to_ascii_uppercase();
    u.ends_with("_DATA")
        && (u.starts_with("CACHEDEV")
            || u.starts_with("MD")
            || u.starts_with("ZFS")
            || u.starts_with("HDA"))
}

fn handle_client(mut stream: TcpStream, state: &Arc<AppState>) -> std::io::Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(15)));

    let mut reader = BufReader::new(stream.try_clone()?);
    let mut req_line = String::new();
    if reader.read_line(&mut req_line)? == 0 {
        return Ok(());
    }
    let mut parts = req_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let target = parts.next().unwrap_or("/");

    let mut provided_key = String::new();
    loop {
        let mut hdr = String::new();
        if reader.read_line(&mut hdr)? == 0 {
            break;
        }
        let trimmed = hdr.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            if k.trim().eq_ignore_ascii_case("x-api-key") {
                provided_key = v.trim().to_string();
            }
        }
    }

    if !state.api_key.is_empty() && provided_key != state.api_key {
        return send_json(&mut stream, 401, r#"{"error":"unauthorized"}"#);
    }

    let (path, query_str) = match target.split_once('?') {
        Some((p, q)) => (p, q),
        None => (target, ""),
    };

    match (method, path) {
        ("GET", "/health") => {
            let (indexed, last_scan_ms) = state
                .index
                .read()
                .map(|g| (g.entries.len(), g.last_scan_ms))
                .unwrap_or((0, 0));
            let scanning = state.scanning.load(Ordering::Relaxed);
            let watcher = state
                .watcher_status
                .read()
                .map(|s| s.clone())
                .unwrap_or_else(|_| "unknown".to_string());
            let events = state.events_processed.load(Ordering::Relaxed);
            let watched_dirs = state.watched_dirs.load(Ordering::Relaxed);
            let watch_errors = state.watch_errors.load(Ordering::Relaxed);
            let status = if scanning && indexed == 0 {
                "indexing"
            } else {
                "ready"
            };
            let mut w_esc = String::new();
            push_json_escaped(&mut w_esc, &watcher);
            let body = format!(
                r#"{{"status":"{status}","indexed":{indexed},"scanning":{scanning},"last_scan_ms":{last_scan_ms},"watcher":"{w_esc}","watched_dirs":{watched_dirs},"watch_errors":{watch_errors},"events":{events}}}"#
            );
            send_json(&mut stream, 200, &body)
        }
        ("POST", "/reindex") => {
            let st = Arc::clone(state);
            std::thread::spawn(move || run_full_scan(&st));
            send_json(&mut stream, 202, r#"{"status":"started"}"#)
        }
        ("GET", "/search") => {
            let started = Instant::now();
            let mut mask = "*".to_string();
            let mut prefix = String::new();
            let mut limit: usize = 10_000;
            let mut include_dirs = false;

            for pair in query_str.split('&').filter(|s| !s.is_empty()) {
                if let Some((k, v)) = pair.split_once('=') {
                    let val = url_decode(v);
                    match k {
                        "mask" | "q" => mask = val,
                        "prefix" | "path" => prefix = val,
                        "limit" => {
                            if let Ok(n) = val.parse::<usize>() {
                                limit = n.clamp(1, 100_000);
                            }
                        }
                        "dirs" => include_dirs = val == "1" || val == "true",
                        _ => {}
                    }
                }
            }

            let matcher = FarMaskMatcher::new(&mask);
            let pref_norm = if prefix.trim().is_empty() {
                String::new()
            } else {
                let norm = normalize_qnap_rel_path(&prefix.replace('\\', "/"));
                norm.trim_end_matches('/').to_lowercase()
            };
            let pref_slash = if pref_norm.is_empty() || pref_norm == "/" {
                String::new()
            } else {
                format!("{pref_norm}/")
            };

            let guard = match state.index.read() {
                Ok(g) => g,
                Err(_) => return send_json(&mut stream, 500, r#"{"error":"lock poisoned"}"#),
            };

            let mut total = 0usize;
            let mut out_items = String::with_capacity(4096);
            let mut returned = 0usize;

            for e in guard.entries.values() {
                if e.is_dir && !include_dirs {
                    continue;
                }
                if !pref_slash.is_empty()
                    && e.path_lower != pref_norm
                    && !e.path_lower.starts_with(&pref_slash)
                {
                    continue;
                }
                if !matcher.matches_lower(&e.path_lower, &e.name_lower) {
                    continue;
                }
                total += 1;
                if returned < limit {
                    if returned > 0 {
                        out_items.push(',');
                    }
                    out_items.push_str(r#"{"p":""#);
                    push_json_escaped(&mut out_items, &e.rel_path);
                    out_items.push_str(&format!(
                        r#"","s":{},"m":{},"d":{}}}"#,
                        e.size, e.mtime, e.is_dir
                    ));
                    returned += 1;
                }
            }
            drop(guard);

            let elapsed_ms = started.elapsed().as_millis();
            let body = format!(
                r#"{{"total":{total},"returned":{returned},"elapsed_ms":{elapsed_ms},"items":[{out_items}]}}"#
            );
            send_json(&mut stream, 200, &body)
        }
        _ => send_json(&mut stream, 404, r#"{"error":"not found"}"#),
    }
}

fn send_json(stream: &mut TcpStream, status: u16, body: &str) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Error",
    };
    let resp = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(resp.as_bytes())
}

fn push_json_escaped(out: &mut String, s: &str) {
    for ch in s.chars() {
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
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                    out.push((h << 4) | l);
                    i += 3;
                    continue;
                }
                out.push(b'%');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ─── Zero-Allocation FAR-style Wildcard & Path Mask Matcher ───────────────────

enum FastMatch {
    Any,
    Exact(String),
    Prefix(String),
    Suffix(String),
    Contains(String),
    /// General wildcard (`*` and `?`), with pre-collected pattern chars.
    Wildcard(Vec<char>),
}

impl FastMatch {
    fn compile(pat: &str) -> Self {
        if pat == "*" || pat == "*.*" {
            return Self::Any;
        }
        if !pat.contains(['*', '?']) {
            return Self::Exact(pat.to_string());
        }
        if !pat.contains('?') {
            if let Some(inner) = pat.strip_prefix('*').and_then(|s| s.strip_suffix('*')) {
                if !inner.contains('*') {
                    return if inner.is_empty() {
                        Self::Any
                    } else {
                        Self::Contains(inner.to_string())
                    };
                }
            }
            if let Some(suf) = pat.strip_prefix('*') {
                if !suf.contains('*') {
                    return Self::Suffix(suf.to_string());
                }
            }
            if let Some(pref) = pat.strip_suffix('*') {
                if !pref.contains('*') {
                    return Self::Prefix(pref.to_string());
                }
            }
        }
        Self::Wildcard(pat.chars().collect())
    }

    #[inline]
    fn matches(&self, text: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Exact(s) => text == s,
            Self::Prefix(s) => text.starts_with(s.as_str()),
            Self::Suffix(s) => text.ends_with(s.as_str()),
            Self::Contains(s) => text.contains(s.as_str()),
            Self::Wildcard(p) => wildcard_match_chars(p, text),
        }
    }
}

struct MaskPattern {
    matcher: FastMatch,
    is_path_pattern: bool,
    prefixed_matcher: Option<FastMatch>,
}

impl MaskPattern {
    fn new(raw: &str) -> Self {
        let norm = raw.trim().replace('\\', "/").to_lowercase();
        let is_path_pattern = norm.contains('/');
        let prefixed_matcher = if is_path_pattern
            && !norm.starts_with('*')
            && !norm.starts_with('/')
            && !(norm.len() >= 2 && norm.as_bytes()[1] == b':')
        {
            Some(FastMatch::compile(&format!("*/{norm}")))
        } else {
            None
        };
        Self {
            matcher: FastMatch::compile(&norm),
            is_path_pattern,
            prefixed_matcher,
        }
    }

    #[inline]
    fn matches(&self, path_lower: &str, name_lower: &str) -> bool {
        if self.is_path_pattern {
            if self.matcher.matches(path_lower) {
                return true;
            }
            if let Some(ref pref) = self.prefixed_matcher {
                return pref.matches(path_lower);
            }
            false
        } else {
            self.matcher.matches(name_lower)
        }
    }
}

struct FarMaskMatcher {
    includes: Vec<MaskPattern>,
    excludes: Vec<MaskPattern>,
}

impl FarMaskMatcher {
    fn new(mask: &str) -> Self {
        let mut split = mask.splitn(2, '|');
        let inc_raw = split.next().unwrap_or("*").trim();
        let exc_raw = split.next().unwrap_or("").trim();

        let includes: Vec<MaskPattern> = inc_raw
            .split([',', ';'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(MaskPattern::new)
            .collect();
        let excludes: Vec<MaskPattern> = exc_raw
            .split([',', ';'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(MaskPattern::new)
            .collect();

        Self {
            includes: if includes.is_empty() {
                vec![MaskPattern::new("*")]
            } else {
                includes
            },
            excludes,
        }
    }

    fn matches_lower(&self, path_lower: &str, name_lower: &str) -> bool {
        let inc = self
            .includes
            .iter()
            .any(|p| p.matches(path_lower, name_lower));
        if !inc {
            return false;
        }
        !self
            .excludes
            .iter()
            .any(|p| p.matches(path_lower, name_lower))
    }
}

fn wildcard_match_chars(p: &[char], text: &str) -> bool {
    // Zero-allocation ASCII fast path for general wildcards.
    if text.is_ascii() && p.iter().all(|c| c.is_ascii()) {
        let tb = text.as_bytes();
        let (mut pi, mut ti) = (0usize, 0usize);
        let (mut star_idx, mut match_idx) = (None, 0usize);
        while ti < tb.len() {
            if pi < p.len() && (p[pi] == '?' || p[pi] as u8 == tb[ti]) {
                pi += 1;
                ti += 1;
            } else if pi < p.len() && p[pi] == '*' {
                star_idx = Some(pi);
                match_idx = ti;
                pi += 1;
            } else if let Some(si) = star_idx {
                pi = si + 1;
                match_idx += 1;
                ti = match_idx;
            } else {
                return false;
            }
        }
        while pi < p.len() && p[pi] == '*' {
            pi += 1;
        }
        return pi == p.len();
    }

    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star_idx, mut match_idx) = (None, 0usize);

    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_idx = Some(pi);
            match_idx = ti;
            pi += 1;
        } else if let Some(si) = star_idx {
            pi = si + 1;
            match_idx += 1;
            ti = match_idx;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}
