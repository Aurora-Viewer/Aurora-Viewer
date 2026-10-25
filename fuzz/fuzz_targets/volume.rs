#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| aurora_fuzz::run("volume", data));
