# FarQsirch — Unified Local + QNAP NAS Search for FAR Manager 3 (x64)

A native Rust plugin for **FAR Manager 3 (x64)** that merges **local filesystem search**, **QNAP Qsirch 7.x** (full-text & OCR content search), and an optional ultra-fast **`nas_indexd`** QNAP Docker sidecar (~10–200 ms for thousands of exact matches, real-time `inotify` updates, zero 1,000-item cap).

---

## Features

- **Unified Parallel Search:** Runs local/SMB search and QNAP NAS search concurrently, deduplicating hits across mapped drives and UNC paths with `[L]`, `[Q]`, and `[L+Q]` tags.
- **Ultra-Fast `nas_indexd` QNAP Sidecar (Optional, Recommended):**
  - Lightweight Rust Docker daemon (~28 MB image) running on your QNAP NAS.
  - Keeps all share paths in RAM with **real-time `inotify` filesystem watching** (zero idle disk polling / drive spin-ups).
  - Evaluates exact FAR-style filename and path masks (`*xlsx*`, `*.pdf;*.docx|*_draft.*`, `*/part1/part2/*.xsl`) in **~10–200 ms** with **no 1,000-item limit**.
- **QNAP Qsirch 7.x Full-Text & OCR Integration:**
  - Automatically used whenever you enter **Containing text** (or if `nas_indexd` isn't configured/reachable).
  - Supports Qsirch Text (`0`), Image OCR (`1`), and Combined (`2`) modes, `NAS_SID` session caching, auto re-login on 401/403, and live background pagination via FAR's `ACTL_SYNCHRO`.
- **Smart NAS Awareness:**
  - Automatically detects mapped Windows network drives (`WNetGetConnectionW`) and maps QNAP paths (`/share/CACHEDEV1_DATA/Public/...`) to your drive letter (`P:\...`) or UNC (`\\nas\Public\...`).
  - Automatically disables slow SMB walking and enables **Current NAS folder only** when you open Search from inside a NAS share.
- **Native FAR Manager UX:**
  - Full-screen Results Dialog with live snippet bar, **`Enter`** (Go to exact file), **`F3`** (View), **`F4`** (Edit), **`Alt+P`** (send to native `OPIF_REALNAMES` Virtual Panel), **`Alt+N`** (New Search), and instant **`ESC`** cancellation during scans.
  - Command-line prefix `qs:` (e.g. `qs:*.xlsx` or `qs:invoice`).
  - Ready-to-use `Alt+F7` LuaMacro (`Shift+Alt+F7` keeps stock FAR Find File).

---

## Architecture

```text
FAR Manager 3 x64 (far_qsirch.dll)
 ├── Local / SMB Search (walkdir, parallel thread, instant ESC cancel)
 └── QNAP NAS Search (parallel thread, WinHTTP, NO_PROXY)
      ├── Mask / path search (empty text) ──► nas_indexd (:9876, RAM + inotify, ~10-200 ms)
      │                                        └─ auto-fallback to Qsirch 7 if offline
      └── Containing text search ───────────► QNAP Qsirch 7 REST API (full-text + OCR)
```

---

## Part 1: Install the FAR Manager 3 (x64) Plugin

### 1. Build `far_qsirch.dll` (or download from Releases)
Requires Rust (`x86_64-pc-windows-msvc`). Uses zero proc-macros and zero build scripts (native Windows WinHTTP):

```powershell
cargo build --release
```

Output: `target\release\far_qsirch.dll`

### 2. Copy into FAR Manager Plugins
Close FAR Manager, then in an **Administrator PowerShell**:

```powershell
Stop-Process -Name "Far" -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "C:\Program Files\Far Manager\Plugins\FarQsirch" | Out-Null
Copy-Item -Force "target\release\far_qsirch.dll" "C:\Program Files\Far Manager\Plugins\FarQsirch\far_qsirch.dll"
```

### 3. *(Optional)* Bind `Alt+F7` via LuaMacro
Copy `macros\alt_f7_qsirch.lua` to your FAR profile's macros folder:

```
%FARPROFILE%\Macros\scripts\alt_f7_qsirch.lua
```
*(Usually `%APPDATA%\Far Manager\Profile\Macros\scripts\`.)*

- **`Alt+F7`** → Opens FarQsirch Unified Search
- **`Shift+Alt+F7`** → Opens stock FAR Find File

---

## Part 2: Deploy `nas_indexd` on Your QNAP NAS *(Optional, Recommended)*

For ~100×–500× faster filename & path searches without Qsirch's 1,000-item ceiling:

### Option A: Build & Load via Docker (Linux / WSL) + QNAP SSH
1. Build and export a QNAP-compatible Docker v1.2 `.tar` archive:
   ```bash
   cd nas_indexd
   docker build -t nas-indexd:latest .
   docker save -o nas-indexd.tar nas-indexd:latest
   python3 build_qnap_tar.py nas-indexd.tar nas-indexd-qnap.tar
   ```
2. Copy `nas-indexd-qnap.tar` to a NAS share (e.g. `Public`).
3. SSH into your QNAP as an administrator (`ssh admin@<nas-ip>`, press `Q` then `Y` if QTS shows the Console Management menu):
   ```bash
   docker load -i /share/Public/nas-indexd-qnap.tar
   ```
4. Check your real share volume paths (`ls -la /share` — e.g. `CACHEDEV1_DATA`, `CACHEDEV2_DATA`, or `ZFS*_DATA` on QuTS hero), then start the container:
   ```bash
   docker run -d --name nas-indexd --restart unless-stopped -p 9876:9876 \
     -v /share/CACHEDEV1_DATA:/data/CACHEDEV1_DATA:ro \
     -v /share/CACHEDEV2_DATA:/data/CACHEDEV2_DATA:ro \
     nas-indexd:latest
   ```
   *(You can also mount individual shares like `-v /share/CACHEDEV1_DATA/Public:/data/Public:ro`. `nas_indexd` automatically strips `CACHEDEV*_DATA/` / `ZFS*_DATA/` / `MD*_DATA/` prefixes.)*
5. Verify it's running:
   ```bash
   curl http://127.0.0.1:9876/health
   ```
   Expected output: `{"status":"ready","indexed":...,"watcher":"active","events":0}`

### Option B: Build Directly on QNAP via `docker compose`
Copy the `nas_indexd/` folder to `/share/Container/nas_indexd`, edit volume mounts in `docker-compose.yml`, and run:
```bash
cd /share/Container/nas_indexd && docker compose up -d --build
```

### `nas_indexd` Environment Variables

| Variable | Default | Description |
|---|---|---|
| `INDEX_ROOT` | `/data` | Root directory to index and watch |
| `LISTEN_ADDR` | `0.0.0.0:9876` | HTTP bind address |
| `REFRESH_SECS` | `0` | Periodic safety full-rescan in seconds (`0` = rely on real-time `inotify` only) |
| `API_KEY` | *(empty)* | Optional shared secret checked via `X-Api-Key` header |
| `INDEX_EXCLUDES` | `.@__thumb,@Recycle,.streams,.qpkg,lost+found,.Trash-1000,.snapshot,@Recently-Snapshot` | Comma-separated folder names to skip |

---

## Part 3: Configure FarQsirch in FAR Manager

Open **Settings** via **`F9 → Options → Plugins configuration → FarQsirch Settings`** (or **`Alt+G`** inside the Unified Search dialog):

- **NAS Host / IP (`Alt+H`):** Use your NAS's **IPv4 address** (e.g. `192.168.1.100`) to avoid Windows IPv6 fallback delays.
- **Port (`Alt+P`):** QTS/Qsirch port (`8080` for HTTP, `443` for HTTPS).
- **Username / Password (`Alt+U` / `Alt+W`):** QTS account for Qsirch 7 (or omit password and set `QSIRCH_PASS` env var).
- **SMB / UNC host override (`Alt+N`):** If you browse your NAS by hostname (`\\qnap-nas\Public`) while **NAS Host** is an IPv4 address, enter `qnap-nas` here so FarQsirch recognizes `\\qnap-nas\...` as your NAS and auto-disables SMB walking.
- **Fast `nas_indexd` port (`Alt+I`):** `9876` (or `0` to use Qsirch 7 only).
- Click **`Test Connection` (`Alt+T`)** to verify both `nas_indexd` and Qsirch 7, then **`Save` (`Alt+S`)**.

Config is saved to `far_qsirch.toml` (next to the DLL if writable, otherwise `%APPDATA%\FarQsirch\far_qsirch.toml`). Optional manual share mappings can also be added:

```toml
[[share_mappings]]
nas_prefix = "/Public"
windows_prefix = "P:\\"
```

---

## Keyboard Shortcuts

### Unified Search Dialog
- **Fields:** `Alt+M` Mask, `Alt+T` Containing text
- **Search Area:** `Alt+C` Current folder, `Alt+R` Root of current drive, `Alt+D` All fixed drives, `Alt+V` All drives (local + net)
- **Sources:** `Alt+L` Include Local / SMB walk, `Alt+Q` Include QNAP Qsirch 7, `Alt+A` Case sensitive, `Alt+F` Current NAS folder only
- **Qsirch Mode:** `Alt+X` Text, `Alt+O` Image OCR, `Alt+B` Combined
- **Buttons:** `Enter` / `Alt+S` Search, `Alt+G` Settings, `Esc` / `Alt+N` Cancel

### Results Dialog (Full-Screen)
- **`Enter`** / **`Alt+G`** — Jump active FAR panel directly to the selected file
- **`Alt+P`** — Open all results in a native FAR Virtual Panel (`OPIF_REALNAMES`)
- **`F3`** / **`Alt+V`** — View selected file
- **`F4`** / **`Alt+E`** — Edit selected file
- **`Alt+N`** — New Search
- **`Esc`** / **`Alt+C`** — Close

---

## Troubleshooting

- **QNAP Container Station GUI says "Invalid file format" when importing `.tar`:** Container Station 3's Web UI only imports archives exported by Container Station itself. Load `nas-indexd-qnap.tar` via SSH (`docker load -i /share/Public/nas-indexd-qnap.tar`) instead.
- **Windows Smart App Control / Defender blocks unsigned Rust DLLs:** Add exclusions for `C:\Program Files\Far Manager\Plugins\FarQsirch` in Windows Security, or sign `far_qsirch.dll` per your local policy.
- **Qsirch doesn't return files from a specific share:** Check **Qsirch → Settings → Excluded Folders** and verify the QTS username in FarQsirch Settings has read permission on that shared folder.

## License

[MIT](LICENSE)
