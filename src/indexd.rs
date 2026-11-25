//! Fast WinHTTP client for `nas_indexd` (the QNAP Container Station in-memory indexer).

use crate::config::Config;
use crate::far_ffi::WideString;
use crate::mini_json::parse_json;
use crate::qsirch::{QsirchItem, QsirchSearchResponse};
use std::ffi::c_void;
use std::path::Path;
use windows_sys::Win32::Networking::WinHttp::*;

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

fn get_json(cfg: &Config, path_and_query: &str) -> Result<crate::mini_json::JsonValue, String> {
    let host = cfg.nas_host.trim();
    if host.is_empty() || cfg.indexd_port == 0 {
        return Err("nas_indexd is not configured".into());
    }

    unsafe {
        let ua = WideString::new("FarQsirch-Indexd/0.1");
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
        let _g_session = WinHttpHandle(h_session);
        WinHttpSetTimeouts(h_session, 2_000, 2_000, 3_000, 10_000);

        let w_host = WideString::new(host);
        let h_connect = WinHttpConnect(h_session, w_host.as_ptr(), cfg.indexd_port, 0);
        if h_connect.is_null() {
            return Err(format!("Cannot connect to nas_indexd at {host}:{}", cfg.indexd_port));
        }
        let _g_connect = WinHttpHandle(h_connect);

        let w_method = WideString::new("GET");
        let w_path = WideString::new(path_and_query);
        let h_request = WinHttpOpenRequest(
            h_connect,
            w_method.as_ptr(),
            w_path.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut(),
            0,
        );
        if h_request.is_null() {
            return Err("WinHttpOpenRequest failed".into());
        }
        let _g_req = WinHttpHandle(h_request);

        let extra_hdr = if cfg.indexd_api_key.trim().is_empty() {
            None
        } else {
            Some(WideString::new(&format!(
                "X-Api-Key: {}\r\n",
                cfg.indexd_api_key.trim()
            )))
        };
        let (hdr_ptr, hdr_len) = match &extra_hdr {
            Some(w) => (w.as_ptr(), u32::MAX),
            None => (std::ptr::null(), 0),
        };

        if WinHttpSendRequest(h_request, hdr_ptr, hdr_len, std::ptr::null_mut(), 0, 0, 0) == 0 {
            let err = windows_sys::Win32::Foundation::GetLastError();
            return Err(format!(
                "Cannot reach nas_indexd at {host}:{} (WinHTTP {err})",
                cfg.indexd_port
            ));
        }
        if WinHttpReceiveResponse(h_request, std::ptr::null_mut()) == 0 {
            let err = windows_sys::Win32::Foundation::GetLastError();
            return Err(format!("No response from nas_indexd (WinHTTP {err})"));
        }

        let mut status: u32 = 0;
        let mut status_sz: u32 = std::mem::size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            h_request,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            std::ptr::null(),
            (&mut status as *mut u32).cast::<c_void>(),
            &mut status_sz,
            std::ptr::null_mut(),
        );

        let mut bytes = Vec::new();
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

        let body = String::from_utf8_lossy(&bytes);
        if status != 200 {
            return Err(format!("nas_indexd HTTP {status}: {}", body.chars().take(120).collect::<String>()));
        }
        parse_json(&body).map_err(|e| format!("Invalid JSON from nas_indexd: {e}"))
    }
}

pub fn test_connection(cfg: &Config) -> Result<String, String> {
    let v = get_json(cfg, "/health")?;
    let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("ok");
    let indexed = v.get("indexed").and_then(|n| n.as_u64()).unwrap_or(0);
    let scan_ms = v.get("last_scan_ms").and_then(|n| n.as_u64()).unwrap_or(0);
    Ok(format!(
        "nas_indexd ({}:{}): {status}, {indexed} files indexed ({scan_ms} ms)",
        cfg.nas_host.trim(),
        cfg.indexd_port
    ))
}

pub fn search(
    cfg: &Config,
    file_mask: &str,
    nas_path_prefix: Option<&str>,
) -> Result<QsirchSearchResponse, String> {
    let limit = cfg.max_indexd_results.clamp(1, 100_000);
    let mut qs = format!("mask={}&limit={limit}", url_encode(file_mask));
    if let Some(pref) = nas_path_prefix {
        if !pref.is_empty() && pref != "/" {
            qs.push_str(&format!("&prefix={}", url_encode(pref)));
        }
    }
    let data = get_json(cfg, &format!("/search?{qs}"))?;
    let total = data.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let raw_items = data
        .get("items")
        .and_then(|v| v.as_array())
        .unwrap_or(&[]);

    let mut items = Vec::with_capacity(raw_items.len());
    for raw in raw_items {
        let nas_path = raw
            .get("p")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if nas_path.is_empty() {
            continue;
        }
        let name = Path::new(&nas_path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let extension = Path::new(&nas_path)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let size = raw.get("s").and_then(|v| v.as_u64()).unwrap_or(0);
        let modified_epoch = raw.get("m").and_then(|v| v.as_i64()).unwrap_or(0);
        items.push(QsirchItem {
            id: String::new(),
            name,
            extension,
            nas_path,
            size,
            modified_epoch,
            snippet: String::new(),
        });
    }

    Ok(QsirchSearchResponse {
        total,
        fetched_raw: total,
        effective_query: String::new(),
        items,
    })
}
