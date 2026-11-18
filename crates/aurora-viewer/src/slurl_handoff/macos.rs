//! macOS: Launch Services gives a link to the application itself, the one
//! already running or the one it has just started, as a `kAEGetURL` Apple
//! event; no launch carries it on its command line and nothing is handed
//! over between processes. The Legacy Viewer receives it the same way
//! (handleGetURLEvent:withReplyEvent:, indra/newview/llappdelegate-objc.mm,
//! originally LGPL 2.1). winit owns the application delegate and has no
//! event for it, so the handler is set on the Apple event manager, before
//! the application finishes launching: the link that started the viewer is
//! sent during that step.

use super::{MAX_URL, Receiver};
use crossbeam_channel::Sender;
use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{AnyThread, DefinedClass, define_class, msg_send, sel};
use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager};

/// `kInternetEventClass` and its `kAEGetURL`, the same four characters.
const GET_URL: u32 = u32::from_be_bytes(*b"GURL");
/// `keyDirectObject`: the event's parameter that holds the link.
const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and the class has
    // no Drop.
    #[unsafe(super(NSObject))]
    #[name = "AuroraLinkReceiver"]
    #[ivars = Sender<String>]
    struct LinkReceiver;

    impl LinkReceiver {
        #[unsafe(method(handleGetURLEvent:withReplyEvent:))]
        fn handle_get_url(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            // SAFETY: the method takes a four-character code and returns a
            // descriptor or nil. Sent by hand: objc2-foundation only has it
            // with objc2-core-services, which is not in the tree.
            let link: Option<Retained<NSAppleEventDescriptor>> = unsafe { msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT] };
            let Some(link) = link.and_then(|link| link.stringValue()) else {
                return;
            };
            let link = link.to_string();
            let link = link.trim();
            if !link.is_empty() && link.len() <= MAX_URL {
                // a viewer that no longer reads them is closing
                let _ = self.ivars().send(link.to_owned());
            }
        }
    }
);

impl LinkReceiver {
    fn new(links: Sender<String>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(links);
        // SAFETY: NSObject's init, on an object whose instance variables
        // are set.
        unsafe { msg_send![super(this), init] }
    }
}

/// Receives the links macOS opens with this viewer, the one that started
/// it included. Called before the event loop runs.
pub fn listen() -> Option<Receiver<String>> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let receiver = LinkReceiver::new(tx);
    let manager = NSAppleEventManager::sharedAppleEventManager();
    // SAFETY: the receiver answers the selector with the two descriptors
    // the manager sends; the classes are four-character codes. Sent by
    // hand for the same reason as above.
    let _: () = unsafe {
        msg_send![
            &*manager,
            setEventHandler: &*receiver,
            andSelector: sel!(handleGetURLEvent:withReplyEvent:),
            forEventClass: GET_URL,
            andEventID: GET_URL,
        ]
    };
    // the manager does not retain its handler: kept for the process's life
    let _ = Retained::into_raw(receiver);
    Some(rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2::ClassType;
    use objc2_foundation::NSString;

    /// The event Launch Services sends for `link`.
    fn get_url_event(link: &str) -> Retained<NSAppleEventDescriptor> {
        // SAFETY: the class method takes two four-character codes, an
        // optional target, kAutoGenerateReturnID and kAnyTransactionID.
        let event: Retained<NSAppleEventDescriptor> = unsafe {
            msg_send![
                NSAppleEventDescriptor::class(),
                appleEventWithEventClass: GET_URL,
                eventID: GET_URL,
                targetDescriptor: None::<&NSAppleEventDescriptor>,
                returnID: -1i16,
                transactionID: 0i32,
            ]
        };
        let text = NSAppleEventDescriptor::descriptorWithString(&NSString::from_str(link));
        // SAFETY: a descriptor and its four-character keyword.
        let _: () = unsafe { msg_send![&*event, setParamDescriptor: &*text, forKeyword: DIRECT_OBJECT] };
        event
    }

    #[test]
    fn a_get_url_event_gives_its_link() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let receiver = LinkReceiver::new(tx);
        let reply = NSAppleEventDescriptor::nullDescriptor();
        let send = |event: &NSAppleEventDescriptor| {
            // SAFETY: the method defined above, with its two descriptors.
            let _: () = unsafe { msg_send![&*receiver, handleGetURLEvent: event, withReplyEvent: &*reply] };
        };
        let link = "secondlife://Test%20Region/128/128/20";
        send(&get_url_event(&format!("  {link}\n")));
        assert_eq!(rx.try_recv().as_deref(), Ok(link));
        // no link in the event, an empty one, one longer than any SLURL
        send(&reply);
        send(&get_url_event(" "));
        send(&get_url_event(&format!("secondlife://{}", "a".repeat(MAX_URL))));
        assert!(rx.try_recv().is_err());
        // the viewer closed: nothing to do with it, and no panic
        drop(rx);
        send(&get_url_event(link));
    }
}
