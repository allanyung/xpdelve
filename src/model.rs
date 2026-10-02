// Crossplane status and package projection portions are adapted from xpdig's
// internal/xplane/model.go and internal/xplane/xpkg/xpkg.go.
// Copyright 2025 Bruno Luiz da Silva. Licensed under Apache-2.0.
// Translated and modified for xpdelve in 2026. See NOTICE.
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use chrono::{DateTime, Local};
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use crate::text;

#[derive(Clone, Debug, Deserialize)]
pub struct TraceNode {
    pub object: Value,
    #[serde(default)]
    pub error: Option<Value>,
    #[serde(default)]
    pub children: Vec<TraceNode>,
}

#[derive(Clone, Debug, Eq)]
pub struct Identity {
    pub group: String,
    pub version: String,
    pub kind: String,
    pub namespace: Option<String>,
    pub name: String,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ResourceKind {
    pub group: String,
    pub kind: String,
}

impl PartialEq for Identity {
    fn eq(&self, other: &Self) -> bool {
        self.group == other.group
            && self.kind == other.kind
            && self.namespace == other.namespace
            && self.name == other.name
    }
}

impl Hash for Identity {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.group.hash(state);
        self.kind.hash(state);
        self.namespace.hash(state);
        self.name.hash(state);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Health {
    Healthy,
    Warning,
    Unhealthy,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HealthFilter {
    #[default]
    All,
    Unhealthy,
    Healthy,
}

impl HealthFilter {
    pub fn matches(self, health: Health) -> bool {
        match self {
            Self::All => true,
            Self::Unhealthy => health != Health::Healthy,
            Self::Healthy => health == Health::Healthy,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConditionState {
    True,
    False,
    Unknown,
    Missing,
}

#[derive(Clone, Copy)]
struct ConditionProjection<'a> {
    ready: ConditionState,
    synced: ConditionState,
    ready_condition: Option<&'a Value>,
    synced_condition: Option<&'a Value>,
    package: bool,
    package_revision: bool,
}

#[derive(Clone, Debug)]
pub struct ProjectedNode {
    pub identity: Identity,
    pub resource_kind: ResourceKind,
    pub uid: Option<String>,
    pub depth: usize,
    pub parent: Option<usize>,
    pub is_last_child: bool,
    pub child_count: usize,
    pub health: Health,
    pub status: String,
    pub status_details: Option<String>,
    pub ready: Option<bool>,
    pub synced: Option<bool>,
    pub ready_last: Option<String>,
    pub synced_last: Option<String>,
    pub paused: bool,
    pub is_package: bool,
    pub package: Option<String>,
    pub version: Option<String>,
    pub state: Option<String>,
    pub object: Arc<Value>,
}

impl ProjectedNode {
    pub fn matches_text(&self, query: &str) -> bool {
        format!(
            "{} {} {}",
            self.identity,
            self.status,
            self.status_details.as_deref().unwrap_or_default()
        )
        .to_lowercase()
        .contains(&query.to_lowercase())
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub nodes: Arc<[ProjectedNode]>,
    pub by_identity: HashMap<Identity, usize>,
}

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("invalid trace JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("trace node {path} has no usable {field}")]
    MissingIdentity { path: String, field: &'static str },
    #[error("trace contains duplicate identity {0}")]
    DuplicateIdentity(Identity),
}

impl fmt::Display for Identity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.group.is_empty() {
            write!(formatter, "{}/{}", self.kind, self.name)
        } else {
            write!(formatter, "{}.{}/{}", self.kind, self.group, self.name)
        }
    }
}

impl Identity {
    fn resource_kind(&self) -> ResourceKind {
        ResourceKind {
            group: self.group.clone(),
            kind: self.kind.clone(),
        }
    }
}

impl fmt::Display for ResourceKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.group.is_empty() {
            formatter.write_str(&self.kind)
        } else {
            write!(formatter, "{}.{}", self.kind, self.group)
        }
    }
}

impl Snapshot {
    pub fn parse(input: &[u8]) -> Result<Self, ModelError> {
        let root: TraceNode = serde_json::from_slice(input)?;
        Self::project(root)
    }

    pub fn project(root: TraceNode) -> Result<Self, ModelError> {
        let mut nodes = Vec::new();
        project_node(root, None, 0, true, "root", &mut nodes)?;

        let mut by_identity = HashMap::with_capacity(nodes.len());
        for (index, node) in nodes.iter().enumerate() {
            if by_identity.insert(node.identity.clone(), index).is_some() {
                return Err(ModelError::DuplicateIdentity(node.identity.clone()));
            }
        }

        Ok(Self {
            nodes: nodes.into(),
            by_identity,
        })
    }

    pub fn visible_indices(
        &self,
        collapsed: &HashSet<Identity>,
        excluded: &HashSet<ResourceKind>,
        filter: Option<&str>,
        kind: Option<&ResourceKind>,
        health: HealthFilter,
    ) -> Vec<usize> {
        let query = filter.map(str::trim).filter(|value| !value.is_empty());
        let health_filtering = health != HealthFilter::All;
        let direct_matches: HashSet<usize> = self
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                let text_matches = query.is_none_or(|query| matches_query(node, query));
                let kind_matches = kind.is_none_or(|kind| &node.resource_kind == kind);
                (text_matches && kind_matches && health.matches(node.health)).then_some(index)
            })
            .collect();
        let mut retained = if query.is_some() || health_filtering {
            direct_matches.clone()
        } else {
            HashSet::new()
        };
        if query.is_some() || health_filtering {
            for index in &direct_matches {
                let mut parent = self.nodes[*index].parent;
                while let Some(parent_index) = parent {
                    retained.insert(parent_index);
                    parent = self.nodes[parent_index].parent;
                }
            }
        }

        let mut hidden_depth = None;
        let mut visible = Vec::new();
        for (index, node) in self.nodes.iter().enumerate() {
            if hidden_depth.is_some_and(|depth| node.depth > depth) {
                continue;
            }
            hidden_depth = None;

            if excluded.contains(&node.resource_kind) {
                hidden_depth = Some(node.depth);
                continue;
            }
            if query.is_none() && !health_filtering && collapsed.contains(&node.identity) {
                hidden_depth = Some(node.depth);
            }
            if health_filtering {
                if retained.contains(&index) {
                    visible.push(index);
                }
                continue;
            }
            let matches_kind = kind.is_none_or(|kind| &node.resource_kind == kind);
            if matches_kind && (query.is_none() || retained.contains(&index)) {
                visible.push(index);
            }
        }
        visible
    }
}

fn project_node(
    node: TraceNode,
    parent: Option<usize>,
    depth: usize,
    is_last_child: bool,
    path: &str,
    projected: &mut Vec<ProjectedNode>,
) -> Result<(), ModelError> {
    let identity = identity(&node.object, path)?;
    let uid = pointer_string(&node.object, "/metadata/uid");
    let paused = node
        .object
        .pointer("/metadata/annotations/crossplane.io~1paused")
        .and_then(Value::as_str)
        == Some("true");
    let package_kind = package_kind(&identity);
    let package_revision = package_revision(&identity);
    let ready_type = if package_revision {
        "RevisionHealthy"
    } else if package_kind {
        "Healthy"
    } else {
        "Ready"
    };
    let synced_type = if package_kind { "Installed" } else { "Synced" };
    let ready_condition = condition(&node.object, ready_type).or_else(|| {
        package_revision
            .then(|| condition(&node.object, "Healthy"))
            .flatten()
    });
    let synced_condition = condition(&node.object, synced_type);
    let ready_state = condition_state(ready_condition);
    let synced_state = condition_state(synced_condition);
    let ready = state_bool(ready_state);
    let synced = state_bool(synced_state);
    let trace_error = node.error.as_ref().map(error_text);
    let projection = ConditionProjection {
        ready: ready_state,
        synced: synced_state,
        ready_condition,
        synced_condition,
        package: package_kind,
        package_revision,
    };
    let status = status_text(&node.object, node.error.as_ref(), projection);
    let status_details = status_details(trace_error.as_deref(), projection);
    let health = if node.object.pointer("/metadata/deletionTimestamp").is_some() {
        Health::Unhealthy
    } else if node.error.as_ref().is_some_and(error_is_not_found) {
        Health::Unknown
    } else if trace_error.is_some() {
        Health::Unhealthy
    } else if relevant_reason_is(ready_condition, synced_condition, "Warning") {
        Health::Warning
    } else if relevant_reason_is(ready_condition, synced_condition, "Unknown") {
        Health::Unknown
    } else {
        resource_health(ready_state, synced_state, package_kind, package_revision)
    };
    let package_reference = if package_revision {
        pointer_string(&node.object, "/spec/image")
    } else if package_kind {
        pointer_string(&node.object, "/spec/package")
    } else {
        None
    };
    let (package, version) =
        package_reference.map_or((None, None), |reference| split_image_reference(&reference));
    let child_count = node.children.len();
    let index = projected.len();
    let resource_kind = identity.resource_kind();
    projected.push(ProjectedNode {
        identity,
        resource_kind,
        uid,
        depth,
        parent,
        is_last_child,
        child_count,
        health,
        status,
        status_details,
        ready,
        synced,
        ready_last: ready_condition.and_then(condition_transition),
        synced_last: synced_condition.and_then(condition_transition),
        paused,
        is_package: package_kind,
        package,
        version,
        state: pointer_string(&node.object, "/spec/desiredState"),
        object: Arc::new(node.object),
    });

    let last = child_count.saturating_sub(1);
    for (child_index, child) in node.children.into_iter().enumerate() {
        project_node(
            child,
            Some(index),
            depth + 1,
            child_index == last,
            &format!("{path}.children[{child_index}]"),
            projected,
        )?;
    }
    Ok(())
}

fn identity(object: &Value, path: &str) -> Result<Identity, ModelError> {
    let api_version = required_string(object, "/apiVersion", path, "apiVersion")?;
    let (group, version) = api_version.split_once('/').map_or_else(
        || (String::new(), api_version.to_owned()),
        |(group, version)| (group.to_owned(), version.to_owned()),
    );
    Ok(Identity {
        group,
        version,
        kind: required_string(object, "/kind", path, "kind")?.to_owned(),
        namespace: pointer_string(object, "/metadata/namespace"),
        name: required_string(object, "/metadata/name", path, "metadata.name")?.to_owned(),
    })
}

fn required_string<'a>(
    value: &'a Value,
    pointer: &str,
    path: &str,
    field: &'static str,
) -> Result<&'a str, ModelError> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ModelError::MissingIdentity {
            path: path.to_owned(),
            field,
        })
}

