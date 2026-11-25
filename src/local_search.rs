//! Fast local/SMB filesystem search with FAR Manager-style wildcard mask matching
//! (supporting both filename masks and path masks like `*/part1/part2/*.xsl`),
//! drive-root enumeration, and optional containing-text search.

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;
use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};

const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOTE: u32 = 4;
const DRIVE_RAMDISK: u32 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchArea {
    /// Current active panel directory and its subfolders.
    CurrentDir,
    /// Root of the current drive (e.g. `C:\`) or UNC share (`\\Server\Share\`).
    CurrentDriveRoot,
    /// Root directories of all local fixed drives (`C:\`, `D:\`, ...).
    AllFixedDrives,
    /// Root directories of all drives (fixed, removable, network, RAM disk).
    AllDrives,
}

/// Resolve the list of local/SMB root directories to walk for the selected `SearchArea`.
pub fn resolve_local_roots(current_dir: &str, area: SearchArea) -> Vec<String> {
    match area {
        SearchArea::CurrentDir => {
            if current_dir.trim().is_empty() {
                Vec::new()
            } else {
                vec![current_dir.to_string()]
            }
        }
        SearchArea::CurrentDriveRoot => {
            if let Some(root) = extract_drive_or_unc_root(current_dir) {
                vec![root]
            } else if !current_dir.trim().is_empty() {
                vec![current_dir.to_string()]
            } else {
                Vec::new()
            }
        }
        SearchArea::AllFixedDrives => enumerate_drive_roots(false),
        SearchArea::AllDrives => enumerate_drive_roots(true),
    }
}

/// Extract `C:\` or `\\Server\Share\` from a Windows path.
pub fn extract_drive_or_unc_root(path: &str) -> Option<String> {
    let trimmed = path.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Some(format!("{}:\\", (bytes[0] as char).to_ascii_uppercase()));
    }
    let norm = trimmed.replace('/', "\\");
    if let Some(rest) = norm.strip_prefix("\\\\") {
        let mut parts = rest.split('\\').filter(|p| !p.is_empty());
        let host = parts.next()?;
        let share = parts.next()?;
        return Some(format!("\\\\{host}\\{share}\\"));
    }
    None
}

fn enumerate_drive_roots(include_removable_and_network: bool) -> Vec<String> {
    let mut roots = Vec::new();
    let mask = unsafe { GetLogicalDrives() };
    for i in 0..26u32 {
        if (mask & (1 << i)) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root_w = [letter as u16, b':' as u16, b'\\' as u16, 0u16];
        let dtype = unsafe { GetDriveTypeW(root_w.as_ptr()) };
        let keep = if include_removable_and_network {
            matches!(dtype, DRIVE_FIXED | DRIVE_REMOVABLE | DRIVE_REMOTE | DRIVE_RAMDISK)
        } else {
            dtype == DRIVE_FIXED
        };
        if keep {
            roots.push(format!("{letter}:\\"));
        }
    }
    roots
}

#[derive(Debug, Clone)]
pub struct LocalItem {
    pub path: String,
    pub size: u64,
    pub modified_epoch: i64,
    pub is_dir: bool,
    pub attributes: u32,
}

#[derive(Debug, Clone)]
struct MaskPattern {
    /// Lowercase pattern with `/` separators.
    pattern: String,
    /// True if pattern contains `/` (matches against full path instead of basename only).
    is_path_pattern: bool,
    /// Optional `*/<pattern>` fallback for relative path patterns like `part1/part2/*.xsl`.
    prefixed_pattern: Option<String>,
}

impl MaskPattern {
    fn new(raw: &str) -> Self {
        let norm = raw.trim().replace('\\', "/").to_lowercase();
        let is_path_pattern = norm.contains('/');
        let prefixed_pattern = if is_path_pattern
            && !norm.starts_with('*')
            && !norm.starts_with('/')
            && !(norm.len() >= 2 && norm.as_bytes()[1] == b':')
        {
            Some(format!("*/{norm}"))
        } else {
            None
        };
        Self {
            pattern: norm,
            is_path_pattern,
            prefixed_pattern,
        }
    }

    fn matches(&self, full_path_norm: &str, file_name_lower: &str) -> bool {
        if self.is_path_pattern {
            if wildcard_match(&self.pattern, full_path_norm) {
                return true;
            }
            if let Some(ref pref) = self.prefixed_pattern {
                return wildcard_match(pref, full_path_norm);
            }
            false
        } else {
            wildcard_match(&self.pattern, file_name_lower)
        }
    }
}

/// FAR-style wildcard matcher supporting:
/// - `*` (any sequence) and `?` (single char)
/// - Multiple patterns separated by `,` or `;`
/// - Optional exclusion list after `|` (e.g. `*.rs;*.toml|*_test.rs`)
/// - Path-segment patterns containing `/` or `\` (e.g. `*/part1/part2/*.xsl`)
#[derive(Debug, Clone)]
pub struct FarMaskMatcher {
    includes: Vec<MaskPattern>,
    excludes: Vec<MaskPattern>,
}

