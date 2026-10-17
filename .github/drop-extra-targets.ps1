# CI only: the hosted Windows runner's Rust carries the GNU and 32-bit
# targets, which nothing here builds; each `rustup update` would download
# their standard library again. Removing an absent target is not an error.
$installed = rustup target list --installed
foreach ($target in 'x86_64-pc-windows-gnu', 'i686-pc-windows-msvc') {
    if ($installed -contains $target) {
        rustup target remove $target
    }
}
exit 0
