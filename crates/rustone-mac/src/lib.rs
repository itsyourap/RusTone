//! macOS shell integration that needs Objective-C or LaunchServices, kept
//! out of the app crate (which forbids unsafe code): `sip:`/`tel:` links.
//!
//! macOS doesn't pass a clicked link on the command line. It launches the
//! app (or uses the running one) and sends it a GetURL Apple Event, which
//! [`on_open_url`] receives. Which app gets the event is the default handler
//! LaunchServices keeps per scheme ([`default_handler`],
//! [`set_default_handler`]); the schemes themselves are declared in the app
//! bundle's Info.plist.

/// Calls `f` with every link macOS asks the app to open. Call once, early
/// in `main`, before the event loop starts, so a link that launched the app
/// is not missed. Does nothing on other systems.
pub fn on_open_url(f: impl Fn(String) + Send + 'static) {
    #[cfg(target_os = "macos")]
    imp::on_open_url(Box::new(f));
    #[cfg(not(target_os = "macos"))]
    drop(f);
}

/// The bundle identifier of the running app, or `None` when it doesn't run
/// from an app bundle (e.g. `cargo run`).
pub fn bundle_id() -> Option<String> {
    #[cfg(target_os = "macos")]
    return imp::bundle_id();
    #[cfg(not(target_os = "macos"))]
    None
}

/// The bundle identifier of the app that opens `scheme` links.
pub fn default_handler(scheme: &str) -> Option<String> {
    #[cfg(target_os = "macos")]
    return imp::default_handler(scheme);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = scheme;
        None
    }
}

/// Makes the app with `bundle_id` open `scheme` links.
pub fn set_default_handler(scheme: &str, bundle_id: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return imp::set_default_handler(scheme, bundle_id);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (scheme, bundle_id);
        Err("only on macOS".into())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::c_void;

    use objc2::rc::Retained;
    use objc2::runtime::NSObject;
    use objc2::{AllocAnyThread, define_class, msg_send, sel};
    use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager, NSBundle, NSString};

    type Handler = Box<dyn Fn(String) + Send>;

    /// `kInternetEventClass` and `kAEGetURL` are both 'GURL'.
    const GET_URL: u32 = u32::from_be_bytes(*b"GURL");
    /// `keyDirectObject` ('----'): the URL.
    const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "RusToneUrlHandler"]
        #[ivars = Handler]
        struct UrlHandler;

        impl UrlHandler {
            #[unsafe(method(handleGetURLEvent:withReplyEvent:))]
            fn handle_get_url(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
                let url = event.paramDescriptorForKeyword(DIRECT_OBJECT).and_then(|d| d.stringValue());
                if let Some(url) = url {
                    (self.ivars())(url.to_string());
                }
            }
        }
    );

    pub fn on_open_url(f: Handler) {
        let handler = UrlHandler::alloc().set_ivars(f);
        // SAFETY: NSObject's designated initialiser.
        let handler: Retained<UrlHandler> = unsafe { msg_send![super(handler), init] };
        let manager = NSAppleEventManager::sharedAppleEventManager();
        // SAFETY: the selector matches `handle_get_url`'s signature; the
        // handler is kept alive for the rest of the process below, since the
        // event manager doesn't retain it.
        unsafe {
            manager.setEventHandler_andSelector_forEventClass_andEventID(&handler, sel!(handleGetURLEvent:withReplyEvent:), GET_URL, GET_URL);
        }
        std::mem::forget(handler);
    }

    pub fn bundle_id() -> Option<String> {
        NSBundle::mainBundle().bundleIdentifier().map(|id| id.to_string())
    }

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSCopyDefaultHandlerForURLScheme(scheme: *const c_void) -> *const c_void;
        fn LSSetDefaultHandlerForURLScheme(scheme: *const c_void, handler: *const c_void) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *const c_void);
    }

    /// NSString and CFString are toll-free bridged.
    fn cf(s: &NSString) -> *const c_void {
        (s as *const NSString).cast()
    }

    pub fn default_handler(scheme: &str) -> Option<String> {
        let scheme = NSString::from_str(scheme);
        // SAFETY: a valid CFString in; the result follows the Copy rule, so
        // it is read and then released.
        unsafe {
            let handler = LSCopyDefaultHandlerForURLScheme(cf(&scheme));
            if handler.is_null() {
                return None;
            }
            let id = (*handler.cast::<NSString>()).to_string();
            CFRelease(handler);
            Some(id)
        }
    }

    pub fn set_default_handler(scheme: &str, bundle_id: &str) -> Result<(), String> {
        let (scheme_cf, bundle) = (NSString::from_str(scheme), NSString::from_str(bundle_id));
        // SAFETY: two valid CFStrings, borrowed for the call.
        let status = unsafe { LSSetDefaultHandlerForURLScheme(cf(&scheme_cf), cf(&bundle)) };
        if status == 0 { Ok(()) } else { Err(format!("LaunchServices refused {scheme}: links (OSStatus {status})")) }
    }
}
