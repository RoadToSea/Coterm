//! Versioned workspace metadata and configuration references. The entire file is
//! protected by the current Windows user's DPAPI key; no plaintext fallback.
use coterm_proto::runtime::{ErrorCategory, RuntimeError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(windows)]
use std::path::Path;
use std::{fs, path::PathBuf};

const MAX_BYTES: usize = 512 * 1024;
const MAX_ENTRIES: usize = 128;
const MAX_GROUPS: usize = 32;

fn default_workspace_id() -> String {
    "default".into()
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceGroup {
    pub id: String,
    pub name: String,
}

fn default_groups() -> Vec<WorkspaceGroup> {
    vec![WorkspaceGroup {
        id: default_workspace_id(),
        name: "Default workspace".into(),
    }]
}

pub fn error(code: &str) -> RuntimeError {
    RuntimeError {
        category: ErrorCategory::Internal,
        code: code.into(),
        message: "Workspace could not be saved or restored. No operations were replayed.".into(),
        details: None,
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedInstance {
    pub config_ref: String,
    pub plugin_id: String,
    pub label: String,
    pub instance_id: Option<String>,
    pub workspace_id: String,
    #[serde(skip_serializing)]
    pub config: Value,
}

// Stored configs are serialized only into the encrypted envelope, never IPC.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Entry {
    config_ref: String,
    plugin_id: String,
    label: String,
    config: Value,
    #[serde(default = "default_workspace_id")]
    workspace_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Layout {
    pub active_config_ref: Option<String>,
    pub instances_collapsed: bool,
    #[serde(default = "default_workspace_id")]
    pub active_workspace_id: String,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            active_config_ref: None,
            instances_collapsed: false,
            active_workspace_id: default_workspace_id(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Document {
    version: u32,
    entries: Vec<Entry>,
    layout: Layout,
    /// Operations the user allowed to skip Agent approval. Empty by default.
    #[serde(default)]
    allowed_operations: Vec<String>,
    #[serde(default = "default_groups")]
    groups: Vec<WorkspaceGroup>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub entries: Vec<SavedInstance>,
    pub layout: Layout,
    pub groups: Vec<WorkspaceGroup>,
    pub allowed_operations: Vec<String>,
    pub history_policy: &'static str,
    pub persistence: &'static str,
}

pub struct WorkspaceStore {
    #[cfg(windows)]
    path: PathBuf,
    entries: Vec<SavedInstance>,
    layout: Layout,
    allowed_operations: Vec<String>,
    groups: Vec<WorkspaceGroup>,
    // Keep the previous session available for recovery in memory, but once this
    // session has instances, only this session is written to the next snapshot.
    session_started: bool,
}

impl WorkspaceStore {
    pub fn empty(path: PathBuf) -> Self {
        #[cfg(not(windows))]
        let _ = path;
        Self {
            #[cfg(windows)]
            path,
            entries: Vec::new(),
            layout: Layout::default(),
            allowed_operations: Vec::new(),
            groups: default_groups(),
            session_started: false,
        }
    }
    pub fn open(path: PathBuf) -> Result<Self, RuntimeError> {
        if !path.exists() {
            return Ok(Self::empty(path));
        }
        if fs::metadata(&path)
            .map_err(|_| error("workspace_read_failed"))?
            .len()
            > (MAX_BYTES + 4096) as u64
        {
            return Err(error("workspace_too_large"));
        }
        let encrypted = fs::read(&path).map_err(|_| error("workspace_read_failed"))?;
        let mut bytes = protect(&encrypted, false)?;
        let parsed = serde_json::from_slice::<Document>(&bytes);
        bytes.fill(0);
        let doc = parsed.map_err(|_| error("workspace_invalid"))?;
        if doc.version != 1 || doc.entries.len() > MAX_ENTRIES {
            return Err(error("workspace_version_unsupported"));
        }
        let mut group_ids = std::collections::HashSet::new();
        let mut group_names = std::collections::HashSet::new();
        if doc.groups.is_empty()
            || doc.groups.len() > MAX_GROUPS
            || doc.groups.iter().any(|group| {
                group.id.is_empty()
                    || group.id.len() > 64
                    || !group_ids.insert(group.id.as_str())
                    || !valid_group_name(&group.name)
                    || !group_names.insert(group.name.to_lowercase())
            })
            || !group_ids.contains(doc.layout.active_workspace_id.as_str())
        {
            return Err(error("workspace_invalid"));
        }
        let mut ids = std::collections::HashSet::new();
        if doc.entries.iter().any(|entry| {
            entry.config_ref.len() > 64
                || entry.config_ref.is_empty()
                || !ids.insert(&entry.config_ref)
                || entry.plugin_id.len() > 256
                || entry.label.len() > 256
                || !group_ids.contains(entry.workspace_id.as_str())
        }) {
            return Err(error("workspace_invalid"));
        }
        if doc.allowed_operations.len() > MAX_ENTRIES
            || doc.allowed_operations.iter().any(|operation| {
                operation.trim().is_empty()
                    || operation.len() > 128
                    || operation.chars().any(char::is_control)
            })
        {
            return Err(error("workspace_invalid"));
        }
        Ok(Self {
            #[cfg(windows)]
            path,
            allowed_operations: doc.allowed_operations,
            layout: doc.layout,
            groups: doc.groups,
            session_started: false,
            entries: doc
                .entries
                .into_iter()
                .map(|entry| SavedInstance {
                    config_ref: entry.config_ref,
                    plugin_id: entry.plugin_id,
                    label: entry.label,
                    config: entry.config,
                    instance_id: None,
                    workspace_id: entry.workspace_id,
                })
                .collect(),
        })
    }
    pub fn snapshot(&self) -> Snapshot {
        // Config is skipped by serde. It never crosses the host UI or MCP boundary.
        Snapshot {
            entries: self.entries.clone(),
            layout: self.layout.clone(),
            groups: self.groups.clone(),
            allowed_operations: self.allowed_operations.clone(),
            history_policy: "session_only",
            persistence: if cfg!(windows) {
                "windows_dpapi"
            } else {
                "memory_only"
            },
        }
    }
    pub fn find(&self, id: &str) -> Result<SavedInstance, RuntimeError> {
        self.entries
            .iter()
            .find(|entry| entry.config_ref == id)
            .cloned()
            .ok_or_else(|| error("workspace_entry_not_found"))
    }
    pub fn insert(
        &mut self,
        plugin_id: String,
        instance_id: String,
        label: String,
        config: Value,
        workspace_id: Option<String>,
    ) -> Result<(), RuntimeError> {
        if contains_runtime_handle(&config) {
            return Err(error("workspace_ephemeral_secret_handle"));
        }
        let workspace_id = workspace_id.unwrap_or_else(|| self.layout.active_workspace_id.clone());
        self.require_group(&workspace_id)?;
        let mut entries = self.entries.clone();
        if entries.len() >= MAX_ENTRIES {
            if let Some(index) = entries.iter().position(|entry| entry.instance_id.is_none()) {
                entries.remove(index);
            } else {
                return Err(error("workspace_capacity"));
            }
        }
        entries.push(SavedInstance {
            config_ref: uuid::Uuid::new_v4().to_string(),
            plugin_id,
            label,
            config,
            instance_id: Some(instance_id),
            workspace_id,
        });
        self.commit(entries, self.layout.clone())
    }
    pub fn bind(
        &mut self,
        id: &str,
        instance_id: String,
        label: String,
    ) -> Result<(), RuntimeError> {
        let mut entries = self.entries.clone();
        let entry = entries
            .iter_mut()
            .find(|entry| entry.config_ref == id)
            .ok_or_else(|| error("workspace_entry_not_found"))?;
        entry.instance_id = Some(instance_id);
        entry.label = label;
        self.commit(entries, self.layout.clone())
    }
    pub fn remove(&mut self, id: &str) -> Result<(), RuntimeError> {
        let entries = self
            .entries
            .iter()
            .filter(|entry| entry.config_ref != id)
            .cloned()
            .collect();
        let mut layout = self.layout.clone();
        if layout.active_config_ref.as_deref() == Some(id) {
            layout.active_config_ref = None;
        }
        self.commit(entries, layout)
    }
    pub fn remove_instance(&mut self, id: &str) -> Result<(), RuntimeError> {
        if let Some(entry) = self
            .entries
            .iter()
            .find(|entry| entry.instance_id.as_deref() == Some(id))
        {
            return self.remove(&entry.config_ref.clone());
        }
        Ok(())
    }
    pub fn save_layout(
        &mut self,
        active: Option<String>,
        collapsed: bool,
        order: Vec<String>,
        active_workspace_id: Option<String>,
    ) -> Result<(), RuntimeError> {
        let active_workspace_id =
            active_workspace_id.unwrap_or_else(|| self.layout.active_workspace_id.clone());
        self.require_group(&active_workspace_id)?;
        let mut entries = self.entries.clone();
        entries.sort_by_key(|entry| {
            order
                .iter()
                .position(|id| Some(id) == entry.instance_id.as_ref())
                .unwrap_or(usize::MAX)
        });
        let active_config_ref = active
            .and_then(|id| {
                entries
                    .iter()
                    .find(|entry| entry.instance_id.as_deref() == Some(&id))
                    .map(|entry| entry.config_ref.clone())
            })
            .or_else(|| self.layout.active_config_ref.clone());
        self.commit(
            entries,
            Layout {
                active_config_ref,
                instances_collapsed: collapsed,
                active_workspace_id,
            },
        )
    }
    fn commit(&mut self, entries: Vec<SavedInstance>, layout: Layout) -> Result<(), RuntimeError> {
        self.commit_state(entries, layout, self.groups.clone())
    }

    fn commit_state(
        &mut self,
        entries: Vec<SavedInstance>,
        layout: Layout,
        groups: Vec<WorkspaceGroup>,
    ) -> Result<(), RuntimeError> {
        let session_started =
            self.session_started || entries.iter().any(|entry| entry.instance_id.is_some());
        let mut saved_layout = layout.clone();
        if session_started
            && !entries.iter().any(|entry| {
                entry.instance_id.is_some()
                    && Some(&entry.config_ref) == saved_layout.active_config_ref.as_ref()
            })
        {
            saved_layout.active_config_ref = None;
        }
        let doc = Document {
            version: 1,
            entries: entries
                .iter()
                .filter(|entry| !session_started || entry.instance_id.is_some())
                .map(|entry| Entry {
                    config_ref: entry.config_ref.clone(),
                    plugin_id: entry.plugin_id.clone(),
                    label: entry.label.clone(),
                    config: entry.config.clone(),
                    workspace_id: entry.workspace_id.clone(),
                })
                .collect(),
            layout: saved_layout,
            allowed_operations: self.allowed_operations.clone(),
            groups: groups.clone(),
        };
        let mut bytes = serde_json::to_vec(&doc).map_err(|_| error("workspace_invalid"))?;
        if bytes.len() > MAX_BYTES {
            bytes.fill(0);
            return Err(error("workspace_too_large"));
        }
        #[cfg(windows)]
        let encrypted = protect(&bytes, true);
        bytes.fill(0);
        #[cfg(windows)]
        atomic_write(&self.path, &encrypted?)?;
        self.entries = entries;
        self.layout = layout;
        self.groups = groups;
        self.session_started = session_started;
        Ok(())
    }

    fn require_group(&self, id: &str) -> Result<(), RuntimeError> {
        if self.groups.iter().any(|group| group.id == id) {
            Ok(())
        } else {
            Err(error("workspace_group_not_found"))
        }
    }

    pub fn create_group(&mut self, name: String) -> Result<WorkspaceGroup, RuntimeError> {
        let name = name.trim().to_owned();
        if !valid_group_name(&name) {
            return Err(error("workspace_group_name_invalid"));
        }
        if self.groups.len() >= MAX_GROUPS {
            return Err(error("workspace_group_capacity"));
        }
        if self
            .groups
            .iter()
            .any(|group| group.name.to_lowercase() == name.to_lowercase())
        {
            return Err(error("workspace_group_name_exists"));
        }
        let group = WorkspaceGroup {
            id: uuid::Uuid::new_v4().to_string(),
            name,
        };
        let mut groups = self.groups.clone();
        groups.push(group.clone());
        let mut layout = self.layout.clone();
        layout.active_workspace_id = group.id.clone();
        self.commit_state(self.entries.clone(), layout, groups)?;
        Ok(group)
    }

    pub fn delete_group(&mut self, id: &str) -> Result<(), RuntimeError> {
        self.require_group(id)?;
        let groups: Vec<_> = self
            .groups
            .iter()
            .filter(|group| group.id != id)
            .cloned()
            .collect();
        let target = groups
            .first()
            .ok_or_else(|| error("workspace_last_group"))?
            .id
            .clone();
        let mut entries = self.entries.clone();
        for entry in &mut entries {
            if entry.workspace_id == id {
                entry.workspace_id = target.clone();
            }
        }
        let mut layout = self.layout.clone();
        if layout.active_workspace_id == id {
            layout.active_workspace_id = target;
        }
        self.commit_state(entries, layout, groups)
    }

    pub fn move_entry(&mut self, config_ref: &str, workspace_id: &str) -> Result<(), RuntimeError> {
        self.require_group(workspace_id)?;
        let mut entries = self.entries.clone();
        let entry = entries
            .iter_mut()
            .find(|entry| entry.config_ref == config_ref)
            .ok_or_else(|| error("workspace_entry_not_found"))?;
        entry.workspace_id = workspace_id.to_owned();
        self.commit(entries, self.layout.clone())
    }

    /// Persists the approval allowlist. The list is stored inside the same encrypted
    /// envelope as instance configuration and never crosses the MCP boundary.
    pub fn save_allowed_operations(&mut self, allowed: Vec<String>) -> Result<(), RuntimeError> {
        if allowed.len() > MAX_ENTRIES {
            return Err(error("workspace_capacity"));
        }
        let mut normalized = Vec::new();
        for operation in allowed {
            let operation = operation.trim().to_owned();
            if operation.is_empty()
                || operation.len() > 128
                || operation.chars().any(char::is_control)
            {
                return Err(error("workspace_invalid"));
            }
            if !normalized.contains(&operation) {
                normalized.push(operation);
            }
        }
        normalized.sort();
        let entries = self.entries.clone();
        let previous = std::mem::replace(&mut self.allowed_operations, normalized);
        if let Err(error) = self.commit(entries, self.layout.clone()) {
            self.allowed_operations = previous;
            return Err(error);
        }
        Ok(())
    }
}

fn valid_group_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name.trim() == name
        && name.chars().count() <= 64
        && !name.chars().any(char::is_control)
}

fn contains_runtime_handle(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            ["id", "pluginInstanceId", "purpose", "expiresAt"]
                .iter()
                .all(|key| object.contains_key(*key))
                || object.values().any(contains_runtime_handle)
        }
        Value::Array(values) => values.iter().any(contains_runtime_handle),
        _ => false,
    }
}

#[cfg(windows)]
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), RuntimeError> {
    use std::io::Write;
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        replace_file(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result.map_err(|_| error("workspace_write_failed"))
}

#[cfg(windows)]
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: both paths are NUL-terminated and live for the synchronous call.
    if unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(windows)]
fn protect(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>, RuntimeError> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
    };
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: DPAPI reads the input synchronously and allocates output with LocalAlloc.
    let ok = unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if ok == 0 {
        return Err(error("workspace_crypto_failed"));
    }
    // SAFETY: successful DPAPI output contains cbData initialized bytes. Wipe before release.
    let result = unsafe {
        let slice = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let result = slice.to_vec();
        slice.fill(0);
        LocalFree(output.pbData.cast());
        result
    };
    Ok(result)
}
#[cfg(not(windows))]
fn protect(_: &[u8], _: bool) -> Result<Vec<u8>, RuntimeError> {
    Err(error("workspace_secure_storage_unsupported"))
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod group_tests;

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn encrypted_roundtrip_layout_references_and_failed_write_preserves_state() {
        let root = std::env::temp_dir().join(format!("coterm-workspace-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("temp dir");
        let path = root.join("workspace.bin");
        let mut store = WorkspaceStore::open(path.clone()).expect("new store");
        store
            .insert(
                "test.plugin".into(),
                "old-instance".into(),
                "Test".into(),
                serde_json::json!({"password":"private-roundtrip-secret"}),
                None,
            )
            .expect("insert");
        let id = store.snapshot().entries[0].config_ref.clone();
        store
            .save_layout(
                Some("old-instance".into()),
                true,
                vec!["old-instance".into()],
                None,
            )
            .expect("layout");
        assert!(!String::from_utf8_lossy(&fs::read(&path).expect("read"))
            .contains("private-roundtrip-secret"));
        let ipc = serde_json::to_string(&store.snapshot()).expect("snapshot");
        assert!(!ipc.contains("password"));
        assert!(!ipc.contains("private-roundtrip-secret"));
        assert!(ipc.contains("\"instanceId\":\"old-instance\""));
        let restored = WorkspaceStore::open(path.clone()).expect("reopen");
        assert_eq!(
            restored.find(&id).expect("config").config["password"],
            "private-roundtrip-secret"
        );
        assert!(restored.snapshot().entries[0].instance_id.is_none());
        assert_eq!(
            restored.snapshot().layout.active_config_ref,
            Some(id.clone())
        );
        assert!(restored.snapshot().layout.instances_collapsed);
        let too_large = Value::String("x".repeat(MAX_BYTES));
        assert_eq!(
            store
                .insert(
                    "test.plugin".into(),
                    "new".into(),
                    "Too large".into(),
                    too_large,
                    None,
                )
                .expect_err("error")
                .code,
            "workspace_too_large"
        );
        assert_eq!(store.snapshot().entries.len(), 1);
        assert_eq!(store.insert("test.plugin".into(), "invalid".into(), "Handle".into(),
            serde_json::json!({"nested":{"id":"secret","pluginInstanceId":"instance","purpose":"password","expiresAt":"soon"}}), None)
            .expect_err("ephemeral handle").code, "workspace_ephemeral_secret_handle");
        store.path = root.join("missing").join("workspace.bin");
        assert_eq!(
            store
                .insert(
                    "test.plugin".into(),
                    "new".into(),
                    "Write failure".into(),
                    Value::Null,
                    None,
                )
                .expect_err("write failure")
                .code,
            "workspace_write_failed"
        );
        assert_eq!(store.snapshot().entries.len(), 1);
        store.path = path.clone();
        store.remove(&id).expect("remove");
        assert!(WorkspaceStore::open(path.clone())
            .expect("open")
            .snapshot()
            .entries
            .is_empty());
        fs::write(&path, b"corrupt").expect("corrupt");
        assert!(WorkspaceStore::open(path).is_err());
        fs::remove_dir_all(root).expect("cleanup");
    }
}
