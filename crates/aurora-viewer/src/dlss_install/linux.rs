//! Linux x86-64: NVIDIA's DLSS SDK files and sl-ngx (MIT) built against
//! them with the system's `g++` (mod.rs), as scripts/dlss-setup.sh does.
//!
//! NVIDIA ships no Streamline for Linux; sl-ngx implements its interface
//! on NVIDIA's NGX. Its sources, NVIDIA's headers and static library and
//! the Vulkan headers come from a pinned commit or tag, each checked
//! against its SHA-256, so the build needs only `g++` (no git, make or
//! Vulkan development package).

use super::{Net, Remote, Report, Step, remote};
use std::path::{Path, PathBuf};
use std::process::Command;

const SDK: &str = "https://raw.githubusercontent.com/NVIDIA/DLSS/v310.9.16";
const VULKAN: &str = "https://raw.githubusercontent.com/KhronosGroup/Vulkan-Headers/vulkan-sdk-1.4.341.0";
/// sl-ngx at the commit scripts/dlss-setup.sh builds.
const SL_NGX: &str = "https://raw.githubusercontent.com/Aurora-Viewer/sl-ngx/4016e7cbd5d05dd07b5c0d7743d76b1a3d248380";

const LICENCE: Remote = remote(
    "LICENSE.txt",
    "sdk/LICENSE.txt",
    26_620,
    "d4216e39ebef5f9b50a6712ebb37beeb5379862a67733a9999c651f21592aaf0",
);
const NGX: Remote = remote(
    "lib/Linux_x86_64/libnvsdk_ngx.a",
    "sdk/libnvsdk_ngx.a",
    89_002,
    "d3c50bb0d9624fc35e502a8e929b8fc1e2b505017ebdb84c0d5b627e11a235df",
);
/// The DLSS model, installed beside sl-ngx.
const SNIPPET: Remote = remote(
    "lib/Linux_x86_64/rel/libnvidia-ngx-dlss.so.310.9.16",
    "libnvidia-ngx-dlss.so.310.9.16",
    59_481_944,
    "30982c937afed85c96b8d313323de9a650a2eec6de1bb0f714b21750f4ce98f6",
);

/// Ray Reconstruction's and frame generation's models, beside it, as
/// scripts/dlss-setup.sh installs them: NGX loads each only when its
/// feature is asked for.
const RR_SNIPPET: Remote = remote(
    "lib/Linux_x86_64/rel/libnvidia-ngx-dlssd.so.310.9.16",
    "libnvidia-ngx-dlssd.so.310.9.16",
    48_238_232,
    "a5d59de18841f15779039dda567a15dd723ba737915f489271fdb5f109fbfb96",
);
const FG_SNIPPET: Remote = remote(
    "lib/Linux_x86_64/rel/libnvidia-ngx-dlssg.so.310.9.16",
    "libnvidia-ngx-dlssg.so.310.9.16",
    7_391_232,
    "574839cae01c835de4d5845c36ca9cf75361f8160bd52397142c36a24e29e6f7",
);

pub const INSTALLED: [&str; 4] = [SNIPPET.name, RR_SNIPPET.name, FG_SNIPPET.name, "libsl_ngx.so"];

