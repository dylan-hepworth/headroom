// Headroom's notifications, through UNUserNotificationCenter.
//
// Tauri's notification plugin goes through the older NSUserNotificationCenter, which macOS has long since deprecated,
// and whose notifications can only be told apart by their title and text. So in the app, notifications go straight to
// UNUserNotificationCenter, with a delegate of our own for clicks.
//
// That center only works for an app signed with a Developer ID. Outside an app bundle (running with `tauri dev`), or
// in a copy built on this Mac without one, main.rs falls back to the plugin, and the plugin's delegate is patched so
// its notifications still show while Headroom is in front and can still be clicked.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    OnceLock,
};

use block2::{DynBlock, RcBlock};
use objc2::AllocAnyThread;
use objc2::{
    define_class, msg_send, rc::Retained, runtime::NSObject, runtime::NSObjectProtocol, runtime::ProtocolObject,
};
use objc2_foundation::{NSArray, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationPresentationOptions,
    UNNotificationRequest, UNNotificationResponse, UNNotificationSound, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};

/// A notification that was clicked.
pub enum Clicked {
    /// One sent through UNUserNotificationCenter, by its ID.
    Id(String),
    /// One sent through the plugin, which macOS only tells us the title and text of.
    Text { title: String, body: String },
}

/// Called when a notification is clicked.
static ON_CLICK: OnceLock<Box<dyn Fn(Clicked) + Send + Sync>> = OnceLock::new();
/// What the plugin's delegate did with a click before Headroom's handler replaced it, if anything.
static PLUGIN_CLICK: OnceLock<Option<usize>> = OnceLock::new();
/// macOS turned down the request to send notifications with an error, rather than the user saying no. It does that
/// for a copy that isn't signed with a Developer ID.
static REFUSED: AtomicBool = AtomicBool::new(false);

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "HeadroomNotifications"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        // Headroom counts as the app in front while one of its windows is, and macOS would hold back its own
        // notifications then; they're shown anyway
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            handler: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            let show = UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound;
            handler.call((show,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            handler: &DynBlock<dyn Fn()>,
        ) {
            let id = response.notification().request().identifier().to_string();
            if let Some(on_click) = ON_CLICK.get() {
                on_click(Clicked::Id(id));
            }
            handler.call(());
        }
    }
);

/// Can notifications go through UNUserNotificationCenter? Not outside an app bundle, where asking for the center
/// raises an Objective-C exception that would take the app down, and not once macOS has refused this copy.
pub fn available() -> bool {
    in_bundle() && !REFUSED.load(Ordering::Relaxed)
}

fn in_bundle() -> bool {
    std::env::current_exe().is_ok_and(|exe| exe.to_string_lossy().contains(".app/Contents/MacOS/"))
}

/// Set up notifications: ask to send them (macOS asks the user the first time), and handle clicks with `on_click`.
pub fn start(on_click: impl Fn(Clicked) + Send + Sync + 'static) {
    let _ = ON_CLICK.set(Box::new(on_click));
    patch_plugin_delegate();
    if !in_bundle() {
        return;
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(), init] };
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    // The center only holds on to its delegate weakly, and this one is needed for as long as Headroom runs
    std::mem::forget(delegate);
    ask();
}

/// Ask to send notifications. macOS only asks the user while they haven't answered; after that it's a no-op.
pub fn ask() {
    if !in_bundle() {
        return;
    }
    let asked = RcBlock::new(|_granted: objc2::runtime::Bool, error: *mut NSError| {
        if !error.is_null() {
            REFUSED.store(true, Ordering::Relaxed);
        }
    });
    UNUserNotificationCenter::currentNotificationCenter().requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &asked,
    );
}

/// Send a notification. `id` is how it's known later, to take it away or to handle a click on it.
pub fn send(id: &str, title: &str, body: &str) {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(id), &content, None);
    UNUserNotificationCenter::currentNotificationCenter().addNotificationRequest_withCompletionHandler(&request, None);
}

/// Take a notification away, from the screen and from Notification Center.
pub fn withdraw(id: &str) {
    let ids = NSArray::from_retained_slice(&[NSString::from_str(id)]);
    UNUserNotificationCenter::currentNotificationCenter().removeDeliveredNotificationsWithIdentifiers(&ids);
}

