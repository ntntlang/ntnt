use std::path::{Path, PathBuf};

use ntnt::intent::IntentFile;
use ntnt::verification::{
    AssertionResolution, BindingStatus, DeclarationStatus, Disposition, EvidenceBinding,
    ExecutabilityStatus, Freshness, IdMode, IdOrigin, LinkageStatus, SourceSpan, VerificationTruth,
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/verification/truth")
        .join(name)
}

fn strict_error(name: &str) -> String {
    IntentFile::parse_with_id_mode(&fixture(name), IdMode::Strict)
        .expect_err("fixture must be rejected")
        .to_string()
}

#[test]
fn duplicate_feature_ids_report_both_source_locations() {
    let error = strict_error("duplicate_feature_ids.intent");
    assert!(
        error.contains("duplicate feature ID 'feature.duplicate'"),
        "{error}"
    );
    assert!(error.contains("duplicate_feature_ids.intent:5"), "{error}");
    assert!(error.contains("first declared at"), "{error}");
    assert!(error.contains("duplicate_feature_ids.intent:2"), "{error}");
}

#[test]
fn duplicate_scenario_ids_report_both_source_locations() {
    let error = strict_error("duplicate_scenario_ids.intent");
    assert!(
        error.contains("duplicate scenario ID 'scenario.duplicate'"),
        "{error}"
    );
    assert!(
        error.contains("duplicate_scenario_ids.intent:10"),
        "{error}"
    );
    assert!(error.contains("duplicate_scenario_ids.intent:5"), "{error}");
}

#[test]
fn duplicate_outcome_ids_report_both_source_locations() {
    let error = strict_error("duplicate_outcome_ids.intent");
    assert!(
        error.contains("duplicate outcome ID 'outcome.duplicate'"),
        "{error}"
    );
    assert!(error.contains("duplicate_outcome_ids.intent:8"), "{error}");
    assert!(error.contains("duplicate_outcome_ids.intent:7"), "{error}");
}

#[test]
fn malformed_ids_report_the_entity_and_source_location() {
    for (fixture_name, kind, id, line) in [
        (
            "malformed_feature_id.intent",
            "feature",
            "feature.Bad ID",
            2,
        ),
        (
            "malformed_scenario_id.intent",
            "scenario",
            "scenario..bad",
            5,
        ),
        ("malformed_outcome_id.intent", "outcome", "outcome.BAD", 7),
    ] {
        let error = strict_error(fixture_name);
        assert!(
            error.contains(&format!("malformed {kind} ID '{id}'")),
            "{error}"
        );
        assert!(error.contains(&format!("{fixture_name}:{line}")), "{error}");
    }
}

#[test]
fn strict_mode_rejects_missing_stable_ids() {
    let error = strict_error("compatibility_derived_ids.intent");
    assert!(error.contains("missing feature ID"), "{error}");
    assert!(
        error.contains("compatibility_derived_ids.intent:1"),
        "{error}"
    );
}

#[test]
fn compatibility_mode_derives_ids_and_emits_source_located_warnings() {
    let intent = IntentFile::parse_with_id_mode(
        &fixture("compatibility_derived_ids.intent"),
        IdMode::Compatibility,
    )
    .unwrap();

    assert_eq!(intent.verification_warnings.len(), 3);
    assert!(intent
        .verification_warnings
        .iter()
        .all(|warning| warning.id.origin() == IdOrigin::CompatibilityDerived));
    assert!(intent.verification_warnings.iter().any(|warning| {
        warning.message.contains("derived feature ID") && warning.span.start_line == 1
    }));
    assert!(intent.verification_warnings.iter().any(|warning| {
        warning.message.contains("derived scenario ID") && warning.span.start_line == 3
    }));
    assert!(intent.verification_warnings.iter().any(|warning| {
        warning.message.contains("derived outcome ID") && warning.span.start_line == 5
    }));
}

