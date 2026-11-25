//! QNAP Qsirch 7.x REST API client using native Windows WinHTTP (`winhttp.dll`),
//! with QTS `authLogin.cgi` session caching, automatic 401 re-authentication,
//! and 503 "Qsirch service loading" backoff.

use crate::config::Config;
use crate::far_ffi::WideString;
use crate::local_search::FarMaskMatcher;
use crate::mini_json::{parse_json, JsonValue};
use base64::Engine;
use std::collections::HashMap;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::Networking::WinHttp::*;

pub const MAX_RESULT_LIMIT: usize = 1000;
const SERVICE_LOADING_MSG: &str = "Qsirch service loading";
const LOADING_BACKOFF_SECS: &[u64] = &[2, 3, 5, 8];
const CONNECT_TIMEOUT_MS: i32 = 2_500;
const SEND_TIMEOUT_MS: i32 = 5_000;
const OFFLINE_COOLDOWN_SECS: u64 = 60;

static OFFLINE_UNTIL: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
static PATH_OP_UNSUPPORTED: AtomicBool = AtomicBool::new(false);

fn offline_map() -> &'static Mutex<HashMap<String, Instant>> {
    OFFLINE_UNTIL.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Clear any cached "NAS offline" cooldown (e.g. after updating Settings or testing connection).
pub fn clear_offline_cooldown() {
    if let Ok(mut map) = offline_map().lock() {
        map.clear();
    }
}

fn check_offline_cooldown(endpoint_key: &str) -> Option<u64> {
    let mut map = offline_map().lock().ok()?;
    if let Some(&until) = map.get(endpoint_key) {
        let now = Instant::now();
        if until > now {
            return Some((until - now).as_secs().max(1));
        }
        map.remove(endpoint_key);
    }
    None
}

fn mark_offline(endpoint_key: &str) {
    if let Ok(mut map) = offline_map().lock() {
        map.insert(
            endpoint_key.to_string(),
            Instant::now() + Duration::from_secs(OFFLINE_COOLDOWN_SECS),
        );
    }
}

