use super::*;

const MAX_APPROVAL_GROUPS: usize = 128;

pub(super) fn migrate(doc: &mut Document) -> Result<(), RuntimeError> {
    if doc.version >= 5 {
        return Ok(());
    }
    doc.approval_rule_groups.clear();
    let legacy = if doc.version == 1 {
        doc.approval_policy.clone().unwrap_or_default()
    } else {
        ApprovalPolicy::default()
    };
    for entry in &mut doc.entries {
        let policy = entry
            .approval_policy
            .clone()
            .unwrap_or_else(|| legacy.clone());
        policy.compile()?;
        if policy == ApprovalPolicy::default() {
            entry.approval_rule_group_id = None;
            continue;
        }
        let base: String = entry
            .label
            .trim()
            .chars()
            .filter(|character| !character.is_control())
            .take(48)
            .collect();
        let base = if base.is_empty() {
            "Rules".into()
        } else {
            base
        };
        let mut name = base.clone();
        let mut suffix = 2;
        while doc
            .approval_rule_groups
            .iter()
            .any(|group| group.name.to_lowercase() == name.to_lowercase())
        {
            name = format!("{base} ({suffix})");
            suffix += 1;
        }
        // A separate group namespace keeps migrated IDs stable before the first v5 save.
        let id = entry.config_ref.clone();
        entry.approval_rule_group_id = Some(id.clone());
        doc.approval_rule_groups.push(ApprovalRuleGroup {
            id,
            name,
            mode: policy.mode,
            patterns: policy.patterns,
        });
    }
    Ok(())
}

pub(super) fn validate(doc: &Document) -> Result<(), RuntimeError> {
    let mut ids = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    if doc.approval_rule_groups.len() > MAX_APPROVAL_GROUPS {
        return Err(error("approval_group_capacity"));
    }
    for group in &doc.approval_rule_groups {
        if group.id.is_empty()
            || group.id.len() > 64
            || !ids.insert(&group.id)
            || !valid_group_name(&group.name)
            || !names.insert(group.name.to_lowercase())
        {
            return Err(error("approval_group_invalid"));
        }
        group.policy().compile()?;
    }
    if doc.entries.iter().any(|entry| {
        entry
            .approval_rule_group_id
            .as_ref()
            .is_some_and(|id| !ids.contains(id))
    }) {
        return Err(error("approval_group_not_found"));
    }
    Ok(())
}

impl WorkspaceStore {
    /// Resolves one saved reference to its group's policy. Missing references/groups
    /// are errors; an explicitly unassigned instance requires approval by default.
    pub fn approval_policy(&self, config_ref: &str) -> Result<ApprovalPolicy, RuntimeError> {
        let entry = self.find(config_ref)?;
        match entry.approval_rule_group_id {
            None => Ok(ApprovalPolicy::default()),
            Some(id) => self
                .approval_rule_groups
                .iter()
                .find(|group| group.id == id)
                .map(ApprovalRuleGroup::policy)
                .ok_or_else(|| error("approval_group_not_found")),
        }
    }

    /// Creates (id=None) or replaces a named group. Invalid IDs, duplicate names,
    /// excessive/invalid rules and write failures leave the saved catalog unchanged.
    pub fn save_approval_group(
        &mut self,
        id: Option<String>,
        name: String,
        policy: ApprovalPolicy,
    ) -> Result<ApprovalRuleGroup, RuntimeError> {
        policy.compile()?;
        let name = name.trim().to_owned();
        if !valid_group_name(&name) {
            return Err(error("approval_group_name_invalid"));
        }
        let mut groups = self.approval_rule_groups.clone();
        let index = match &id {
            Some(id) => Some(
                groups
                    .iter()
                    .position(|group| &group.id == id)
                    .ok_or_else(|| error("approval_group_not_found"))?,
            ),
            None => None,
        };
        if groups.iter().enumerate().any(|(position, group)| {
            Some(position) != index && group.name.to_lowercase() == name.to_lowercase()
        }) {
            return Err(error("approval_group_name_exists"));
        }
        if index.is_none() && groups.len() >= MAX_APPROVAL_GROUPS {
            return Err(error("approval_group_capacity"));
        }
        let group = ApprovalRuleGroup {
            id: id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name,
            mode: policy.mode,
            patterns: policy.patterns,
        };
        if let Some(index) = index {
            groups[index] = group.clone();
        } else {
            groups.push(group.clone());
        }
        self.commit_state(
            self.entries.clone(),
            self.layout.clone(),
            self.groups.clone(),
            groups,
        )?;
        Ok(group)
    }

    /// Binds a saved instance to one existing group, or unassigns it with None.
    /// Missing targets and write failures do not change the old binding or bypass.
    pub fn assign_approval_group(
        &mut self,
        config_ref: &str,
        group_id: Option<String>,
    ) -> Result<(), RuntimeError> {
        if let Some(id) = &group_id {
            if !self
                .approval_rule_groups
                .iter()
                .any(|group| &group.id == id)
            {
                return Err(error("approval_group_not_found"));
            }
        }
        let mut entries = self.entries.clone();
        let entry = entries
            .iter_mut()
            .find(|entry| entry.config_ref == config_ref)
            .ok_or_else(|| error("workspace_entry_not_found"))?;
        entry.approval_rule_group_id = group_id;
        // Explicitly edited recovery records must survive the next session checkpoint.
        if entry.instance_id.is_none() {
            entry.retained = true;
        }
        self.commit(entries, self.layout.clone())
    }

    /// Removes an unbound group. Missing IDs, live or saved references, and failed
    /// writes are errors; deleting an instance never calls this method.
    pub fn delete_approval_group(&mut self, group_id: &str) -> Result<(), RuntimeError> {
        if !self
            .approval_rule_groups
            .iter()
            .any(|group| group.id == group_id)
        {
            return Err(error("approval_group_not_found"));
        }
        if self
            .entries
            .iter()
            .any(|entry| entry.approval_rule_group_id.as_deref() == Some(group_id))
        {
            return Err(error("approval_group_in_use"));
        }
        let groups = self
            .approval_rule_groups
            .iter()
            .filter(|group| group.id != group_id)
            .cloned()
            .collect();
        self.commit_state(
            self.entries.clone(),
            self.layout.clone(),
            self.groups.clone(),
            groups,
        )
    }
}
