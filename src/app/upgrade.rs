use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use super::*;
use crate::kubernetes::CompositionRevisionInfo;

#[derive(Clone, Debug, Default)]
pub(super) struct RevisionIndex {
    by_name: HashMap<String, CompositionRevisionInfo>,
    by_composition: HashMap<String, Vec<CompositionRevisionInfo>>,
}

impl RevisionIndex {
    pub(super) fn new(revisions: Vec<CompositionRevisionInfo>) -> Self {
        let mut index = Self::default();
        for revision in revisions {
            index
                .by_composition
                .entry(revision.composition.clone())
                .or_default()
                .push(revision.clone());
            index.by_name.insert(revision.name.clone(), revision);
        }
        index
    }
}

/// The session's CompositionRevision inventory. The last successful result is kept while a
/// refresh runs or after one fails, so re-entering the upgrade view is instant.
#[derive(Clone, Debug, Default)]
pub(super) struct RevisionDiscovery {
    index: Option<Arc<RevisionIndex>>,
    refreshing: bool,
    error: Option<String>,
    generation: u64,
    /// Resources being upgraded, with the revision they were pointed at. They show as
    /// upgrading from the moment the action starts until a trace reflects the change.
    pending: HashMap<Identity, String>,
}

impl RevisionDiscovery {
    #[cfg(test)]
    pub(super) fn loaded(index: RevisionIndex) -> Self {
        Self {
            index: Some(Arc::new(index)),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(super) fn loading() -> Self {
        Self {
            refreshing: true,
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(super) fn failed() -> Self {
        Self {
            error: Some("discovery failed".into()),
            ..Self::default()
        }
    }

    pub(super) fn mark_pending(&mut self, identity: Identity, revision: String) {
        self.pending.insert(identity, revision);
    }

    pub(super) fn clear_pending(&mut self, identity: &Identity) {
        self.pending.remove(identity);
    }

    #[cfg(test)]
    pub(super) fn pending_contains(&self, identity: &Identity) -> bool {
        self.pending.contains_key(identity)
    }

    pub(super) fn is_refreshing(&self) -> bool {
        self.refreshing
    }

    pub(super) fn has_cache(&self) -> bool {
        self.index.is_some()
    }

    pub(super) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Starts a refresh, keeping any cached index. Returns the generation its result
    /// must carry to be accepted.
    pub(super) fn begin_refresh(&mut self) -> u64 {
        self.refreshing = true;
        self.error = None;
        self.generation += 1;
        self.generation
    }

    /// Applies a refresh result unless a newer refresh has superseded it.
    pub(super) fn finish(
        &mut self,
        generation: u64,
        result: Result<Vec<CompositionRevisionInfo>, String>,
    ) -> bool {
        if generation != self.generation {
            return false;
        }
        self.refreshing = false;
        match result {
            Ok(revisions) => {
                self.index = Some(Arc::new(RevisionIndex::new(revisions)));
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        true
    }
}

#[derive(Clone, Debug)]
pub(super) struct UpgradePlan {
    pub(super) target: Target,
    pub(super) composition: String,
    pub(super) from_name: String,
    pub(super) from_number: i64,
    pub(super) to_name: String,
    pub(super) to_number: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum UpgradeState {
    UpToDate,
    Available,
    Automatic,
    NotApplicable,
    NoRevision,
    UnknownRevision,
    Checking,
    Upgrading,
    Failed,
}

impl UpgradeState {
    pub(super) fn label(self, ascii: bool) -> &'static str {
        match (self, ascii) {
            (Self::UpToDate, _) => "Up to date",
            (Self::Available, true) => "Available ^",
            (Self::Available, false) => "Available ↑",
            (Self::Automatic, _) => "Automatic",
            (Self::NotApplicable, _) => "-",
            (Self::NoRevision, _) => "No revision",
            (Self::UnknownRevision, _) => "Unknown",
            (Self::Checking, true) => "Checking...",
            (Self::Checking, false) => "Checking…",
            (Self::Upgrading, true) => "Upgrading...",
            (Self::Upgrading, false) => "Upgrading…",
            (Self::Failed, _) => "Unavailable",
        }
    }

    pub(super) fn style(self, theme: &Theme) -> Style {
        match self {
            Self::UpToDate => theme.fg(theme.palette.green),
            Self::Available => theme.fg(theme.palette.peach),
            Self::Failed => theme.fg(theme.palette.red),
            Self::Automatic | Self::NotApplicable | Self::NoRevision | Self::UnknownRevision => {
                theme.subtle()
            }
            Self::Checking | Self::Upgrading => theme.activity(),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct UpgradeInfo {
    pub(super) state: UpgradeState,
    pub(super) current: Option<i64>,
    pub(super) latest: Option<i64>,
    pub(super) plan: Option<UpgradePlan>,
}

impl UpgradeInfo {
    fn of(state: UpgradeState) -> Self {
        Self {
            state,
            current: None,
            latest: None,
            plan: None,
        }
    }
}

/// `spec.<field>` or, for Crossplane v2 XRs, `spec.crossplane.<field>`.
fn spec_field<'a>(object: &'a Value, field: &str) -> Option<&'a Value> {
    object
        .pointer(&format!("/spec/{field}"))
        .or_else(|| object.pointer(&format!("/spec/crossplane/{field}")))
}

/// Pure projection of one resource against the discovered revisions. The only upgrade
/// offered is to the highest revision number of the Composition the resource already uses.
pub(super) fn upgrade_info(node: &ProjectedNode, discovery: &RevisionDiscovery) -> UpgradeInfo {
    let mut info = projected_upgrade_info(node, discovery);
    if discovery.pending.contains_key(&node.identity) {
        info.state = UpgradeState::Upgrading;
        info.plan = None;
    }
    info
}

fn projected_upgrade_info(node: &ProjectedNode, discovery: &RevisionDiscovery) -> UpgradeInfo {
    let object = node.object.as_ref();
    let Some(composition) = spec_field(object, "compositionRef")
        .and_then(|reference| reference.get("name"))
        .and_then(Value::as_str)
    else {
        return UpgradeInfo::of(UpgradeState::NotApplicable);
    };
    let Some(index) = &discovery.index else {
        return UpgradeInfo::of(if discovery.error.is_some() {
            UpgradeState::Failed
        } else {
            UpgradeState::Checking
        });
    };
    if spec_field(object, "compositionUpdatePolicy").and_then(Value::as_str) != Some("Manual") {
        return UpgradeInfo::of(UpgradeState::Automatic);
    }
    let Some(current_name) = spec_field(object, "compositionRevisionRef")
        .and_then(|reference| reference.get("name"))
        .and_then(Value::as_str)
    else {
        return UpgradeInfo::of(UpgradeState::NoRevision);
    };
    let Some(current) = index
        .by_name
        .get(current_name)
        .filter(|revision| revision.composition == composition)
    else {
        return UpgradeInfo::of(UpgradeState::UnknownRevision);
    };
    let Some(latest) = index
        .by_composition
        .get(composition)
        .into_iter()
        .flatten()
        .filter(|revision| {
            revision.composite_kind == node.identity.kind
                && revision.composite_group == node.identity.group
        })
        .max_by_key(|revision| revision.number)
    else {
        return UpgradeInfo::of(UpgradeState::NotApplicable);
    };
    let mut info = UpgradeInfo {
        state: UpgradeState::UpToDate,
        current: Some(current.number),
        latest: Some(latest.number),
        plan: None,
    };
    if latest.number > current.number {
        info.state = UpgradeState::Available;
        info.plan = Some(UpgradePlan {
            target: Target {
                identity: node.identity.clone(),
                expected_uid: node.uid.clone(),
            },
            composition: composition.to_owned(),
            from_name: current.name.clone(),
            from_number: current.number,
            to_name: latest.name.clone(),
            to_number: latest.number,
        });
    }
    info
}

impl App {
    pub(super) fn active_marks(&self) -> &HashMap<Identity, Option<String>> {
        if self.upgrade_view {
            &self.upgrade_marked
        } else {
            &self.marked
        }
    }

    /// Marks the targets as upgrading so the view reflects it before any refresh.
    pub(super) fn begin_upgrades(&mut self, plans: &[UpgradePlan]) {
        for plan in plans {
            self.revisions
                .mark_pending(plan.target.identity.clone(), plan.to_name.clone());
        }
    }

    /// Drops the upgrading state once a trace shows the new revision reference (or the
    /// resource is gone).
    pub(super) fn prune_pending_upgrades(&mut self) {
        let Some(snapshot) = self.snapshot.clone() else {
            return;
        };
        let done: Vec<Identity> = self
            .revisions
            .pending
            .iter()
            .filter(|(identity, target)| {
                snapshot.by_identity.get(*identity).is_none_or(|index| {
                    spec_field(
                        snapshot.nodes[*index].object.as_ref(),
                        "compositionRevisionRef",
                    )
                    .and_then(|reference| reference.get("name"))
                    .and_then(Value::as_str)
                        == Some(target.as_str())
                })
            })
            .map(|(identity, _)| identity.clone())
            .collect();
        for identity in &done {
            self.revisions.clear_pending(identity);
        }
    }

    pub(super) fn upgrade_info_for(&self, node: &ProjectedNode) -> UpgradeInfo {
        upgrade_info(node, &self.revisions)
    }

    pub(super) fn upgrade_available_count(&self) -> usize {
        self.snapshot
            .iter()
            .flat_map(|snapshot| snapshot.nodes.iter())
            .filter(|node| self.upgrade_info_for(node).plan.is_some())
            .count()
    }

    pub(super) fn enter_upgrade_view(&mut self) -> UiAction {
        self.upgrade_view = true;
        self.upgrade_marked.clear();
        self.tree_selection = None;
        self.last_tree_click = None;
        // Cached results stay visible while the refresh runs.
        self.status = if self.revisions.has_cache() {
            "Refreshing Composition revisions (showing cached results)...".into()
        } else {
            "Discovering Composition revisions...".into()
        };
        UiAction::DiscoverRevisions
    }

    pub(super) fn leave_upgrade_view(&mut self) {
        self.upgrade_view = false;
        self.upgrade_marked.clear();
        self.tree_selection = None;
        self.last_tree_click = None;
        self.resource_horizontal_scroll = 0;
        self.set_selection(self.selection.visible_index);
    }

    pub(super) fn apply_revisions(
        &mut self,
        generation: u64,
        result: Result<Vec<CompositionRevisionInfo>, String>,
    ) {
        let error = result.as_ref().err().cloned();
        if !self.revisions.finish(generation, result) {
            return;
        }
        match error {
            None if self.upgrade_view => {
                self.status = format!(
                    "{} revision upgrades available",
                    self.upgrade_available_count()
                );
            }
            None => {}
            Some(error) => {
                let cached = if self.revisions.has_cache() {
                    "; showing cached results"
                } else {
                    ""
                };
                self.status =
                    text::sanitize(&format!("Revision discovery failed{cached}: {error}"));
            }
        }
    }

    pub(super) fn mark_available_upgrades(&mut self) {
        let Some(snapshot) = self.snapshot.clone() else {
            return;
        };
        let mut added = 0;
        for index in self.visible() {
            let node = &snapshot.nodes[index];
            if self.upgrade_info_for(node).plan.is_some()
                && self
                    .upgrade_marked
                    .insert(node.identity.clone(), node.uid.clone())
                    .is_none()
            {
                added += 1;
            }
        }
        self.status = format!("Marked {added} visible upgrade(s)");
    }

    pub(super) fn request_upgrade_review(&mut self) {
        if self.config.read_only {
            self.status = "Upgrades are disabled in read-only mode".into();
            return;
        }
        let nodes = if self.upgrade_marked.is_empty() {
            self.selected_node().into_iter().collect()
        } else {
            self.snapshot
                .iter()
                .flat_map(|snapshot| snapshot.nodes.iter())
                .filter(|node| self.upgrade_marked.contains_key(&node.identity))
                .collect::<Vec<_>>()
        };
        let plans: Vec<_> = nodes
            .into_iter()
            .filter_map(|node| self.upgrade_info_for(node).plan)
            .collect();
        if plans.is_empty() {
            self.status = "No revision upgrades available for the selected resources".into();
        } else {
            self.modal = Some(Modal::UpgradeReview { plans });
        }
    }

    /// Keys with a different meaning in the upgrade view. `None` falls through to the
    /// ordinary navigation, filtering, and help keys.
    pub(super) fn handle_upgrade_key(&mut self, key: KeyEvent) -> Option<UiAction> {
        match (key.code, key.modifiers) {
            (KeyCode::Char('u'), KeyModifiers::NONE) => self.leave_upgrade_view(),
            (KeyCode::Esc, _) => {
                if self.upgrade_marked.is_empty() {
                    self.leave_upgrade_view();
                } else {
                    self.upgrade_marked.clear();
                }
            }
            (KeyCode::Char(' '), KeyModifiers::NONE) => self.toggle_mark(),
            (KeyCode::Char('a'), KeyModifiers::NONE) => self.mark_available_upgrades(),
            (KeyCode::Enter, _) => self.request_upgrade_review(),
            (KeyCode::Char('d' | 'x'), KeyModifiers::CONTROL)
            | (KeyCode::Char('p' | 'e'), KeyModifiers::NONE) => {
                self.status = "Not available in the upgrade view; press u to go back".into();
            }
            _ => return None,
        }
        Some(UiAction::None)
    }
}
