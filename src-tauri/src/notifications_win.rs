// Headroom's notifications on Windows, as toasts.
//
// Tauri's notification plugin can show a toast but can't give it buttons or say when it's clicked, so Headroom sends
// its own, with the same calls as on a Mac (see notifications.rs). A toast needs an app ID that Windows knows: the
// installer's Start menu shortcut carries one, and Headroom also registers it itself, with its name and icon, so a
// copy that wasn't installed (a build run from its folder) can send them too.
//
// Clicks come back while Headroom is running, which it is whenever it sends one. A toast clicked later, from the
// notification center after Headroom has quit, just opens nothing.

use std::sync::{Mutex, OnceLock};

use windows::{
    core::{IInspectable, Interface, HSTRING, PCWSTR},
    Data::Xml::Dom::XmlDocument,
    Foundation::TypedEventHandler,
    UI::Notifications::{ToastActivatedEventArgs, ToastNotification, ToastNotificationManager},
    Win32::{
        System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ},
        UI::Shell::SetCurrentProcessExplicitAppUserModelID,
    },
};

use crate::platform::APP_ID;

/// A notification that was clicked, or one of its buttons. Windows always says which by ID, so there's no `Text` kind
/// as there is on a Mac; it's here so the code that handles clicks is the same on both.
#[allow(dead_code)]
pub enum Clicked {
    Id(String, Option<String>),
    Text { title: String, body: String },
}

/// Called when a notification is clicked.
static ON_CLICK: OnceLock<Box<dyn Fn(Clicked) + Send + Sync>> = OnceLock::new();

/// The buttons for each kind of notification, by the kind's name (see `kinds`).
static KINDS: Mutex<Vec<(String, Vec<(String, String)>)>> = Mutex::new(vec![]);

/// The toasts that are out, by ID, kept so their click handlers stay alive until they're taken away.
static OUT: Mutex<Vec<(String, ToastNotification)>> = Mutex::new(vec![]);

const GROUP: &str = "headroom";

/// Toasts always go through here on Windows; there's no older center to fall back to.
pub fn available() -> bool {
    true
}

/// Set up notifications: tell Windows who Headroom is, and handle clicks with `on_click`.
pub fn start(on_click: impl Fn(Clicked) + Send + Sync + 'static) {
    let _ = ON_CLICK.set(Box::new(on_click));
    register();
}

/// Windows doesn't ask the user before an app's first notification; they're on until turned off in Settings.
pub fn ask() {}

/// Give Windows Headroom's app ID, name and icon, so its toasts say they're from Headroom.
fn register() {
    let icon = crate::platform::app_dir().map(|d| d.join("notification-icon.png"));
    if let Some(icon) = &icon {
        let _ = std::fs::create_dir_all(icon.parent().unwrap_or(icon));
        let _ = std::fs::write(icon, include_bytes!("../icons/128x128@2x.png"));
    }
    let key = HSTRING::from(format!("Software\\Classes\\AppUserModelId\\{APP_ID}"));
    let set = |name: &str, value: &str| {
        let data: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: a NUL-terminated wide string, with its size in bytes
        unsafe {
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                &key,
                &HSTRING::from(name),
                REG_SZ.0,
                Some(data.as_ptr().cast()),
                (data.len() * 2) as u32,
            );
        }
    };
    set("DisplayName", "Headroom");
    if let Some(icon) = icon {
        set("IconUri", &icon.display().to_string());
    }
    // SAFETY: a string that outlives the call
    unsafe {
        let id = HSTRING::from(APP_ID);
        let _ = SetCurrentProcessExplicitAppUserModelID(PCWSTR(id.as_ptr()));
    }
}

/// The kinds of notification that have buttons: each kind's name, and its buttons' IDs and words.
pub fn kinds(kinds: &[(String, Vec<(String, String)>)]) {
    *KINDS.lock().unwrap() = kinds.to_vec();
}

/// Text made safe to put in the toast's XML.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Send a notification. `id` is how it's known later, to take it away or to handle a click on it. A `picture` shows
/// beside the text. With a `kind` (see `kinds`), it gets that kind's buttons.
pub fn send(id: &str, title: &str, body: &str, picture: Option<&std::path::Path>, kind: Option<&str>) {
    let image = picture.map_or(String::new(), |p| {
        format!(r#"<image placement="appLogoOverride" hint-crop="circle" src="{}"/>"#, escape(&p.display().to_string()))
    });
    let buttons: String = kind
        .and_then(|kind| KINDS.lock().unwrap().iter().find(|(name, _)| name == kind).map(|(_, b)| b.clone()))
        .unwrap_or_default()
        .iter()
        .map(|(button, words)| {
            format!(r#"<action content="{}" arguments="{}" activationType="foreground"/>"#, escape(words), escape(&format!("{id}\n{button}")))
        })
        .collect();
    let xml = format!(
        r#"<toast launch="{}" activationType="foreground"><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text>{image}</binding></visual><actions>{buttons}</actions></toast>"#,
        escape(id),
        escape(title),
        escape(body)
    );
    let shown = (|| -> windows::core::Result<()> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(xml))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;
        // A tag is at most 64 characters
        toast.SetTag(&HSTRING::from(tag(id)))?;
        toast.SetGroup(&HSTRING::from(GROUP))?;
        toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(|_, args| {
            let Some(args) = args.as_ref() else { return Ok(()) };
            let said = args.cast::<ToastActivatedEventArgs>()?.Arguments()?.to_string();
            let (id, button) = match said.split_once('\n') {
                Some((id, button)) => (id.to_string(), Some(button.to_string())),
                None => (said, None),
            };
            if let Some(on_click) = ON_CLICK.get() {
                on_click(Clicked::Id(id, button));
            }
            Ok(())
        }))?;
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))?.Show(&toast)?;
        let mut out = OUT.lock().unwrap();
        out.push((id.to_string(), toast));
        // The last few dozen are plenty to keep listening to
        if out.len() > 40 {
            out.remove(0);
        }
        Ok(())
    })();
    if let Err(e) = shown {
        crate::platform::note(&format!("Couldn't show a notification: {e}"));
    }
}

fn tag(id: &str) -> String {
    id.chars().rev().take(64).collect::<Vec<_>>().into_iter().rev().collect()
}

/// Take a notification away, from the screen and from the notification center.
pub fn withdraw(id: &str) {
    OUT.lock().unwrap().retain(|(out, _)| out != id);
    if let Ok(history) = ToastNotificationManager::History() {
        let _ = history.RemoveGroupedTagWithId(&HSTRING::from(tag(id)), &HSTRING::from(GROUP), &HSTRING::from(APP_ID));
    }
}

/// Only for the plugin's notifications on a Mac, which Windows never sends.
pub fn withdraw_plugin(_title: &str, _body: &str, _sent: f64) {}
