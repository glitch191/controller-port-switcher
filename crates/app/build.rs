//! Builds the six proxy DLLs (three names, x64 and x86) with a nested cargo call, so the
//! app can embed them and stay a single executable. Also embeds the icons and manifest.

use std::env;
use std::path::PathBuf;
use std::process::Command;

const DLLS: [&str; 3] = ["xinput1_3", "xinput1_4", "xinput9_1_0"];
const TARGETS: [(&str, &str); 2] = [("X64", "x86_64-pc-windows-msvc"), ("X86", "i686-pc-windows-msvc")];

fn main() {
    let app_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = app_dir.parent().and_then(|p| p.parent()).expect("workspace root").to_path_buf();
    // A separate target directory avoids waiting on the lock of the outer build.
    let target_dir = root.join("target").join("proxy");

    for (arch, triple) in TARGETS {
        let mut cmd = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        cmd.current_dir(root.join("crates").join("proxy"))
            // Always optimized: the DLLs are small and link without a C runtime only with LTO.
            .args(["build", "--quiet", "--release", "--target", triple, "--target-dir"])
            .arg(&target_dir);
        for dll in DLLS {
            cmd.args(["-p", dll]);
        }
        // Flags meant for the outer build would override the per-target config.
        for var in ["CARGO_ENCODED_RUSTFLAGS", "RUSTFLAGS", "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET"] {
            cmd.env_remove(var);
        }
        let status = cmd.status().expect("failed to run cargo for the proxy DLLs");
        assert!(
            status.success(),
            "building the {triple} proxy DLLs failed; if the target is missing, run: rustup target add {triple}"
        );
        for dll in DLLS {
            let path = target_dir.join(triple).join("release").join(format!("{dll}.dll"));
            println!("cargo:rustc-env=CPS_DLL_{arch}_{}={}", dll.to_uppercase(), path.display());
        }
    }

    for dir in ["crates/core/src", "crates/core/Cargo.toml", "crates/proxy"] {
        println!("cargo:rerun-if-changed={}", root.join(dir).display());
    }
    println!("cargo:rerun-if-changed={}", root.join("assets").display());

    embed_resource::compile(root.join("assets").join("app.rc"), embed_resource::NONE)
        .manifest_required()
        .expect("compiling assets/app.rc failed");
}
