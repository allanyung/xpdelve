use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use json_patch::Patch as JsonPatch;
use kube::api::{
    Api, DeleteParams, GroupVersionKind, ListParams, Patch, PatchParams, Preconditions,
    PropagationPolicy,
};
use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::core::DynamicObject;
use kube::discovery::{ApiCapabilities, ApiResource, Discovery, Scope};
use kube::{Client, Config};
use serde_json::{Value, json};
use tokio::sync::RwLock;

use crate::model::Identity;
use crate::text;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeletePropagation {
    Foreground,
    Background,
    Orphan,
}

impl DeletePropagation {
    pub fn next(self) -> Self {
        match self {
            Self::Foreground => Self::Background,
            Self::Background => Self::Orphan,
            Self::Orphan => Self::Foreground,
        }
    }

    pub fn explanation(self) -> &'static str {
        match self {
            Self::Foreground => "Dependents are deleted before the resource.",
            Self::Background => "The resource is removed while dependents delete asynchronously.",
            Self::Orphan => "The resource is removed, but its dependents are retained.",
        }
    }
}

impl std::fmt::Display for DeletePropagation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Foreground => formatter.write_str("Foreground"),
            Self::Background => formatter.write_str("Background"),
            Self::Orphan => formatter.write_str("Orphan"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Target {
    pub identity: Identity,
    pub expected_uid: Option<String>,
}

pub struct Kubernetes {
    client: Client,
    discovery: RwLock<Discovery>,
}

impl Kubernetes {
    pub async fn connect(kubeconfig: Option<&Path>, context: Option<&str>) -> Result<Arc<Self>> {
        let options = KubeConfigOptions {
            context: context.map(str::to_owned),
            cluster: None,
            user: None,
        };
        let config = match kubeconfig {
            Some(path) => {
                let kubeconfig = Kubeconfig::read_from(path)
                    .with_context(|| format!("failed to read kubeconfig {}", path.display()))?;
                Config::from_custom_kubeconfig(kubeconfig, &options).await?
            }
            None if context.is_some() => Config::from_kubeconfig(&options).await?,
            None => Config::infer().await?,
        };
        let client = Client::try_from(config)?;
        let discovery = discover(&client).await?;
        Ok(Arc::new(Self {
            client,
            discovery: RwLock::new(discovery),
        }))
    }

    pub async fn yaml(&self, target: &Target) -> Result<String> {
        let object = self.get_current(target).await?;
        let redacted = redact_object(serde_json::to_value(&object)?);
        serde_yaml::to_string(&redacted)
            .map(|yaml| text::sanitize(&yaml))
            .context("failed to render object as YAML")
    }

    pub async fn events(&self, target: &Target) -> Result<String> {
        let object = self.get_current(target).await?;
        let uid = object
            .metadata
            .uid
            .as_deref()
            .context("live object has no UID")?;
        let candidates = [
            GroupVersionKind::gvk("events.k8s.io", "v1", "Event"),
            GroupVersionKind::gvk("", "v1", "Event"),
        ];
        let discovery = self.discovery.read().await;
        let (resource, capabilities) = candidates
            .iter()
            .find_map(|gvk| discovery.resolve_gvk(gvk))
            .context("the cluster does not serve an Events API")?;
        let events = match target.identity.namespace.as_deref() {
            Some(namespace) => {
                Api::<DynamicObject>::namespaced_with(self.client.clone(), namespace, &resource)
            }
            None => Api::<DynamicObject>::all_with(self.client.clone(), &resource),
        };
        if capabilities.scope != Scope::Namespaced {
            bail!("the discovered Events API has an unexpected scope");
        }
        let field = if resource.group == "events.k8s.io" {
            "regarding.uid"
        } else {
            "involvedObject.uid"
        };
        let list = events
            .list(
                &ListParams::default()
                    .fields(&format!("{field}={uid}"))
                    .limit(500),
            )
            .await?;
        if list.items.is_empty() {
            return Ok("No related events found.".into());
        }
        let mut lines = Vec::with_capacity(list.items.len());
        for event in list.items {
            let event_type = event
                .data
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("?");
            let reason = event
                .data
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("?");
            let message = event_message(&event.data);
            let count = event_count(&event.data);
            lines.push(text::sanitize(&format!(
                "{event_type:<8} {reason:<28} x{count}  {message}"
            )));
        }
        Ok(lines.join("\n"))
    }

    pub async fn kubectl_resource(&self, target: &Target) -> Result<String> {
        let (resource, _) = self.resolve(&target.identity).await?;
        let resource = if resource.group.is_empty() {
            resource.plural
        } else {
            format!("{}.{}", resource.plural, resource.group)
        };
        Ok(format!("{resource}/{}", target.identity.name))
    }

    pub async fn delete(&self, target: &Target, propagation: DeletePropagation) -> Result<()> {
        let expected_uid = target
            .expected_uid
            .as_deref()
            .context("trace object has no UID; refresh before deleting")?;
        let (api, current) = self.api_and_current(target).await?;
        let uid = current.metadata.uid.context("live object has no UID")?;
        let resource_version = current
            .metadata
            .resource_version
            .context("live object has no resourceVersion")?;
        ensure_uid(expected_uid, &uid, &target.identity)?;
        let propagation_policy = match propagation {
            DeletePropagation::Foreground => PropagationPolicy::Foreground,
            DeletePropagation::Background => PropagationPolicy::Background,
            DeletePropagation::Orphan => PropagationPolicy::Orphan,
        };
        api.delete(
            &target.identity.name,
            &DeleteParams {
                propagation_policy: Some(propagation_policy),
                preconditions: Some(Preconditions {
                    uid: Some(uid),
                    resource_version: Some(resource_version),
                }),
                ..DeleteParams::default()
            },
        )
        .await?;
        Ok(())
    }

    pub async fn set_paused(&self, target: &Target, paused: bool) -> Result<()> {
        const KEY: &str = "crossplane.io/paused";
        let expected_uid = target
            .expected_uid
            .as_deref()
            .context("trace object has no UID; refresh before mutating it")?;
        let (api, current) = self.api_and_current(target).await?;
        let uid = current
            .metadata
            .uid
            .as_deref()
            .context("live object has no UID")?;
        ensure_uid(expected_uid, uid, &target.identity)?;
        let resource_version = current
            .metadata
            .resource_version
            .as_deref()
            .context("live object has no resourceVersion")?;
        let current_value = current
            .metadata
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.get(KEY));
        if (paused && current_value.is_some_and(|value| value == "true"))
            || (!paused && current_value.is_none())
        {
            return Ok(());
        }

        let mut operations = vec![
            json!({
                "op": "test",
                "path": "/metadata/uid",
                "value": uid,
            }),
            json!({
                "op": "test",
                "path": "/metadata/resourceVersion",
                "value": resource_version,
            }),
        ];
        let path = "/metadata/annotations/crossplane.io~1paused";
        if paused {
            if current.metadata.annotations.is_none() {
                operations.push(json!({"op": "add", "path": "/metadata/annotations", "value": {}}));
            }
            operations.push(json!({"op": "add", "path": path, "value": "true"}));
        } else {
            operations.push(json!({"op": "test", "path": path, "value": current_value}));
            operations.push(json!({"op": "remove", "path": path}));
        }
        patch(&api, &target.identity.name, operations).await?;
        Ok(())
    }

    pub async fn remove_finalizers(&self, target: &Target, selected: &[String]) -> Result<()> {
        let expected_uid = target
            .expected_uid
            .as_deref()
            .context("trace object has no UID; refresh before mutating it")?;
        let (api, current) = self.api_and_current(target).await?;
        let uid = current
            .metadata
            .uid
            .as_deref()
            .context("live object has no UID")?;
        ensure_uid(expected_uid, uid, &target.identity)?;
        let resource_version = current
            .metadata
            .resource_version
            .as_deref()
            .context("live object has no resourceVersion")?;
        let mut finalizers = current.metadata.finalizers.clone().unwrap_or_default();
        let original_len = finalizers.len();
        finalizers.retain(|finalizer| !selected.contains(finalizer));
        if finalizers.len() == original_len {
            return Ok(());
        }
        patch(
            &api,
            &target.identity.name,
            vec![
                json!({"op": "test", "path": "/metadata/uid", "value": uid}),
                json!({"op": "test", "path": "/metadata/resourceVersion", "value": resource_version}),
                json!({"op": "add", "path": "/metadata/finalizers", "value": finalizers}),
            ],
        )
        .await?;
        Ok(())
    }

    async fn get_current(&self, target: &Target) -> Result<DynamicObject> {
        let (_, object) = self.api_and_current(target).await?;
        Ok(object)
    }

    async fn api_and_current(
        &self,
        target: &Target,
    ) -> Result<(Api<DynamicObject>, DynamicObject)> {
        let (resource, capabilities) = self.resolve(&target.identity).await?;
        let api = dynamic_api(
            self.client.clone(),
            &resource,
            &capabilities,
            target.identity.namespace.as_deref(),
        )?;
        let current = api
            .get(&target.identity.name)
            .await
            .with_context(|| format!("failed to fetch {}", target.identity))?;
        Ok((api, current))
    }

    async fn resolve(&self, identity: &Identity) -> Result<(ApiResource, ApiCapabilities)> {
        let gvk = GroupVersionKind::gvk(&identity.group, &identity.version, &identity.kind);
        if let Some(resolved) = self.discovery.read().await.resolve_gvk(&gvk) {
            return Ok(resolved);
        }
        let refreshed = discover(&self.client).await?;
        let resolved = refreshed.resolve_gvk(&gvk);
        *self.discovery.write().await = refreshed;
        resolved.ok_or_else(|| anyhow!("Kubernetes API does not serve {}", identity))
    }
}