/// Patch the plugin's delegate (mac-notification-sys's) for the copies that fall back to it.
///
/// macOS asks the older notification center's delegate whether to show a notification from the frontmost app, which
/// Headroom is whenever Settings is open, and hides it unless the delegate says yes. The plugin's delegate doesn't
/// answer, so the answer is added to its class. Without it, "Send Test Alert" would do nothing you could see. Clicks go
/// to the same delegate: what it did before still runs, then `on_click` hears about it.
fn patch_plugin_delegate() {
    use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};

    extern "C-unwind" fn always(_: &AnyObject, _: Sel, _: &AnyObject, _: &AnyObject) -> Bool {
        Bool::YES
    }
    // The method's type: returns a BOOL, and takes self, the selector, and two objects
    #[cfg(target_arch = "aarch64")]
    let types = c"B@:@@";
    #[cfg(not(target_arch = "aarch64"))]
    let types = c"c@:@@";

    let Some(class) = AnyClass::get(c"NotificationCenterDelegate") else { return };
    // SAFETY: `always` and `plugin_clicked` have the signatures their type strings describe
    unsafe {
        let imp: Imp = std::mem::transmute(always as extern "C-unwind" fn(_, _, _, _) -> _);
        objc2::ffi::class_addMethod(
            class as *const AnyClass as *mut AnyClass,
            objc2::sel!(userNotificationCenter:shouldPresentNotification:),
            imp,
            types.as_ptr(),
        );
        let clicked: Imp = std::mem::transmute(plugin_clicked as extern "C-unwind" fn(_, _, _, _));
        let before = objc2::ffi::class_replaceMethod(
            class as *const AnyClass as *mut AnyClass,
            objc2::sel!(userNotificationCenter:didActivateNotification:),
            clicked,
            c"v@:@@".as_ptr(),
        );
        let _ = PLUGIN_CLICK.set(before.map(|imp| imp as usize));
    }
}

extern "C-unwind" fn plugin_clicked(
    this: &objc2::runtime::AnyObject,
    cmd: objc2::runtime::Sel,
    center: &objc2::runtime::AnyObject,
    notification: &objc2::runtime::AnyObject,
) {
    use objc2::runtime::{AnyObject, Sel};
    if let Some(Some(before)) = PLUGIN_CLICK.get() {
        // SAFETY: it was the method's implementation, with this same signature, before it was replaced
        let before: extern "C-unwind" fn(&AnyObject, Sel, &AnyObject, &AnyObject) =
            unsafe { std::mem::transmute(*before) };
        before(this, cmd, center, notification);
    }
    // SAFETY: an NSUserNotification's title and informativeText, checked for nil
    let (title, body) = unsafe {
        let title: *mut NSString = msg_send![notification, title];
        let body: *mut NSString = msg_send![notification, informativeText];
        (title.as_ref().map(|t| t.to_string()), body.as_ref().map(|b| b.to_string()))
    };
    if let (Some(on_click), Some(title)) = (ON_CLICK.get(), title) {
        on_click(Clicked::Text { title, body: body.unwrap_or_default() });
    }
}

/// Take away a notification the plugin sent, from the screen and from Notification Center. It's found by its title and
/// text, delivered within a few seconds of `sent` (seconds since 1970), so another with the same words sent later
/// stays. Call it on the main thread.
pub fn withdraw_plugin(title: &str, body: &str, sent: f64) {
    use objc2::runtime::{AnyClass, AnyObject};

    let Some(class) = AnyClass::get(c"NSUserNotificationCenter") else { return };
    // SAFETY: these are NSUserNotificationCenter's and NSUserNotification's documented methods, and every object is
    // checked for nil before it's used
    unsafe {
        let center: *mut AnyObject = msg_send![class, defaultUserNotificationCenter];
        let Some(center) = center.as_ref() else { return };
        let delivered: *mut AnyObject = msg_send![center, deliveredNotifications];
        let Some(delivered) = delivered.as_ref() else { return };
        let count: usize = msg_send![delivered, count];
        for i in (0..count).rev() {
            let alert: *mut AnyObject = msg_send![delivered, objectAtIndex: i];
            let Some(alert) = alert.as_ref() else { continue };
            let its_title: *mut NSString = msg_send![alert, title];
            let its_body: *mut NSString = msg_send![alert, informativeText];
            let date: *mut AnyObject = msg_send![alert, actualDeliveryDate];
            let at: f64 = match date.as_ref() {
                Some(date) => msg_send![date, timeIntervalSince1970],
                None => 0.0,
            };
            let same = its_title.as_ref().is_some_and(|t| t.to_string() == title)
                && its_body.as_ref().map_or(body.is_empty(), |b| b.to_string() == body);
            if same && (sent - 1.0..sent + 3.0).contains(&at) {
                let _: () = msg_send![center, removeDeliveredNotification: alert];
            }
        }
    }
}