#[test]
fn zero_outcome_behavioral_features_are_unproven() {
    let intent =
        IntentFile::parse_with_id_mode(&fixture("zero_outcome_behavioral.intent"), IdMode::Strict)
            .unwrap();
    let truth = VerificationTruth::from_intent(&intent);

    assert_eq!(truth.features.len(), 1);
    assert!(truth.features[0].is_unproven());
    assert!(!truth.feature_is_verified("feature.zero-outcomes"));
    assert_eq!(truth.features[0].declaration, DeclarationStatus::Declared);
    assert!(truth.obligations.is_empty());
    assert_eq!(truth.verified_coverage().covered, 0);
    assert_eq!(truth.verified_coverage().total, 0);
}

#[test]
fn justified_documentation_only_features_are_visible_but_not_behavioral() {
    let intent =
        IntentFile::parse_with_id_mode(&fixture("documentation_only.intent"), IdMode::Strict)
            .unwrap();
    let truth = VerificationTruth::from_intent(&intent);

    assert_eq!(truth.features.len(), 1);
    assert_eq!(
        truth.features[0].declaration,
        DeclarationStatus::DocumentationOnly
    );
    assert_eq!(
        truth.features[0].rationale.as_deref(),
        Some("Defines shared vocabulary and makes no behavioral claim")
    );
    assert!(!truth.features[0].is_unproven());
    assert!(!truth.feature_is_verified("feature.domain-terminology"));
    assert_eq!(truth.behavioral_feature_count(), 0);
}

#[test]
fn documentation_only_cannot_suppress_an_outcome() {
    let error = strict_error("outcome_documentation_only.intent");
    assert!(
        error.contains("documentation-only is valid only on a feature"),
        "{error}"
    );
    assert!(
        error.contains("outcome_documentation_only.intent:7"),
        "{error}"
    );
}

#[test]
fn linked_but_unexecuted_obligations_only_have_implementation_coverage() {
    let intent =
        IntentFile::parse_with_id_mode(&fixture("linked_unexecuted.intent"), IdMode::Strict)
            .unwrap();
    let mut truth = VerificationTruth::from_intent(&intent);
    truth
        .mark_implementation_linked("outcome.linked.result")
        .unwrap();

    assert_eq!(truth.implementation_coverage().covered, 1);
    assert_eq!(truth.implementation_coverage().total, 1);
    assert_eq!(truth.executable_coverage().covered, 0);
    assert_eq!(truth.executable_coverage().total, 1);
    assert_eq!(truth.verified_coverage().covered, 0);
    assert_eq!(truth.verified_coverage().total, 1);
}

#[test]
fn unknown_and_unresolved_assertions_fail_closed() {
    let span = SourceSpan::single_line("verification.tnt", 12, 5, 20);
    for resolution in [
        AssertionResolution::Unknown,
        AssertionResolution::Unresolved,
    ] {
        let binding = EvidenceBinding {
            id: "binding.example".to_string(),
            obligation_id: "outcome.example".to_string(),
            source: span.clone(),
            declaration: DeclarationStatus::Declared,
            linkage: LinkageStatus::Linked,
            binding: BindingStatus::Bound,
            executability: ExecutabilityStatus::Executable,
            disposition: Disposition::Passed,
            freshness: Freshness::Current,
            assertion_resolution: resolution,
            evidence_atoms: 1,
        };

        assert!(!binding.satisfies_obligation());
    }
}

#[test]
fn truth_dimensions_remain_orthogonal() {
    let binding = EvidenceBinding {
        id: "binding.blocked".to_string(),
        obligation_id: "outcome.example".to_string(),
        source: SourceSpan::single_line("verification.tnt", 7, 1, 12),
        declaration: DeclarationStatus::Declared,
        linkage: LinkageStatus::Linked,
        binding: BindingStatus::Bound,
        executability: ExecutabilityStatus::Blocked,
        disposition: Disposition::Planned,
        freshness: Freshness::Stale,
        assertion_resolution: AssertionResolution::Resolved,
        evidence_atoms: 0,
    };

    assert_eq!(binding.linkage, LinkageStatus::Linked);
    assert_eq!(binding.binding, BindingStatus::Bound);
    assert_eq!(binding.executability, ExecutabilityStatus::Blocked);
    assert_eq!(binding.disposition, Disposition::Planned);
    assert_eq!(binding.freshness, Freshness::Stale);
    assert!(!binding.satisfies_obligation());
}

