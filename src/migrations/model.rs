//! Shared catalog and immutable history models; native identities remain adapter-owned.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct Catalog {
    pub objects: BTreeMap<String, Value>,
    pub uncovered: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct HistoryEntry {
    pub position: i64,
    pub id: String,
    pub name: String,
    pub checksum: String,
    pub state: String,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct History {
    pub present: bool,
    pub entries: Vec<HistoryEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<Value>,
}

impl History {
    /// Counts source coverage separately from execution/import evidence.
    pub fn counts(&self) -> Value {
        let adopted = self
            .entries
            .first()
            .filter(|e| e.state == "adopted")
            .and_then(|e| usize::try_from(e.position).ok())
            .unwrap_or(0);
        let applied = self.entries.iter().filter(|e| e.state == "applied").count();
        let imported = self
            .entries
            .iter()
            .filter(|e| e.state == "imported")
            .count();
        serde_json::json!({"covered_count":adopted+applied+imported,"applied_count":applied,"adopted_count":adopted,"imported_count":imported})
    }
}
