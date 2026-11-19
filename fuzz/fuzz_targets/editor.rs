#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| aurora_fuzz::run("editor", data));
