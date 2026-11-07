fn main() {
    // Same icon as the app (shown in Task Manager and file properties).
    #[cfg(windows)]
    embed_resource::compile("../ferric-qt/assets/ferric.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("embed icon resource");
    println!("cargo:rerun-if-changed=../ferric-qt/assets/ferric.ico");
}
