fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR")
        .ok_or_else(|| std::io::Error::other("Cargo manifest directory is unavailable"))?;
    let root = std::path::PathBuf::from(manifest_dir).join("../../..");
    for name in ["ssh", "serial", "local"] {
        let directory = root.join("crates/coterm-driver");
        let source = directory.join("dist").join(name).join("frontend/index.js");
        println!(
            "cargo:rerun-if-changed={}",
            directory
                .join("manifests")
                .join(format!("{name}.json"))
                .display()
        );
        println!("cargo:rerun-if-changed={}", source.display());
        assert!(
            source.is_file(),
            "Missing {name} frontend; run pnpm drivers:build before building the desktop"
        );
    }
    tauri_build::build();
    Ok(())
}
