//! Bidirectional path mapping between QNAP Qsirch NAS paths and Windows paths
//! (mapped network drives and UNC paths), plus deduplication key normalization.

use crate::config::{Config, ShareMapping};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::NO_ERROR;
use windows_sys::Win32::NetworkManagement::WNet::WNetGetConnectionW;

static MAPPED_DRIVES_CACHE: OnceLock<Mutex<Option<(Instant, Vec<DriveMapping>)>>> = OnceLock::new();
const DRIVE_CACHE_TTL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct DriveMapping {
    /// e.g. `"Z:"` (uppercase)
    pub drive: String,
    /// e.g. `"\\\\192.168.1.100\\Multimedia"` (no trailing backslash)
    pub unc_root: String,
}

#[derive(Debug, Clone)]
pub struct PathMapper {
    nas_hosts_lower: Vec<String>,
    unc_host: String,
    explicit: Vec<ShareMapping>,
    drives: Vec<DriveMapping>,
}

impl PathMapper {
    pub fn from_config(cfg: &Config) -> Self {
        Self::new(cfg, detect_mapped_drives())
    }

    pub fn new(cfg: &Config, drives: Vec<DriveMapping>) -> Self {
        let mut nas_hosts_lower = Vec::new();
        let h1 = cfg.nas_host.trim().to_lowercase();
        if !h1.is_empty() {
            nas_hosts_lower.push(h1);
        }
        let h2 = cfg.unc_host.trim().to_lowercase();
        if !h2.is_empty() && !nas_hosts_lower.contains(&h2) {
            nas_hosts_lower.push(h2);
        }

        let mut explicit = cfg.share_mappings.clone();
        // Longest NAS prefix first so more specific mappings win.
        explicit.sort_by(|a, b| b.nas_prefix.len().cmp(&a.nas_prefix.len()));

        Self {
            nas_hosts_lower,
            unc_host: cfg.effective_unc_host(),
            explicit,
            drives,
        }
    }

