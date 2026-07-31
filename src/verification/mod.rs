//! Stable obligation identity and orthogonal verification truth types.
//!
//! Slice 1A deliberately contains no renderer, schema, threshold, policy,
//! planner, executor, or exit-status behavior.

pub mod ids;
pub mod model;
mod report;

pub use ids::{IdKind, IdMode, IdOrigin, IdWarning, SourceSpan, StableId};
pub use model::{
    AssertionResolution, BindingStatus, DeclarationStatus, Disposition, EvidenceBinding,
    ExecutabilityStatus, ExecutableCoverage, FeatureProofStatus, FeatureTruth, Freshness,
    ImplementationCoverage, LinkageStatus, Obligation, VerificationTruth, VerifiedCoverage,
};
pub use report::{
    CoverageMetric, CoverageSummary, CoverageThresholds, EvidenceAtom, EvidenceAttempt,
    EvidenceRequirement, EvidenceResult, EvidenceSelection, RunReport, REPORT_SCHEMA_ID,
    REPORT_SCHEMA_VERSION,
};