const SDK_HEADERS: [Remote; 23] = [
    remote(
        "include/nvsdk_ngx.h",
        "sdk/include/nvsdk_ngx.h",
        40289,
        "dc38e7467cf415379c9d12ae1b6e4a494c453ed92720fb53e92aecb523e7b848",
    ),
    remote(
        "include/nvsdk_ngx_defs.h",
        "sdk/include/nvsdk_ngx_defs.h",
        44313,
        "ea23f33497cd274860d1c25a97644fce807dcb0037c594547203343103fad03e",
    ),
    remote(
        "include/nvsdk_ngx_defs_dlssd.h",
        "sdk/include/nvsdk_ngx_defs_dlssd.h",
        14693,
        "2e97fe1595b1334f71b51724f2cb67496e8c296a7a2eec5606da53055882e716",
    ),
    remote(
        "include/nvsdk_ngx_defs_dlssg.h",
        "sdk/include/nvsdk_ngx_defs_dlssg.h",
        23208,
        "c29765ab9e0125f9d724ae9811cc2abe03738d17486ec8eee80d78a1504549e1",
    ),
    remote(
        "include/nvsdk_ngx_defs_vk.h",
        "sdk/include/nvsdk_ngx_defs_vk.h",
        3196,
        "0a24d0861ace7d6b9362a67b7f08bea1b33ea123c5ce730b19133de8a891d031",
    ),
    remote(
        "include/nvsdk_ngx_helpers.h",
        "sdk/include/nvsdk_ngx_helpers.h",
        5317,
        "5bcbadfe7478b802cf6d3aca4dc5ddd7d0889b99726e69c63f9e9bd555f44471",
    ),
    remote(
        "include/nvsdk_ngx_helpers_cuda.h",
        "sdk/include/nvsdk_ngx_helpers_cuda.h",
        3593,
        "f2851a76107bdf4fbcb7b261f3c35573c312d8ecb4bc18fb02d9e2cec7705042",
    ),
    remote(
        "include/nvsdk_ngx_helpers_d3d.h",
        "sdk/include/nvsdk_ngx_helpers_d3d.h",
        31976,
        "ec75224f36ed6580baaf250fa83620405ad81a1d98c2c436654a0c9cf6a6b8ba",
    ),
    remote(
        "include/nvsdk_ngx_helpers_dlssd.h",
        "sdk/include/nvsdk_ngx_helpers_dlssd.h",
        732,
        "a25fdda925b479dfe869593aeba311ef08793da54e0391f2fe2fe756cbef5532",
    ),
    remote(
        "include/nvsdk_ngx_helpers_dlssd_cuda.h",
        "sdk/include/nvsdk_ngx_helpers_dlssd_cuda.h",
        29175,
        "7e35cde214d905346a487b9a28c05d35e1c55b480e237fe3de69580a7e332730",
    ),
    remote(
        "include/nvsdk_ngx_helpers_dlssd_d3d.h",
        "sdk/include/nvsdk_ngx_helpers_dlssd_d3d.h",
        72668,
        "6df9d02c6f47feee3afc8d12340a9f554a7ddce4ddacb1b6a13a42b26c2c6293",
    ),
    remote(
        "include/nvsdk_ngx_helpers_dlssd_vk.h",
        "sdk/include/nvsdk_ngx_helpers_dlssd_vk.h",
        38337,
        "11df8fbda532a97fe867c8ef2b40058b49b7898183710f3468e55b1eedb8f40b",
    ),
    remote(
        "include/nvsdk_ngx_helpers_dlssg.h",
        "sdk/include/nvsdk_ngx_helpers_dlssg.h",
        732,
        "6881d21419c40c78ec948b326902ece09fc112b0223144d51c4905f3fced7a1f",
    ),
    remote(
        "include/nvsdk_ngx_helpers_dlssg_d3d.h",
        "sdk/include/nvsdk_ngx_helpers_dlssg_d3d.h",
        17445,
        "6335ebcb95de39a361e87fe0257d6ce0723e5d8deaa15db3b35afacf2b3f4220",
    ),
    remote(
        "include/nvsdk_ngx_helpers_dlssg_vk.h",
        "sdk/include/nvsdk_ngx_helpers_dlssg_vk.h",
        17077,
        "b371d90b9f4372b9a415e68a0afbdc2f31b7b552e219a9637d848749bc965316",
    ),
    remote(
        "include/nvsdk_ngx_helpers_vk.h",
        "sdk/include/nvsdk_ngx_helpers_vk.h",
        19720,
        "c192bff72138f12f770db48e12c1d8f712dfc81ea0e8e055ebb27d8dd1b31623",
    ),
    remote(
        "include/nvsdk_ngx_loader.h",
        "sdk/include/nvsdk_ngx_loader.h",
        13252,
        "abdf5f66a692620538357d29fdf78e3e2a677ee8140b7a638962ecb766585e75",
    ),
    remote(
        "include/nvsdk_ngx_params.h",
        "sdk/include/nvsdk_ngx_params.h",
        7744,
        "943bc8cc5cdae03b6303016fbad3183636f2335ae27a2d18776798c3b4efabbc",
    ),
    remote(
        "include/nvsdk_ngx_params_dlssd.h",
        "sdk/include/nvsdk_ngx_params_dlssd.h",
        1199,
        "d41e3ba8f1d72583b55273092c99bf426dc4a0d46de827229e25d5b1325a370e",
    ),
    remote(
        "include/nvsdk_ngx_params_dlssg.h",
        "sdk/include/nvsdk_ngx_params_dlssg.h",
        6137,
        "c24fa5aa68ffa2808c828b75cfa32843f1606f80a145f84055ac450d7e6c74ad",
    ),
    remote(
        "include/nvsdk_ngx_standalone_common.h",
        "sdk/include/nvsdk_ngx_standalone_common.h",
        12985,
        "38413c3338b0aea0202c07da95d426bfee7080a42913823fa2d9dd585b087b53",
    ),
    remote(
        "include/nvsdk_ngx_standalone_cuda.h",
        "sdk/include/nvsdk_ngx_standalone_cuda.h",
        10906,
        "b23436ed5040051ac8e81554c9af080f2463c941435bbc3b82222e5431a09499",
    ),
    remote(
        "include/nvsdk_ngx_vk.h",
        "sdk/include/nvsdk_ngx_vk.h",
        38382,
        "2d364ce7132881eb669e9498fd570d74cba563b1fbebc82c235f8e1ad2dd8b6d",
    ),
];
const VULKAN_HEADERS: [Remote; 15] = [
    remote(
        "include/vulkan/vk_platform.h",
        "vk/vulkan/vk_platform.h",
        2828,
        "949d517bb83e1d88fd4f1cef02bd3cb9ab50d44e8354cc68227cf2dccfdd3307",
    ),
    remote(
        "include/vulkan/vulkan.h",
        "vk/vulkan/vulkan.h",
        1616,
        "72b952cc6de70ee12d118d3095e80346b42ff01cfc6f2bbb37ec01800eab1da6",
    ),
    remote(
        "include/vulkan/vulkan_core.h",
        "vk/vulkan/vulkan_core.h",
        1265235,
        "03964df1e28a162285726ec7cf9098e5e0271cb1df21933e7bfef9f6fa08937f",
    ),
    remote(
        "include/vk_video/vulkan_video_codec_av1std.h",
        "vk/vk_video/vulkan_video_codec_av1std.h",
        16687,
        "9b4ebcef0d6844b226803fa91b4c4c8bf9eb941aa31b19cea48ded0886a8f9e2",
    ),
    remote(
        "include/vk_video/vulkan_video_codec_av1std_decode.h",
        "vk/vk_video/vulkan_video_codec_av1std_decode.h",
        4250,
        "f03abf49fcaf2bd179d48d768164f9494264b58401b98899e8860ba297a1e7aa",
    ),
    remote(
        "include/vk_video/vulkan_video_codec_av1std_encode.h",
        "vk/vk_video/vulkan_video_codec_av1std_encode.h",
        5979,
        "40f84c98d0341246ead3e864e1378df47d861505c5c139da5ed85c352d5e2300",
    ),
    remote(
        "include/vk_video/vulkan_video_codec_h264std.h",
        "vk/vk_video/vulkan_video_codec_h264std.h",
        13861,
        "f6691f82e4637adda20e56d56951c673cc1ecdf2fa77e09cc8eba4b1f45118c1",
    ),
    remote(
        "include/vk_video/vulkan_video_codec_h264std_decode.h",
        "vk/vk_video/vulkan_video_codec_h264std_decode.h",
        2723,
        "8c4682860954d6bfc603e200d9e12586bc57265bd37b73a12b1ecf0c1ad42721",
    ),
    remote(
        "include/vk_video/vulkan_video_codec_h264std_encode.h",
        "vk/vk_video/vulkan_video_codec_h264std_encode.h",
        6878,
        "75cc54e489b1ec7e3c635680cabaf7c9c2ddd4071a485440e5368e76ede30a85",
    ),
    remote(
        "include/vk_video/vulkan_video_codec_h265std.h",
        "vk/vk_video/vulkan_video_codec_h265std.h",
        23439,
        "01b5f3d0fd9d273a68ed90f898ba94c05c0305280a753bf95fab60d60727c22e",
    ),
    remote(
        "include/vk_video/vulkan_video_codec_h265std_decode.h",
        "vk/vk_video/vulkan_video_codec_h265std_decode.h",
        2429,
        "926a24d94afed2b1ab030e81c3b9e2e6730cca530f7a5a5bd821e7b896b93a41",
    ),
    remote(
        "include/vk_video/vulkan_video_codec_h265std_encode.h",
        "vk/vk_video/vulkan_video_codec_h265std_encode.h",
        7462,
        "8a56e0c78496affa84eca39ec0f5636df681c04623dd0291aeeadd012df324be",
    ),
    remote(
        "include/vk_video/vulkan_video_codec_vp9std.h",
        "vk/vk_video/vulkan_video_codec_vp9std.h",
        5384,
        "e18e20b197945929d5871836bfde923e38b350b09e1cbe430f1bd802d9eda34e",
    ),
    remote(
        "include/vk_video/vulkan_video_codec_vp9std_decode.h",
        "vk/vk_video/vulkan_video_codec_vp9std_decode.h",
        2371,
        "c3929bbdd0ab79128c7f9bfb9641c9a017b3983529315b771caaafc74fdbb89a",
    ),
    remote(
        "include/vk_video/vulkan_video_codecs_common.h",
        "vk/vk_video/vulkan_video_codecs_common.h",
        660,
        "be6d2495d19e96aca6aa5c11e5c418d1cd72beac23fe19eb9169c06e2843f0af",
    ),
];
const SL_NGX_FILES: [Remote; 16] = [
    remote(
        "LICENSE",
        "sl-ngx/LICENSE",
        1066,
        "0325cadf9a34252af6be9e74bff463aaceb911852875ca0b4d3dd6bac57aaf97",
    ),
    remote(
        "exports.map",
        "sl-ngx/exports.map",
        32,
        "43c7dde61669d79a1d46d7a5f7e433d397ea875f6aca40bba49420919605f08a",
    ),
    remote(
        "src/sl_ngx.cpp",
        "sl-ngx/src/sl_ngx.cpp",
        48437,
        "b75b66db5ad2289853aa1882170fa66d84f521b5875c2e2073e439ea9cc844e1",
    ),
    remote(
        "include/sl/LICENSE-Streamline.txt",
        "sl-ngx/include/sl/LICENSE-Streamline.txt",
        1591,
        "7b6f23e7d6f3ad6292f9308d2b42cdc3d82ae4e9b2abb55f230279d83bedd43d",
    ),
    remote(
        "include/sl/sl.h",
        "sl-ngx/include/sl/sl.h",
        2201,
        "e1e81a7428d15b30db37587e9469bd68a56d630d820a4accec0aee3b17e157dd",
    ),
    remote(
        "include/sl/sl_appidentity.h",
        "sl-ngx/include/sl/sl_appidentity.h",
        206,
        "1337385ac9867d66fa6beb34c750e79aaf25a74a156a47e83f24937299dade87",
    ),
    remote(
        "include/sl/sl_consts.h",
        "sl-ngx/include/sl/sl_consts.h",
        12814,
        "17de74afda2cb96204ff380464bed926f660d92015fe0304dff4e329509c88b9",
    ),
    remote(
        "include/sl/sl_core_api.h",
        "sl-ngx/include/sl/sl_core_api.h",
        17127,
        "328dc3a2c1dee579c200ca97e2d5b1ec38c893be4ba2745ef5ad506af7172e81",
    ),
    remote(
        "include/sl/sl_core_types.h",
        "sl-ngx/include/sl/sl_core_types.h",
        34283,
        "f420cb052ce14489fc9fa0d933c3ed81a010deb01fa13b1aa8d45da1c5ba3234",
    ),
    remote(
        "include/sl/sl_device_wrappers.h",
        "sl-ngx/include/sl/sl_device_wrappers.h",
        205,
        "af7741305b1a468c3a83efaa2005ecd53d2ffd7aadb8ec6868c9dcb4a8a8e3a1",
    ),
    remote(
        "include/sl/sl_dlss.h",
        "sl-ngx/include/sl/sl_dlss.h",
        8489,
        "d2c8c61fa71794c8ba424cebce5b3079ebfab8e47ed98b4b00cf8b257ecfd6b5",
    ),
    remote(
        "include/sl/sl_dlss_d.h",
        "sl-ngx/include/sl/sl_dlss_d.h",
        8222,
        "6e51cf9815301c597d0657970a668147e34a20db5a76d0af254ec11248f5c4fe",
    ),
    remote(
        "include/sl/sl_helpers_vk.h",
        "sl-ngx/include/sl/sl_helpers_vk.h",
        15100,
        "82604567d239d00ac0ac2270cec744f6f11266e1d9d5187a95d4954f76d8c41c",
    ),
    remote(
        "include/sl/sl_result.h",
        "sl-ngx/include/sl/sl_result.h",
        2535,
        "2a0f6c12863bdc00b38910a5ec85d1f083c5671ee817a920afe626ac2a9100f7",
    ),
    remote(
        "include/sl/sl_struct.h",
        "sl-ngx/include/sl/sl_struct.h",
        4839,
        "28deda67ea1a74371dd4f33ff4842b7db03b2db007001da6db06f6982297928d",
    ),
    remote(
        "include/sl/sl_version.h",
        "sl-ngx/include/sl/sl_version.h",
        3641,
        "2a43f565b45a07d4a4259dc61038ec93a7fb8ce7cb3509ea1fbeeb217f9a1ce8",
    ),
];