    /// Normalize a QNAP internal path to `/<Share>/<subpath...>`.
    /// Strips `/share/CACHEDEV1_DATA/`, `/share/MD0_DATA/`, `/share/ZFS530_DATA/`, or `/share/`.
    pub fn normalize_nas_path(raw: &str) -> String {
        let s = raw.trim().replace('\\', "/");
        let s = s.trim_start_matches('/');
        let parts: Vec<&str> = s.split('/').filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            return "/".to_string();
        }
        let start = if parts[0].eq_ignore_ascii_case("share") {
            if parts.len() >= 2 && is_qnap_volume_dir(parts[1]) {
                2
            } else {
                1
            }
        } else {
            0
        };
        if start >= parts.len() {
            "/".to_string()
        } else {
            format!("/{}", parts[start..].join("/"))
        }
    }

    /// Map a Qsirch NAS path to a Windows path (preferring explicit mappings, then mapped drives, then UNC).
    pub fn nas_to_windows(&self, nas_path: &str) -> String {
        let norm = Self::normalize_nas_path(nas_path);
        let norm_lower = norm.to_lowercase();

        // 1. Explicit user share_mappings
        for m in &self.explicit {
            let pref = Self::normalize_nas_path(&m.nas_prefix);
            let pref_lower = pref.to_lowercase();
            if norm_lower == pref_lower || norm_lower.starts_with(&format!("{pref_lower}/")) {
                let rest = norm[pref.len()..].trim_start_matches('/');
                return join_windows_prefix(&m.windows_prefix, rest);
            }
        }

        // Split into share + relative path
        let trimmed = norm.trim_start_matches('/');
        let mut split = trimmed.splitn(2, '/');
        let share = split.next().unwrap_or("");
        let rest = split.next().unwrap_or("");

        if share.is_empty() {
            return if self.unc_host.is_empty() {
                nas_path.replace('/', "\\")
            } else {
                format!("\\\\{}", self.unc_host)
            };
        }

        // 2. Auto-detected Windows mapped drives matching this NAS host + share
        for d in &self.drives {
            if let Some((host, mapped_share, mapped_sub)) = parse_unc(&d.unc_root) {
                let host_matches = self.nas_hosts_lower.is_empty()
                    || self.nas_hosts_lower.iter().any(|h| h == &host.to_lowercase());
                if host_matches && mapped_share.eq_ignore_ascii_case(share) {
                    if mapped_sub.is_empty() {
                        return join_windows_prefix(&format!("{}\\", d.drive), rest);
                    }
                    let sub_prefix = format!("{}/", mapped_sub.to_lowercase());
                    let rest_lower = rest.to_lowercase();
                    if rest_lower == mapped_sub.to_lowercase() {
                        return format!("{}\\", d.drive);
                    } else if rest_lower.starts_with(&sub_prefix) {
                        let tail = &rest[mapped_sub.len()..].trim_start_matches('/');
                        return join_windows_prefix(&format!("{}\\", d.drive), tail);
                    }
                }
            }
        }

        // 3. Fallback to UNC path \\<unc_host>\<share>\<rest>
        if self.unc_host.is_empty() {
            norm.replace('/', "\\")
        } else if rest.is_empty() {
            format!("\\\\{}\\{}", self.unc_host, share)
        } else {
            format!("\\\\{}\\{}\\{}", self.unc_host, share, rest.replace('/', "\\"))
        }
    }

    /// Reverse-map a Windows directory (e.g. `Z:\Docs` or `\\NAS\Multimedia\Docs`) to a NAS path prefix
    /// like `"/Multimedia/Docs"` if it points to the configured NAS.
    pub fn windows_to_nas_prefix(&self, win_path: &str) -> Option<String> {
        let trimmed = win_path.trim().trim_end_matches(['\\', '/']);
        if trimmed.is_empty() {
            return None;
        }
        let lower = trimmed.to_lowercase();

        // 1. Check explicit mappings
        for m in &self.explicit {
            let wp = m.windows_prefix.trim_end_matches(['\\', '/']);
            let wp_lower = wp.to_lowercase();
            if lower == wp_lower {
                return Some(Self::normalize_nas_path(&m.nas_prefix));
            }
            if lower.starts_with(&format!("{wp_lower}\\")) {
                let tail = trimmed[wp.len()..].replace('\\', "/");
                let base = Self::normalize_nas_path(&m.nas_prefix);
                return Some(format!("{}{}", base.trim_end_matches('/'), tail));
            }
        }

        // 2. Expand mapped drive to UNC if applicable
        let expanded = self.expand_drive_to_unc(trimmed);
        let unc = expanded.as_deref().unwrap_or(trimmed);

        if let Some((host, share, sub)) = parse_unc(unc) {
            let host_matches = self.nas_hosts_lower.is_empty()
                || self.nas_hosts_lower.iter().any(|h| h == &host.to_lowercase());
            if host_matches && !share.is_empty() {
                return if sub.is_empty() {
                    Some(format!("/{share}"))
                } else {
                    Some(format!("/{share}/{sub}"))
                };
            }
        }
        None
    }

    /// Produce a canonical lowercase key for deduplicating local + Qsirch results.
    /// Expands mapped drives to UNC so `Z:\foo.txt` and `\\NAS\Share\foo.txt` produce the same key.
    pub fn dedup_key(&self, win_path: &str) -> String {
        let trimmed = win_path.trim().trim_end_matches(['\\', '/']);
        let expanded = self.expand_drive_to_unc(trimmed);
        let base = expanded.as_deref().unwrap_or(trimmed);
        let mut norm = base.replace('/', "\\").to_lowercase();
        // If UNC host matches nas_host or unc_host, normalize host token to first configured host.
        if let Some(primary) = self.nas_hosts_lower.first() {
            if let Some((host, share, sub)) = parse_unc(&norm) {
                if self.nas_hosts_lower.iter().any(|h| h == &host) {
                    norm = if sub.is_empty() {
                        format!("\\\\{primary}\\{share}")
                    } else {
                        format!("\\\\{primary}\\{share}\\{}", sub.replace('/', "\\"))
                    };
                }
            }
        }
        norm
    }

    fn expand_drive_to_unc(&self, path: &str) -> Option<String> {
        let bytes = path.as_bytes();
        if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
            let drive = format!("{}:", (bytes[0] as char).to_ascii_uppercase());
            if let Some(dm) = self.drives.iter().find(|d| d.drive == drive) {
                let tail = path[2..].trim_start_matches(['\\', '/']);
                return if tail.is_empty() {
                    Some(dm.unc_root.clone())
                } else {
                    Some(format!("{}\\{}", dm.unc_root, tail.replace('/', "\\")))
                };
            }
        }
        None
    }
}