fn event_message(event: &Value) -> &str {
    event
        .get("note")
        .or_else(|| event.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn event_count(event: &Value) -> u64 {
    event
        .pointer("/series/count")
        .or_else(|| event.get("deprecatedCount"))
        .or_else(|| event.get("count"))
        .and_then(Value::as_u64)
        .unwrap_or(1)
}

async fn discover(client: &Client) -> Result<Discovery> {
    match Discovery::new(client.clone()).run_aggregated().await {
        Ok(discovery) => Ok(discovery),
        Err(aggregated_error) => Discovery::new(client.clone()).run().await.with_context(|| {
            format!(
                "aggregated discovery failed ({aggregated_error}); legacy discovery also failed"
            )
        }),
    }
}

fn dynamic_api(
    client: Client,
    resource: &ApiResource,
    capabilities: &ApiCapabilities,
    namespace: Option<&str>,
) -> Result<Api<DynamicObject>> {
    match capabilities.scope {
        Scope::Cluster => Ok(Api::all_with(client, resource)),
        Scope::Namespaced => namespace
            .filter(|namespace| !namespace.is_empty())
            .map(|namespace| Api::namespaced_with(client, namespace, resource))
            .ok_or_else(|| anyhow!("namespaced resource {} has no namespace", resource.kind)),
    }
}

async fn patch(api: &Api<DynamicObject>, name: &str, operations: Vec<Value>) -> Result<()> {
    let patch: JsonPatch = serde_json::from_value(Value::Array(operations))?;
    api.patch(name, &PatchParams::default(), &Patch::<()>::Json(patch))
        .await?;
    Ok(())
}

fn ensure_uid(expected_uid: &str, live_uid: &str, identity: &Identity) -> Result<()> {
    if expected_uid != live_uid {
        bail!(
            "{} was recreated since the trace was captured; refresh before retrying",
            identity
        );
    }
    Ok(())
}

pub(crate) fn redact_object(mut object: Value) -> Value {
    let is_secret = object
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind == "Secret");
    if let Some(metadata) = object.get_mut("metadata").and_then(Value::as_object_mut) {
        metadata.remove("managedFields");
    }
    if is_secret && let Some(map) = object.as_object_mut() {
        if map.contains_key("data") {
            map.insert("data".into(), Value::String("<redacted>".into()));
        }
        if map.contains_key("stringData") {
            map.insert("stringData".into(), Value::String("<redacted>".into()));
        }
    }
    object
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn propagation_cycles_from_safe_default() {
        assert_eq!(
            DeletePropagation::Foreground.next(),
            DeletePropagation::Background
        );
        assert_eq!(
            DeletePropagation::Background.next(),
            DeletePropagation::Orphan
        );
        assert_eq!(
            DeletePropagation::Orphan.next(),
            DeletePropagation::Foreground
        );
    }

    #[test]
    fn redacts_secret_payload_and_managed_fields() {
        let redacted = redact_object(json!({
            "kind": "Secret",
            "metadata": {"managedFields": [{"manager": "test"}]},
            "data": {"password": "encoded"},
            "stringData": {"token": "clear"}
        }));
        assert_eq!(redacted["data"], "<redacted>");
        assert_eq!(redacted["stringData"], "<redacted>");
        assert!(redacted["metadata"].get("managedFields").is_none());
    }

    #[test]
    fn event_fields_support_modern_and_legacy_shapes() {
        let modern = json!({"note": "modern", "series": {"count": 7}});
        let legacy = json!({"message": "legacy", "count": 3});
        assert_eq!(event_message(&modern), "modern");
        assert_eq!(event_count(&modern), 7);
        assert_eq!(event_message(&legacy), "legacy");
        assert_eq!(event_count(&legacy), 3);
    }
}
