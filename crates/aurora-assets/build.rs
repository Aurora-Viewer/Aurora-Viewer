fn main() {
    for file in ["native/vhacd/bridge.cpp", "native/vhacd/VHACD.h"] {
        println!("cargo:rerun-if-changed={file}");
    }
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .flag_if_supported("/EHsc")
        .warnings(false)
        .file("native/vhacd/bridge.cpp")
        .compile("aurora_vhacd");
}
