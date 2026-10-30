use super::*;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> anyhow::Result<Self> {
        let root =
            std::env::temp_dir().join(format!("coterm-workspace-groups-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root)?;
        Ok(Self(root))
    }

    fn path(&self) -> PathBuf {
        self.0.join("workspace.bin")
    }
    fn store(&self) -> Result<WorkspaceStore, RuntimeError> {
        WorkspaceStore::open(self.path())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn groups_move_live_and_saved_entries_without_changing_instance_identity() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let mut store = fixture.store()?;
    store.insert(
        "test".into(),
        "live".into(),
        "Connection".into(),
        serde_json::json!({"private":"configuration"}),
        None,
    )?;
    let config_ref = store.snapshot().entries[0].config_ref.clone();
    let group = store.create_group("  Devices  ".into())?;
    assert_eq!(group.name, "Devices");
    store.move_entry(&config_ref, &group.id)?;
    let snapshot = store.snapshot();
    assert_eq!(snapshot.entries[0].workspace_id, group.id);
    assert_eq!(snapshot.entries[0].instance_id.as_deref(), Some("live"));
    assert!(!serde_json::to_string(&snapshot)?.contains("configuration"));
    store.delete_group(&group.id)?;
    assert_eq!(store.snapshot().entries[0].workspace_id, "default");
    assert_eq!(store.snapshot().entries[0].config_ref, config_ref);
    assert_eq!(store.snapshot().layout.active_workspace_id, "default");
    assert_eq!(
        store.delete_group("default").err().map(|error| error.code),
        Some("workspace_last_group".into())
    );
    Ok(())
}

#[test]
fn invalid_names_references_and_group_capacity_do_not_change_state() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let mut store = fixture.store()?;
    for invalid in [
        String::new(),
        "  ".into(),
        "bad\nname".into(),
        "界".repeat(65),
    ] {
        assert_eq!(
            store.create_group(invalid).err().map(|error| error.code),
            Some("workspace_group_name_invalid".into())
        );
    }
    let group = store.create_group("Devices".into())?;
    assert_eq!(
        store
            .create_group("devices".into())
            .err()
            .map(|error| error.code),
        Some("workspace_group_name_exists".into())
    );
    assert_eq!(
        store
            .move_entry("missing", &group.id)
            .err()
            .map(|error| error.code),
        Some("workspace_entry_not_found".into())
    );
    assert_eq!(
        store.delete_group("missing").err().map(|error| error.code),
        Some("workspace_group_not_found".into())
    );
    assert_eq!(
        store
            .insert(
                "test".into(),
                "live".into(),
                "Connection".into(),
                Value::Null,
                Some("missing".into())
            )
            .err()
            .map(|error| error.code),
        Some("workspace_group_not_found".into())
    );
    assert!(store.snapshot().entries.is_empty());
    for index in 2..MAX_GROUPS {
        store.create_group(format!("Group {index}"))?;
    }
    assert_eq!(
        store
            .create_group("Overflow".into())
            .err()
            .map(|error| error.code),
        Some("workspace_group_capacity".into())
    );
    assert_eq!(store.snapshot().groups.len(), MAX_GROUPS);
    Ok(())
}

#[test]
fn layout_and_explicit_creation_group_share_the_same_membership() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let mut store = fixture.store()?;
    let group = store.create_group("Lab".into())?;
    store.insert(
        "test".into(),
        "first".into(),
        "First".into(),
        Value::Null,
        None,
    )?;
    store.insert(
        "test".into(),
        "second".into(),
        "Second".into(),
        Value::Null,
        Some("default".into()),
    )?;
    store.save_layout(
        Some("second".into()),
        true,
        vec!["second".into(), "first".into()],
        Some("default".into()),
    )?;
    let snapshot = store.snapshot();
    assert_eq!(snapshot.entries[0].instance_id.as_deref(), Some("second"));
    assert_eq!(snapshot.entries[1].workspace_id, group.id);
    assert_eq!(
        snapshot.layout.active_config_ref.as_deref(),
        Some(snapshot.entries[0].config_ref.as_str())
    );
    assert!(snapshot.layout.instances_collapsed);
    assert_eq!(snapshot.layout.active_workspace_id, "default");
    Ok(())
}