fn pointer_string(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn condition<'a>(object: &'a Value, condition_type: &str) -> Option<&'a Value> {
    object
        .pointer("/status/conditions")
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find(|condition| {
                condition.get("type").and_then(Value::as_str) == Some(condition_type)
            })
        })
}

fn condition_state(condition: Option<&Value>) -> ConditionState {
    match condition
        .and_then(|condition| condition.get("status"))
        .and_then(Value::as_str)
    {
        Some("True") => ConditionState::True,
        Some("False") => ConditionState::False,
        Some(_) => ConditionState::Unknown,
        None if condition.is_some() => ConditionState::Unknown,
        None => ConditionState::Missing,
    }
}

fn state_bool(value: ConditionState) -> Option<bool> {
    match value {
        ConditionState::True => Some(true),
        ConditionState::False => Some(false),
        ConditionState::Unknown | ConditionState::Missing => None,
    }
}

fn resource_health(
    ready: ConditionState,
    synced: ConditionState,
    package: bool,
    package_revision: bool,
) -> Health {
    if package_revision {
        return if ready == ConditionState::True {
            Health::Healthy
        } else if ready == ConditionState::Missing {
            Health::Unknown
        } else {
            Health::Unhealthy
        };
    }
    if package {
        return match (ready, synced) {
            (ConditionState::True, ConditionState::True) => Health::Healthy,
            (ConditionState::Missing, ConditionState::Missing) => Health::Unknown,
            _ => Health::Unhealthy,
        };
    }
    match (ready, synced) {
        (ConditionState::Missing, ConditionState::Missing) => Health::Unknown,
        (ConditionState::False | ConditionState::Unknown, _)
        | (_, ConditionState::False | ConditionState::Unknown) => Health::Unhealthy,
        (ConditionState::True, ConditionState::True | ConditionState::Missing)
        | (ConditionState::Missing, ConditionState::True) => Health::Healthy,
    }
}

