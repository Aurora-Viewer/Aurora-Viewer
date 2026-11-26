//! iPadOS: no USB HID access, so no 3D mouse. This stands in for the
//! `hidapi` crate with the calls backend.rs makes: it lists no device, and
//! nothing is ever opened or read.

use std::ffi::CStr;

pub struct HidError;

impl std::fmt::Display for HidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("no HID devices on this system")
    }
}

pub struct HidApi;

impl HidApi {
    pub fn new() -> Result<Self, HidError> {
        Ok(Self)
    }

    pub fn reset_devices(&mut self) -> Result<(), HidError> {
        Ok(())
    }

    pub fn add_devices(&mut self, _vendor: u16, _product: u16) -> Result<(), HidError> {
        Ok(())
    }

    pub fn device_list(&self) -> std::iter::Empty<&DeviceInfo> {
        std::iter::empty()
    }

    pub fn open_path(&self, _path: &CStr) -> Result<HidDevice, HidError> {
        Err(HidError)
    }
}

pub struct DeviceInfo;

impl DeviceInfo {
    pub fn path(&self) -> &CStr {
        c""
    }

    pub fn vendor_id(&self) -> u16 {
        0
    }

    pub fn product_id(&self) -> u16 {
        0
    }

    pub fn usage_page(&self) -> u16 {
        0
    }

    pub fn usage(&self) -> u16 {
        0
    }

    pub fn product_string(&self) -> Option<&str> {
        None
    }
}

pub struct HidDevice;

impl HidDevice {
    pub fn read_timeout(&self, _data: &mut [u8], _timeout: i32) -> Result<usize, HidError> {
        Err(HidError)
    }
}
