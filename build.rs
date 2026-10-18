fn main() {
    println!("cargo:rerun-if-changed=src/platform/selection.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/platform/selection.m")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .compile("give_selection");
        println!("cargo:rustc-link-lib=framework=AppKit");
    }
}