/// NVIDIA's DLSS SDK licence.
pub fn licence(net: &Net) -> Result<String, String> {
    Ok(String::from_utf8_lossy(&net.download(SDK, &LICENCE, |_| {})?).into_owned())
}

/// The files of [`INSTALLED`] in `work`: downloaded, checked, and sl-ngx
/// built.
pub fn prepare(net: &Net, work: &Path, report: Report) -> Result<(), String> {
    // before downloading anything: the one tool the build needs
    let cxx = std::env::var_os("CXX").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("g++"));
    if !Command::new(&cxx).arg("--version").output().is_ok_and(|o| o.status.success()) {
        return Err(aurora_i18n::tr!("ui-options-dlss-install-needs-gxx").to_string());
    }
    let vulkan = system_library("libvulkan.so.1").ok_or("the Vulkan loader libvulkan.so.1 was not found")?;

    let all: Vec<(&str, &Remote)> = [(SDK, &NGX), (SDK, &SNIPPET), (SDK, &RR_SNIPPET), (SDK, &FG_SNIPPET)]
        .into_iter()
        .chain(SDK_HEADERS.iter().map(|r| (SDK, r)))
        .chain(VULKAN_HEADERS.iter().map(|r| (VULKAN, r)))
        .chain(SL_NGX_FILES.iter().map(|r| (SL_NGX, r)))
        .collect();
    let total: u64 = all.iter().map(|(_, r)| r.size).sum();
    let mut before = 0;
    for (base, r) in all {
        let bytes = net.download(base, r, |got| {
            report(Step::Downloading, 0.85 * (before + got) as f32 / total as f32);
        })?;
        let path = work.join(r.name);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        before += r.size;
    }

    // sl-ngx's Makefile, with the Vulkan loader by its path (no -lvulkan:
    // its unversioned name comes only with a development package)
    report(Step::Building, 0.9);
    let out = Command::new(&cxx)
        .current_dir(work)
        .args([
            "-O2",
            "-std=c++17",
            "-fPIC",
            "-fvisibility=hidden",
            "-Wno-deprecated-declarations",
            "-shared",
            "-Isl-ngx/include",
            "-Isdk/include",
            "-Ivk",
            "sl-ngx/src/sl_ngx.cpp",
            "sdk/libnvsdk_ngx.a",
            "-Wl,--exclude-libs,ALL",
            "-Wl,--version-script=sl-ngx/exports.map",
            "-Wl,--no-undefined",
        ])
        .arg(&vulkan)
        .args(["-ldl", "-o", "libsl_ngx.so"])
        .output()
        .map_err(|e| format!("{}: {e}", cxx.display()))?;
    if !out.status.success() {
        let text = String::from_utf8_lossy(&out.stderr);
        let first: Vec<&str> = text.lines().filter(|l| l.contains("error")).take(4).collect();
        log::warn!("DLSS install: g++ said:\n{text}");
        return Err(format!("sl-ngx did not build: {}", first.join(" / ")));
    }
    Ok(())
}

