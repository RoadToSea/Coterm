use super::*;

#[cfg(windows)]
#[test]
fn instance_names_reserve_offline_entries_without_changing_the_storage_format() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let mut store = fixture.store()?;
    store.insert("ssh".into(), "one".into(), " Board ".into(), Value::Null, None)?;
    let reference = store.snapshot().entries[0].config_ref.clone();
    let mut reopened = fixture.store()?;
    assert_eq!(reopened.find(&reference)?.name, "Board");
    assert!(reopened.find(&reference)?.instance_id.is_none());
    let duplicate = reopened.insert(
        "serial".into(),
        "two".into(),
        "Board".into(),
        Value::Null,
        None,
    );
    assert_eq!(
        duplicate.err().map(|error| error.code).as_deref(),
        Some("instance_name_exists")
    );
    reopened.bind(&reference, "restored".into(), "Board".into())?;
    let snapshot = serde_json::to_value(reopened.snapshot())?;
    assert_eq!(snapshot["entries"][0]["name"], "Board");
    assert!(snapshot["entries"][0].get("label").is_none());
    let saved: Value = serde_json::from_slice(&protect(&fs::read(fixture.path())?, false)?)?;
    assert_eq!(saved["version"], 5);
    assert_eq!(saved["entries"][0]["label"], "Board");
    assert!(saved["entries"][0].get("name").is_none());
    reopened.remove(&reference)?;
    reopened.insert("serial".into(), "two".into(), "Board".into(), Value::Null, None)?;
    Ok(())
}

#[cfg(windows)]
#[test]
fn bypass_survives_rebinding_but_does_not_follow_other_instances() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let mut store = fixture.store()?;
    for id in ["one", "two"] {
        store.insert("ssh".into(), id.into(), id.into(), Value::Null, None)?;
    }
    let reference = store.snapshot().entries[0].config_ref.clone();
    let other = store.snapshot().entries[1].config_ref.clone();
    store.save_bypass("one", true)?;
    let mut reopened = fixture.store()?;
    assert!(reopened.find(&reference)?.bypass);
    assert!(!reopened.find(&other)?.bypass);
    reopened.bind(&reference, "recreated".into(), "renamed".into())?;
    reopened.bind(&other, "other".into(), "other".into())?;
    reopened.save_bypass("one", false)?;
    assert!(
        fixture.store()?.find(&reference)?.bypass,
        "stale IDs cannot change saved state"
    );
    reopened.save_bypass("recreated", false)?;
    assert!(!fixture.store()?.find(&reference)?.bypass);
    reopened.remove(&reference)?;
    reopened.insert(
        "ssh".into(),
        "new".into(),
        "renamed".into(),
        Value::Null,
        None,
    )?;
    assert!(fixture
        .store()?
        .snapshot()
        .entries
        .iter()
        .all(|entry| !entry.bypass));
    Ok(())
}

#[cfg(windows)]
#[test]
fn bypass_failed_writes_preserve_state_and_temporary_instances_never_write() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let mut store = fixture.store()?;
    store.save_bypass("temporary", true)?;
    assert!(!fixture.path().exists());
    store.insert("ssh".into(), "one".into(), "one".into(), Value::Null, None)?;
    let reference = store.snapshot().entries[0].config_ref.clone();
    store.save_bypass("one", true)?;
    let previous = fs::read(fixture.path())?;
    store.path = fixture.0.join("missing").join("workspace.bin");
    assert!(store.save_bypass("one", false).is_err());
    assert!(store.find(&reference)?.bypass);
    assert!(fixture.store()?.find(&reference)?.bypass);
    assert_eq!(fs::read(fixture.path())?, previous);
    Ok(())
}

#[cfg(windows)]
#[test]
fn older_workspace_versions_default_bypass_to_disabled() -> anyhow::Result<()> {
    for version in 1..=4 {
        let fixture = Fixture::new()?;
        let legacy = serde_json::json!({"version":version,"entries":[
            {"configRef":"one","driverId":"ssh","label":"one","config":{}}
        ],"layout":{"activeConfigRef":null,"instancesCollapsed":false}});
        fs::write(
            fixture.path(),
            protect(&serde_json::to_vec(&legacy)?, true)?,
        )?;
        let mut store = fixture.store()?;
        assert!(!store.find("one")?.bypass);
        store.bind("one", "new".into(), "one".into())?;
        store.save_bypass("new", true)?;
        assert!(fixture.store()?.find("one")?.bypass);
    }
    Ok(())
}

