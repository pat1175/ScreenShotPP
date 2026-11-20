fn main() {
    #[cfg(target_os = "macos")]
    {
        use swift_rs::SwiftLinker;
        SwiftLinker::new("11.0")
            .with_package("swift-lib", "./swift-lib/")
            .link();
        println!("cargo:rustc-link-lib=framework=Vision");
        println!("cargo:rustc-link-lib=framework=CoreGraphics");
        println!("cargo:rustc-link-lib=framework=ImageIO");
        println!("cargo:rustc-link-lib=framework=AppKit");
    }

    // Windows : icône de l'exécutable + manifeste (DPI par moniteur, contrôles
    // communs v6 requis par les boîtes de dialogue natives). Compilé avec rc.exe,
    // donc seulement sur un hôte Windows.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icons/icon.ico");
        res.set_manifest_file("windows/app.manifest");
        res.compile().expect("ressources Windows");
    }
    println!("cargo:rerun-if-changed=windows/app.manifest");
    println!("cargo:rerun-if-changed=icons/icon.ico");
}