impl FarMaskMatcher {
    pub fn new(mask: &str) -> Self {
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

    pub fn is_match_all(&self) -> bool {
        self.excludes.is_empty()
            && self.includes.len() == 1
            && !self.includes[0].is_path_pattern
            && (self.includes[0].pattern == "*" || self.includes[0].pattern == "*.*")
    }

    /// Match against a full path (Windows or NAS) and its file name.
    pub fn matches_path(&self, full_path: &str, file_name: &str) -> bool {
        let path_norm = full_path.replace('\\', "/").to_lowercase();
        let name_lower = file_name.to_lowercase();

        let included = self
            .includes
            .iter()
            .any(|p| p.matches(&path_norm, &name_lower));
        if !included {
            return false;
        }
        !self
            .excludes
            .iter()
            .any(|p| p.matches(&path_norm, &name_lower))
    }

    #[allow(dead_code)]
    pub fn matches(&self, filename: &str) -> bool {
        self.matches_path(filename, filename)
    }
}

/// Classic wildcard matcher (`*` and `?`) operating on Unicode scalar values.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" || pattern == "*.*" {
        return true;
    }
    let p: Vec<char> = pattern.chars().collect();
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

pub fn search_local(
    roots: &[String],
    file_mask: &str,
    containing_text: &str,
    case_sensitive: bool,
    max_results: usize,
    max_content_bytes: u64,
    cancel: &AtomicBool,
    scanned_counter: &AtomicUsize,
    found_counter: &AtomicUsize,
) -> Result<Vec<LocalItem>, String> {
    if roots.is_empty() {
        return Ok(Vec::new());
    }

    let matcher = FarMaskMatcher::new(file_mask);
    let needle = containing_text.trim();
    let needle_lower = if case_sensitive {
        String::new()
    } else {
        needle.to_lowercase()
    };

    let mut results = Vec::new();
    let mut existing_roots = 0usize;

    for root_dir in roots {
        if cancel.load(Ordering::Relaxed) || results.len() >= max_results {
            break;
        }
        let root = Path::new(root_dir);
        if !root.exists() {
            continue;
        }
        existing_roots += 1;

        for entry in WalkDir::new(root).follow_links(false).into_iter().flatten() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            if entry.depth() == 0 {
                continue;
            }
            scanned_counter.fetch_add(1, Ordering::Relaxed);
            let full_path = entry.path().to_string_lossy();
            let file_name = entry.file_name().to_string_lossy();
            if !matcher.matches_path(&full_path, &file_name) {
                continue;
            }

            let is_dir = entry.file_type().is_dir();
            if !needle.is_empty() && is_dir {
                continue;
            }

            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let size = if is_dir { 0 } else { meta.len() };

            if !needle.is_empty() {
                if size == 0 || size > max_content_bytes {
                    continue;
                }
                if !file_contains_text(entry.path(), needle, &needle_lower, case_sensitive) {
                    continue;
                }
            }

            let modified_epoch = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);

            #[cfg(windows)]
            let attributes = {
                use std::os::windows::fs::MetadataExt;
                meta.file_attributes()
            };
            #[cfg(not(windows))]
            let attributes = if is_dir { 0x10 } else { 0x20 };

            results.push(LocalItem {
                path: full_path.into_owned(),
                size,
                modified_epoch,
                is_dir,
                attributes,
            });
            found_counter.fetch_add(1, Ordering::Relaxed);

            if results.len() >= max_results {
                break;
            }
        }
    }

    if existing_roots == 0 {
        return Err(format!("No accessible local search roots: {:?}", roots));
    }

    Ok(results)
}

fn file_contains_text(path: &Path, needle: &str, needle_lower: &str, case_sensitive: bool) -> bool {
    let Ok(mut f) = File::open(path) else {
        return false;
    };
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return false;
    }
    let text = String::from_utf8_lossy(&buf);
    if case_sensitive {
        text.contains(needle)
    } else {
        text.to_lowercase().contains(needle_lower)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn far_mask_matching() {
        let m = FarMaskMatcher::new("*.pdf;*.docx|*_draft.*");
        assert!(m.matches("Report.PDF"));
        assert!(m.matches("notes.docx"));
        assert!(!m.matches("notes_draft.docx"));
        assert!(!m.matches("image.png"));
    }

    #[test]
    fn path_segment_mask_matching() {
        let m = FarMaskMatcher::new("*/part1/part2/*.xsl");
        assert!(m.matches_path(r"C:\work\part1\part2\style.xsl", "style.xsl"));
        assert!(m.matches_path("/Multimedia/part1/part2/transform.XSL", "transform.XSL"));
        assert!(!m.matches_path(r"C:\work\part1\other\style.xsl", "style.xsl"));
        assert!(!m.matches_path(r"C:\work\part1\part2\style.xml", "style.xml"));

        // Relative path pattern without leading `*/` also matches subpaths
        let m2 = FarMaskMatcher::new(r"part1\part2\*.xsl|*\archive\*");
        assert!(m2.matches_path(r"D:\repo\part1\part2\a.xsl", "a.xsl"));
        assert!(!m2.matches_path(r"D:\archive\part1\part2\a.xsl", "a.xsl"));
    }

    #[test]
    fn extract_roots() {
        assert_eq!(
            extract_drive_or_unc_root(r"C:\Users\vladi\docs").as_deref(),
            Some(r"C:\")
        );
        assert_eq!(
            extract_drive_or_unc_root(r"\\qnap\Multimedia\Photos").as_deref(),
            Some(r"\\qnap\Multimedia\")
        );
    }

    #[test]
    fn wildcard_question_mark() {
        assert!(wildcard_match("doc_??.txt", "doc_01.txt"));
        assert!(!wildcard_match("doc_??.txt", "doc_1.txt"));
    }
}
