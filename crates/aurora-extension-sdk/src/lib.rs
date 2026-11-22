//! Developer helpers for implementing Aurora's serialized extension guest API.

pub use aurora_extension_api::*;

use std::fmt;

/// A guest callback receives one typed API event and returns bounded declarative actions.
pub trait ExtensionHandler {
    /// Handle one lifecycle or viewer event. Host capabilities are checked after this returns.
    fn on_event(&mut self, event: &ExtensionEvent) -> Vec<ExtensionAction>;
}

/// An SDK error raised while parsing, handling or serializing one request.
#[derive(Debug)]
pub enum SdkError {
    /// The guest input was malformed, oversized or used an incompatible API version.
    InvalidRequest,
    /// The handler returned an action that violates the stable API contract.
    InvalidResponse,
    /// The response could not be serialized.
    Serialization,
}

impl fmt::Display for SdkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidRequest => "invalid extension request",
            Self::InvalidResponse => "invalid extension response",
            Self::Serialization => "extension response serialization failed",
        })
    }
}

impl std::error::Error for SdkError {}

/// Decode one host request, call a handler and return a validated JSON response.
pub fn handle_request<H: ExtensionHandler>(handler: &mut H, input: &[u8]) -> Result<Vec<u8>, SdkError> {
    if input.len() > MAX_ENVELOPE_BYTES {
        return Err(SdkError::InvalidRequest);
    }
    let request: ExtensionRequest = serde_json::from_slice(input).map_err(|_| SdkError::InvalidRequest)?;
    request.validate().map_err(|_| SdkError::InvalidRequest)?;
    let response = ExtensionResponse {
        api_version: API_VERSION,
        request_id: request.request_id,
        actions: handler.on_event(&request.event),
    };
    response.validate_for(&request).map_err(|_| SdkError::InvalidResponse)?;
    let output = serde_json::to_vec(&response).map_err(|_| SdkError::Serialization)?;
    if output.len() > MAX_ENVELOPE_BYTES {
        return Err(SdkError::InvalidResponse);
    }
    Ok(output)
}

/// Build the v1 WASM exports `alloc` and `handle` for a stateless guest handler.
///
/// The host creates a fresh Wasmi instance for each event, so the handler type must
/// be `Default` and must not rely on guest memory persisting between invocations.
#[macro_export]
macro_rules! export_extension {
    ($handler:ty) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn alloc(size: i32) -> i32 {
            $crate::guest_alloc(size)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn handle(input_ptr: i32, input_len: i32, output_ptr: i32, output_capacity: i32) -> i32 {
            $crate::guest_handle::<$handler>(input_ptr, input_len, output_ptr, output_capacity)
        }
    };
}

/// Allocate guest linear memory for the host ABI. Exported by [`export_extension!`].
#[doc(hidden)]
#[cfg(target_arch = "wasm32")]
pub fn guest_alloc(size: i32) -> i32 {
    if size < 0 {
        return -1;
    }
    let Some(aligned) = (size as usize).checked_add(7).map(|value| value & !7) else {
        return -1;
    };
    let cursor_ptr = core::ptr::addr_of_mut!(HEAP_CURSOR);
    // SAFETY: WebAssembly guest execution is single-threaded; this module-local
    // cursor is accessed only by the exported allocator.
    let mut cursor = unsafe { cursor_ptr.read() };
    if cursor == 0 {
        cursor = core::arch::wasm32::memory_size(0).saturating_mul(65_536);
    }
    let Some(next) = cursor.checked_add(aligned) else {
        return -1;
    };
    let required_pages = next.div_ceil(65_536);
    let current_pages = core::arch::wasm32::memory_size(0);
    if required_pages > current_pages && core::arch::wasm32::memory_grow(0, required_pages - current_pages) == usize::MAX {
        return -1;
    }
    // SAFETY: same single-threaded module-local allocator invariant as above.
    unsafe { cursor_ptr.write(next) };
    i32::try_from(cursor).unwrap_or(-1)
}

/// Native builds cannot expose guest pointers; this implementation keeps host tests safe.
#[doc(hidden)]
#[cfg(not(target_arch = "wasm32"))]
pub fn guest_alloc(_size: i32) -> i32 {
    -1
}

#[cfg(target_arch = "wasm32")]
static mut HEAP_CURSOR: usize = 0;

/// Parse and process a request from guest memory. Exported by [`export_extension!`].
#[doc(hidden)]
pub fn guest_handle<H: ExtensionHandler + Default>(input_ptr: i32, input_len: i32, output_ptr: i32, output_capacity: i32) -> i32 {
    if input_ptr < 0 || input_len < 0 || output_ptr < 0 || output_capacity < 0 || input_len as usize > MAX_ENVELOPE_BYTES {
        return -1;
    }
    let input_len = input_len as usize;
    let output_capacity = output_capacity as usize;
    // SAFETY: the host calls the guest ABI only with validated, in-bounds ranges
    // in this module's own linear memory. A malformed guest remains sandboxed.
    let input = unsafe { core::slice::from_raw_parts(input_ptr as *const u8, input_len) };
    let mut handler = H::default();
    let Ok(output) = handle_request(&mut handler, input) else {
        return -1;
    };
    if output.len() > output_capacity {
        return -1;
    }
    // SAFETY: the host provides a writable output range, disjoint from input,
    // whose capacity is checked above; the runtime independently enforces it.
    unsafe { core::ptr::copy_nonoverlapping(output.as_ptr(), output_ptr as *mut u8, output.len()) };
    i32::try_from(output.len()).unwrap_or(-1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fixture;

    impl ExtensionHandler for Fixture {
        fn on_event(&mut self, event: &ExtensionEvent) -> Vec<ExtensionAction> {
            match event {
                ExtensionEvent::NearbyChat { message_id, .. } => vec![ExtensionAction::AnnotateChat {
                    message_id: *message_id,
                    text: "fixture response".into(),
                }],
                _ => Vec::new(),
            }
        }
    }

    #[test]
    fn sdk_builds_a_versioned_response_for_a_generic_event() {
        let request = ExtensionRequest {
            api_version: API_VERSION,
            request_id: 3,
            event: ExtensionEvent::NearbyChat {
                message_id: 9,
                speaker: "Fixture".into(),
                text: "sample".into(),
            },
        };
        let output = handle_request(&mut Fixture, &serde_json::to_vec(&request).expect("request JSON")).expect("response");
        let response: ExtensionResponse = serde_json::from_slice(&output).expect("response JSON");
        assert_eq!(response.request_id, request.request_id);
        response.validate_for(&request).expect("valid API response");
    }

    #[test]
    fn sdk_rejects_malformed_requests_and_unauthorized_actions() {
        assert!(matches!(handle_request(&mut Fixture, b"not JSON"), Err(SdkError::InvalidRequest)));
        struct Invalid;
        impl ExtensionHandler for Invalid {
            fn on_event(&mut self, _event: &ExtensionEvent) -> Vec<ExtensionAction> {
                vec![ExtensionAction::AnnotateChat {
                    message_id: 77,
                    text: "wrong message".into(),
                }]
            }
        }
        let request = ExtensionRequest {
            api_version: API_VERSION,
            request_id: 4,
            event: ExtensionEvent::NearbyChat {
                message_id: 9,
                speaker: "Fixture".into(),
                text: "sample".into(),
            },
        };
        let input = serde_json::to_vec(&request).expect("request JSON");
        assert!(matches!(handle_request(&mut Invalid, &input), Err(SdkError::InvalidResponse)));
    }
}
