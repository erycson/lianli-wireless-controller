fn main() {
    embed_resource::compile("assets/app.rc", embed_resource::NONE).manifest_optional().unwrap();
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rustc-link-arg-bin=lianli-controller=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bin=lianli-controller=/MANIFESTUAC:level='requireAdministrator' uiAccess='false'");
}
