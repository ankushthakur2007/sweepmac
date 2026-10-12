fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-framework");
        println!("cargo:rustc-link-arg=ServiceManagement");
    }
}