#[cfg(windows)]
#[test]
fn approval_rules_roundtrip_and_failed_writes_preserve_the_saved_policy() -> anyhow::Result<()> {
    use coterm_core::approval_policy::{ApprovalMode, ApprovalPolicy};
    let fixture = Fixture::new()?;
    let mut store = fixture.store()?;
    for id in ["one", "two"] {
        store.insert("ssh".into(), id.into(), id.into(), Value::Null, None)?;
    }
    let reference = store.snapshot().entries[0].config_ref.clone();
    let other = store.snapshot().entries[1].config_ref.clone();
    let policy = ApprovalPolicy {
        mode: ApprovalMode::ApproveMatches,
        patterns: vec![r"\brm\b".into()],
    };
    store.save_approval_policy("one", policy.clone())?;
    assert_eq!(fixture.store()?.approval_policy(&reference)?, policy);
    assert_eq!(
        fixture.store()?.approval_policy(&other)?,
        ApprovalPolicy::default()
    );
    assert!(store
        .save_approval_policy("missing", policy.clone())
        .is_err());
    let serialized = serde_json::to_value(store.snapshot())?;
    assert!(serialized.get("allowedOperations").is_none());
    assert!(serialized.get("approvalPolicy").is_none());
    let disk: Value = serde_json::from_slice(&protect(&fs::read(fixture.path())?, false)?)?;
    assert_eq!(disk["version"], 5);
    assert!(disk.get("approvalPolicy").is_none());
    store.path = fixture.0.join("missing").join("workspace.bin");
    assert!(store
        .save_approval_policy("one", ApprovalPolicy::default())
        .is_err());
    assert_eq!(store.approval_policy(&reference)?, policy);
    assert_eq!(fixture.store()?.approval_policy(&reference)?, policy);
    assert_eq!(
        store.approval_policy(&other)?,
        ApprovalPolicy::default()
    );
    Ok(())
}

#[cfg(windows)]
#[test]
fn legacy_rules_are_copied_to_saved_instances_but_not_to_new_instances() -> anyhow::Result<()> {
    use coterm_core::approval_policy::{ApprovalMode, ApprovalPolicy};
    let fixture = Fixture::new()?;
    let policy = ApprovalPolicy {
        mode: ApprovalMode::AllowMatches,
        patterns: vec!["^ls$".into()],
    };
    let legacy = serde_json::json!({"version":1,"entries":[
        {"configRef":"first","driverId":"ssh","label":"Same driver A","config":{}},
        {"configRef":"second","driverId":"ssh","label":"Same driver B","config":{}}
    ],"layout":{"activeConfigRef":null,"instancesCollapsed":false},"approvalPolicy":policy});
    fs::write(
        fixture.path(),
        protect(&serde_json::to_vec(&legacy)?, true)?,
    )?;
    let mut store = fixture.store()?;
    assert_eq!(store.approval_policy("first")?, policy);
    assert_eq!(store.approval_policy("second")?, policy);
    store.bind("first", "new-first".into(), "Same driver A".into())?;
    store.bind("second", "new-second".into(), "Same driver B".into())?;
    store.save_approval_policy("new-first", ApprovalPolicy::default())?;
    store.insert(
        "ssh".into(),
        "third".into(),
        "New instance".into(),
        Value::Null,
        None,
    )?;
    let reopened = fixture.store()?;
    assert_eq!(
        reopened.approval_policy("first")?,
        ApprovalPolicy::default()
    );
    assert_eq!(reopened.approval_policy("second")?, policy);
    assert_eq!(
        reopened.approval_policy(&reopened.snapshot().entries[2].config_ref)?,
        ApprovalPolicy::default()
    );
    assert!(reopened
        .snapshot()
        .entries
        .iter()
        .all(|entry| entry.instance_id.is_none()));
    Ok(())
}