fn mark_online(endpoint_key: &str) {
    if let Ok(mut map) = offline_map().lock() {
        map.remove(endpoint_key);
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct QsirchItem {
    pub id: String,
    pub name: String,
    pub extension: String,
    pub nas_path: String,
    pub size: u64,
    pub modified_epoch: i64,
    pub snippet: String,
}

#[derive(Debug, Clone)]
pub struct QsirchSearchResponse {
    pub total: usize,
    pub fetched_raw: usize,
    pub effective_query: String,
    pub items: Vec<QsirchItem>,
}

pub struct QsirchClient {
    host: String,
    port: u16,
    use_ssl: bool,
    accept_invalid_certs: bool,
    timeout_ms: i32,
    username: String,
    password: String,
    sid: Option<String>,
    cache_path: Option<PathBuf>,
    // Drop order: close child connection handle before parent session handle.
    h_connect: Option<WinHttpHandle>,
    h_session: Option<WinHttpHandle>,
}

impl QsirchClient {
    pub fn from_config(cfg: &Config, cache_path: Option<PathBuf>) -> Result<Self, String> {
        let host = cfg.nas_host.trim().to_string();
        if host.is_empty() {
            return Err("NAS host is not configured".into());
        }
        let username = cfg.effective_username();
        if username.is_empty() {
            return Err("QNAP username is not configured".into());
        }
        let password = cfg.effective_password();
        let timeout_ms = (cfg.qsirch_timeout_secs.max(3) * 1000).min(i32::MAX as u64) as i32;

        let mut client = Self {
            host,
            port: cfg.nas_port,
            use_ssl: cfg.use_ssl,
            accept_invalid_certs: cfg.accept_invalid_certs,
            timeout_ms,
            username,
            password,
            sid: None,
            cache_path,
            h_session: None,
            h_connect: None,
        };
        client.load_cached_sid();
        Ok(client)
    }

    fn ensure_connection(&mut self) -> Result<*mut c_void, String> {
        if let Some(ref conn) = self.h_connect {
            return Ok(conn.0);
        }
        unsafe {
            let ua = WideString::new("FarQsirch/0.1");
            // Use NO_PROXY to avoid 2-5s Windows WPAD proxy auto-discovery delays on LAN NAS hosts.
            let h_session = WinHttpOpen(
                ua.as_ptr(),
                WINHTTP_ACCESS_TYPE_NO_PROXY,
                std::ptr::null(),
                std::ptr::null(),
                0,
            );
            if h_session.is_null() {
                return Err("WinHttpOpen failed".into());
            }
            WinHttpSetTimeouts(
                h_session,
                CONNECT_TIMEOUT_MS,
                CONNECT_TIMEOUT_MS,
                SEND_TIMEOUT_MS,
                self.timeout_ms,
            );
            let endpoint_key = format!("{}:{}", self.host, self.port);
            let w_host = WideString::new(&self.host);
            let h_connect = WinHttpConnect(h_session, w_host.as_ptr(), self.port, 0);
            if h_connect.is_null() {
                WinHttpCloseHandle(h_session);
                mark_offline(&endpoint_key);
                return Err(format!("Cannot connect to {}", self.base_url_display()));
            }
            self.h_session = Some(WinHttpHandle(h_session));
            self.h_connect = Some(WinHttpHandle(h_connect));
            Ok(h_connect)
        }
    }

    fn base_url_display(&self) -> String {
        let scheme = if self.use_ssl { "https" } else { "http" };
        format!("{scheme}://{}:{}", self.host, self.port)
    }

    fn cache_key(&self) -> String {
        format!("{}:{}:{}", self.host, self.port, self.username)
    }

    fn load_cached_sid(&mut self) {
        let Some(path) = &self.cache_path else {
            return;
        };
        let key = self.cache_key();
        if let Ok(text) = std::fs::read_to_string(path) {
            for line in text.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    if k == key && !v.trim().is_empty() {
                        self.sid = Some(v.trim().to_string());
                        return;
                    }
                }
            }
        }
    }

    fn save_cached_sid(&self, sid: &str) {
        let Some(path) = &self.cache_path else {
            return;
        };
        let key = self.cache_key();
        let mut lines: Vec<String> = std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.starts_with(&format!("{key}=")))
            .map(String::from)
            .collect();
        lines.push(format!("{key}={sid}"));
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, lines.join("\n"));
    }

    fn drop_cached_sid(&self) {
        let Some(path) = &self.cache_path else {
            return;
        };
        let key = self.cache_key();
        if let Ok(text) = std::fs::read_to_string(path) {
            let lines: Vec<&str> = text
                .lines()
                .filter(|l| !l.starts_with(&format!("{key}=")))
                .collect();
            let _ = std::fs::write(path, lines.join("\n"));
        }
    }

    /// Authenticate via QTS `/cgi-bin/authLogin.cgi` (Base64 password) to obtain `authSid`.
    pub fn login(&mut self) -> Result<String, String> {
        if self.password.is_empty() {
            return Err("QNAP password is empty (set in Settings or QSIRCH_PASS env var)".into());
        }
        let b64_pwd = base64::engine::general_purpose::STANDARD.encode(self.password.as_bytes());
        let form_body = format!(
            "user={}&pwd={}",
            url_encode(&self.username),
            url_encode(&b64_pwd)
        );
        let headers = "Content-Type: application/x-www-form-urlencoded\r\n";

        let (status, body) = self.winhttp_request(
            "POST",
            "/cgi-bin/authLogin.cgi",
            Some(headers),
            Some(form_body.as_bytes()),
        )?;

        if status != 200 {
            return Err(format!("authLogin.cgi returned HTTP {status}"));
        }

        let passed = extract_xml_tag(&body, "authPassed").unwrap_or_default();
        if passed == "1" {
            if let Some(sid) = extract_xml_tag(&body, "authSid") {
                if !sid.is_empty() {
                    self.sid = Some(sid.clone());
                    self.save_cached_sid(&sid);
                    return Ok(sid);
                }
            }
        }
        let err_val = extract_xml_tag(&body, "errorValue").unwrap_or_else(|| "unknown".into());
        Err(format!(
            "QTS authentication failed for '{}' (errorValue={})",
            self.username, err_val
        ))
    }

    fn ensure_sid(&mut self) -> Result<String, String> {
        if let Some(sid) = &self.sid {
            return Ok(sid.clone());
        }
        self.login()
    }

    fn get_json(&mut self, path: &str, query: &[(&str, String)]) -> Result<JsonValue, String> {
        let mut sid = self.ensure_sid()?;
        let full_path = if query.is_empty() {
            path.to_string()
        } else {
            let qs: Vec<String> = query
                .iter()
                .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
                .collect();
            format!("{path}?{}", qs.join("&"))
        };

        let mut reauthed = false;
        for attempt in 0..=LOADING_BACKOFF_SECS.len() {
            let cookie_hdr = format!("Cookie: NAS_SID={sid}\r\nAccept: application/json\r\n");
            let (status, body) =
                self.winhttp_request("GET", &full_path, Some(&cookie_hdr), None)?;

            if (status == 401 || status == 403) && !reauthed {
                reauthed = true;
                self.sid = None;
                self.drop_cached_sid();
                sid = self.login()?;
                continue;
            }

            if status == 503 && body.contains(SERVICE_LOADING_MSG) {
                if let Some(&wait) = LOADING_BACKOFF_SECS.get(attempt) {
                    std::thread::sleep(Duration::from_secs(wait));
                    continue;
                }
            }

            if status != 200 {
                let preview: String = body.chars().take(180).collect();
                return Err(format!("Qsirch HTTP {status}: {preview}"));
            }

            return parse_json(&body).map_err(|e| format!("Invalid JSON from Qsirch: {e}"));
        }

        Err("Qsirch service remained busy (503) after retries".into())
    }

    fn winhttp_request(
        &mut self,
        method: &str,
        path_and_query: &str,
        extra_headers: Option<&str>,
        body: Option<&[u8]>,
    ) -> Result<(u16, String), String> {
        let h_connect = self.ensure_connection()?;
        let endpoint_key = format!("{}:{}", self.host, self.port);
        unsafe {
            let w_method = WideString::new(method);
            let w_path = WideString::new(path_and_query);
            let req_flags = if self.use_ssl {
                WINHTTP_FLAG_SECURE
            } else {
                0
            };

            let h_request = WinHttpOpenRequest(
                h_connect,
                w_method.as_ptr(),
                w_path.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                req_flags,
            );
            if h_request.is_null() {
                return Err("WinHttpOpenRequest failed".into());
            }
            let _g_req = WinHttpHandle(h_request);

            if self.use_ssl && self.accept_invalid_certs {
                let sec_flags: u32 = SECURITY_FLAG_IGNORE_UNKNOWN_CA
                    | SECURITY_FLAG_IGNORE_CERT_WRONG_USAGE
                    | SECURITY_FLAG_IGNORE_CERT_CN_INVALID
                    | SECURITY_FLAG_IGNORE_CERT_DATE_INVALID;
                WinHttpSetOption(
                    h_request,
                    WINHTTP_OPTION_SECURITY_FLAGS,
                    (&sec_flags as *const u32).cast::<c_void>(),
                    std::mem::size_of::<u32>() as u32,
                );
            }

            let w_headers = extra_headers.map(WideString::new);
            let (hdr_ptr, hdr_len) = match &w_headers {
                Some(w) => (w.as_ptr(), u32::MAX),
                None => (std::ptr::null(), 0),
            };
            let (body_ptr, body_len) = match body {
                Some(b) => (b.as_ptr() as *mut c_void, b.len() as u32),
                None => (std::ptr::null_mut(), 0),
            };

            if WinHttpSendRequest(
                h_request,
                hdr_ptr,
                hdr_len,
                body_ptr,
                body_len,
                body_len,
                0,
            ) == 0
            {
                mark_offline(&endpoint_key);
                let err = windows_sys::Win32::Foundation::GetLastError();
                return Err(format!(
                    "Cannot reach {} (WinHTTP error {err})",
                    self.base_url_display()
                ));
            }

            if WinHttpReceiveResponse(h_request, std::ptr::null_mut()) == 0 {
                let err = windows_sys::Win32::Foundation::GetLastError();
                return Err(format!(
                    "No response from {} (WinHTTP error {err})",
                    self.base_url_display()
                ));
            }
            mark_online(&endpoint_key);

            let mut status_code: u32 = 0;
            let mut status_size: u32 = std::mem::size_of::<u32>() as u32;
            WinHttpQueryHeaders(
                h_request,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                std::ptr::null(),
                (&mut status_code as *mut u32).cast::<c_void>(),
                &mut status_size,
                std::ptr::null_mut(),
            );

            let mut bytes: Vec<u8> = Vec::new();
            loop {
                let mut avail: u32 = 0;
                if WinHttpQueryDataAvailable(h_request, &mut avail) == 0 || avail == 0 {
                    break;
                }
                let start = bytes.len();
                bytes.resize(start + avail as usize, 0);
                let mut read: u32 = 0;
                if WinHttpReadData(
                    h_request,
                    bytes[start..].as_mut_ptr().cast::<c_void>(),
                    avail,
                    &mut read,
                ) == 0
                {
                    break;
                }
                bytes.truncate(start + read as usize);
                if read == 0 {
                    break;
                }
            }

            Ok((status_code as u16, String::from_utf8_lossy(&bytes).into_owned()))
        }
    }

    /// Quick health check against `/qsirch/latest/api/status/brief`.
    pub fn test_connection(&mut self) -> Result<String, String> {
        clear_offline_cooldown();
        let v = self.get_json("/qsirch/latest/api/status/brief", &[])?;
        let state = v
            .get("status")
            .or_else(|| v.get("state"))
            .and_then(|s| s.as_str())
            .unwrap_or("OK");
        Ok(format!(
            "Connected to {} (status: {state})",
            self.base_url_display()
        ))
    }

    /// Query Qsirch 7 `/qsirch/latest/api/search`, translating FAR mask/text and applying
    /// client-side mask + NAS path filtering (required because Qsirch 7 ignores GET ext/path params).
    pub fn search(
        &mut self,
        file_mask: &str,
        containing_text: &str,
        nas_path_prefix: Option<&str>,
        limit: usize,
        advanced_mode: u8,
        cancel: &AtomicBool,
        status_out: &Mutex<String>,
    ) -> Result<QsirchSearchResponse, String> {
        let endpoint_key = format!("{}:{}", self.host, self.port);
        if let Some(secs_left) = check_offline_cooldown(&endpoint_key) {
            return Err(format!(
                "NAS unreachable (skipping for {secs_left}s; run Settings -> Test to retry now)"
            ));
        }
        let base_q = build_qsirch_query(file_mask, containing_text);
        let nas_scope_norm = if PATH_OP_UNSUPPORTED.load(Ordering::Relaxed) {
            None
        } else {
            nas_path_prefix
                .map(crate::path_map::PathMapper::normalize_nas_path)
                .filter(|p| !p.is_empty() && p != "/")
        };

        // Push the current NAS folder down to Qsirch server-side via `path:"/Share/Sub"`
        // so Qsirch narrows inside its index instead of scanning unrelated shares.
        let used_path_op = nas_scope_norm.is_some();
        let primary_q = match &nas_scope_norm {
            Some(scope) => {
                let escaped = scope.replace('"', "");
                if base_q == "." {
                    format!("path:\"{escaped}\"")
                } else {
                    format!("{base_q} path:\"{escaped}\"")
                }
            }
            None => base_q.clone(),
        };
        let fallback_q = if containing_text.trim().is_empty() {
            ".".to_string()
        } else {
            containing_text.trim().to_string()
        };
        let want_highlight = !containing_text.trim().is_empty();
        let has_server_narrow = primary_q.contains("category:") || primary_q.contains("path:\"");

        let res = self.search_with_query(
            &primary_q,
            file_mask,
            nas_path_prefix,
            limit,
            advanced_mode,
            want_highlight,
            has_server_narrow,
            false,
            cancel,
            status_out,
        );

        // If `path:"..."` or a keyword/category query was rejected or returned 0 hits,
        // retry once with `base_q` (if different) or a fast single-page fallback (`.` / text).
        if !cancel.load(Ordering::Relaxed) && primary_q != fallback_q {
            let should_retry = match &res {
                Err(_) => true,
                Ok(r) => r.total == 0,
            };
            if should_retry {
                let retry_q = if base_q != primary_q && base_q != fallback_q {
                    &base_q
                } else {
                    &fallback_q
                };
                if let Ok(fallback_res) = self.search_with_query(
                    retry_q,
                    file_mask,
                    nas_path_prefix,
                    limit,
                    advanced_mode,
                    want_highlight,
                    false,
                    retry_q == ".",
                    cancel,
                    status_out,
                ) {
                    if used_path_op && fallback_res.total > 0 {
                        // This Qsirch build did not match `path:"..."`; skip it on future searches.
                        PATH_OP_UNSUPPORTED.store(true, Ordering::Relaxed);
                    }
                    if fallback_res.total > 0 || retry_q == &fallback_q {
                        return Ok(fallback_res);
                    }
                }
                if retry_q != &fallback_q && !cancel.load(Ordering::Relaxed) {
                    if let Ok(final_res) = self.search_with_query(
                        &fallback_q,
                        file_mask,
                        nas_path_prefix,
                        limit,
                        advanced_mode,
                        want_highlight,
                        false,
                        fallback_q == ".",
                        cancel,
                        status_out,
                    ) {
                        return Ok(final_res);
                    }
                }
            }
        }

        res
    }

    #[allow(clippy::too_many_arguments)]
    fn search_with_query(
        &mut self,
        q: &str,
        file_mask: &str,
        nas_path_prefix: Option<&str>,
        limit: usize,
        advanced_mode: u8,
        want_highlight: bool,
        has_server_category: bool,
        is_wildcard_fallback: bool,
        cancel: &AtomicBool,
        status_out: &Mutex<String>,
    ) -> Result<QsirchSearchResponse, String> {
        let mask_matcher = FarMaskMatcher::new(file_mask);
        let effective_limit = limit.clamp(1, MAX_RESULT_LIMIT);

        let has_path_mask = file_mask.contains(['/', '\\']);
        let has_unpushed_nas_scope =
            nas_path_prefix.is_some_and(|p| !p.is_empty()) && !q.contains("path:\"");
        let needs_extra_window = !is_wildcard_fallback
            && (has_path_mask
                || has_unpushed_nas_scope
                || (!mask_matcher.is_match_all() && !has_server_category));

        // Respect the user's configured `Max Qsirch results` directly as the page size
        // (capped by Qsirch's 1000-item ceiling; wildcard fallback stays lean at <= 100).
        let page_size = if is_wildcard_fallback {
            effective_limit.min(100)
        } else {
            effective_limit
        };
        let max_pages = if needs_extra_window { 2 } else { 1 };

        let prefix_lower = nas_path_prefix.map(|p| {
            let norm = crate::path_map::PathMapper::normalize_nas_path(p);
            norm.trim_end_matches('/').to_lowercase()
        });

        let mut items = Vec::with_capacity(effective_limit.min(256));
        let mut offset = 0usize;
        let mut total = 0usize;

        for page in 0..max_pages {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            if let Ok(mut s) = status_out.lock() {
                let q_short: String = q.chars().take(36).collect();
                *s = format!("p{}/{max_pages} q=\"{q_short}\"", page + 1);
            }
            let mut params = vec![
                ("q", q.to_string()),
                ("limit", page_size.to_string()),
                ("offset", offset.to_string()),
                ("advanced_mode", advanced_mode.min(2).to_string()),
                ("store_history", "0".to_string()),
            ];
            // Only request content snippets when actually searching for text inside files;
            // highlight=content on filename-only searches is very slow on large NAS indexes.
            if want_highlight {
                params.push(("highlight", "content".to_string()));
            }

            let data = self.get_json("/qsirch/latest/api/search", &params)?;
            total = data
                .get("total")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| "Qsirch returned empty result set (no 'total' field)".to_string())?
                as usize;

            let raw_items = data
                .get("items")
                .and_then(|v| v.as_array())
                .unwrap_or(&[]);
            if raw_items.is_empty() {
                break;
            }
            offset += raw_items.len();

            for raw in raw_items {
                let item = parse_qsirch_item(raw);
                if item.nas_path.is_empty() {
                    continue;
                }

                let filename = Path::new(&item.nas_path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&item.name);
                if !mask_matcher.matches_path(&item.nas_path, filename) {
                    continue;
                }

                if let Some(ref pref) = prefix_lower {
                    if !pref.is_empty() && pref != "/" {
                        let norm_item =
                            crate::path_map::PathMapper::normalize_nas_path(&item.nas_path)
                                .to_lowercase();
                        if norm_item != *pref && !norm_item.starts_with(&format!("{pref}/")) {
                            continue;
                        }
                    }
                }

                items.push(item);
                if items.len() >= effective_limit {
                    break;
                }
            }

            // Stop after page 1 as soon as we have matches (Qsirch sorts by relevance),
            // so we don't waste a second round-trip when page 1 already found the files.
            if !items.is_empty() || items.len() >= effective_limit || offset >= total {
                break;
            }
        }

        Ok(QsirchSearchResponse {
            total,
            fetched_raw: offset,
            effective_query: q.to_string(),
            items,
        })
    }

    /// Fetch a single page of Qsirch results for live background streaming.
    /// Returns `(total_reported, raw_items_fetched, exact_matched_items)`.
    #[allow(clippy::too_many_arguments)]
    pub fn fetch_one_page(
        &mut self,
        q: &str,
        offset: usize,
        page_size: usize,
        file_mask: &str,
        nas_path_prefix: Option<&str>,
        advanced_mode: u8,
        want_highlight: bool,
    ) -> Result<(usize, usize, Vec<QsirchItem>), String> {
        let mask_matcher = FarMaskMatcher::new(file_mask);
        let prefix_lower = nas_path_prefix.map(|p| {
            let norm = crate::path_map::PathMapper::normalize_nas_path(p);
            norm.trim_end_matches('/').to_lowercase()
        });

        let mut params = vec![
            ("q", q.to_string()),
            ("limit", page_size.clamp(1, MAX_RESULT_LIMIT).to_string()),
            ("offset", offset.to_string()),
            ("advanced_mode", advanced_mode.min(2).to_string()),
            ("store_history", "0".to_string()),
        ];
        if want_highlight {
            params.push(("highlight", "content".to_string()));
        }

        let data = self.get_json("/qsirch/latest/api/search", &params)?;
        let total = data.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let raw_items = data
            .get("items")
            .and_then(|v| v.as_array())
            .unwrap_or(&[]);

        let mut matched = Vec::new();
        for raw in raw_items {
            let item = parse_qsirch_item(raw);
            if item.nas_path.is_empty() {
                continue;
            }
            let filename = Path::new(&item.nas_path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&item.name);
            if !mask_matcher.matches_path(&item.nas_path, filename) {
                continue;
            }
            if let Some(ref pref) = prefix_lower {
                if !pref.is_empty() && pref != "/" {
                    let norm_item =
                        crate::path_map::PathMapper::normalize_nas_path(&item.nas_path)
                            .to_lowercase();
                    if norm_item != *pref && !norm_item.starts_with(&format!("{pref}/")) {
                        continue;
                    }
                }
            }
            matched.push(item);
        }

        Ok((total, raw_items.len(), matched))
    }
}

