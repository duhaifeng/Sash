fn main() {
    println!("cargo:rerun-if-changed=assets/macos-info.plist");
    println!("cargo:rerun-if-changed=Cargo.toml");
    if std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("macos") {
        return;
    }
    // The About panel and `NSBundle` read the version from this embedded plist
    // when the binary runs outside an app bundle, so it comes from Cargo.toml.
    let template = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("assets/macos-info.plist");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let plist = std::fs::read_to_string(&template)
        .unwrap()
        .replace("@VERSION@", &version);
    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("info.plist");
    std::fs::write(&out, plist).unwrap();
    println!(
        "cargo:rustc-link-arg=-Wl,-sectcreate,__TEXT,__info_plist,{}",
        out.display()
    );
}
