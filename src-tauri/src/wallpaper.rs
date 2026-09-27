// The desktop wallpaper, for the menu bar preview in Settings, so it looks like your own menu bar.
//
// For a picture, macOS tells us the file. For an aerial (the slow-motion videos macOS offers as wallpapers), it
// doesn't: asking gets the default wallpaper instead. So for those we read which aerial is set from the wallpaper
// store, and take a still from its video with Quick Look. Either way the result is shrunk to a small JPEG and cached,
// so it's only made once per wallpaper.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use base64::Engine;

/// How wide the cached copy is, in pixels. The preview is under 600 points wide, so this is sharp on a Retina screen.
const WIDTH: u32 = 1200;

/// The wallpaper as a `data:` URL the settings window can show, or `None` if we couldn't get one. `picture` is the
/// file macOS reports for the desktop (see `Screen`), which is right unless the wallpaper is an aerial.
pub fn data_url(cache_dir: &Path, picture: Option<PathBuf>) -> Option<String> {
    let (key, source) = match current_aerial() {
        Some(id) => (format!("aerial-{id}"), aerial_still(&id, cache_dir)?),
        None => {
            let path = picture?;
            let modified = fs::metadata(&path).and_then(|m| m.modified()).ok()?;
            let stamp = modified.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
            (format!("picture-{:x}-{stamp}", hash(&path.to_string_lossy())), path)
        }
    };

    let jpeg = cache_dir.join(format!("wallpaper-{key}.jpg"));
    if !jpeg.exists() {
        // Only one cached wallpaper is kept, the current one
        clear_old(cache_dir, "wallpaper-");
        let resized = Command::new("sips")
            .args(["-s", "format", "jpeg", "-s", "formatOptions", "80", "-Z", &WIDTH.to_string()])
            .arg(&source)
            .arg("--out")
            .arg(&jpeg)
            .output()
            .ok()?;
        if !resized.status.success() {
            return None;
        }
    }
    let bytes = fs::read(&jpeg).ok()?;
    Some(format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

fn wallpaper_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/com.apple.wallpaper"))
}

/// The asset ID of the aerial set as the desktop wallpaper, if it's an aerial.
fn current_aerial() -> Option<String> {
    let index = plist::Value::from_file(wallpaper_dir()?.join("Store/Index.plist")).ok()?;
    let choice = index
        .as_dictionary()?
        .get("AllSpacesAndDisplays")?
        .as_dictionary()?
        .get("Desktop")?
        .as_dictionary()?
        .get("Content")?
        .as_dictionary()?
        .get("Choices")?
        .as_array()?
        .first()?
        .as_dictionary()?;
    if choice.get("Provider")?.as_string()? != "com.apple.wallpaper.choice.aerials" {
        return None;
    }
    // The aerial's settings are a plist of their own, stored inside this one
    let config = plist::Value::from_reader(std::io::Cursor::new(choice.get("Configuration")?.as_data()?)).ok()?;
    Some(config.as_dictionary()?.get("assetID")?.as_string()?.to_string())
}

/// A still from the aerial's video, made with Quick Look. If the video hasn't been downloaded, macOS still keeps a
/// small thumbnail of it, which is better than nothing.
fn aerial_still(id: &str, cache_dir: &Path) -> Option<PathBuf> {
    let aerials = wallpaper_dir()?.join("aerials");
    let video = aerials.join(format!("videos/{id}.mov"));
    if video.exists() {
        let out = cache_dir.join("quicklook");
        let _ = fs::create_dir_all(&out);
        let made = Command::new("qlmanage")
            .args(["-t", "-s", &WIDTH.to_string(), "-o"])
            .arg(&out)
            .arg(&video)
            .output()
            .is_ok_and(|o| o.status.success());
        let still = out.join(format!("{id}.mov.png"));
        if made && still.exists() {
            return Some(still);
        }
    }
    Some(aerials.join(format!("thumbnails/{id}.png"))).filter(|p| p.exists())
}

/// What the preview needs to know about the main screen, so it can show the wallpaper the size it really is.
pub struct Screen {
    /// The file macOS reports as the desktop picture.
    pub picture: Option<PathBuf>,
    /// The screen's size, in points.
    pub width: f64,
    pub height: f64,
    /// How tall the menu bar is, in points. Taller on MacBooks with a notch.
    pub menu_bar: f64,
}

/// Look at the main screen. AppKit only answers this on the main thread.
pub fn main_screen() -> Option<Screen> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSScreen, NSWorkspace};

    let mtm = MainThreadMarker::new()?;
    let screen = NSScreen::mainScreen(mtm)?;
    let frame = screen.frame();
    let visible = screen.visibleFrame();
    let picture = NSWorkspace::sharedWorkspace()
        .desktopImageURLForScreen(&screen)
        .and_then(|url| url.path())
        .map(|path| PathBuf::from(path.to_string()));
    Some(Screen {
        picture,
        width: frame.size.width,
        height: frame.size.height,
        // The visible frame leaves out the menu bar at the top (and the Dock, wherever it is)
        menu_bar: (frame.origin.y + frame.size.height) - (visible.origin.y + visible.size.height),
    })
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
