//! Shared catalog and immutable history models; native identities remain adapter-owned.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct Catalog {
    pub objects: BTreeMap<String, Value>,
    pub uncovered: Vec<String>,
    /// PostgreSQL: the keys of `objects` that declared extensions own
    /// (#430): each declared extension, its members, and its relations with
    /// what follows them. Not part of the catalog's identity, so never
    /// serialized; empty without declared extensions.
    #[serde(skip)]
    pub owned: BTreeSet<String>,
    /// PostgreSQL: for each object whose restore-stable value (#460,
    /// `postgres::restore`) is not the value as PostgreSQL stores it, the
    /// stored value, which plans with canonicalization_version 1
    /// fingerprinted. Not part of the catalog's identity, so never
    /// serialized; empty when every value is stored restore-stable.
    #[serde(skip)]
    pub stored: BTreeMap<String, Value>,
}
impl Catalog {
    /// `json_digest` of this catalog with its values as stored, when any
    /// differs from its restore-stable value.
    pub fn stored_digest(&self) -> Option<String> {
        if self.stored.is_empty() {
            return None;
        }
        let mut objects = self.objects.clone();
        for (key, value) in &self.stored {
            objects.insert(key.clone(), value.clone());
        }
        Some(super::inventory::json_digest(&Catalog {
            objects,
            uncovered: self.uncovered.clone(),
            ..Catalog::default()
        }))
    }
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