#[cfg(windows)]
#[test]
fn legacy_v1_migrates_into_default_group_and_preserves_encrypted_config() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let legacy = serde_json::json!({"version":1,"entries":[{
        "configRef":"legacy","pluginId":"test","label":"Legacy","config":{"secret":"kept-private"}
    }],"layout":{"activeConfigRef":"legacy","instancesCollapsed":true}});
    fs::write(
        fixture.path(),
        protect(&serde_json::to_vec(&legacy)?, true)?,
    )?;
    let mut store = fixture.store()?;
    assert_eq!(store.snapshot().groups[0].id, "default");
    assert_eq!(store.find("legacy")?.workspace_id, "default");
    let group = store.create_group("Migrated".into())?;
    store.move_entry("legacy", &group.id)?;
    let reopened = fixture.store()?;
    assert_eq!(reopened.find("legacy")?.workspace_id, group.id);
    assert_eq!(reopened.find("legacy")?.config["secret"], "kept-private");
    assert!(!String::from_utf8_lossy(&fs::read(fixture.path())?).contains("kept-private"));
    assert!(!serde_json::to_string(&reopened.snapshot())?.contains("kept-private"));
    Ok(())
}

#[cfg(windows)]
#[test]
fn only_latest_used_session_is_saved_and_empty_visits_do_not_destroy_recovery() -> anyhow::Result<()>
{
    let fixture = Fixture::new()?;
    let mut first = fixture.store()?;
    first.insert("test".into(), "a".into(), "A".into(), Value::Null, None)?;
    first.insert("test".into(), "b".into(), "B".into(), Value::Null, None)?;
    let a = first.snapshot().entries[0].config_ref.clone();
    let mut visit = fixture.store()?;
    visit.save_layout(None, false, Vec::new(), None)?;
    assert_eq!(
        fixture.store()?.snapshot().entries.len(),
        2,
        "opening without using instances preserves previous recovery"
    );
    visit.bind(&a, "a-restored".into(), "A".into())?;
    assert_eq!(
        visit.snapshot().entries.len(),
        2,
        "the rest of the previous session can still be restored now"
    );
    assert_eq!(
        fixture.store()?.snapshot().entries.len(),
        1,
        "unrestored older sessions are not accumulated on disk"
    );
    visit.insert("test".into(), "c".into(), "C".into(), Value::Null, None)?;
    let mut next = fixture.store()?;
    assert_eq!(next.snapshot().entries.len(), 2);
    let c = next
        .snapshot()
        .entries
        .iter()
        .find(|entry| entry.label == "C")
        .ok_or_else(|| anyhow::anyhow!("missing C"))?
        .config_ref
        .clone();
    next.bind(&c, "c-restored".into(), "C".into())?;
    next.remove(&c)?;
    assert!(
        fixture.store()?.snapshot().entries.is_empty(),
        "deleting the last used instance cannot revive old sessions"
    );
    Ok(())
}

#[cfg(windows)]
#[test]
fn failed_write_preserves_groups_membership_and_session_checkpoint() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let mut store = fixture.store()?;
    store.insert(
        "test".into(),
        "live".into(),
        "Live".into(),
        Value::Null,
        None,
    )?;
    let group = store.create_group("Keep".into())?;
    let reference = store.snapshot().entries[0].config_ref.clone();
    let before = serde_json::to_value(store.snapshot())?;
    store.path = fixture.0.join("absent").join("workspace.bin");
    assert!(store.move_entry(&reference, &group.id).is_err());
    assert!(store.delete_group(&group.id).is_err());
    assert!(store.create_group("Failed".into()).is_err());
    assert_eq!(serde_json::to_value(store.snapshot())?, before);
    store.path = fixture.path();
    assert_eq!(fixture.store()?.snapshot().groups.len(), 2);
    Ok(())
}