struct WinHttpHandle(*mut c_void);
impl Drop for WinHttpHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                WinHttpCloseHandle(self.0);
            }
        }
    }
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Translate FAR `file_mask` and `containing_text` into a Qsirch 7 `q=` query string.
///
/// Verified against Qsirch 7 REST API:
/// - `'.'` matches all indexed files (`'*'` returns 0).
/// - `category:<Name>` (without quotes) filters by Qsirch category (`Excel`, `PDF`, `Word`,
///   `Email`, `Documents`, `Images`, `Music`, `Videos`).
/// - Exact filename/path wildcard matching (`*.xsl`, `*xlsx*`, `*/part1/part2/*.xsl`) is
///   enforced client-side on the returned items.
pub fn build_qsirch_query(file_mask: &str, containing_text: &str) -> String {
    let text = containing_text.trim();
    let category = infer_qsirch_category(file_mask);

    if !text.is_empty() {
        return match category {
            Some(cat) => format!("{text} category:{cat}"),
            None => text.to_string(),
        };
    }

    if let Some(cat) = category {
        return format!(". category:{cat}");
    }

    // Extract meaningful tokens from the filename stem, or from path folder segments
    // (e.g. `*2026\Předpisy\*` -> `"Předpisy" "2026"`) so Qsirch uses its inverted index
    // instead of a slow full-NAS `.` wildcard scan.
    let include_part = file_mask.split('|').next().unwrap_or(file_mask);
    let mut stem_terms: Vec<String> = Vec::new();
    for raw_part in include_part.split([',', ';']) {
        let norm = raw_part.trim().replace('\\', "/");
        let leaf = norm.rsplit('/').next().unwrap_or(&norm).trim();
        let stem = match leaf.rsplit_once('.') {
            Some((s, _)) => s,
            None => leaf,
        };
        let mut found_in_leaf = false;
        for tok in stem.split(['*', '?']).map(str::trim).filter(|s| s.len() >= 2) {
            found_in_leaf = true;
            if !stem_terms.iter().any(|t| t.eq_ignore_ascii_case(tok)) {
                stem_terms.push(tok.to_string());
            }
        }
        if !found_in_leaf && norm.contains('/') {
            // Pull non-wildcard folder segments (deepest first, up to 2 tokens)
            for seg in norm.split('/').rev().skip(1) {
                for tok in seg.split(['*', '?']).map(str::trim).filter(|s| s.len() >= 2) {
                    if !stem_terms.iter().any(|t| t.eq_ignore_ascii_case(tok)) {
                        stem_terms.push(tok.to_string());
                    }
                    if stem_terms.len() >= 2 {
                        break;
                    }
                }
                if stem_terms.len() >= 2 {
                    break;
                }
            }
        }
    }

    if stem_terms.is_empty() {
        ".".to_string()
    } else if stem_terms.len() == 1 {
        stem_terms.remove(0)
    } else {
        stem_terms.join(" ")
    }
}

