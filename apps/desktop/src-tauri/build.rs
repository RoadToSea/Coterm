fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    for name in ["ssh", "serial", "local"] {
        let directory = root.join("drivers").join(format!("driver-{name}"));
        let source = directory.join("dist/frontend/index.js");
        println!(
            "cargo:rerun-if-changed={}",
            directory.join("manifest.json").display()
        );
        println!("cargo:rerun-if-changed={}", source.display());
        assert!(
            source.is_file(),
            "Missing {name} frontend; run pnpm drivers:build before building the desktop"
        );
    }
    tauri_build::build()
}