fn condition_reason(condition: &Value) -> Option<String> {
    let reason = text::sanitize(
        condition
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    let message = text::sanitize(
        condition
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    match (reason.is_empty(), message.is_empty()) {
        (true, true) => None,
        (false, true) => Some(reason.to_owned()),
        (true, false) => Some(message.to_owned()),
        (false, false) => Some(format!("{reason}: {message}")),
    }
}

fn relevant_reason_is(
    ready_condition: Option<&Value>,
    synced_condition: Option<&Value>,
    expected: &str,
) -> bool {
    [ready_condition, synced_condition]
        .into_iter()
        .flatten()
        .filter_map(|condition| condition.get("reason").and_then(Value::as_str))
        .any(|reason| reason.eq_ignore_ascii_case(expected))
}

fn condition_transition(condition: &Value) -> Option<String> {
    let timestamp = condition.get("lastTransitionTime")?.as_str()?;
    DateTime::parse_from_rfc3339(timestamp).ok().map(|time| {
        time.with_timezone(&Local)
            .format("%d %b %y %H:%M")
            .to_string()
    })
}

fn status_text(
    object: &Value,
    trace_error: Option<&Value>,
    projection: ConditionProjection<'_>,
) -> String {
    if object.pointer("/metadata/deletionTimestamp").is_some() {
        return "Deleting".into();
    }
    if let Some(error) = trace_error {
        return if error_is_not_found(error) {
            "Deleted"
        } else {
            "Error"
        }
        .into();
    }
    if let Some(status) = problem_status(projection) {
        return status.into();
    }
    if projection.package_revision {
        return projection
            .ready_condition
            .and_then(condition_reason)
            .unwrap_or_default();
    }
    if projection.ready == ConditionState::True && projection.synced == ConditionState::True {
        return projection
            .ready_condition
            .and_then(condition_reason)
            .unwrap_or_else(|| {
                if projection.package {
                    "Healthy"
                } else {
                    "Ready"
                }
                .into()
            });
    }
    if !projection.package
        && projection.ready == ConditionState::True
        && projection.synced == ConditionState::Missing
    {
        return projection
            .ready_condition
            .and_then(condition_reason)
            .unwrap_or_else(|| "Ready".into());
    }
    if !projection.package
        && projection.ready == ConditionState::Missing
        && projection.synced == ConditionState::True
    {
        return projection
            .synced_condition
            .and_then(condition_reason)
            .unwrap_or_else(|| "Synced".into());
    }
    "-".into()
}

fn problem_status(projection: ConditionProjection<'_>) -> Option<&'static str> {
    if relevant_reason_is(
        projection.ready_condition,
        projection.synced_condition,
        "Warning",
    ) {
        return Some("Warning");
    }
    if relevant_reason_is(
        projection.ready_condition,
        projection.synced_condition,
        "Unknown",
    ) {
        return Some("Unknown");
    }
    // Revisions derive health from RevisionHealthy (or legacy Healthy), not Installed.
    let synced = if projection.package_revision {
        ConditionState::Missing
    } else {
        projection.synced
    };
    match (projection.ready, synced) {
        (_, ConditionState::False) => Some("Error"),
        (ConditionState::False, _)
            if relevant_reason_is(projection.ready_condition, None, "Creating") =>
        {
            Some("Creating")
        }
        (ConditionState::False, _) => Some("Unready"),
        (ConditionState::Unknown, _) | (_, ConditionState::Unknown) => Some("Unknown"),
        _ => None,
    }
}

fn status_details(
    trace_error: Option<&str>,
    projection: ConditionProjection<'_>,
) -> Option<String> {
    let mut details = Vec::new();
    if let Some(error) = trace_error {
        details.push(format!("Error: {error}"));
    }
    let synced_condition = projection.synced_condition.filter(|condition| {
        !projection.package_revision
            || relevant_reason_is(Some(condition), None, "Warning")
            || relevant_reason_is(Some(condition), None, "Unknown")
    });
    for condition in [synced_condition, projection.ready_condition]
        .into_iter()
        .flatten()
    {
        if !matches!(
            condition_state(Some(condition)),
            ConditionState::False | ConditionState::Unknown
        ) && !relevant_reason_is(Some(condition), None, "Warning")
            && !relevant_reason_is(Some(condition), None, "Unknown")
        {
            continue;
        }
        let condition_type = condition
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("Condition");
        let state = condition
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("Unknown");
        let message =
            condition_reason(condition).unwrap_or_else(|| "No reason or message reported.".into());
        details.push(text::sanitize(&format!(
            "{condition_type}={state}\n{message}"
        )));
    }
    (!details.is_empty()).then(|| details.join("\n\n"))
}

fn package_kind(identity: &Identity) -> bool {
    identity.group == "pkg.crossplane.io"
        && matches!(
            identity.kind.as_str(),
            "Provider"
                | "Configuration"
                | "Function"
                | "ProviderRevision"
                | "ConfigurationRevision"
                | "FunctionRevision"
        )
}

fn package_revision(identity: &Identity) -> bool {
    identity.group == "pkg.crossplane.io" && identity.kind.ends_with("Revision")
}

fn split_image_reference(reference: &str) -> (Option<String>, Option<String>) {
    let last_slash = reference.rfind('/');
    let tag = reference
        .rfind(':')
        .filter(|index| last_slash.is_none_or(|slash| *index > slash));
    tag.map_or_else(
        || (Some(reference.into()), None),
        |index| {
            (
                Some(reference[..index].into()),
                Some(reference[index + 1..].into()),
            )
        },
    )
}

fn error_text(error: &Value) -> String {
    let message = error
        .get("message")
        .or_else(|| error.pointer("/ErrStatus/message"))
        .or_else(|| error.pointer("/errStatus/message"))
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty());
    let fallback = error
        .get("reason")
        .or_else(|| error.pointer("/ErrStatus/reason"))
        .or_else(|| error.pointer("/errStatus/reason"))
        .and_then(Value::as_str)
        .filter(|reason| !reason.is_empty())
        .unwrap_or("Unknown trace error");
    text::sanitize(message.unwrap_or(fallback))
}

fn error_is_not_found(error: &Value) -> bool {
    let code = error
        .get("code")
        .or_else(|| error.pointer("/ErrStatus/code"))
        .or_else(|| error.pointer("/errStatus/code"))
        .and_then(Value::as_u64);
    let reason = error
        .get("reason")
        .or_else(|| error.pointer("/ErrStatus/reason"))
        .or_else(|| error.pointer("/errStatus/reason"))
        .and_then(Value::as_str);
    code == Some(404) || reason == Some("NotFound")
}

fn matches_query(node: &ProjectedNode, query: &str) -> bool {
    let query = query.to_lowercase();
    let (field, needle) = query.split_once(':').unwrap_or(("", query.as_str()));
    match field {
        "kind" => node.identity.kind.to_lowercase().contains(needle),
        "group" => node.identity.group.to_lowercase().contains(needle),
        "namespace" => node
            .identity
            .namespace
            .as_deref()
            .unwrap_or_default()
            .to_lowercase()
            .contains(needle),
        "status" => format!(
            "{} {}",
            node.status,
            node.status_details.as_deref().unwrap_or_default()
        )
        .to_lowercase()
        .contains(needle),
        "ready" => bool_text(node.ready).contains(needle),
        "synced" => bool_text(node.synced).contains(needle),
        _ => node.matches_text(&query),
    }
}

fn bool_text(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACE: &str = r#"{
      "object": {"apiVersion":"example.io/v1","kind":"Root","metadata":{"name":"root"}},
      "children": [{"object": {"apiVersion":"v1","kind":"Secret","metadata":{"name":"child","namespace":"default"},"status":{"conditions":[{"type":"Ready","status":"False","reason":"Waiting"},{"type":"Synced","status":"True"}]}}}]
    }"#;

    fn condition_object(kind: &str, name: &str, status: &str, reason: Option<&str>) -> Value {
        serde_json::json!({
            "apiVersion": "example.io/v1",
            "kind": kind,
            "metadata": {"name": name},
            "status": {"conditions": [{"type": "Ready", "status": status, "reason": reason}]}
        })
    }

    fn healthy_object(kind: &str, name: &str) -> Value {
        condition_object(kind, name, "True", None)
    }

    #[test]
    fn projects_depth_first_with_core_identity() {
        let snapshot = Snapshot::parse(TRACE.as_bytes()).unwrap();
        assert_eq!(snapshot.nodes.len(), 2);
        assert_eq!(snapshot.nodes[1].depth, 1);
        assert_eq!(snapshot.nodes[1].identity.to_string(), "Secret/child");
        assert_eq!(snapshot.nodes[1].health, Health::Unhealthy);
        assert_eq!(snapshot.nodes[1].status, "Unready");
        assert_eq!(
            snapshot.nodes[1].status_details.as_deref(),
            Some("Ready=False\nWaiting")
        );
    }

    #[test]
    fn filter_retains_ancestors() {
        let snapshot = Snapshot::parse(TRACE.as_bytes()).unwrap();
        let visible = snapshot.visible_indices(
            &HashSet::new(),
            &HashSet::new(),
            Some("kind:secret"),
            None,
            HealthFilter::All,
        );
        assert_eq!(visible, vec![0, 1]);
    }

    #[test]
    fn kind_filter_only_retains_exact_kind_matches() {
        let snapshot = Snapshot::parse(TRACE.as_bytes()).unwrap();
        let kind = ResourceKind {
            group: String::new(),
            kind: "Secret".into(),
        };
        let visible = snapshot.visible_indices(
            &HashSet::new(),
            &HashSet::new(),
            None,
            Some(&kind),
            HealthFilter::All,
        );
        assert_eq!(visible, vec![1]);
    }

    #[test]
    fn kind_filter_does_not_restore_non_matching_text_filter_ancestors() {
        let snapshot = Snapshot::parse(TRACE.as_bytes()).unwrap();
        let kind = ResourceKind {
            group: String::new(),
            kind: "Secret".into(),
        };
        let visible = snapshot.visible_indices(
            &HashSet::new(),
            &HashSet::new(),
            Some("kind:secret"),
            Some(&kind),
            HealthFilter::All,
        );
        assert_eq!(visible, vec![1]);
    }

    #[test]
    fn health_filter_retains_ancestors_and_treats_nonhealthy_states_as_unhealthy() {
        let trace = serde_json::json!({
            "object": healthy_object("Root", "root"),
            "children": [
                {
                    "object": healthy_object("Branch", "branch"),
                    "children": [{
                        "object": condition_object("Broken", "broken", "False", None)
                    }]
                },
                {"object": condition_object("Warning", "warning", "False", Some("Warning"))},
                {"object": {"apiVersion":"example.io/v1","kind":"Unknown","metadata":{"name":"unknown"}}},
                {"object": healthy_object("Healthy", "healthy")}
            ]
        });
        let snapshot = Snapshot::parse(trace.to_string().as_bytes()).unwrap();
        let collapsed = HashSet::from([snapshot.nodes[0].identity.clone()]);

        let unhealthy = snapshot.visible_indices(
            &collapsed,
            &HashSet::new(),
            None,
            None,
            HealthFilter::Unhealthy,
        );
        let healthy = snapshot.visible_indices(
            &HashSet::new(),
            &HashSet::new(),
            None,
            None,
            HealthFilter::Healthy,
        );

        assert_eq!(unhealthy, vec![0, 1, 2, 3, 4]);
        assert_eq!(healthy, vec![0, 1, 5]);
    }

    #[test]
    fn health_filter_combines_with_kind_before_retaining_ancestors() {
        let snapshot = Snapshot::parse(TRACE.as_bytes()).unwrap();
        let kind = ResourceKind {
            group: String::new(),
            kind: "Secret".into(),
        };

        let visible = snapshot.visible_indices(
            &HashSet::new(),
            &HashSet::new(),
            None,
            Some(&kind),
            HealthFilter::Unhealthy,
        );

        assert_eq!(visible, vec![0, 1]);
    }

    #[test]
    fn excluded_kind_hides_its_entire_subtree() {
        let snapshot = Snapshot::parse(
            br#"{
                "object":{"apiVersion":"example.io/v1","kind":"Root","metadata":{"name":"root"}},
                "children":[
                    {"object":{"apiVersion":"protection.crossplane.io/v1beta1","kind":"Usage","metadata":{"name":"usage"}},"children":[
                        {"object":{"apiVersion":"v1","kind":"Secret","metadata":{"name":"hidden-child"}}}
                    ]},
                    {"object":{"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"visible-sibling"}}}
                ]
            }"#,
        )
        .unwrap();
        let excluded = HashSet::from([ResourceKind {
            group: "protection.crossplane.io".into(),
            kind: "Usage".into(),
        }]);

        let visible =
            snapshot.visible_indices(&HashSet::new(), &excluded, None, None, HealthFilter::All);

        assert_eq!(visible, vec![0, 3]);
    }

    #[test]
    fn excluded_kind_is_scoped_by_api_group() {
        let snapshot = Snapshot::parse(
            br#"{
                "object":{"apiVersion":"example.io/v1","kind":"Root","metadata":{"name":"root"}},
                "children":[
                    {"object":{"apiVersion":"protection.crossplane.io/v1beta1","kind":"Usage","metadata":{"name":"hidden"}}},
                    {"object":{"apiVersion":"other.example.io/v1","kind":"Usage","metadata":{"name":"visible"}}}
                ]
            }"#,
        )
        .unwrap();
        let excluded = HashSet::from([ResourceKind {
            group: "protection.crossplane.io".into(),
            kind: "Usage".into(),
        }]);

        let visible =
            snapshot.visible_indices(&HashSet::new(), &excluded, None, None, HealthFilter::All);

        assert_eq!(visible, vec![0, 2]);
    }

    #[test]
    fn synced_failure_takes_priority_over_ready_failure() {
        let snapshot = Snapshot::parse(
            br#"{"object":{"apiVersion":"example.io/v1","kind":"Root","metadata":{"name":"root"},"status":{"conditions":[{"type":"Synced","status":"False","reason":"SyncFailed","message":"cannot connect"},{"type":"Ready","status":"False","reason":"NotReady"}]}}}"#,
        )
        .unwrap();
        assert_eq!(snapshot.nodes[0].status, "Error");
        assert_eq!(
            snapshot.nodes[0].status_details.as_deref(),
            Some("Synced=False\nSyncFailed: cannot connect\n\nReady=False\nNotReady")
        );
    }

    #[test]
    fn problem_labels_and_details_cover_missing_messages_and_package_conditions() {
        for (kind, conditions, expected_status, expected_details) in [
            (
                "Resource",
                serde_json::json!([{"type":"Ready","status":"False"}]),
                "Unready",
                "Ready=False\nNo reason or message reported.",
            ),
            (
                "Resource",
                serde_json::json!([{"type":"Synced","status":"Unknown"}]),
                "Unknown",
                "Synced=Unknown\nNo reason or message reported.",
            ),
            (
                "Provider",
                serde_json::json!([{"type":"Installed","status":"False","reason":"InstallFailed"},{"type":"Healthy","status":"False","reason":"UnhealthyPackage"}]),
                "Error",
                "Installed=False\nInstallFailed\n\nHealthy=False\nUnhealthyPackage",
            ),
            (
                "ProviderRevision",
                serde_json::json!([{"type":"RevisionHealthy","status":"False","message":"runtime failed"}]),
                "Unready",
                "RevisionHealthy=False\nruntime failed",
            ),
            (
                "ConfigurationRevision",
                serde_json::json!([{"type":"Healthy","status":"False","reason":"UnhealthyPackageRevision"}]),
                "Unready",
                "Healthy=False\nUnhealthyPackageRevision",
            ),
        ] {
            let group = if kind == "Resource" {
                "example.io/v1"
            } else {
                "pkg.crossplane.io/v1"
            };
            let trace = serde_json::json!({"object":{"apiVersion":group,"kind":kind,"metadata":{"name":"example"},"status":{"conditions":conditions}}});
            let snapshot = Snapshot::parse(&serde_json::to_vec(&trace).unwrap()).unwrap();
            assert_eq!(snapshot.nodes[0].status, expected_status);
            assert_eq!(
                snapshot.nodes[0].status_details.as_deref(),
                Some(expected_details)
            );
        }
    }

    #[test]
    fn missing_conditions_are_not_errors_and_healthy_details_stay_in_status() {
        let snapshot = Snapshot::parse(TRACE.as_bytes()).unwrap();
        assert_eq!(snapshot.nodes[0].status, "-");
        assert!(snapshot.nodes[0].status_details.is_none());
        let trace = serde_json::json!({"object": healthy_object("Resource", "example")});
        let snapshot = Snapshot::parse(&serde_json::to_vec(&trace).unwrap()).unwrap();
        assert_eq!(snapshot.nodes[0].status, "Ready");
        assert!(snapshot.nodes[0].status_details.is_none());
    }

    #[test]
    fn deleting_keeps_trace_and_condition_details_searchable_and_safe() {
        let trace = serde_json::json!({
            "object":{"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"example","deletionTimestamp":"2026-09-20T08:49:00Z"},"status":{"conditions":[{"type":"Ready","status":"False","reason":"Waiting","message":"first line\nsecond line\u{1b}[31m\u{202e}"}]}},
            "error":{"message":"access denied"}
        });
        let snapshot = Snapshot::parse(&serde_json::to_vec(&trace).unwrap()).unwrap();
        let node = &snapshot.nodes[0];
        assert_eq!(node.status, "Deleting");
        let details = node.status_details.as_deref().unwrap();
        assert!(details.contains("Error: access denied"));
        assert!(details.contains("Waiting: first line\nsecond line\\x1b[31m\\u{202E}"));
        for query in ["access denied", "status:waiting", "status:deleting"] {
            assert!(matches_query(node, query));
        }
        assert!(!matches_query(node, "status:absent"));
    }

    #[test]
    fn generic_resource_with_ready_true_and_no_synced_is_healthy() {
        let snapshot = Snapshot::parse(
            br#"{"object":{"apiVersion":"external-secrets.io/v1beta1","kind":"ExternalSecret","metadata":{"name":"credentials"},"status":{"conditions":[{"type":"Ready","status":"True","reason":"SecretSynced","message":"Secret was synced"}]}}}"#,
        )
        .unwrap();
        let node = &snapshot.nodes[0];
        assert_eq!(node.health, Health::Healthy);
        assert_eq!(node.ready, Some(true));
        assert_eq!(node.synced, None);
        assert_eq!(node.status, "SecretSynced: Secret was synced");
    }

    #[test]
    fn generic_resource_with_synced_true_and_no_ready_is_healthy() {
        let snapshot = Snapshot::parse(
            br#"{"object":{"apiVersion":"example.io/v1","kind":"SyncOnly","metadata":{"name":"example"},"status":{"conditions":[{"type":"Synced","status":"True","reason":"ReconcileSuccess"}]}}}"#,
        )
        .unwrap();
        assert_eq!(snapshot.nodes[0].health, Health::Healthy);
        assert_eq!(snapshot.nodes[0].status, "ReconcileSuccess");
    }

    #[test]
    fn warning_reason_uses_warning_health() {
        let snapshot = Snapshot::parse(
            br#"{"object":{"apiVersion":"example.io/v1","kind":"Resource","metadata":{"name":"example"},"status":{"conditions":[{"type":"Ready","status":"False","reason":"Warning","message":"Needs attention"}]}}}"#,
        )
        .unwrap();
        let node = &snapshot.nodes[0];
        assert_eq!(node.health, Health::Warning);
        assert_eq!(node.status, "Warning");
        assert_eq!(
            node.status_details.as_deref(),
            Some("Ready=False\nWarning: Needs attention")
        );
    }

    #[test]
    fn unknown_reason_uses_neutral_health() {
        let snapshot = Snapshot::parse(
            br#"{"object":{"apiVersion":"example.io/v1","kind":"Resource","metadata":{"name":"example"},"status":{"conditions":[{"type":"Ready","status":"False","reason":"Unknown","message":"State unavailable"}]}}}"#,
        )
        .unwrap();
        let node = &snapshot.nodes[0];
        assert_eq!(node.health, Health::Unknown);
        assert_eq!(node.status, "Unknown");
        assert_eq!(
            node.status_details.as_deref(),
            Some("Ready=False\nUnknown: State unavailable")
        );
    }

    #[test]
    fn deleting_resource_is_unhealthy_even_when_conditions_are_true() {
        let snapshot = Snapshot::parse(
            br#"{"object":{"apiVersion":"protection.crossplane.io/v1beta1","kind":"Usage","metadata":{"name":"example","deletionTimestamp":"2026-09-20T08:49:00Z"},"status":{"conditions":[{"type":"Ready","status":"True","reason":"Available"}]}}}"#,
        )
        .unwrap();
        let node = &snapshot.nodes[0];
        assert_eq!(node.health, Health::Unhealthy);
        assert_eq!(node.status, "Deleting");
    }

    #[test]
    fn trace_error_is_unhealthy_even_when_conditions_are_true() {
        let snapshot = Snapshot::parse(
            br#"{"object":{"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"example"},"status":{"conditions":[{"type":"Ready","status":"True"}]}},"error":{"ErrStatus":{"message":"not found"}}}"#,
        )
        .unwrap();
        let node = &snapshot.nodes[0];
        assert_eq!(node.health, Health::Unhealthy);
        assert_eq!(node.status, "Error");
        assert_eq!(node.status_details.as_deref(), Some("Error: not found"));
    }

    #[test]
    fn not_found_trace_error_is_neutral_during_disappearance() {
        let snapshot = Snapshot::parse(
            br#"{"object":{"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"example"}},"error":{"ErrStatus":{"code":404,"reason":"NotFound","message":"configmaps \"example\" not found"}}}"#,
        )
        .unwrap();
        let node = &snapshot.nodes[0];
        assert_eq!(node.health, Health::Unknown);
        assert_eq!(node.status, "Deleted");
        assert_eq!(
            node.status_details.as_deref(),
            Some("Error: configmaps \"example\" not found")
        );
    }

    #[test]
    fn trace_status_error_displays_nested_message_instead_of_json() {
        let snapshot = Snapshot::parse(
            br#"{"object":{"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"missing"}},"error":{"ErrStatus":{"apiVersion":"v1","code":404,"message":"configmaps \"missing\" not found","reason":"NotFound","status":"Failure"}}}"#,
        )
        .unwrap();
        assert_eq!(
            snapshot.nodes[0].status_details.as_deref(),
            Some("Error: configmaps \"missing\" not found")
        );
        assert_eq!(snapshot.nodes[0].status, "Deleted");
    }

    #[test]
    fn trace_error_without_message_uses_reason_not_json() {
        let snapshot = Snapshot::parse(
            br#"{"object":{"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"missing"}},"error":{"ErrStatus":{"code":404,"reason":"NotFound"}}}"#,
        )
        .unwrap();
        assert_eq!(snapshot.nodes[0].status, "Deleted");
        assert_eq!(
            snapshot.nodes[0].status_details.as_deref(),
            Some("Error: NotFound")
        );
    }

    #[test]
    fn deleted_status_uses_structured_errors_and_preserves_deleting_precedence() {
        for error in [
            serde_json::json!({"code":404,"message":"object missing"}),
            serde_json::json!({"reason":"NotFound","message":"object missing"}),
            serde_json::json!({"ErrStatus":{"code":404,"message":"object missing"}}),
            serde_json::json!({"ErrStatus":{"reason":"NotFound","message":"object missing"}}),
            serde_json::json!({"errStatus":{"code":404,"message":"object missing"}}),
            serde_json::json!({"errStatus":{"reason":"NotFound","message":"object missing"}}),
        ] {
            let mut trace = serde_json::json!({"object":{"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"example"}},"error":error});
            let snapshot = Snapshot::parse(&serde_json::to_vec(&trace).unwrap()).unwrap();
            let node = &snapshot.nodes[0];
            assert_eq!(node.status, "Deleted");
            assert_eq!(node.health, Health::Unknown);
            assert_eq!(
                node.status_details.as_deref(),
                Some("Error: object missing")
            );
            assert!(matches_query(node, "status:deleted"));
            assert!(matches_query(node, "status:object missing"));

            trace["object"]["metadata"]["deletionTimestamp"] =
                serde_json::json!("2026-09-20T08:49:00Z");
            let snapshot = Snapshot::parse(&serde_json::to_vec(&trace).unwrap()).unwrap();
            assert_eq!(snapshot.nodes[0].status, "Deleting");
            assert_eq!(snapshot.nodes[0].health, Health::Unhealthy);
            assert_eq!(snapshot.nodes[0].status_details, node.status_details);
        }
    }

    #[test]
    fn creating_status_uses_false_readiness_reason_without_hiding_failures() {
        for (ready, reason, synced, trace_error, expected) in [
            ("False", "Creating", None, false, "Creating"),
            ("False", "Creating", Some("True"), false, "Creating"),
            ("False", "Creating", Some("False"), false, "Error"),
            ("False", "Creating", None, true, "Error"),
            ("False", "Waiting", None, false, "Unready"),
            ("Unknown", "Creating", None, false, "Unknown"),
            (
                "True",
                "Creating",
                None,
                false,
                "Creating: Unready resources: child",
            ),
        ] {
            let mut conditions = vec![
                serde_json::json!({"type":"Ready","status":ready,"reason":reason,"message":"Unready resources: child"}),
            ];
            if let Some(synced) = synced {
                conditions.push(serde_json::json!({"type":"Synced","status":synced,"reason":"ReconcileError","message":"cannot connect"}));
            }
            let mut trace = serde_json::json!({"object":{"apiVersion":"example.io/v1","kind":"Resource","metadata":{"name":"example"},"status":{"conditions":conditions}}});
            if trace_error {
                trace["error"] = serde_json::json!({"message":"access denied"});
            }
            let snapshot = Snapshot::parse(&serde_json::to_vec(&trace).unwrap()).unwrap();
            let node = &snapshot.nodes[0];
            assert_eq!(node.status, expected);
            if ready == "False" {
                assert_eq!(node.health, Health::Unhealthy);
                assert!(
                    node.status_details
                        .as_deref()
                        .unwrap()
                        .contains(&format!("Ready=False\n{reason}: Unready resources: child"))
                );
                assert!(matches_query(node, "status:unready resources: child"));
            } else if ready == "True" {
                assert_eq!(node.health, Health::Healthy);
                assert!(node.status_details.is_none());
            }
        }
    }

    #[test]
    fn package_revision_uses_healthy_condition_and_image_tag() {
        let snapshot = Snapshot::parse(
            br#"{"object":{"apiVersion":"pkg.crossplane.io/v1","kind":"ProviderRevision","metadata":{"name":"provider-abc"},"spec":{"image":"xpkg.example/provider:v1.2.3","desiredState":"Active"},"status":{"conditions":[{"type":"RevisionHealthy","status":"True","reason":"HealthyPackageRevision"},{"type":"RuntimeHealthy","status":"True","reason":"Successful"},{"type":"RuntimeActive","status":"True","reason":"Active"}]}}}"#,
        )
        .unwrap();
        let node = &snapshot.nodes[0];
        assert_eq!(node.health, Health::Healthy);
        assert_eq!(node.status, "HealthyPackageRevision");
        assert_eq!(node.package.as_deref(), Some("xpkg.example/provider"));
        assert_eq!(node.version.as_deref(), Some("v1.2.3"));
        assert_eq!(node.state.as_deref(), Some("Active"));
    }

    #[test]
    fn package_revision_accepts_legacy_healthy_condition() {
        let snapshot = Snapshot::parse(
            br#"{"object":{"apiVersion":"pkg.crossplane.io/v1","kind":"ConfigurationRevision","metadata":{"name":"configuration-abc"},"status":{"conditions":[{"type":"Healthy","status":"True","reason":"HealthyPackageRevision"}]}}}"#,
        )
        .unwrap();
        assert_eq!(snapshot.nodes[0].health, Health::Healthy);
    }

    #[test]
    fn healthy_package_revision_ignores_installed_failure() {
        let snapshot = Snapshot::parse(br#"{"object":{"apiVersion":"pkg.crossplane.io/v1","kind":"ProviderRevision","metadata":{"name":"example"},"status":{"conditions":[{"type":"RevisionHealthy","status":"True","reason":"HealthyPackageRevision"},{"type":"Installed","status":"False","reason":"Irrelevant"}]}}}"#).unwrap();
        assert_eq!(snapshot.nodes[0].health, Health::Healthy);
        assert_eq!(snapshot.nodes[0].status, "HealthyPackageRevision");
        assert!(snapshot.nodes[0].status_details.is_none());
    }

    #[test]
    fn explicit_unknown_conditions_are_unhealthy() {
        let snapshot = Snapshot::parse(
            br#"{"object":{"apiVersion":"example.io/v1","kind":"Root","metadata":{"name":"root"},"status":{"conditions":[{"type":"Ready","status":"Unknown"},{"type":"Synced","status":"Unknown"}]}}}"#,
        )
        .unwrap();
        assert_eq!(snapshot.nodes[0].health, Health::Unhealthy);
    }

    #[test]
    fn api_version_is_not_part_of_stable_identity() {
        let one = Identity {
            group: "example.io".into(),
            version: "v1".into(),
            kind: "Widget".into(),
            namespace: Some("default".into()),
            name: "example".into(),
        };
        let two = Identity {
            version: "v2".into(),
            ..one.clone()
        };
        assert_eq!(one, two);
        let identities = HashSet::from([one]);
        assert!(identities.contains(&two));
    }

    #[test]
    fn transition_time_omits_timezone() {
        let snapshot = Snapshot::parse(
            br#"{"object":{"apiVersion":"example.io/v1","kind":"Root","metadata":{"name":"root"},"status":{"conditions":[{"type":"Ready","status":"True","lastTransitionTime":"2026-09-19T14:04:00Z"},{"type":"Synced","status":"True"}]}}}"#,
        )
        .unwrap();
        let timestamp = snapshot.nodes[0].ready_last.as_deref().unwrap();
        assert_eq!(timestamp.len(), 15);
        assert!(!timestamp.ends_with("UTC"));
    }
}
