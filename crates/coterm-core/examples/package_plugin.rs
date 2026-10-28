//! Build an offline plugin archive using the same validation as the installer.
use coterm_core::{plugin_install::PreparedPluginPackage, plugin_process::load_plugin_package};
use coterm_proto::runtime::manifest::PluginManifest;
use std::{
    error::Error,
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

fn add_directory(
    writer: &mut ZipWriter<File>,
    root: &Path,
    directory: &Path,
    manifest: &PluginManifest,
) -> Result<(), Box<dyn Error>> {
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let kind = entry.file_type()?;
        let path = entry.path();
        if kind.is_symlink() {
            return Err("Plugin packages cannot contain symbolic links".into());
        }
        if kind.is_dir() {
            add_directory(writer, root, &path, manifest)?;
        } else if kind.is_file() {
            let name = path
                .strip_prefix(root)?
                .to_str()
                .ok_or("Package paths must be UTF-8")?
                .replace('\\', "/");
            let executable = manifest.backend.values().any(|entry| entry == &name);
            let options = SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated)
                .unix_permissions(if executable { 0o755 } else { 0o644 });
            writer.start_file(name, options)?;
            io::copy(&mut File::open(&path)?, writer)?;
        } else {
            return Err("Plugin packages can only contain regular files and directories".into());
        }
    }
    Ok(())
}

async fn run() -> Result<(), Box<dyn Error>> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.len() != 2 {
        return Err("Usage: pnpm plugin:pack <plugin-dist-directory> <output.coterm>".into());
    }
    let source = PathBuf::from(&args[0]).canonicalize()?;
    let output = std::path::absolute(PathBuf::from(&args[1]))?;
    let parent = output
        .parent()
        .ok_or("Output requires a parent directory")?;
    fs::create_dir_all(parent)?;
    if parent.canonicalize()?.starts_with(&source) {
        return Err("Output must be outside the plugin directory".into());
    }
    let manifest: PluginManifest =
        serde_json::from_slice(&fs::read(source.join("manifest.json"))?)?;
    let platform = manifest
        .backend
        .keys()
        .next()
        .ok_or("Manifest must declare a platform")?;
    for target in manifest.backend.keys() {
        load_plugin_package(&source, target).await?;
    }
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)?;
    let result: Result<(), Box<dyn Error>> = async {
        let mut writer = ZipWriter::new(file);
        add_directory(&mut writer, &source, &source, &manifest)?;
        writer.finish()?.sync_all()?;
        let preview = PreparedPluginPackage::prepare(
            output.clone(),
            std::env::temp_dir().join("coterm-package-validation"),
            platform,
        )
        .await?;
        drop(preview);
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_file(&output);
    }
    result?;
    println!("Offline plugin: {}", output.display());
    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Plugin packaging failed: {error}");
        std::process::exit(1);
    }
}
