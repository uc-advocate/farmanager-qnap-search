//! Native FAR Manager 3 Virtual Panel (`OPIF_REALNAMES`) for unified search results.
//!
//! Because `OPIF_REALNAMES` is set and each item's `FileName` is a full Windows path
//! (local path, mapped drive, or `\\NAS\Share\...`), FAR natively supports `F3` View,
//! `F4` Edit, `F5` Copy, `F8` Delete, `Enter` Execute, and `Ctrl+PgUp` Go to folder.

use crate::far_ffi::*;
use crate::merger::UnifiedResult;
use std::collections::HashMap;
use windows_sys::Win32::Foundation::HANDLE;

struct AllocatedList {
    _strings: Vec<WideString>,
    _items: Box<[PluginPanelItem]>,
}

pub struct SearchPanel {
    title: WideString,
    format: WideString,
    cur_dir: WideString,
    results: Vec<UnifiedResult>,
    active_lists: HashMap<usize, AllocatedList>,
}

impl SearchPanel {
    pub fn new(query_summary: &str, results: Vec<UnifiedResult>) -> Box<Self> {
        let title_str = format!(" Qsirch Search: {} ({} items) ", query_summary, results.len());
        Box::new(Self {
            title: WideString::new(&title_str),
            format: WideString::new("Qsirch"),
            cur_dir: WideString::new(""),
            results,
            active_lists: HashMap::new(),
        })
    }

    pub fn into_handle(self: Box<Self>) -> HANDLE {
        Box::into_raw(self) as HANDLE
    }

    pub unsafe fn from_handle_mut<'a>(handle: HANDLE) -> Option<&'a mut Self> {
        if handle.is_null() {
            None
        } else {
            Some(&mut *(handle as *mut Self))
        }
    }

    pub unsafe fn drop_handle(handle: HANDLE) {
        if !handle.is_null() {
            let _ = Box::from_raw(handle as *mut Self);
        }
    }

    pub fn fill_open_panel_info(&self, info: &mut OpenPanelInfo) {
        info.Flags = OPIF_REALNAMES | OPIF_ADDDOTS;
        info.HostFile = std::ptr::null();
        info.CurDir = self.cur_dir.as_ptr();
        info.Format = self.format.as_ptr();
        info.PanelTitle = self.title.as_ptr();
        info.StartSortMode = SM_UNSORTED;
        info.StartSortOrder = 0;
    }

    pub fn fill_find_data(&mut self, info: &mut GetFindDataInfo) -> isize {
        let mut strings: Vec<WideString> = Vec::with_capacity(self.results.len() * 2);
        let mut items: Vec<PluginPanelItem> = Vec::with_capacity(self.results.len());

        for r in &self.results {
            let w_path = WideString::new(&r.windows_path);
            let desc_text = if r.snippet.is_empty() {
                r.source.tag().trim().to_string()
            } else {
                format!("{} {}", r.source.tag().trim(), r.snippet)
            };
            let w_desc = WideString::new(&desc_text);

            let path_ptr = w_path.as_ptr();
            let desc_ptr = w_desc.as_ptr();
            strings.push(w_path);
            strings.push(w_desc);

            let ft = unix_secs_to_filetime(r.modified_epoch);
            items.push(PluginPanelItem {
                CreationTime: ft,
                LastAccessTime: ft,
                LastWriteTime: ft,
                ChangeTime: ft,
                FileSize: r.size,
                AllocationSize: r.size,
                FileName: path_ptr,
                AlternateFileName: std::ptr::null(),
                Description: desc_ptr,
                Owner: std::ptr::null(),
                CustomColumnData: std::ptr::null(),
                CustomColumnNumber: 0,
                Flags: 0,
                UserData: UserDataItem::default(),
                FileAttributes: r.attributes as usize,
                NumberOfLinks: 1,
                CRC32: 0,
                Reserved: [0; 2],
            });
        }

        let mut boxed = items.into_boxed_slice();
        let ptr = boxed.as_mut_ptr();
        let count = boxed.len();

        self.active_lists.insert(
            ptr as usize,
            AllocatedList {
                _strings: strings,
                _items: boxed,
            },
        );

        info.PanelItem = ptr;
        info.ItemsNumber = count;
        1
    }

    pub fn free_find_data(&mut self, ptr: *mut PluginPanelItem) {
        if !ptr.is_null() {
            let _ = self.active_lists.remove(&(ptr as usize));
        }
    }
}