#[test]
fn behavioral_evidence_does_not_require_implementation_linkage() {
    let binding = EvidenceBinding {
        id: "binding.behavioral".to_string(),
        obligation_id: "outcome.behavioral".to_string(),
        source: SourceSpan::single_line("verification.tnt", 9, 1, 20),
        declaration: DeclarationStatus::Declared,
        linkage: LinkageStatus::Unlinked,
        binding: BindingStatus::Bound,
        executability: ExecutabilityStatus::Executable,
        disposition: Disposition::Passed,
        freshness: Freshness::Current,
        assertion_resolution: AssertionResolution::Resolved,
        evidence_atoms: 1,
    };

    assert!(binding.satisfies_obligation());

    let intent =
        IntentFile::parse_with_id_mode(&fixture("linked_unexecuted.intent"), IdMode::Strict)
            .unwrap();
    let mut truth = VerificationTruth::from_intent(&intent);
    let obligation = &mut truth.obligations[0];
    obligation.linkage = LinkageStatus::Unlinked;
    obligation.binding = BindingStatus::Bound;
    obligation.executability = ExecutabilityStatus::Executable;
    obligation.disposition = Disposition::Passed;
    obligation.freshness = Freshness::Current;
    obligation.evidence_bindings.push(binding);

    assert!(obligation.is_verified());
}

#[test]
fn compatibility_parser_does_not_reinterpret_constraint_metadata() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/crypto_chart/crypto.intent");
    let intent = IntentFile::parse(&path).expect("legacy Constraint declarations must still lint");

    assert_eq!(intent.features.len(), 4);
    assert_eq!(
        intent
            .features
            .iter()
            .map(|feature| feature.scenarios.len())
            .sum::<usize>(),
        4
    );
}

#[test]
fn outcome_statement_can_quote_documentation_only_syntax() {
    let content = r#"Feature: Parser documentation
  id: feature.parser-documentation

  Scenario: Explain invalid suppression
    id: scenario.parser-documentation.invalid-suppression
    When syntax is documented
    → id: outcome.parser-documentation.message; parser rejects 'verification: documentation-only' on non-feature nodes
"#;

    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "parser-documentation.intent".to_string(),
        IdMode::Strict,
    )
    .expect("quoted syntax is prose, not an outcome-level directive");

    assert!(
        intent.features[0].scenarios[0].outcomes[0].contains("'verification: documentation-only'")
    );
}

#[test]
fn compatibility_outcome_warning_points_to_the_outcome_marker() {
    let content = r#"Feature: Source spans
  id: feature.source-spans

  Scenario: Derive an outcome ID
    id: scenario.source-spans.derived-outcome
    When source locations are recorded
    → the widget id: field is set
"#;

    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "source-spans.intent".to_string(),
        IdMode::Compatibility,
    )
    .unwrap();
    let warning = intent
        .verification_warnings
        .iter()
        .find(|warning| warning.message.contains("derived outcome ID"))
        .expect("outcome warning");

    assert_eq!(warning.span.start_line, 7);
    assert_eq!(warning.span.start_column, 5);
}

#[test]
fn constraint_boundary_preserves_an_active_component_scenario() {
    let content = r#"Component: Reusable check
  id: component.reusable-check

  Scenario: Existing component behavior
    id: scenario.component.existing-behavior
    When the component runs
    → id: outcome.component.existing-behavior; result is valid

Constraint: Legacy boundary
  id: constraint.legacy-boundary

Feature: Following feature
  id: feature.following
"#;

    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "component-constraint.intent".to_string(),
        IdMode::Strict,
    )
    .expect("Constraint must not discard the active component scenario");

    assert_eq!(intent.components.len(), 1);
    assert_eq!(intent.components[0].scenarios.len(), 1);
    assert_eq!(
        intent.components[0].scenarios[0].verification_id.as_str(),
        "scenario.component.existing-behavior"
    );
    assert_eq!(intent.components[0].scenarios[0].outcomes.len(), 1);
}
