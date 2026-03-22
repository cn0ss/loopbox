fn main() {
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rerun-if-changed=src/native/macos/sparkle_bridge.m");

        cc::Build::new()
            .file("src/native/macos/sparkle_bridge.m")
            .flag("-fobjc-arc")
            .compile("loopbox_sparkle_bridge");

        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=AppKit");
    }
}