/// If all include masks map to the same verified Qsirch 7 category, return that category name.
fn infer_qsirch_category(file_mask: &str) -> Option<&'static str> {
    let include_part = file_mask.split('|').next().unwrap_or(file_mask);
    let mut resolved: Option<&'static str> = None;

    for raw_part in include_part.split([',', ';']) {
        let norm = raw_part.trim().replace('\\', "/");
        let leaf = norm.rsplit('/').next().unwrap_or(&norm).trim();
        let token = if let Some((_, ext)) = leaf.rsplit_once('.') {
            ext.trim().trim_matches(['*', '?']).to_ascii_lowercase()
        } else {
            leaf.trim_matches(['*', '?']).to_ascii_lowercase()
        };
        let cat = match token.as_str() {
            "xls" | "xlsx" | "xlsm" | "xlsb" | "ods" => "Excel",
            "pdf" => "PDF",
            "doc" | "docx" | "rtf" | "odt" => "Word",
            "eml" | "msg" => "Email",
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "tif" | "tiff" | "heic" => "Images",
            "mp3" | "wav" | "flac" | "aac" | "m4a" | "ogg" => "Music",
            "mp4" | "mkv" | "avi" | "mov" | "wmv" | "webm" => "Videos",
            _ => return None,
        };
        match resolved {
            None => resolved = Some(cat),
            Some(prev) if prev == cat => {}
            Some(_) => return None,
        }
    }

    resolved
}

