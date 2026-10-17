use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    match env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos") => link_metal_runtime(),
        Ok("windows") => link_vulkan_sdk(),
        _ => {}
    }
}

fn link_metal_runtime() {
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

fn link_vulkan_sdk() {
    println!("cargo:rerun-if-env-changed=VULKAN_SDK");
    let Some(sdk) = env::var_os("VULKAN_SDK").map(PathBuf::from) else {
        return;
    };
    // The native SDK keeps its import library in Lib. The x64 SDK can also
    // include ARM64 libraries for cross compilation in Lib-ARM64.
    let arm_lib = sdk.join("Lib-ARM64");
    let library_dir =
        if env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("aarch64") && arm_lib.is_dir() {
            arm_lib
        } else {
            sdk.join("Lib")
        };
    assert!(
        library_dir.join("vulkan-1.lib").is_file(),
        "Vulkan SDK import library is missing from {}",
        library_dir.display()
    );
    // transcribe.cpp's link manifest names vulkan-1 but omits the SDK search path.
    println!("cargo:rustc-link-search=native={}", library_dir.display());
}
