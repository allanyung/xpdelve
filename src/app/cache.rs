use std::sync::{Mutex, MutexGuard, PoisonError};

use super::render::PlanEntry;
use super::upgrade::{UpgradeInfos, upgrade_infos};
use super::*;

/// Memoized whole-tree derivations, so a frame costs work proportional to the viewport.
/// Each entry holds the inputs it was built from and is reused only while all still match.
#[derive(Default)]
pub(super) struct FrameCache {
    visible: Mutex<Option<VisibleEntry>>,
    pub(super) plan: Mutex<Option<PlanEntry>>,
    upgrades: Mutex<Option<UpgradeEntry>>,
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct VisibleEntry {
    snapshot: Arc<Snapshot>,
    collapsed: HashSet<Identity>,
    excluded: HashSet<ResourceKind>,
    filter: String,
    kind: Option<ResourceKind>,
    health: HealthFilter,
    visible: Arc<Vec<usize>>,
}

impl VisibleEntry {
    fn matches(&self, app: &App, snapshot: &Arc<Snapshot>) -> bool {
        Arc::ptr_eq(&self.snapshot, snapshot)
            && self.health == app.health_filter
            && self.filter == app.filter
            && self.kind == app.kind_filter
            && self.collapsed == app.collapsed
            && self.excluded == app.excluded_kinds
    }
}

struct UpgradeEntry {
    snapshot: Arc<Snapshot>,
    stamp: upgrade::Stamp,
    infos: Arc<UpgradeInfos>,
}

impl App {
    /// Visible node indices in tree order, recomputed only when the snapshot or a
    /// filter, collapse or exclusion input changes.
    pub(super) fn visible_shared(&self) -> Arc<Vec<usize>> {
        let Some(snapshot) = &self.snapshot else {
            return Arc::default();
        };
        let mut slot = lock(&self.frame_cache.visible);
        if let Some(entry) = slot.as_ref()
            && entry.matches(self, snapshot)
        {
            return Arc::clone(&entry.visible);
        }
        let visible = Arc::new(snapshot.visible_indices(
            &self.collapsed,
            &self.excluded_kinds,
            (!self.filter.is_empty()).then_some(self.filter.as_str()),
            self.kind_filter.as_ref(),
            self.health_filter,
        ));
        *slot = Some(VisibleEntry {
            snapshot: Arc::clone(snapshot),
            collapsed: self.collapsed.clone(),
            excluded: self.excluded_kinds.clone(),
            filter: self.filter.clone(),
            kind: self.kind_filter.clone(),
            health: self.health_filter,
            visible: Arc::clone(&visible),
        });
        visible
    }

    /// Upgrade projection of every node, recomputed only when the snapshot or the
    /// revision inventory changes.
    pub(super) fn upgrade_infos(&self) -> Option<Arc<UpgradeInfos>> {
        let snapshot = self.snapshot.as_ref()?;
        let mut slot = lock(&self.frame_cache.upgrades);
        if let Some(entry) = slot.as_ref()
            && Arc::ptr_eq(&entry.snapshot, snapshot)
            && entry.stamp == self.revisions.stamp()
        {
            return Some(Arc::clone(&entry.infos));
        }
        let infos = Arc::new(upgrade_infos(snapshot, &self.revisions));
        *slot = Some(UpgradeEntry {
            snapshot: Arc::clone(snapshot),
            stamp: self.revisions.stamp(),
            infos: Arc::clone(&infos),
        });
        Some(infos)
    }
}
