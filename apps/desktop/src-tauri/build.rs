fn main() {
    // Cargo check also runs Tauri's resource scanner before development assets exist.
    let staging = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/bundled-drivers");
    if let Err(error) = std::fs::create_dir_all(staging) { panic!("Could not create driver staging directory: {error}"); }
    tauri_build::build()
}
