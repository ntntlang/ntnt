//! Stable obligation identity and orthogonal verification truth types.
//!
//! Compatibility-derived identities keep existing Intent files parseable, but
//! remain rename/reorder-sensitive migration markers. Durable cross-run evidence
//! must use strict parsing with explicit IDs. Parser-owned identity metadata lives
//! in `IdentifiedIntent`, preserving the legacy public `IntentFile`/`Feature`/
//! `Scenario` AST shapes. Legacy Constraint scenarios stay visible in that AST
//! without becoming feature obligations.
//!
//! Slice 1A deliberately exposes read-only truth queries and contains no evidence
//! ingestion authority, renderer, serialization contract, schema, threshold,
//! policy, planner, executor, resource lifecycle, or exit-status behavior.

pub mod ids;
pub mod model;

pub use ids::{IdKind, IdMode, IdOrigin, IdWarning, SourceSpan, StableId};
pub use model::{
    AssertionResolution, BindingStatus, DeclarationStatus, Disposition, EvidenceBinding,
    ExecutabilityStatus, ExecutableCoverage, FeatureTruth, Freshness, ImplementationCoverage,
    LinkageStatus, Obligation, VerificationTruth, VerifiedCoverage,
};