#[cfg(windows)]
#[test]
fn legacy_interface_grants_never_become_instance_command_rules() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let legacy = serde_json::json!({"version":1,"entries":[
        {"configRef":"first","driverId":"ssh","label":"Server","config":{}}
    ],"layout":{"activeConfigRef":null,"instancesCollapsed":false},"allowedOperations":["ssh.command"]});
    fs::write(
        fixture.path(),
        protect(&serde_json::to_vec(&legacy)?, true)?,
    )?;
    assert_eq!(
        fixture.store()?.approval_policy("first")?,
        coterm_core::approval_policy::ApprovalPolicy::default()
    );
    Ok(())
}

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
        "configRef":"legacy","driverId":"test","label":"Legacy","config":{"secret":"kept-private"}
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
        .find(|entry| entry.name == "C")
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

#[test]
fn configuration_updates_are_bound_and_preserve_identity_and_rules() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let mut store = fixture.store()?;
    store.insert(
        "serial".into(),
        "one".into(),
        "Board".into(),
        serde_json::json!({"port":"COM6","mode":"stream"}),
        None,
    )?;
    store.insert(
        "serial".into(),
        "two".into(),
        "Other".into(),
        serde_json::json!({"port":"COM7"}),
        None,
    )?;
    let before = store.snapshot().entries;
    let reference = before[0].config_ref.clone();
    let config = serde_json::json!({"port":"COM6","mode":"shell","prompt":"# "});
    store.save_configuration("one", config.clone())?;
    assert_eq!(store.find(&reference)?.config, config);
    assert_eq!(store.snapshot().entries[1].config, before[1].config);
    assert_eq!(store.find(&reference)?.name, before[0].name);
    assert_eq!(
        store.find(&reference)?.approval_rule_group_id,
        before[0].approval_rule_group_id
    );
    assert!(store.save_configuration("stale", Value::Null).is_err());
    assert!(store.save_configuration("one", serde_json::json!({"id":"secret","driverInstanceId":"one","purpose":"auth","expiresAt":"later"})).is_err());
    #[cfg(windows)]
    {
        assert_eq!(fixture.store()?.find(&reference)?.config, config);
        store.path = fixture.0.join("absent").join("workspace.bin");
        assert!(store.save_configuration("one", Value::Null).is_err());
        assert_eq!(store.find(&reference)?.config, config);
        assert_eq!(fixture.store()?.find(&reference)?.config, config);
    }
    Ok(())
}

#[cfg(windows)]
#[test]
fn legacy_driver_names_migrate_without_changing_saved_identity_or_policies() -> anyhow::Result<()> {
    use coterm_core::approval_policy::{ApprovalMode, ApprovalPolicy};
    for version in [1, 2] {
        let fixture = Fixture::new()?;
        let policy = ApprovalPolicy {
            mode: ApprovalMode::ApproveMatches,
            patterns: vec!["^reboot$".into()],
        };
        let entries: Vec<_> = ["ssh", "serial", "local"]
            .into_iter()
            .map(|name| {
                serde_json::json!({"configRef":name,"pluginId":format!("coterm-plugin-{name}"),
                "label":name,"config":{"private":"kept"},"approvalPolicy":policy,"retained":true})
            })
            .collect();
        let legacy = serde_json::json!({"version":version,"entries":entries,
            "layout":{"activeConfigRef":"serial","instancesCollapsed":true}});
        fs::write(
            fixture.path(),
            protect(&serde_json::to_vec(&legacy)?, true)?,
        )?;
        let mut store = fixture.store()?;
        for name in ["ssh", "serial", "local"] {
            let entry = store.find(name)?;
            assert_eq!(entry.driver_id, format!("coterm-driver-{name}"));
            assert_eq!(entry.config_ref, name);
            assert!(entry.retained);
            assert_eq!(store.approval_policy(name)?, policy);
            assert_eq!(entry.config["private"], "kept");
            assert!(entry.instance_id.is_none());
        }
        store.save_layout(None, true, vec![], None)?;
        let plaintext = protect(&fs::read(fixture.path())?, false)?;
        let migrated: Value = serde_json::from_slice(&plaintext)?;
        assert_eq!(migrated["version"], 5);
        assert!(migrated["entries"][0].get("pluginId").is_none());
        assert_eq!(migrated["entries"][0]["driverId"], "coterm-driver-ssh");
        assert_eq!(fixture.store()?.approval_policy("serial")?, policy);
        assert_eq!(
            fixture
                .store()?
                .snapshot()
                .layout
                .active_config_ref
                .as_deref(),
            Some("serial")
        );
        assert!(!String::from_utf8_lossy(&fs::read(fixture.path())?).contains("kept"));
    }
    Ok(())
}