fn parse_qsirch_item(raw: &JsonValue) -> QsirchItem {
    let id = raw
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let name = raw
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let extension = raw
        .get("extension")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let size = raw.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
    let modified_epoch = parse_modified_epoch(raw.get("modified"));

    // Resolve real full path: Qsirch 7 stores the full path in preview.info[key=="path"];
    // raw["path"] is only the parent directory.
    let mut nas_path = String::new();
    if let Some(info_arr) = raw
        .get("preview")
        .and_then(|p| p.get("info"))
        .and_then(|i| i.as_array())
    {
        for entry in info_arr {
            if entry.get("key").and_then(|k| k.as_str()) == Some("path") {
                if let Some(val) = entry.get("value").and_then(|v| v.as_str()) {
                    nas_path = val.to_string();
                    break;
                }
            }
        }
    }

    if nas_path.is_empty() {
        let dir = raw
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim_end_matches('/');
        let full_name = if !extension.is_empty()
            && !name
                .to_lowercase()
                .ends_with(&format!(".{}", extension.to_lowercase()))
        {
            format!("{name}.{extension}")
        } else {
            name.clone()
        };
        if !dir.is_empty() && !full_name.is_empty() {
            nas_path = format!("{dir}/{full_name}");
        } else {
            nas_path = dir.to_string();
        }
    }

    let snippet = raw
        .get("content")
        .and_then(|v| v.as_str())
        .map(clean_snippet)
        .unwrap_or_default();

    QsirchItem {
        id,
        name,
        extension,
        nas_path,
        size,
        modified_epoch,
        snippet,
    }
}

