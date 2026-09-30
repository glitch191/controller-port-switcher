// Sets exported ordinals to match the system DLL of the same name, and links without a
// C runtime entry point (the proxy is no_std and needs no initialization). Only memcpy
// and memcmp come from the static vcruntime library.
fn main() {
    let def = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("exports.def");
    println!("cargo:rustc-cdylib-link-arg=/DEF:{}", def.display());
    println!("cargo:rustc-cdylib-link-arg=/NOENTRY");
    println!("cargo:rustc-cdylib-link-arg=/DEFAULTLIB:libvcruntime");
    // On x86, vcruntime memcpy references IsProcessorFeaturePresent.
    println!("cargo:rustc-cdylib-link-arg=/DEFAULTLIB:kernel32");
    // Unoptimized builds keep unwinding code from `core` that needs the full static CRT.
    // They are only for `cargo build --workspace`; the app always embeds release DLLs.
    if std::env::var("PROFILE").as_deref() != Ok("release") {
        println!("cargo:rustc-cdylib-link-arg=/DEFAULTLIB:libucrt");
    }
    println!("cargo:rerun-if-changed=exports.def");
}