#[cfg(windows)]
trait PolicyFixture {
    fn save_approval_policy(
        &mut self,
        instance_id: &str,
        policy: ApprovalPolicy,
    ) -> Result<(), RuntimeError>;
}
#[cfg(windows)]
impl PolicyFixture for WorkspaceStore {
    fn save_approval_policy(
        &mut self,
        instance_id: &str,
        policy: ApprovalPolicy,
    ) -> Result<(), RuntimeError> {
        let entry = self
            .snapshot()
            .entries
            .into_iter()
            .find(|entry| entry.instance_id.as_deref() == Some(instance_id))
            .ok_or_else(|| error("workspace_entry_not_found"))?;
        if policy == ApprovalPolicy::default() {
            return self.assign_approval_group(&entry.config_ref, None);
        }
        let group = self.save_approval_group(
            entry.approval_rule_group_id.clone(),
            format!("rules-{instance_id}"),
            policy,
        )?;
        self.assign_approval_group(&entry.config_ref, Some(group.id))
    }
}

#[cfg(windows)]
#[test]
fn approval_groups_survive_without_instances_and_share_one_saved_policy() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let mut store = fixture.store()?;
    let policy = ApprovalPolicy {
        mode: coterm_core::approval_policy::ApprovalMode::AllowMatches,
        patterns: vec!["^pwd$".into()],
    };
    let group = store.save_approval_group(None, "Board rules".into(), policy.clone())?;
    assert_eq!(
        fixture.store()?.snapshot().approval_rule_groups,
        vec![group.clone()]
    );
    for id in ["one", "two"] {
        store.insert("ssh".into(), id.into(), id.into(), Value::Null, None)?;
    }
    let refs: Vec<_> = store
        .snapshot()
        .entries
        .iter()
        .map(|entry| entry.config_ref.clone())
        .collect();
    for reference in &refs {
        store.assign_approval_group(reference, Some(group.id.clone()))?;
    }
    store.save_bypass("two", true)?;
    let changed = ApprovalPolicy {
        patterns: vec!["^uname$".into()],
        ..policy.clone()
    };
    store.save_approval_group(
        Some(group.id.clone()),
        "Renamed rules".into(),
        changed.clone(),
    )?;
    let reopened = fixture.store()?;
    for reference in &refs {
        assert_eq!(reopened.approval_policy(reference)?, changed);
    }
    assert!(reopened.find(&refs[1])?.bypass);
    assert!(store.delete_approval_group(&group.id).is_err());
    store.remove(&refs[0])?;
    store.remove(&refs[1])?;
    assert_eq!(fixture.store()?.snapshot().approval_rule_groups.len(), 1);
    let disk: Value = serde_json::from_slice(&protect(&fs::read(fixture.path())?, false)?)?;
    assert!(disk["entries"].as_array().is_some_and(Vec::is_empty));
    assert!(disk.get("approvalRuleGroups").is_some());
    store.delete_approval_group(&group.id)?;
    assert!(fixture.store()?.snapshot().approval_rule_groups.is_empty());
    Ok(())
}

