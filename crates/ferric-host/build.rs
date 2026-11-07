fn main() {
    // Same icon as the app (shown in Task Manager and file properties).
    #[cfg(windows)]
    embed_resource::compile("../ferric-qt/assets/ferric.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("embed icon resource");
    println!("cargo:rerun-if-changed=../ferric-qt/assets/ferric.ico");
    enforce_official_release_metadata();
}

fn enforce_official_release_metadata() {
    println!("cargo:rerun-if-env-changed=FERRIC_OFFICIAL_BUILD");
    println!("cargo:rerun-if-env-changed=FERRIC_EXTENSION_ID");
    println!("cargo:rerun-if-env-changed=FERRIC_FIREFOX_EXTENSION_ID");

    if std::env::var("FERRIC_OFFICIAL_BUILD").ok().as_deref() != Some("1") {
        return;
    }

    for key in ["FERRIC_EXTENSION_ID", "FERRIC_FIREFOX_EXTENSION_ID"] {
        let present = std::env::var(key).is_ok_and(|value| !value.trim().is_empty());
        assert!(present, "{key} must be set for FERRIC_OFFICIAL_BUILD=1");
    }
}
