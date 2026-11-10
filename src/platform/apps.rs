//! Installed-program details: running processes, display names and icons.

use std::collections::HashSet;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::UI::Shell::SHDefExtractIconW;
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, EnumWindows, GetIconInfo, GetWindowTextLengthW, GetWindowThreadProcessId, HICON, ICONINFO, IsWindowVisible,
};
use windows::core::{BOOL, HSTRING, w};

const ICON_PIXELS: u32 = 64;

pub struct RunningApp {
    pub path: PathBuf,
    /// Has a visible window, i.e. the user sees it as an open application.
    pub windowed: bool,
}

pub struct Icon {
    pub width: usize,
    pub height: usize,
    /// Unmultiplied RGBA, row by row from the top.
    pub rgba: Vec<u8>,
}

/// Running programs outside the Windows directory, one entry per executable.
pub fn running() -> Vec<RunningApp> {
    let refresh = ProcessRefreshKind::nothing().with_exe(sysinfo::UpdateKind::Always);
    let Ok(mut system) = System::new() else { return Vec::new() };
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);
    let windowed = processes_with_windows();
    // Windows reports the same directory as both C:\Windows and C:\WINDOWS.
    let windows_dir = std::env::var_os("SystemRoot").map(|dir| dir.to_string_lossy().to_lowercase());
    let mut apps: Vec<RunningApp> = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let path = process.exe()?.to_path_buf();
            Some(RunningApp { windowed: windowed.contains(&pid.as_u32()), path })
        })
        .filter(|app| {
            !windows_dir.as_ref().is_some_and(|dir| app.path.to_string_lossy().to_lowercase().starts_with(dir.as_str()))
        })
        .collect();
    // Windowed entries first, so that deduplicating by path keeps the "open" flag.
    apps.sort_by(|a, b| a.path.cmp(&b.path).then(b.windowed.cmp(&a.windowed)));
    apps.dedup_by(|a, b| a.path == b.path);
    apps
}

fn processes_with_windows() -> HashSet<u32> {
    unsafe extern "system" fn collect(window: HWND, context: LPARAM) -> BOOL {
        unsafe {
            if IsWindowVisible(window).as_bool() && GetWindowTextLengthW(window) > 0 {
                let mut pid = 0u32;
                GetWindowThreadProcessId(window, Some(&mut pid));
                (*(context.0 as *mut HashSet<u32>)).insert(pid);
            }
        }
        true.into()
    }
    let mut pids = HashSet::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&raw mut pids as isize));
    }
    pids
}

/// The FileDescription version field ("Google Chrome" for chrome.exe), or else the file name.
pub fn display_name(path: &Path) -> String {
    unsafe { description(&HSTRING::from(path.as_os_str())) }
        .unwrap_or_else(|| path.file_stem().map_or_else(|| path.display().to_string(), |stem| stem.to_string_lossy().into_owned()))
}

unsafe fn description(path: &HSTRING) -> Option<String> {
    unsafe {
        let size = GetFileVersionInfoSizeW(path, None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u32; (size as usize).div_ceil(4)];
        let block: *const c_void = data.as_ptr().cast();
        GetFileVersionInfoW(path, None, size, data.as_mut_ptr().cast()).ok()?;

        let mut value: *mut c_void = std::ptr::null_mut();
        let mut length = 0u32;
        if !VerQueryValueW(block, w!("\\VarFileInfo\\Translation"), &mut value, &mut length).as_bool() || length < 4 {
            return None;
        }
        let translation = value.cast::<u16>();
        let key = format!("\\StringFileInfo\\{:04x}{:04x}\\FileDescription", *translation, *translation.add(1));
        if !VerQueryValueW(block, &HSTRING::from(key), &mut value, &mut length).as_bool() || length == 0 {
            return None;
        }
        let text = std::slice::from_raw_parts(value.cast::<u16>(), length as usize);
        let end = text.iter().position(|unit| *unit == 0).unwrap_or(text.len());
        let name = String::from_utf16_lossy(&text[..end]).trim().to_owned();
        (!name.is_empty()).then_some(name)
    }
}

pub fn icon(path: &Path) -> Option<Icon> {
    let path = HSTRING::from(path.as_os_str());
    unsafe {
        let mut handle = HICON::default();
        SHDefExtractIconW(&path, 0, 0, Some(&mut handle), None, ICON_PIXELS).ok().ok()?;
        let mut info = ICONINFO::default();
        let result = GetIconInfo(handle, &mut info).ok().and_then(|_| pixels(info.hbmColor));
        let _ = DeleteObject(info.hbmColor.into());
        let _ = DeleteObject(info.hbmMask.into());
        let _ = DestroyIcon(handle);
        result
    }
}

unsafe fn pixels(bitmap: HBITMAP) -> Option<Icon> {
    unsafe {
        let mut described = BITMAP::default();
        let copied = GetObjectW(bitmap.into(), size_of::<BITMAP>() as i32, Some((&raw mut described).cast()));
        if copied == 0 || described.bmWidth <= 0 || described.bmHeight <= 0 {
            return None;
        }
        let (width, height) = (described.bmWidth, described.bmHeight);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut rgba = vec![0u8; width as usize * height as usize * 4];
        let dc = GetDC(None);
        let lines = GetDIBits(dc, bitmap, 0, height as u32, Some(rgba.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS);
        ReleaseDC(None, dc);
        if lines == 0 {
            return None;
        }
        // Legacy icons carry no alpha channel at all.
        let opaque = rgba.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0);
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
            if opaque {
                pixel[3] = 255;
            }
        }
        Some(Icon { width: width as usize, height: height as usize, rgba })
    }
}