#[cfg(windows)]
#[test]
fn approval_group_failed_writes_and_invalid_rules_preserve_catalog_and_bindings(
) -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let mut store = fixture.store()?;
    store.insert("ssh".into(), "one".into(), "one".into(), Value::Null, None)?;
    let reference = store.snapshot().entries[0].config_ref.clone();
    let group = store.save_approval_group(None, "Rules".into(), ApprovalPolicy::default())?;
    assert!(store
        .save_approval_group(None, "rules".into(), ApprovalPolicy::default())
        .is_err());
    let invalid = ApprovalPolicy {
        patterns: vec!["[".into()],
        ..ApprovalPolicy::default()
    };
    assert!(store
        .save_approval_group(Some(group.id.clone()), "Rules".into(), invalid)
        .is_err());
    assert!(store
        .assign_approval_group(&reference, Some("missing".into()))
        .is_err());
    let previous = fs::read(fixture.path())?;
    store.path = fixture.0.join("missing").join("workspace.bin");
    assert!(store
        .assign_approval_group(&reference, Some(group.id.clone()))
        .is_err());
    assert!(store.delete_approval_group(&group.id).is_err());
    assert!(store
        .save_approval_group(
            Some(group.id.clone()),
            "Changed".into(),
            ApprovalPolicy::default()
        )
        .is_err());
    assert!(store.find(&reference)?.approval_rule_group_id.is_none());
    assert_eq!(store.snapshot().approval_rule_groups, vec![group]);
    assert_eq!(fs::read(fixture.path())?, previous);
    Ok(())
}

#[cfg(windows)]
#[test]
fn legacy_equal_policies_migrate_to_separate_groups_and_keep_empty_reverse_semantics(
) -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let policy = serde_json::json!({"mode":"approve_matches","patterns":[]});
    let entries: Vec<_> = ["one", "two"].into_iter().map(|id| serde_json::json!({
        "configRef":id,"driverId":"ssh","label":"Same name","config":{},"approvalPolicy":policy,"bypass":true
    })).collect();
    let legacy = serde_json::json!({"version":4,"entries":entries,"layout":{"activeConfigRef":null,"instancesCollapsed":false}});
    fs::write(
        fixture.path(),
        protect(&serde_json::to_vec(&legacy)?, true)?,
    )?;
    let mut store = fixture.store()?;
    let one = store.find("one")?;
    let two = store.find("two")?;
    assert_eq!(
        fixture.store()?.find("one")?.approval_rule_group_id,
        one.approval_rule_group_id
    );
    assert_ne!(one.approval_rule_group_id, two.approval_rule_group_id);
    assert!(one.bypass && two.bypass);
    assert!(!store
        .approval_policy("one")?
        .compile()?
        .requires_approval(&["pwd"]));
    store.assign_approval_group("one", None)?;
    assert!(fixture
        .store()?
        .find("one")?
        .approval_rule_group_id
        .is_none());
    assert!(fixture
        .store()?
        .find("two")?
        .approval_rule_group_id
        .is_some());
    assert_eq!(store.snapshot().approval_rule_groups.len(), 2);
    let disk: Value = serde_json::from_slice(&protect(&fs::read(fixture.path())?, false)?)?;
    assert!(disk["entries"][0].get("approvalPolicy").is_none());
    Ok(())
}

#[cfg(windows)]
#[test]
fn assigning_offline_recovery_entries_keeps_their_binding_at_the_next_checkpoint(
) -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let mut store = fixture.store()?;
    store.insert("ssh".into(), "old".into(), "old".into(), Value::Null, None)?;
    let reference = store.snapshot().entries[0].config_ref.clone();
    let group = store.save_approval_group(None, "Shared".into(), ApprovalPolicy::default())?;
    let mut reopened = fixture.store()?;
    reopened.insert("ssh".into(), "new".into(), "new".into(), Value::Null, None)?;
    reopened.assign_approval_group(&reference, Some(group.id.clone()))?;
    assert_eq!(
        fixture.store()?.find(&reference)?.approval_rule_group_id,
        Some(group.id)
    );
    Ok(())
}

#[cfg(windows)]
#[test]
fn damaged_group_catalogs_cannot_restore_dangling_or_ambiguous_bindings() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let group = serde_json::json!({"id":"group","name":"Rules","mode":"allow_matches","patterns":[]});
    for groups in [vec![], vec![group.clone(), group.clone()]] {
        let document = serde_json::json!({"version":5,"entries":[{"configRef":"one","driverId":"ssh","label":"one","config":{},"approvalRuleGroupId":"group"}],
            "approvalRuleGroups":groups,"layout":{"activeConfigRef":null,"instancesCollapsed":false}});
        fs::write(fixture.path(), protect(&serde_json::to_vec(&document)?, true)?)?;
        assert!(fixture.store().is_err());
    }
    Ok(())
}
