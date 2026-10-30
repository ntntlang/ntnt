//! Shared catalog and immutable history models; native identities remain adapter-owned.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct Catalog {
    pub objects: BTreeMap<String, Value>,
    pub uncovered: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct HistoryEntry {
    pub position: i64,
    pub id: String,
    pub name: String,
    pub checksum: String,
    pub state: String,
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct History {
    pub present: bool,
    pub entries: Vec<HistoryEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<Value>,
}
