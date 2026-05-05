fn main() {
    #[cfg(target_os = "macos")]
    {
        use std::path::PathBuf;

        println!("cargo:rerun-if-changed=src/native/macos/sparkle_bridge.m");

        cc::Build::new()
            .file("src/native/macos/sparkle_bridge.m")
            .flag("-fobjc-arc")
            .compile("loopbox_sparkle_bridge");

        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=AppKit");

        if std::env::var_os("CARGO_FEATURE_GHOSTTY_VT").is_some() {
            println!("cargo:rerun-if-env-changed=DEP_GHOSTTY_VT_INCLUDE");
            if let Some(lib_dir) = std::env::var_os("DEP_GHOSTTY_VT_INCLUDE")
                .map(PathBuf::from)
                .and_then(|include_dir| include_dir.parent().map(|prefix| prefix.join("lib")))
            {
                println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
            }
        }
    }
}
