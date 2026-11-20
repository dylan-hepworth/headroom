// The desktop wallpaper, for the taskbar preview in Settings, so it looks like your own taskbar. Windows says which
// picture it is; it's shrunk to a small JPEG and cached, so it's only made once per wallpaper.

use std::{
    fs,
    path::{Path, PathBuf},
};

use base64::Engine;

/// How wide the cached copy is, in pixels.
const WIDTH: u32 = 1200;

/// The wallpaper as a `data:` URL the settings window can show, or `None` if we couldn't get one.
pub fn data_url(cache_dir: &Path, picture: Option<PathBuf>) -> Option<String> {
    let path = picture?;
    let modified = fs::metadata(&path).and_then(|m| m.modified()).ok()?;
    let stamp = modified.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let key = format!("picture-{:x}-{stamp}", hash(&path.to_string_lossy()));
    let jpeg = cache_dir.join(format!("wallpaper-{key}.jpg"));
    if !jpeg.exists() {
        // Only one cached wallpaper is kept, the current one
        clear_old(cache_dir, "wallpaper-");
        let small = image::open(&path).ok()?.resize(WIDTH, WIDTH, image::imageops::FilterType::Triangle).to_rgb8();
        small.save_with_format(&jpeg, image::ImageFormat::Jpeg).ok()?;
    }
    let bytes = fs::read(&jpeg).ok()?;
    Some(format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// What the preview needs to know about the main screen, so it can show the wallpaper the size it really is.
pub struct Screen {
    /// The wallpaper's file.
    pub picture: Option<PathBuf>,
    /// The screen's size, in logical pixels.
    pub width: f64,
    pub height: f64,
    /// How tall the taskbar is, in logical pixels. Called the menu bar to match the Mac's.
    pub menu_bar: f64,
}

/// Look at the main screen.
pub fn main_screen() -> Option<Screen> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SystemParametersInfoW, SM_CXSCREEN, SM_CYSCREEN, SPI_GETDESKWALLPAPER, SPI_GETWORKAREA,
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };
    let mut buf = [0u16; 1024];
    let mut work = windows::Win32::Foundation::RECT::default();
    // SAFETY: buffers sized for what each call writes
    let (picture, width, height) = unsafe {
        let got = SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            buf.len() as u32,
            Some(buf.as_mut_ptr().cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some((&mut work as *mut windows::Win32::Foundation::RECT).cast()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let path = String::from_utf16_lossy(&buf[..len]);
        let picture = (got.is_ok() && !path.is_empty()).then(|| PathBuf::from(path));
        (picture, GetSystemMetrics(SM_CXSCREEN) as f64, GetSystemMetrics(SM_CYSCREEN) as f64)
    };
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some(Screen { picture, width, height, menu_bar: (height - work.bottom as f64).max(0.0) })
}

fn clear_old(dir: &Path, prefix: &str) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(prefix) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// A short, stable name for a path, for the cache file.
fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}
