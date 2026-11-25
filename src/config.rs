//! Configuration persistence (`far_qsirch.toml`) with zero proc-macro dependencies.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareMapping {
    /// NAS share prefix as returned by Qsirch, e.g. `"/Multimedia"` or `"/share/Multimedia"`.
    pub nas_prefix: String,
    /// Target Windows path prefix, e.g. `"M:\\"` or `"\\\\NAS\\Multimedia"`.
    pub windows_prefix: String,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub nas_host: String,
    pub nas_port: u16,
    pub use_ssl: bool,
    pub accept_invalid_certs: bool,
    pub username: String,
    pub password: String,
    /// Optional SMB host override if different from `nas_host` (e.g. NetBIOS name vs IP).
    pub unc_host: String,
    /// Optional `nas_indexd` fast indexer port on the NAS (e.g. `9876`, `0` = disabled).
    pub indexd_port: u16,
    /// Optional `X-Api-Key` header for `nas_indexd`.
    pub indexd_api_key: String,
    pub max_indexd_results: usize,
    pub max_qsirch_results: usize,
    pub max_local_results: usize,
    pub qsirch_timeout_secs: u64,
    pub default_include_local: bool,
    pub default_include_qsirch: bool,
    /// 0 = Standard full-text, 1 = Image OCR, 2 = Combined text + OCR.
    pub default_advanced_mode: u8,
    /// Max file size (in MiB) scanned for containing-text during local search.
    pub local_max_content_mb: u64,
    /// Explicit share-to-Windows path mappings (auto-detected mapped drives also apply).
    pub share_mappings: Vec<ShareMapping>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            nas_host: String::new(),
            nas_port: 8080,
            use_ssl: false,
            accept_invalid_certs: true,
            username: String::new(),
            password: String::new(),
            unc_host: String::new(),
            indexd_port: 0,
            indexd_api_key: String::new(),
            max_indexd_results: 10_000,
            max_qsirch_results: 200,
            max_local_results: 5000,
            qsirch_timeout_secs: 15,
            default_include_local: true,
            default_include_qsirch: true,
            default_advanced_mode: 0,
            local_max_content_mb: 16,
            share_mappings: Vec::new(),
        }
    }
}

impl Config {
    pub fn is_qsirch_configured(&self) -> bool {
        !self.nas_host.trim().is_empty() && !self.effective_username().is_empty()
    }

    pub fn is_indexd_configured(&self) -> bool {
        !self.nas_host.trim().is_empty() && self.indexd_port > 0
    }

    pub fn effective_username(&self) -> String {
        if !self.username.trim().is_empty() {
            return self.username.trim().to_string();
        }
        std::env::var("QSIRCH_USER").unwrap_or_default()
    }

    pub fn effective_password(&self) -> String {
        if !self.password.is_empty() {
            return self.password.clone();
        }
        std::env::var("QSIRCH_PASS").unwrap_or_default()
    }

    pub fn effective_unc_host(&self) -> String {
        let host = self.unc_host.trim();
        if !host.is_empty() {
            host.to_string()
        } else {
            self.nas_host.trim().to_string()
        }
    }

