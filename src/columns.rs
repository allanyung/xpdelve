use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Result, bail};
use serde_json::Value;
use unicode_width::UnicodeWidthStr;

use crate::config::ExtraColumnConfig;
use crate::model::{ProjectedNode, ResourceKind, Snapshot};
use crate::text;

const BUILTIN_HEADERS: &[&str] = &[
    "OBJECT",
    "GROUP",
    "SYNCED",
    "SYNCED LAST",
    "READY",
    "READY LAST",
    "STATUS",
    "VERSION",
    "INSTALLED",
    "INSTALLED LAST",
    "HEALTHY",
    "HEALTHY LAST",
    "STATE",
];

pub(crate) fn validate(config: &BTreeMap<String, Vec<ExtraColumnConfig>>) -> Result<()> {
    for (key, columns) in config {
        if resource_kind(key).is_none() {
            bail!("extra_columns.{key}: expected Kind.group (or Kind for the core API group)");
        }
        let mut headers = HashSet::new();
        for column in columns {
            let header = header(&column.name);
            if header.is_empty() || column.name.chars().any(char::is_control) {
                bail!(
                    "extra_columns.{key}: column name must be non-empty and contain no control characters"
                );
            }
            if BUILTIN_HEADERS.contains(&header.as_str()) {
                bail!("extra_columns.{key}: column {header} conflicts with a built-in column");
            }
            if !headers.insert(header.clone()) {
                bail!("extra_columns.{key}: duplicate column {header}");
            }
            if !valid_pointer(&column.path) {
                bail!(
                    "extra_columns.{key}: column {header} path must be a non-empty JSON Pointer with valid ~0/~1 escapes"
                );
            }
            if column
                .width
                .is_some_and(|width| width == 0 || width > usize::from(u16::MAX))
            {
                bail!("extra_columns.{key}: column {header} width must be between 1 and 65535");
            }
        }
    }
    Ok(())
}

fn resource_kind(key: &str) -> Option<ResourceKind> {
    let (kind, group) = key.split_once('.').unwrap_or((key, ""));
    if !kind.starts_with(|character: char| character.is_ascii_alphabetic())
        || !kind
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
        || key.ends_with('.')
        || group.len() > 253
        || (!group.is_empty()
            && group.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label.chars().all(|character| {
                        character.is_ascii_lowercase()
                            || character.is_ascii_digit()
                            || character == '-'
                    })
            }))
    {
        return None;
    }
    Some(ResourceKind {
        kind: kind.into(),
        group: group.into(),
    })
}

fn valid_pointer(path: &str) -> bool {
    if !path.starts_with('/') {
        return false;
    }
    let mut characters = path.chars();
    while let Some(character) = characters.next() {
        if character == '~' && !matches!(characters.next(), Some('0' | '1')) {
            return false;
        }
    }
    true
}

fn cell_text(value: &str) -> String {
    text::sanitize(value).replace(['\n', '\t'], " ")
}

fn header(name: &str) -> String {
    cell_text(name.trim()).to_uppercase()
}

pub(crate) struct ExtraColumn {
    pub header: String,
    pub width: usize,
    paths: HashMap<ResourceKind, String>,
    fixed_width: Option<usize>,
}

impl ExtraColumn {
    pub fn value(&self, node: &ProjectedNode) -> String {
        let Some(path) = self.paths.get(&node.resource_kind) else {
            return "-".into();
        };
        let Some(value) = node.object.pointer(path) else {
            return "-".into();
        };
        if node.resource_kind.group.is_empty()
            && node.resource_kind.kind == "Secret"
            && matches!(path.split('/').nth(1), Some("data" | "stringData"))
        {
            return "<redacted>".into();
        }
        match value {
            Value::String(value) => cell_text(value),
            Value::Bool(_) | Value::Number(_) => value.to_string(),
            _ => "-".into(),
        }
    }
}

