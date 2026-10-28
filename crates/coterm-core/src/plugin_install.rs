//! Offline ZIP packages are staged and validated without executing their contents.
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::{Path, PathBuf},
};

use coterm_proto::runtime::{manifest::PluginManifest, ErrorCategory, RuntimeError};

use crate::{
    plugin_manager::ProcessPluginRuntime,
    plugin_process::{load_plugin_package, MAX_FRONTEND_BYTES},
};

pub const MAX_ARCHIVE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;

fn error(code: &str) -> RuntimeError {
    RuntimeError {
        category: match code {
            "plugin_install_conflict" => ErrorCategory::Conflict,
            "plugin_install_io" => ErrorCategory::Transport,
            _ => ErrorCategory::InvalidRequest,
        },
        code: code.to_owned(),
        message: code.to_owned(),
        details: None,
    }
}

/// Installer errors must never return user paths or raw filesystem diagnostics.
fn sanitized(mut failure: RuntimeError) -> RuntimeError {
    failure.message = failure.code.clone();
    failure.details = None;
    failure
}

struct StagingDirectory {
    root: PathBuf,
    keep: bool,
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

pub struct PreparedPluginPackage {
    directory: StagingDirectory,
    pub manifest: PluginManifest,
}

/// Reject Windows aliases on every OS so one archive has the same meaning everywhere.
fn archive_path(name: &str) -> Result<PathBuf, RuntimeError> {
    if name.len() > 512
        || name.contains('\\')
        || name
            .chars()
            .any(|c| c.is_control() || ":<>\"|?*".contains(c))
    {
        return Err(error("plugin_archive_unsafe"));
    }
    let name = name.strip_suffix('/').unwrap_or(name);
    let mut path = PathBuf::new();
    for part in name.split('/') {
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        let device = matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$"
        ) || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|tail| {
                let mut chars = tail.chars();
                matches!(chars.next(), Some('1'..='9' | '¹' | '²' | '³')) && chars.next().is_none()
            })
        });
        if part.is_empty() || part == "." || part == ".." || part.ends_with(['.', ' ']) || device {
            return Err(error("plugin_archive_unsafe"));
        }
        path.push(part);
    }
    Ok(path)
}

fn extract(archive: &Path, staging_parent: &Path) -> Result<StagingDirectory, RuntimeError> {
    let input = File::open(archive).map_err(|_| error("plugin_install_io"))?;
    let metadata = input.metadata().map_err(|_| error("plugin_install_io"))?;
    if !metadata.is_file() || metadata.len() > MAX_ARCHIVE_BYTES {
        return Err(error("plugin_archive_limit"));
    }
    let mut zip = zip::ZipArchive::new(input).map_err(|_| error("plugin_archive_invalid"))?;
    if zip.is_empty() || zip.len() > MAX_ENTRIES {
        return Err(error("plugin_archive_limit"));
    }
    fs::create_dir_all(staging_parent).map_err(|_| error("plugin_install_io"))?;
    let root = staging_parent.join(uuid::Uuid::new_v4().to_string());
    fs::create_dir(&root).map_err(|_| error("plugin_install_io"))?;
    let directory = StagingDirectory { root, keep: false };
    let mut seen = HashSet::new();
    let mut expanded = 0u64;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|_| error("plugin_archive_invalid"))?;
        let name = entry.name().map_err(|_| error("plugin_archive_invalid"))?;
        let relative = archive_path(&name)?;
        if !seen.insert(name.trim_end_matches('/').to_lowercase()) {
            return Err(error("plugin_archive_unsafe"));
        }
        let mode = entry.unix_mode().unwrap_or(0);
        let file_type = mode & 0o170000;
        if !matches!(file_type, 0 | 0o100000 | 0o040000)
            || (file_type == 0o040000 && !entry.is_dir())
            || (file_type == 0o100000 && entry.is_dir())
        {
            return Err(error("plugin_archive_unsafe"));
        }
        expanded = expanded.saturating_add(entry.size());
        if entry.size() > MAX_FILE_BYTES || expanded > MAX_EXPANDED_BYTES {
            return Err(error("plugin_archive_limit"));
        }
        let target = directory.root.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(|_| error("plugin_archive_unsafe"))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|_| error("plugin_archive_unsafe"))?;
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|_| error("plugin_archive_unsafe"))?;
        let declared = entry.size();
        let copied = io::copy(
            &mut (&mut entry).take(declared.saturating_add(1)),
            &mut output,
        )
        .map_err(|_| error("plugin_archive_invalid"))?;
        if copied != declared {
            return Err(error("plugin_archive_invalid"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = if mode & 0o111 != 0 { 0o755 } else { 0o644 };
            fs::set_permissions(&target, fs::Permissions::from_mode(permissions))
                .map_err(|_| error("plugin_install_io"))?;
        }
    }
    Ok(directory)
}