fn clean_snippet(raw: &str) -> String {
    raw.replace("<qusion>", "")
        .replace("</qusion>", "")
        .replace(['\r', '\n', '\t'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(200)
        .collect()
}

fn parse_modified_epoch(val: Option<&JsonValue>) -> i64 {
    let Some(v) = val else {
        return 0;
    };
    if let Some(n) = v.as_i64() {
        return n;
    }
    if let Some(f) = v.as_f64() {
        return f as i64;
    }
    if let Some(s) = v.as_str() {
        let t = s.trim();
        if let Ok(n) = t.parse::<i64>() {
            return n;
        }
        if let Ok(f) = t.parse::<f64>() {
            return f as i64;
        }
    }
    0
}

fn extract_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let rest = &xml[start..];
    let end = rest.find(&close)?;
    let inner = rest[..end].trim();
    if let Some(stripped) = inner
        .strip_prefix("<![CDATA[")
        .and_then(|s| s.strip_suffix("]]>"))
    {
        Some(stripped.to_string())
    } else {
        Some(inner.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_tag_extraction_handles_cdata() {
        let xml = "<QDocRoot><authPassed><![CDATA[1]]></authPassed><authSid><![CDATA[abc123]]></authSid></QDocRoot>";
        assert_eq!(extract_xml_tag(xml, "authPassed").as_deref(), Some("1"));
        assert_eq!(extract_xml_tag(xml, "authSid").as_deref(), Some("abc123"));
    }

    #[test]
    fn query_builder_uses_qsirch_categories_and_dot() {
        assert_eq!(build_qsirch_query("*", ""), ".");
        assert_eq!(build_qsirch_query("*.*", ""), ".");
        assert_eq!(build_qsirch_query("*.pdf", ""), ". category:PDF");
        assert_eq!(build_qsirch_query("*xlsx*", ""), ". category:Excel");
        assert_eq!(build_qsirch_query("*.pdf;*.docx", ""), ".");
        assert_eq!(build_qsirch_query("*/part1/part2/*.xsl", ""), "part2 part1");
        assert_eq!(build_qsirch_query(r"*2026\Předpisy\*", ""), "Předpisy 2026");
        assert_eq!(
            build_qsirch_query("*.pdf", "invoice 2026"),
            "invoice 2026 category:PDF"
        );
    }

    #[test]
    fn parse_qsirch_item_prefers_preview_info_path() {
        let raw = r#"{
            "id": "1",
            "name": "budget",
            "extension": "pdf",
            "path": "/Multimedia/Docs",
            "size": 12345,
            "modified": "1754460005",
            "content": "Annual <qusion>budget</qusion>\nreport",
            "preview": {
                "info": [
                    {"key": "path", "value": "/Multimedia/Docs/budget.pdf"}
                ]
            }
        }"#;
        let v = parse_json(raw).unwrap();
        let item = parse_qsirch_item(&v);
        assert_eq!(item.nas_path, "/Multimedia/Docs/budget.pdf");
        assert_eq!(item.size, 12345);
        assert_eq!(item.modified_epoch, 1754460005);
        assert_eq!(item.snippet, "Annual budget report");
    }
}