pub(crate) fn resolve(
    config: &BTreeMap<String, Vec<ExtraColumnConfig>>,
    snapshot: &Snapshot,
    visible: &[usize],
) -> Vec<ExtraColumn> {
    if config.is_empty() || visible.is_empty() {
        return Vec::new();
    }
    let kinds: HashSet<_> = visible
        .iter()
        .map(|index| &snapshot.nodes[*index].resource_kind)
        .collect();
    let mut columns: Vec<ExtraColumn> = Vec::new();
    let mut by_header = HashMap::new();
    for (key, definitions) in config {
        let Some(kind) = resource_kind(key).filter(|kind| kinds.contains(kind)) else {
            continue;
        };
        for definition in definitions {
            let header = header(&definition.name);
            let index = *by_header.entry(header.clone()).or_insert_with(|| {
                columns.push(ExtraColumn {
                    width: header.width(),
                    header,
                    paths: HashMap::new(),
                    fixed_width: None,
                });
                columns.len() - 1
            });
            let column = &mut columns[index];
            column.paths.insert(kind.clone(), definition.path.clone());
            if let Some(width) = definition.width {
                column.fixed_width = Some(column.fixed_width.unwrap_or_default().max(width));
            }
        }
    }
    for column in &mut columns {
        column.width = column.fixed_width.unwrap_or_else(|| {
            visible
                .iter()
                .map(|index| column.value(&snapshot.nodes[*index]).width())
                .max()
                .unwrap_or_default()
                .max(column.header.width())
        });
    }
    columns
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn snapshot() -> Snapshot {
        Snapshot::parse(br#"{"object":{"apiVersion":"example.org/v1","kind":"Widget","metadata":{"name":"root"},"status":{"url":"https://workspace","count":42}},"children":[{"object":{"apiVersion":"v1","kind":"Secret","metadata":{"name":"child","annotations":{"example.org/owner":"team"}},"data":{"token":"private"},"stringData":{"token":"also-private"}}}]}"#).unwrap()
    }

    fn config(source: &str) -> BTreeMap<String, Vec<ExtraColumnConfig>> {
        toml::from_str::<Config>(source).unwrap().extra_columns
    }

    #[test]
    fn resolves_union_in_key_then_declaration_order_and_shares_headers() {
        let config = config(
            r#"
            [[extra_columns."Widget.example.org"]]
            name = "VALUE"
            path = "/status/url"
            width = 30
            [[extra_columns."Widget.example.org"]]
            name = "COUNT"
            path = "/status/count"
            [[extra_columns.Secret]]
            name = "owner"
            path = "/metadata/annotations/example.org~1owner"
            [[extra_columns.Secret]]
            name = "value"
            path = "/data/token"
            width = 20
        "#,
        );
        validate(&config).unwrap();
        let snapshot = snapshot();
        let columns = resolve(&config, &snapshot, &[0, 1]);
        assert_eq!(
            columns
                .iter()
                .map(|column| column.header.as_str())
                .collect::<Vec<_>>(),
            ["OWNER", "VALUE", "COUNT"]
        );
        assert_eq!(columns[0].value(&snapshot.nodes[0]), "-");
        assert_eq!(columns[0].value(&snapshot.nodes[1]), "team");
        assert_eq!(columns[1].value(&snapshot.nodes[0]), "https://workspace");
        assert_eq!(columns[1].value(&snapshot.nodes[1]), "<redacted>");
        assert_eq!(columns[1].width, 30);
        assert_eq!(columns[2].value(&snapshot.nodes[0]), "42");
        let filtered = resolve(&config, &snapshot, &[0]);
        assert_eq!(
            filtered
                .iter()
                .map(|column| column.header.as_str())
                .collect::<Vec<_>>(),
            ["VALUE", "COUNT"]
        );
        assert!(resolve(&config, &snapshot, &[]).is_empty());
    }

    #[test]
    fn rejects_invalid_definitions() {
        for (key, name, path, width) in [
            ("Widget.example.org/root", "VALUE", "/status/url", 10),
            ("Widget.", "VALUE", "/status/url", 10),
            ("Widget.example..org", "VALUE", "/status/url", 10),
            ("Widget.example.org", " ", "/status/url", 10),
            ("Widget.example.org", "status", "/status/url", 10),
            ("Widget.example.org", "READY LAST", "/status/url", 10),
            ("Widget.example.org", "VALUE", "status.url", 10),
            ("Widget.example.org", "VALUE", "/status/~2url", 10),
            ("Widget.example.org", "VALUE", "/status/url~", 10),
            ("Widget.example.org", "VALUE", "/status/url", 0),
            ("Widget.example.org", "VALUE", "/status/url", 65536),
        ] {
            let source = format!(
                "[[extra_columns.\"{key}\"]]\nname = '{name}'\npath = '{path}'\nwidth = {width}"
            );
            assert!(validate(&config(&source)).is_err(), "accepted {source}");
        }
        let duplicate = config(
            "[[extra_columns.Secret]]\nname='VALUE'\npath='/data'\n[[extra_columns.Secret]]\nname=' value '\npath='/metadata/name'",
        );
        assert!(validate(&duplicate).is_err());
        assert!(
            toml::from_str::<Config>(
                "[[extra_columns.Secret]]\nname='VALUE'\npath='/data'\nunknown=true"
            )
            .is_err()
        );
    }

    #[test]
    fn renders_scalars_and_safe_single_line_text() {
        let snapshot = Snapshot::parse(br#"{"object":{"apiVersion":"example.org/v2","kind":"Widget","metadata":{"name":"another","namespace":"other"},"status":{"value":"a\nb\t\u001b[31m\u202e","enabled":false,"items":["first"],"empty":null,"object":{"nested":true}}}}"#).unwrap();
        for (path, expected) in [
            ("/status/value", "a b \\x1b[31m\\u{202E}"),
            ("/status/enabled", "false"),
            ("/status/items/0", "first"),
            ("/status/items", "-"),
            ("/status/object", "-"),
            ("/status/empty", "-"),
            ("/status/missing", "-"),
        ] {
            let config = config(&format!(
                "[[extra_columns.\"Widget.example.org\"]]\nname='VALUE'\npath='{path}'"
            ));
            let columns = resolve(&config, &snapshot, &[0]);
            assert_eq!(columns[0].value(&snapshot.nodes[0]), expected);
            assert_eq!(columns[0].width, expected.width().max(5));
        }
    }

    #[test]
    fn redacts_secret_payloads_but_not_metadata_or_custom_secret_kinds() {
        let snapshot = snapshot();
        for path in ["/data", "/data/token", "/stringData", "/stringData/token"] {
            let config = config(&format!(
                "[[extra_columns.Secret]]\nname='VALUE'\npath='{path}'"
            ));
            assert_eq!(
                resolve(&config, &snapshot, &[1])[0].value(&snapshot.nodes[1]),
                "<redacted>"
            );
        }
        let snapshot = Snapshot::parse(br#"{"object":{"apiVersion":"example.org/v1","kind":"Secret","metadata":{"name":"custom"},"data":{"value":"visible"}}}"#).unwrap();
        let config =
            config("[[extra_columns.\"Secret.example.org\"]]\nname='VALUE'\npath='/data/value'");
        assert_eq!(
            resolve(&config, &snapshot, &[0])[0].value(&snapshot.nodes[0]),
            "visible"
        );
    }

    #[test]
    fn matching_is_exact_by_kind_and_group_but_ignores_version_name_and_namespace() {
        let snapshot = Snapshot::parse(br#"{"object":{"apiVersion":"alpha.example.org/v1","kind":"Widget","metadata":{"name":"one","namespace":"a"}},"children":[{"object":{"apiVersion":"alpha.example.org/v2","kind":"Widget","metadata":{"name":"two","namespace":"b"}}},{"object":{"apiVersion":"beta.example.org/v1","kind":"Widget","metadata":{"name":"other"}}},{"object":{"apiVersion":"alpha.example.org/v1","kind":"Other","metadata":{"name":"other-kind"}}}]}"#).unwrap();
        let config = config(
            "[[extra_columns.\"Widget.alpha.example.org\"]]\nname='NAME'\npath='/metadata/name'",
        );
        let columns = resolve(&config, &snapshot, &[0, 1, 2, 3]);
        assert_eq!(columns.len(), 1);
        assert_eq!(columns[0].value(&snapshot.nodes[0]), "one");
        assert_eq!(columns[0].value(&snapshot.nodes[1]), "two");
        assert_eq!(columns[0].value(&snapshot.nodes[2]), "-");
        assert_eq!(columns[0].value(&snapshot.nodes[3]), "-");
        assert!(resolve(&config, &snapshot, &[2, 3]).is_empty());
    }
}
