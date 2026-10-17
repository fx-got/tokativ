use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    // ggml's Metal backend uses Objective-C @available checks. Rust links with
    // -nodefaultlibs, so Clang's availability runtime must be linked explicitly.
    let output = cc::Build::new()
        .get_compiler()
        .to_command()
        .arg("--print-resource-dir")
        .output()
        .expect("macOS builds require Clang from Xcode Command Line Tools or Nix");
    assert!(output.status.success(), "could not find the Clang runtime");
    let resource_dir = String::from_utf8(output.stdout).expect("Clang resource path is UTF-8");
    let library_dir = PathBuf::from(resource_dir.trim()).join("lib/darwin");
    assert!(
        library_dir.join("libclang_rt.osx.a").is_file(),
        "Clang's macOS runtime is missing from {}",
        library_dir.display()
    );
    println!("cargo:rustc-link-search=native={}", library_dir.display());
    println!("cargo:rustc-link-lib=static=clang_rt.osx");
}