    /// Resolve config file path: next to the plugin DLL if writable, otherwise `%APPDATA%\FarQsirch\far_qsirch.toml`.
    pub fn config_path(module_path: Option<&Path>) -> PathBuf {
        if let Some(mod_path) = module_path {
            if let Some(dir) = mod_path.parent() {
                let candidate = dir.join("far_qsirch.toml");
                if candidate.exists() {
                    return candidate;
                }
                let probe = dir.join(".far_qsirch_write_test");
                if std::fs::write(&probe, b"").is_ok() {
                    let _ = std::fs::remove_file(&probe);
                    return candidate;
                }
            }
        }
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("FarQsirch").join("far_qsirch.toml");
        }
        PathBuf::from("far_qsirch.toml")
    }

    pub fn session_cache_path(module_path: Option<&Path>) -> PathBuf {
        let cfg = Self::config_path(module_path);
        cfg.parent()
            .unwrap_or_else(|| Path::new("."))
            .join("far_qsirch_session.txt")
    }

    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse_toml(&text),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create config directory: {e}"))?;
            }
        }
        std::fs::write(path, self.to_toml())
            .map_err(|e| format!("Failed to write {}: {e}", path.display()))
    }

    pub fn to_toml(&self) -> String {
        let mut out = String::new();
        out.push_str("# FarQsirch configuration\n");
        out.push_str(&format!("nas_host = \"{}\"\n", escape_toml(&self.nas_host)));
        out.push_str(&format!("nas_port = {}\n", self.nas_port));
        out.push_str(&format!("use_ssl = {}\n", self.use_ssl));
        out.push_str(&format!(
            "accept_invalid_certs = {}\n",
            self.accept_invalid_certs
        ));
        out.push_str(&format!("username = \"{}\"\n", escape_toml(&self.username)));
        out.push_str(&format!("password = \"{}\"\n", escape_toml(&self.password)));
        out.push_str(&format!("unc_host = \"{}\"\n", escape_toml(&self.unc_host)));
        out.push_str(&format!("indexd_port = {}\n", self.indexd_port));
        out.push_str(&format!(
            "indexd_api_key = \"{}\"\n",
            escape_toml(&self.indexd_api_key)
        ));
        out.push_str(&format!(
            "max_indexd_results = {}\n",
            self.max_indexd_results
        ));
        out.push_str(&format!(
            "max_qsirch_results = {}\n",
            self.max_qsirch_results
        ));
        out.push_str(&format!("max_local_results = {}\n", self.max_local_results));
        out.push_str(&format!(
            "qsirch_timeout_secs = {}\n",
            self.qsirch_timeout_secs
        ));
        out.push_str(&format!(
            "default_include_local = {}\n",
            self.default_include_local
        ));
        out.push_str(&format!(
            "default_include_qsirch = {}\n",
            self.default_include_qsirch
        ));
        out.push_str(&format!(
            "default_advanced_mode = {}\n",
            self.default_advanced_mode
        ));
        out.push_str(&format!(
            "local_max_content_mb = {}\n",
            self.local_max_content_mb
        ));
        for m in &self.share_mappings {
            out.push_str("\n[[share_mappings]]\n");
            out.push_str(&format!(
                "nas_prefix = \"{}\"\n",
                escape_toml(&m.nas_prefix)
            ));
            out.push_str(&format!(
                "windows_prefix = \"{}\"\n",
                escape_toml(&m.windows_prefix)
            ));
        }
        out
    }

    pub fn parse_toml(text: &str) -> Self {
        let mut cfg = Self::default();
        let mut in_mapping = false;
        let mut cur_nas = String::new();
        let mut cur_win = String::new();

        for raw_line in text.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line == "[[share_mappings]]" {
                if in_mapping && !cur_nas.is_empty() && !cur_win.is_empty() {
                    cfg.share_mappings.push(ShareMapping {
                        nas_prefix: std::mem::take(&mut cur_nas),
                        windows_prefix: std::mem::take(&mut cur_win),
                    });
                }
                in_mapping = true;
                continue;
            }
            if line.starts_with('[') {
                if in_mapping && !cur_nas.is_empty() && !cur_win.is_empty() {
                    cfg.share_mappings.push(ShareMapping {
                        nas_prefix: std::mem::take(&mut cur_nas),
                        windows_prefix: std::mem::take(&mut cur_win),
                    });
                }
                in_mapping = false;
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let key = k.trim();
            let val = v.trim();
            if in_mapping {
                match key {
                    "nas_prefix" => cur_nas = unquote_toml(val),
                    "windows_prefix" => cur_win = unquote_toml(val),
                    _ => {}
                }
                continue;
            }
            match key {
                "nas_host" => cfg.nas_host = unquote_toml(val),
                "nas_port" => {
                    if let Ok(n) = val.parse() {
                        cfg.nas_port = n;
                    }
                }
                "use_ssl" => cfg.use_ssl = val == "true",
                "accept_invalid_certs" => cfg.accept_invalid_certs = val == "true",
                "username" => cfg.username = unquote_toml(val),
                "password" => cfg.password = unquote_toml(val),
                "unc_host" => cfg.unc_host = unquote_toml(val),
                "indexd_port" => {
                    if let Ok(n) = val.parse() {
                        cfg.indexd_port = n;
                    }
                }
                "indexd_api_key" => cfg.indexd_api_key = unquote_toml(val),
                "max_indexd_results" => {
                    if let Ok(n) = val.parse() {
                        cfg.max_indexd_results = n;
                    }
                }
                "max_qsirch_results" => {
                    if let Ok(n) = val.parse() {
                        cfg.max_qsirch_results = n;
                    }
                }
                "max_local_results" => {
                    if let Ok(n) = val.parse() {
                        cfg.max_local_results = n;
                    }
                }
                "qsirch_timeout_secs" => {
                    if let Ok(n) = val.parse() {
                        cfg.qsirch_timeout_secs = n;
                    }
                }
                "default_include_local" => cfg.default_include_local = val == "true",
                "default_include_qsirch" => cfg.default_include_qsirch = val == "true",
                "default_advanced_mode" => {
                    if let Ok(n) = val.parse() {
                        cfg.default_advanced_mode = n;
                    }
                }
                "local_max_content_mb" => {
                    if let Ok(n) = val.parse() {
                        cfg.local_max_content_mb = n;
                    }
                }
                _ => {}
            }
        }
        if in_mapping && !cur_nas.is_empty() && !cur_win.is_empty() {
            cfg.share_mappings.push(ShareMapping {
                nas_prefix: cur_nas,
                windows_prefix: cur_win,
            });
        }
        cfg
    }
}

fn escape_toml(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn unquote_toml(val: &str) -> String {
    let trimmed = val.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        let inner = &trimmed[1..trimmed.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('\\') => out.push('\\'),
                    Some('"') => out.push('"'),
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(other) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => out.push('\\'),
                }
            } else {
                out.push(c);
            }
        }
        out
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_toml() {
        let mut cfg = Config::default();
        cfg.nas_host = "10.0.0.5".into();
        cfg.nas_port = 443;
        cfg.use_ssl = true;
        cfg.username = "admin".into();
        cfg.share_mappings.push(ShareMapping {
            nas_prefix: "/Multimedia".into(),
            windows_prefix: "M:\\".into(),
        });
        let s = cfg.to_toml();
        let parsed = Config::parse_toml(&s);
        assert_eq!(parsed.nas_host, "10.0.0.5");
        assert_eq!(parsed.nas_port, 443);
        assert!(parsed.use_ssl);
        assert_eq!(parsed.share_mappings.len(), 1);
        assert_eq!(parsed.share_mappings[0].windows_prefix, "M:\\");
    }
}