/// What an earlier version of the setup left there, and other versions of
/// the DLSS models.
pub fn remove_old(folder: &Path) {
    let _ = std::fs::remove_file(folder.join("libnvsdk_ngx.so"));
    if let Ok(dir) = std::fs::read_dir(folder) {
        for e in dir.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            // (another version of a model, which NGX could load instead)
            let stale = [&SNIPPET, &RR_SNIPPET, &FG_SNIPPET].iter().any(|r| {
                let stem = r.name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
                name.starts_with(&format!("{stem}.")) && name != r.name
            });
            if stale {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// `name` as the dynamic loader finds it: its cache (`ldconfig -p`), else
/// the usual folders.
fn system_library(name: &str) -> Option<PathBuf> {
    let cache = ["/sbin/ldconfig", "/usr/sbin/ldconfig", "ldconfig"]
        .iter()
        .find_map(|c| Command::new(c).arg("-p").output().ok().filter(|o| o.status.success()))
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    cache
        .lines()
        .filter_map(|l| l.trim().split_once(" => "))
        .find(|(left, _)| left.split_whitespace().next() == Some(name) && left.contains("x86-64"))
        .map(|(_, path)| PathBuf::from(path.trim()))
        .filter(|p| p.is_file())
        .or_else(|| {
            [
                "/lib/x86_64-linux-gnu",
                "/usr/lib/x86_64-linux-gnu",
                "/lib64",
                "/usr/lib64",
                "/usr/lib",
                "/lib",
            ]
            .iter()
            .map(|d| Path::new(d).join(name))
            .find(|p| p.is_file())
        })
}