impl PreparedPluginPackage {
    pub async fn prepare(
        archive: PathBuf,
        staging_parent: PathBuf,
        platform: &str,
    ) -> Result<Self, RuntimeError> {
        let directory = tokio::task::spawn_blocking(move || extract(&archive, &staging_parent))
            .await
            .map_err(|_| error("plugin_install_io"))??;
        let package = load_plugin_package(&directory.root, platform)
            .await
            .map_err(sanitized)?;
        archive_path(&package.manifest.id)?;
        if let Some(frontend) = &package.frontend_entry {
            let metadata = tokio::fs::metadata(frontend)
                .await
                .map_err(|_| error("frontend_unavailable"))?;
            if metadata.len() > MAX_FRONTEND_BYTES {
                return Err(error("frontend_too_large"));
            }
            tokio::fs::read_to_string(frontend)
                .await
                .map_err(|_| error("frontend_unavailable"))?;
        }
        // Validate every declared platform entry, not just the current target.
        for target in package.manifest.backend.keys() {
            load_plugin_package(&directory.root, target)
                .await
                .map_err(sanitized)?;
        }
        Ok(Self {
            directory,
            manifest: package.manifest,
        })
    }

    /// Publish with a same-filesystem rename, then register without starting a process.
    /// The alpha installer intentionally refuses every same-ID overwrite.
    pub async fn install(
        self,
        plugins_dir: PathBuf,
        runtime: &ProcessPluginRuntime,
    ) -> Result<PluginManifest, RuntimeError> {
        if runtime.is_discovered(&self.manifest.id).await {
            return Err(error("plugin_install_conflict"));
        }
        let mut prepared = tokio::task::spawn_blocking(move || {
            fs::create_dir_all(&plugins_dir).map_err(|_| error("plugin_install_io"))?;
            let destination = plugins_dir.join(&self.manifest.id);
            if destination.symlink_metadata().is_ok() {
                return Err(error("plugin_install_conflict"));
            }
            let mut prepared = self;
            fs::rename(&prepared.directory.root, &destination)
                .map_err(|_| error("plugin_install_io"))?;
            prepared.directory.root = destination;
            Ok(prepared)
        })
        .await
        .map_err(|_| error("plugin_install_io"))??;
        let manifest = runtime
            .register_installed(&prepared.directory.root)
            .await
            .map_err(sanitized)?;
        prepared.directory.keep = true;
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coterm_proto::runtime::{lifecycle::RuntimeLimits, stream::FlowWindow};
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> io::Result<Self> {
            let root =
                std::env::temp_dir().join(format!("coterm-install-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root)?;
            Ok(Self(root))
        }
        fn archive(&self, files: &[(&str, &[u8])]) -> Result<PathBuf, Box<dyn std::error::Error>> {
            let path = self.0.join(format!("{}.coterm", uuid::Uuid::new_v4()));
            let mut zip = zip::ZipWriter::new(File::create(&path)?);
            for (name, bytes) in files {
                zip.start_file(*name, SimpleFileOptions::default().unix_permissions(0o755))?;
                zip.write_all(bytes)?;
            }
            zip.finish()?;
            Ok(path)
        }
        fn package(&self, id: &str, platform: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
            let mut manifest: serde_json::Value =
                serde_json::from_str(include_str!("../../../plugins/plugin-local/manifest.json"))?;
            manifest["id"] = id.into();
            manifest["backend"] = serde_json::json!({ platform: "backend/plugin" });
            let bytes = serde_json::to_vec(&manifest)?;
            self.archive(&[
                ("manifest.json", &bytes),
                ("backend/plugin", b"not executed"),
            ])
        }
        fn staging(&self) -> PathBuf {
            self.0.join("staging")
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn runtime() -> ProcessPluginRuntime {
        ProcessPluginRuntime::new(
            "windows-x64",
            RuntimeLimits {
                max_control_bytes: 1024 * 1024,
                max_data_chunk_bytes: 65536,
                initial_window: FlowWindow {
                    chunks: 16,
                    bytes: 1024 * 1024,
                },
            },
        )
    }

    #[tokio::test]
    async fn preview_does_not_execute_and_install_is_immediately_discoverable() -> TestResult {
        let scratch = Scratch::new()?;
        let archive = scratch.package("test.offline", "windows-x64")?;
        let prepared =
            PreparedPluginPackage::prepare(archive, scratch.staging(), "windows-x64").await?;
        assert_eq!(prepared.manifest.id, "test.offline");
        let runtime = runtime();
        assert!(runtime.loaded_manifests().await.is_empty());
        let plugins = scratch.0.join("plugins");
        prepared.install(plugins.clone(), &runtime).await?;
        assert!(plugins.join("test.offline/manifest.json").is_file());
        assert!(!runtime.is_running("test.offline").await);
        assert_eq!(runtime.loaded_manifests().await.len(), 1);
        let catalog = runtime
            .discover(vec![crate::plugin_manager::DiscoveredPackage {
                directory: "test.offline".to_owned(),
                root: plugins.join("test.offline"),
                priority: 0,
                origin: crate::plugin_manager::PackageOrigin::Installed,
            }])
            .await;
        assert!(
            catalog.issues.is_empty(),
            "Refreshing an installed package must not report a conflict"
        );
        assert_eq!(catalog.plugins.len(), 1);
        // A fresh runtime discovers the persistent installation after a restart.
        let fresh = self::runtime();
        let reloaded = fresh
            .discover(vec![crate::plugin_manager::DiscoveredPackage {
                directory: "test.offline".to_owned(),
                root: plugins.join("test.offline"),
                priority: 0,
                origin: crate::plugin_manager::PackageOrigin::Installed,
            }])
            .await;
        assert!(reloaded.issues.is_empty());
        assert_eq!(reloaded.plugins.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn duplicate_install_keeps_the_original_and_cleans_staging() -> TestResult {
        let scratch = Scratch::new()?;
        let runtime = runtime();
        let archive = scratch.package("test.offline", "windows-x64")?;
        let plugins = scratch.0.join("plugins");
        PreparedPluginPackage::prepare(archive.clone(), scratch.staging(), "windows-x64")
            .await?
            .install(plugins.clone(), &runtime)
            .await?;
        let failure = PreparedPluginPackage::prepare(archive, scratch.staging(), "windows-x64")
            .await?
            .install(plugins.clone(), &runtime)
            .await
            .err()
            .ok_or("duplicate unexpectedly installed")?;
        assert_eq!(failure.code, "plugin_install_conflict");
        assert_eq!(
            fs::read(plugins.join("test.offline/backend/plugin"))?,
            b"not executed"
        );
        assert_eq!(fs::read_dir(scratch.staging())?.count(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn unsafe_and_duplicate_archive_paths_are_rejected_and_cleaned() -> TestResult {
        let scratch = Scratch::new()?;
        for name in [
            "../escape",
            "/absolute",
            "C:/escape",
            "backend\\escape",
            "CON",
            "CONIN$",
            "CONOUT$",
            "COM¹",
            "lpt².txt",
            "foo/../escape",
            "file.",
            "dir//file",
        ] {
            assert!(archive_path(name).is_err(), "accepted {name}");
        }
        for files in [
            vec![("../escape", b"x".as_slice())],
            vec![("File", b"x".as_slice()), ("file", b"y".as_slice())],
        ] {
            let archive = scratch.archive(&files)?;
            let failure = PreparedPluginPackage::prepare(archive, scratch.staging(), "windows-x64")
                .await
                .err()
                .ok_or("unsafe package accepted")?;
            assert_eq!(failure.code, "plugin_archive_unsafe");
            assert_eq!(fs::read_dir(scratch.staging())?.count(), 0);
        }
        assert!(!scratch.0.join("escape").exists());
        Ok(())
    }

    #[tokio::test]
    async fn invalid_and_wrong_platform_packages_never_publish_or_expose_paths() -> TestResult {
        let scratch = Scratch::new()?;
        let malformed = scratch.archive(&[("manifest.json", b"not json")])?;
        let wrong_platform = scratch.package("test.offline", "linux-x64")?;
        let missing = scratch.archive(&[("backend/plugin", b"unused")])?;
        let invalid_id = scratch.package("..", "windows-x64")?;
        for (archive, expected) in [
            (malformed, "plugin_manifest_invalid"),
            (wrong_platform, "plugin_platform_unsupported"),
            (missing, "plugin_manifest_unavailable"),
            (invalid_id, "plugin_archive_unsafe"),
        ] {
            let failure = PreparedPluginPackage::prepare(archive, scratch.staging(), "windows-x64")
                .await
                .err()
                .ok_or("invalid package accepted")?;
            assert_eq!(failure.code, expected);
            assert_eq!(failure.message, expected);
            assert!(failure.details.is_none());
            assert_eq!(fs::read_dir(scratch.staging())?.count(), 0);
        }
        let mut manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../../plugins/plugin-local/manifest.json"))?;
        manifest["backend"] = serde_json::json!({ "windows-x64": "backend/plugin" });
        manifest["frontend"] = serde_json::json!({ "entry": "frontend/index.js" });
        let manifest = serde_json::to_vec(&manifest)?;
        for (frontend, expected) in [
            (
                vec![b'x'; MAX_FRONTEND_BYTES as usize + 1],
                "frontend_too_large",
            ),
            (vec![0xff], "frontend_unavailable"),
        ] {
            let archive = scratch.archive(&[
                ("manifest.json", &manifest),
                ("backend/plugin", b"not executed"),
                ("frontend/index.js", &frontend),
            ])?;
            let failure = PreparedPluginPackage::prepare(archive, scratch.staging(), "windows-x64")
                .await
                .err()
                .ok_or("invalid frontend accepted")?;
            assert_eq!(failure.code, expected);
            assert_eq!(failure.message, expected);
            assert_eq!(fs::read_dir(scratch.staging())?.count(), 0);
        }
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_preview_removes_extracted_files() -> TestResult {
        let scratch = Scratch::new()?;
        let archive = scratch.package("test.offline", "windows-x64")?;
        let preview =
            PreparedPluginPackage::prepare(archive, scratch.staging(), "windows-x64").await?;
        assert_eq!(fs::read_dir(scratch.staging())?.count(), 1);
        drop(preview);
        assert_eq!(fs::read_dir(scratch.staging())?.count(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn oversized_archive_is_rejected_before_extraction() -> TestResult {
        let scratch = Scratch::new()?;
        let path = scratch.0.join("large.coterm");
        File::create(&path)?.set_len(MAX_ARCHIVE_BYTES + 1)?;
        let failure = PreparedPluginPackage::prepare(path, scratch.staging(), "windows-x64")
            .await
            .err()
            .ok_or("large package accepted")?;
        assert_eq!(failure.code, "plugin_archive_limit");
        assert!(!scratch.staging().exists());
        Ok(())
    }

    #[tokio::test]
    async fn links_and_excessive_entry_counts_are_rejected() -> TestResult {
        let scratch = Scratch::new()?;
        let linked = scratch.0.join("link.coterm");
        let mut writer = zip::ZipWriter::new(File::create(&linked)?);
        writer.add_symlink(
            "backend/plugin",
            "../../outside",
            SimpleFileOptions::default(),
        )?;
        writer.finish()?;
        let failure = PreparedPluginPackage::prepare(linked, scratch.staging(), "windows-x64")
            .await
            .err()
            .ok_or("symlink accepted")?;
        assert_eq!(failure.code, "plugin_archive_unsafe");
        assert_eq!(fs::read_dir(scratch.staging())?.count(), 0);
        let crowded = scratch.0.join("crowded.coterm");
        let mut writer = zip::ZipWriter::new(File::create(&crowded)?);
        for number in 0..=MAX_ENTRIES {
            writer.start_file(number.to_string(), SimpleFileOptions::default())?;
        }
        writer.finish()?;
        let failure = PreparedPluginPackage::prepare(crowded, scratch.staging(), "windows-x64")
            .await
            .err()
            .ok_or("excessive entries accepted")?;
        assert_eq!(failure.code, "plugin_archive_limit");
        Ok(())
    }

    #[tokio::test]
    async fn an_existing_disk_package_is_preserved_before_runtime_discovery() -> TestResult {
        let scratch = Scratch::new()?;
        let destination = scratch.0.join("plugins/test.offline");
        fs::create_dir_all(&destination)?;
        fs::write(destination.join("preserve.txt"), b"original")?;
        let archive = scratch.package("test.offline", "windows-x64")?;
        let failure = PreparedPluginPackage::prepare(archive, scratch.staging(), "windows-x64")
            .await?
            .install(scratch.0.join("plugins"), &runtime())
            .await
            .err()
            .ok_or("existing directory overwritten")?;
        assert_eq!(failure.code, "plugin_install_conflict");
        assert_eq!(fs::read(destination.join("preserve.txt"))?, b"original");
        assert_eq!(fs::read_dir(scratch.staging())?.count(), 0);
        Ok(())
    }
}
