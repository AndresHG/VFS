fn main() {
    let mut attributes = tauri_build::Attributes::new();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // tauri-build 2.7.6 links its manifest into binaries only, so a test executable has
        // no Common Controls v6 and dies at load with STATUS_ENTRYPOINT_NOT_FOUND
        // (`TaskDialogIndirect`). The same manifest goes into every target instead; see
        // https://github.com/tauri-apps/tauri/issues/13419.
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        println!("cargo:rerun-if-changed=windows-app-manifest.xml");
        embed_resource::compile_for_everything("app-manifest.rc", embed_resource::NONE)
            .manifest_required()
            .expect("embed the Windows application manifest");
    }
    tauri_build::try_build(attributes).expect("tauri-build");
}