fn is_qnap_volume_dir(seg: &str) -> bool {
    let u = seg.to_ascii_uppercase();
    u.ends_with("_DATA")
        && (u.starts_with("CACHEDEV") || u.starts_with("MD") || u.starts_with("ZFS") || u.starts_with("HDA"))
}

fn join_windows_prefix(prefix: &str, rest: &str) -> String {
    let base = prefix.trim_end_matches(['\\', '/']);
    let tail = rest.trim_start_matches(['\\', '/']).replace('/', "\\");
    if tail.is_empty() {
        if base.ends_with(':') {
            format!("{base}\\")
        } else {
            base.to_string()
        }
    } else {
        format!("{base}\\{tail}")
    }
}

/// Parse `\\host\share\optional\sub\path` into `(host, share, "optional/sub/path")`.
fn parse_unc(path: &str) -> Option<(String, String, String)> {
    let s = path.replace('/', "\\");
    let rest = s.strip_prefix("\\\\")?;
    let mut parts = rest.split('\\').filter(|p| !p.is_empty());
    let host = parts.next()?.to_string();
    let share = parts.next()?.to_string();
    let sub: Vec<&str> = parts.collect();
    Some((host, share, sub.join("/")))
}

/// Enumerate Windows mapped drives (`A:`..`Z:`) via `WNetGetConnectionW`, cached for 30s
/// so repeated `PathMapper::from_config` calls don't stall on offline network drives.
pub fn detect_mapped_drives() -> Vec<DriveMapping> {
    let cache = MAPPED_DRIVES_CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = cache.lock() {
        if let Some((ts, ref cached)) = *guard {
            if ts.elapsed() < DRIVE_CACHE_TTL {
                return cached.clone();
            }
        }
    }

    let mut out = Vec::new();
    for letter in b'A'..=b'Z' {
        let local = [letter as u16, b':' as u16, 0u16];
        let mut buf = vec![0u16; 512];
        let mut len = buf.len() as u32;
        let rc = unsafe { WNetGetConnectionW(local.as_ptr(), buf.as_mut_ptr(), &mut len) };
        if rc == NO_ERROR {
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            let unc = String::from_utf16_lossy(&buf[..end])
                .trim_end_matches('\\')
                .to_string();
            if unc.starts_with("\\\\") {
                out.push(DriveMapping {
                    drive: format!("{}:", letter as char),
                    unc_root: unc,
                });
            }
        }
    }

    if let Ok(mut guard) = cache.lock() {
        *guard = Some((Instant::now(), out.clone()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_qnap_volume_paths() {
        assert_eq!(
            PathMapper::normalize_nas_path("/share/CACHEDEV1_DATA/Multimedia/Docs/a.pdf"),
            "/Multimedia/Docs/a.pdf"
        );
        assert_eq!(
            PathMapper::normalize_nas_path("/share/Multimedia/a.pdf"),
            "/Multimedia/a.pdf"
        );
        assert_eq!(
            PathMapper::normalize_nas_path("Multimedia/a.pdf"),
            "/Multimedia/a.pdf"
        );
    }

    #[test]
    fn maps_via_mapped_drive_and_deduplicates() {
        let mut cfg = Config::default();
        cfg.nas_host = "192.168.1.50".into();
        cfg.unc_host = "qnap-nas".into();
        let drives = vec![DriveMapping {
            drive: "M:".into(),
            unc_root: "\\\\qnap-nas\\Multimedia".into(),
        }];
        let mapper = PathMapper::new(&cfg, drives);

        let win = mapper.nas_to_windows("/share/CACHEDEV1_DATA/Multimedia/Reports/2026.pdf");
        assert_eq!(win, "M:\\Reports\\2026.pdf");

        let k1 = mapper.dedup_key("M:\\Reports\\2026.pdf");
        let k2 = mapper.dedup_key("\\\\192.168.1.50\\Multimedia\\Reports\\2026.pdf");
        assert_eq!(k1, k2);

        let rev = mapper.windows_to_nas_prefix("M:\\Reports");
        assert_eq!(rev.as_deref(), Some("/Multimedia/Reports"));
    }

    #[test]
    fn fallback_to_unc_when_unmapped() {
        let mut cfg = Config::default();
        cfg.nas_host = "10.0.0.3".into();
        let mapper = PathMapper::new(&cfg, vec![]);
        assert_eq!(
            mapper.nas_to_windows("/Public/readme.txt"),
            "\\\\10.0.0.3\\Public\\readme.txt"
        );
    }
}
