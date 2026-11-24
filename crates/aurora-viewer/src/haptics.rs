//! A short tick under the finger, where the device can give one (iPadOS,
//! with a Magic Keyboard's trackpad or an Apple Pencil Pro: an iPad itself
//! has no Taptic Engine and stays silent). Nothing elsewhere.

/// A touch button answered, a gesture was taken.
pub fn tap() {
    #[cfg(target_os = "ios")]
    crate::ios::haptic(false);
}

/// A long press opened its menu.
pub fn press() {
    #[cfg(target_os = "ios")]
    crate::ios::haptic(true);
}
