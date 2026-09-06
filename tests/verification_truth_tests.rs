use std::path::{Path, PathBuf};

use ntnt::intent::{FeatureVerification, IntentFile};
use ntnt::verification::{DeclarationStatus, IdMode, IdOrigin, VerificationTruth};

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
fn repeated_explicit_ids_on_one_declaration_fail_closed() {
    let repeated_feature = r#"Feature: Repeated feature ID
  id: feature.first
  id: feature.second
"#;
    let feature_error = IntentFile::parse_content_with_id_mode(
        repeated_feature,
        "repeated-feature-id.intent".to_string(),
        IdMode::Strict,
    )
    .expect_err("a feature cannot replace its stable identity")
    .to_string();
    assert!(
        feature_error.contains("repeated feature ID"),
        "{feature_error}"
    );
    assert!(
        feature_error.contains("first declared at"),
        "{feature_error}"
    );

    let repeated_scenario = r#"Feature: Repeated scenario ID
  id: feature.repeated-scenario

  Scenario: Repeated ID
    id: scenario.first
    id: scenario.second
    When one action runs
    → id: outcome.repeated-scenario.claim; one claim holds
"#;
    let scenario_error = IntentFile::parse_content_with_id_mode(
        repeated_scenario,
        "repeated-scenario-id.intent".to_string(),
        IdMode::Strict,
    )
    .expect_err("a scenario cannot replace its stable identity")
    .to_string();
    assert!(
        scenario_error.contains("repeated scenario ID"),
        "{scenario_error}"
    );
    assert!(
        scenario_error.contains("first declared at"),
        "{scenario_error}"
    );

    let repeated_component_scenario = r#"Component: Repeated scenario ID

  Scenario: Repeated ID
    id: scenario.component-first
    id: scenario.component-second
    When one action runs
    → id: outcome.component.claim; one claim holds
"#;
    let component_error = IntentFile::parse_content_with_id_mode(
        repeated_component_scenario,
        "repeated-component-scenario-id.intent".to_string(),
        IdMode::Strict,
    )
    .expect_err("a component scenario cannot replace its stable identity")
    .to_string();
    assert!(
        component_error.contains("repeated scenario ID"),
        "{component_error}"
    );
    assert!(
        component_error.contains("first declared at"),
        "{component_error}"
    );
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
fn legacy_public_ast_struct_literals_remain_constructible() {
    let scenario = ntnt::intent::Scenario {
        name: "Legacy scenario".to_string(),
        description: None,
        given_clause: None,
        when_clause: "legacy action runs".to_string(),
        outcomes: vec!["legacy result".to_string()],
        resolved_test: None,
        component_refs: Vec::new(),
    };
    let feature = ntnt::intent::Feature {
        id: Some("legacy.feature".to_string()),
        name: "Legacy feature".to_string(),
        description: None,
        tests: Vec::new(),
        scenarios: vec![scenario],
    };
    let intent = ntnt::intent::IntentFile {
        features: vec![feature],
        source_path: "legacy.intent".to_string(),
        title: None,
        glossary: None,
        components: Vec::new(),
        invariants: Vec::new(),
        test_data: Vec::new(),
    };

    assert_eq!(intent.features[0].scenarios.len(), 1);

    let parsed: IntentFile = IntentFile::parse_content(
        "Feature: Legacy compatibility parse",
        "legacy-parse.intent".to_string(),
    )
    .expect("legacy parse_content remains compatibility-mode IntentFile parsing");
    assert_eq!(parsed.features.len(), 1);

    let _: fn(&Path) -> Result<IntentFile, ntnt::IntentError> = IntentFile::parse;
}

#[test]
fn compatibility_mode_derives_ids_and_emits_source_located_warnings() {
    let intent = IntentFile::parse_with_id_mode(
        &fixture("compatibility_derived_ids.intent"),
        IdMode::Compatibility,
    )
    .unwrap();

    assert_eq!(intent.verification_warnings().len(), 3);
    assert!(intent
        .verification_warnings()
        .iter()
        .all(|warning| warning.id.origin() == IdOrigin::CompatibilityDerived));
    assert!(intent.verification_warnings().iter().any(|warning| {
        warning.message.contains("derived feature ID") && warning.span.start_line == 1
    }));
    assert!(intent.verification_warnings().iter().any(|warning| {
        warning.message.contains("derived scenario ID") && warning.span.start_line == 3
    }));
    assert!(intent.verification_warnings().iter().any(|warning| {
        warning.message.contains("derived outcome ID") && warning.span.start_line == 5
    }));
}

#[test]
fn compatibility_outcome_ids_are_scoped_by_scenario_identity() {
    let content = r#"Feature: Repeated feature name

  Scenario: Shared behavior
    When the same action runs
    → the same outcome occurs

Feature: Repeated feature name

  Scenario: Shared behavior
    When the same action runs
    → the same outcome occurs
"#;

    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "scoped-derived-outcomes.intent".to_string(),
        IdMode::Compatibility,
    )
    .expect("separate feature scenarios must not derive colliding outcome IDs");

    let first = intent.outcome_stable_id(0, 0, 0).unwrap();
    let second = intent.outcome_stable_id(1, 0, 0).unwrap();
    assert_ne!(first, second);
    assert_eq!(first.origin(), IdOrigin::CompatibilityDerived);
    assert_eq!(second.origin(), IdOrigin::CompatibilityDerived);
}

#[test]
fn compatibility_child_ids_preserve_exact_parent_identity() {
    let content = r#"Feature: First spelling
  id: feature.foo_bar

  Scenario: Shared behavior
    When the same action runs
    → the same outcome occurs

Feature: Second spelling
  id: feature.foo-bar

  Scenario: Shared behavior
    When the same action runs
    → the same outcome occurs
"#;

    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "exact-parent-identity.intent".to_string(),
        IdMode::Compatibility,
    )
    .expect("lossless parent identity must keep child IDs distinct");

    assert_ne!(
        intent.scenario_stable_id(0, 0),
        intent.scenario_stable_id(1, 0)
    );
}

#[test]
fn compatibility_component_scenarios_are_scoped_by_declaration() {
    let content = r#"Component: Shared declaration

  Scenario: Shared behavior
    When the same action runs
    → the same outcome occurs

Component: Shared declaration

  Scenario: Shared behavior
    When the same action runs
    → the same outcome occurs
"#;

    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "component-id-scopes.intent".to_string(),
        IdMode::Compatibility,
    )
    .expect("separate components must not derive colliding scenario identities");

    assert_ne!(
        intent.component_scenario_stable_id(0, 0),
        intent.component_scenario_stable_id(1, 0)
    );
    assert_ne!(
        intent.component_outcome_stable_id(0, 0, 0),
        intent.component_outcome_stable_id(1, 0, 0)
    );
}

#[test]
fn truth_construction_preserves_outcome_metadata_cardinality() {
    let content = r#"Feature: Cardinality
  id: feature.cardinality

  Scenario: One claim
    id: scenario.cardinality.one-claim
    When one action runs
    → id: outcome.cardinality.claim; one claim holds
"#;
    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "cardinality.intent".to_string(),
        IdMode::Strict,
    )
    .unwrap();
    let truth = VerificationTruth::from_intent(&intent).unwrap();
    assert_eq!(intent.features[0].scenarios[0].outcomes.len(), 1);
    assert_eq!(truth.obligations().len(), 1);
    assert_eq!(truth.obligations()[0].statement(), "one claim holds");
}

#[test]
fn constraint_scope_ends_at_section_separator() {
    let content = r#"Feature: Sectioned behavior
  id: feature.sectioned

Constraint: Legacy example
  Scenario: Constraint scenario
    When legacy behavior runs
    → legacy result

---

  Scenario: Behavioral scenario
    id: scenario.sectioned.behavioral
    When behavioral work runs
    → id: outcome.sectioned.behavioral; behavioral result
"#;

    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "constraint-separator.intent".to_string(),
        IdMode::Strict,
    )
    .expect("a section separator ends legacy Constraint containment");
    let truth = VerificationTruth::from_intent(&intent).unwrap();
    assert_eq!(truth.obligations().len(), 1);
    assert_eq!(
        truth.obligations()[0].id().as_str(),
        "outcome.sectioned.behavioral"
    );
}

#[test]
fn markdown_heading_ends_constraint_scope_for_feature_behavior() {
    let content = r#"Feature: Heading behavior
  id: feature.heading

Constraint: Legacy example
  Scenario: Constraint scenario
    When legacy behavior runs
    → legacy result

## Behavioral Scenarios

  Scenario: Behavioral scenario
    id: scenario.heading.behavioral
    When behavioral work runs
    → id: outcome.heading.behavioral; behavioral result
"#;

    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "constraint-heading.intent".to_string(),
        IdMode::Strict,
    )
    .expect("a Markdown heading ends legacy Constraint containment");
    let truth = VerificationTruth::from_intent(&intent).unwrap();
    assert_eq!(truth.obligations().len(), 1);
    assert_eq!(
        truth.obligations()[0].id().as_str(),
        "outcome.heading.behavioral"
    );
}

#[test]
fn markdown_heading_ends_constraint_scope_for_component_behavior() {
    let content = r#"Component: Reusable behavior
  id: component.reusable

Constraint: Legacy example
  Scenario: Constraint scenario
    When legacy behavior runs
    → legacy result

## Behavioral Scenarios

  Scenario: Behavioral scenario
    id: scenario.reusable.behavioral
    When reusable work runs
    → id: outcome.reusable.behavioral; reusable result
"#;

    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "component-constraint-heading.intent".to_string(),
        IdMode::Strict,
    )
    .expect("a Markdown heading ends component Constraint containment");
    assert_eq!(intent.components[0].scenarios.len(), 2);
    assert_eq!(
        intent.component_scenario_stable_id(0, 1).unwrap().as_str(),
        "scenario.reusable.behavioral"
    );
    assert_eq!(
        intent
            .component_outcome_stable_id(0, 1, 0)
            .unwrap()
            .as_str(),
        "outcome.reusable.behavioral"
    );
}

#[test]
fn strict_component_scenario_after_separator_requires_a_redeclared_owner() {
    let content = r#"Component: Reusable behavior
  id: component.reusable

Constraint: Legacy example
  Scenario: Constraint scenario
    When legacy behavior runs
    → legacy result

---

Scenario: Orphaned scenario
  id: scenario.orphaned
  When orphaned work runs
  → id: outcome.orphaned; orphaned result
"#;

    let error = IntentFile::parse_content_with_id_mode(
        content,
        "component-separator-owner.intent".to_string(),
        IdMode::Strict,
    )
    .expect_err("strict scenarios after a component section must redeclare their owner")
    .to_string();
    assert!(
        error.contains("component-separator-owner.intent:11"),
        "{error}"
    );
    assert!(
        error.contains("scenario requires an active Feature or Component owner"),
        "{error}"
    );
}

#[test]
fn test_data_declaration_ends_constraint_scope_for_feature_behavior() {
    let content = r#"Feature: Test data boundary
  id: feature.test-data-boundary

Constraint: Legacy example
  Scenario: Constraint scenario
    When legacy behavior runs
    → legacy result

Test Data: Boundary cases
  id: test-data.boundary

Scenario: Behavioral scenario
  id: scenario.test-data-boundary.behavioral
  When behavioral work runs
  → id: outcome.test-data-boundary.behavioral; behavioral result
"#;

    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "constraint-test-data.intent".to_string(),
        IdMode::Strict,
    )
    .expect("Test Data ends feature Constraint containment");
    let truth = VerificationTruth::from_intent(&intent).unwrap();
    assert_eq!(intent.test_data[0].id, "test-data.boundary");
    assert_eq!(truth.obligations().len(), 1);
    assert_eq!(
        truth.obligations()[0].id().as_str(),
        "outcome.test-data-boundary.behavioral"
    );
}

#[test]
fn test_cases_declaration_ends_constraint_scope_for_component_behavior() {
    let content = r#"Component: Test case boundary
  id: component.test-case-boundary

Constraint: Legacy example
  Scenario: Constraint scenario
    When legacy behavior runs
    → legacy result

Test Cases: Boundary cases
  id: test-cases.boundary

Scenario: Behavioral scenario
  id: scenario.test-case-boundary.behavioral
  When behavioral work runs
  → id: outcome.test-case-boundary.behavioral; behavioral result
"#;

    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "component-constraint-test-cases.intent".to_string(),
        IdMode::Strict,
    )
    .expect("Test Cases ends component Constraint containment");
    assert_eq!(intent.test_data[0].id, "test-cases.boundary");
    assert_eq!(intent.components[0].scenarios.len(), 2);
    assert_eq!(
        intent.component_scenario_stable_id(0, 1).unwrap().as_str(),
        "scenario.test-case-boundary.behavioral"
    );
    assert_eq!(
        intent
            .component_outcome_stable_id(0, 1, 0)
            .unwrap()
            .as_str(),
        "outcome.test-case-boundary.behavioral"
    );
}

#[test]
fn legacy_test_declaration_ends_constraint_scope_without_hiding_later_behavior() {
    let content = r#"Feature: Legacy test boundary
  id: feature.legacy-test-boundary

Constraint: Legacy example
  Scenario: Constraint scenario
    When legacy behavior runs
    → legacy result

test:
  - request: GET /health
    assert:
      - status: 200

Scenario: Behavioral scenario
  id: scenario.legacy-test-boundary.behavioral
  When behavioral work runs
  → id: outcome.legacy-test-boundary.behavioral; behavioral result
"#;

    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "constraint-legacy-test.intent".to_string(),
        IdMode::Strict,
    )
    .expect("legacy test declaration ends Constraint containment");
    let truth = VerificationTruth::from_intent(&intent).unwrap();
    assert_eq!(intent.features[0].tests.len(), 1);
    assert_eq!(truth.features()[0].unrepresented_legacy_test_count(), 1);
    assert_eq!(truth.obligations().len(), 1);
}

#[test]
fn technical_bindings_heading_unwinds_before_feature_behavior() {
    let content = r#"Feature: Binding boundary
  id: feature.binding-boundary

Constraint: Legacy example
  Scenario: Constraint scenario
    When legacy behavior runs
    → legacy result

## Glossary [Technical Bindings]
health check:
  action: GET /health

## Behavioral Scenarios

Scenario: Behavioral scenario
  id: scenario.binding-boundary.behavioral
  When behavioral work runs
  → id: outcome.binding-boundary.behavioral; behavioral result
"#;

    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "constraint-technical-bindings.intent".to_string(),
        IdMode::Strict,
    )
    .expect("a later heading must unwind Technical Bindings state");
    let truth = VerificationTruth::from_intent(&intent).unwrap();
    assert_eq!(truth.obligations().len(), 1);
    assert_eq!(
        truth.obligations()[0].id().as_str(),
        "outcome.binding-boundary.behavioral"
    );
}

#[test]
fn technical_bindings_heading_unwinds_before_component_behavior() {
    let content = r#"Component: Binding boundary
  id: component.binding-boundary

Constraint: Legacy example
  Scenario: Constraint scenario
    When legacy behavior runs
    → legacy result

## Glossary [Technical Bindings]
health check:
  action: GET /health

## Behavioral Scenarios

Scenario: Behavioral scenario
  id: scenario.component-binding-boundary.behavioral
  When behavioral work runs
  → id: outcome.component-binding-boundary.behavioral; behavioral result
"#;

    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "component-constraint-technical-bindings.intent".to_string(),
        IdMode::Strict,
    )
    .expect("a later heading must unwind component Technical Bindings state");
    assert_eq!(intent.components[0].scenarios.len(), 2);
    assert_eq!(
        intent.component_scenario_stable_id(0, 1).unwrap().as_str(),
        "scenario.component-binding-boundary.behavioral"
    );
    assert_eq!(
        intent
            .component_outcome_stable_id(0, 1, 0)
            .unwrap()
            .as_str(),
        "outcome.component-binding-boundary.behavioral"
    );
}

#[test]
fn blank_title_does_not_consume_a_following_feature_declaration() {
    let content = r#"## Title
Feature: Titled behavior
  id: feature.titled

Scenario: Titled scenario
  id: scenario.titled
  When titled work runs
  → id: outcome.titled; titled result
"#;

    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "blank-title-boundary.intent".to_string(),
        IdMode::Strict,
    )
    .expect("a blank Title section must not consume a named declaration");
    let truth = VerificationTruth::from_intent(&intent).unwrap();
    assert_eq!(intent.features.len(), 1);
    assert_eq!(truth.obligations().len(), 1);
}

#[test]
fn separator_finalizes_invariant_and_rejects_an_orphan_outcome() {
    let content = r#"Invariant: Stable invariant
  id: invariant.stable
  Assertions:
    → invariant result

---

→ orphaned result
"#;

    let error = IntentFile::parse_content_with_id_mode(
        content,
        "invariant-separator-boundary.intent".to_string(),
        IdMode::Strict,
    )
    .expect_err("an outcome after an invariant-closing separator must be rejected")
    .to_string();
    assert!(
        error.contains("invariant-separator-boundary.intent:8"),
        "{error}"
    );
    assert!(
        error.contains("outcome requires an active Scenario or Invariant owner"),
        "{error}"
    );
}

#[test]
fn constraint_declaration_cannot_mutate_invariant_or_test_data_state() {
    let content = r#"Invariant: Stable invariant
  id: invariant.stable
  Assertions:
    → invariant result

Constraint: Legacy invariant boundary
  id: invariant.corrupted
  → constraint prose

Test Data: Stable data
  id: test-data.stable

Constraint: Legacy test-data boundary
  id: test-data.corrupted

Feature: Behavioral owner
  id: feature.boundary-owner

Scenario: Behavioral scenario
  id: scenario.boundary-owner
  When behavior runs
  → id: outcome.boundary-owner; behavioral result
"#;

    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "constraint-parent-boundaries.intent".to_string(),
        IdMode::Compatibility,
    )
    .expect("Constraint declarations must close prior parser sections");
    assert_eq!(intent.invariants[0].id, "invariant.stable");
    assert_eq!(intent.invariants[0].assertions, ["invariant result"]);
    assert_eq!(intent.test_data[0].id, "test-data.stable");
    assert_eq!(
        VerificationTruth::from_intent(&intent)
            .unwrap()
            .obligations()
            .len(),
        1
    );
}

#[test]
fn test_data_named_declaration_exits_preserve_sections_and_parent_ids() {
    let content = r#"Test Data: Feature data
  id: test-data.feature

Feature: Feature owner
  id: feature.after-test-data

Scenario: Feature scenario
  id: scenario.after-test-data
  When feature work runs
  → id: outcome.after-test-data; feature result

Test Cases: Component data
  id: test-data.component

Component: Component owner
  id: component.after-test-data

Scenario: Component scenario
  id: scenario.component-after-test-data
  When component work runs
  → id: outcome.component-after-test-data; component result
"#;

    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "test-data-declaration-exits.intent".to_string(),
        IdMode::Strict,
    )
    .expect("named declarations must finalize pending test-data sections");
    assert_eq!(
        intent
            .test_data
            .iter()
            .map(|data| data.id.as_str())
            .collect::<Vec<_>>(),
        ["test-data.feature", "test-data.component"]
    );
    assert_eq!(
        intent.features[0].id.as_deref(),
        Some("feature.after-test-data")
    );
    assert_eq!(intent.components[0].id, "component.after-test-data");
    assert_eq!(
        VerificationTruth::from_intent(&intent)
            .unwrap()
            .obligations()
            .len(),
        1
    );
}

#[test]
fn strict_legacy_tests_require_a_feature_owner() {
    for (name, content) in [
        (
            "top-level",
            r#"test:
  - request: GET /health
    assert:
      - status: 200
"#,
        ),
        (
            "component",
            r#"Component: Reusable
  id: component.reusable

test:
  - request: GET /health
    assert:
      - status: 200
"#,
        ),
    ] {
        let error = IntentFile::parse_content_with_id_mode(
            content,
            format!("{name}-orphan-test.intent"),
            IdMode::Strict,
        )
        .expect_err("strict legacy tests require a Feature owner")
        .to_string();
        assert!(
            error.contains("test requires an active Feature owner"),
            "{error}"
        );
    }
}

#[test]
fn constraint_outcome_verification_syntax_remains_legacy_prose() {
    let content = r#"Feature: Parent feature
  id: feature.parent

Constraint: Legacy containment
  Scenario: Constraint example
    id: scenario.BAD
    When legacy behavior is described
    → id: outcome.BAD; legacy constraint prose
"#;

    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "constraint-outcome-prose.intent".to_string(),
        IdMode::Compatibility,
    )
    .unwrap();
    let scenario = &intent.features[0].scenarios[0];
    assert_eq!(
        scenario.outcomes,
        vec!["id: outcome.BAD; legacy constraint prose"]
    );
    assert_eq!(
        intent.outcome_stable_id(0, 0, 0).unwrap().origin(),
        IdOrigin::CompatibilityDerived
    );
}

#[test]
fn scenario_ids_must_precede_identity_bearing_outcomes() {
    let content = r#"Feature: Parent feature
  id: feature.parent

  Scenario: Late identity
    When behavior runs
    → an earlier result
    id: scenario.parent.late-identity
    → a later result
"#;

    let error = IntentFile::parse_content_with_id_mode(
        content,
        "late-scenario-id.intent".to_string(),
        IdMode::Compatibility,
    )
    .expect_err("a scenario ID cannot replace the parent of existing outcome IDs")
    .to_string();
    assert!(
        error.contains("scenario ID must appear before outcomes"),
        "{error}"
    );
}

#[test]
fn zero_outcome_behavioral_features_are_unproven() {
    let intent =
        IntentFile::parse_with_id_mode(&fixture("zero_outcome_behavioral.intent"), IdMode::Strict)
            .unwrap();
    let truth = VerificationTruth::from_intent(&intent).unwrap();

    assert_eq!(truth.features().len(), 1);
    assert!(truth.features()[0].is_unproven());
    assert!(!truth.feature_is_verified("feature.zero-outcomes"));
    assert_eq!(
        truth.features()[0].declaration(),
        DeclarationStatus::Declared
    );
    assert!(truth.obligations().is_empty());
    assert_eq!(truth.verified_coverage().covered, 0);
    assert_eq!(truth.verified_coverage().total, 0);
}

#[test]
fn justified_documentation_only_features_are_visible_but_not_behavioral() {
    let intent =
        IntentFile::parse_with_id_mode(&fixture("documentation_only.intent"), IdMode::Strict)
            .unwrap();
    let truth = VerificationTruth::from_intent(&intent).unwrap();

    assert_eq!(truth.features().len(), 1);
    assert_eq!(
        truth.features()[0].declaration(),
        DeclarationStatus::DocumentationOnly
    );
    assert_eq!(
        truth.features()[0].rationale(),
        Some("Defines shared vocabulary and makes no behavioral claim")
    );
    assert!(!truth.features()[0].is_unproven());
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
fn compatibility_constraints_remain_visible_without_becoming_feature_truth() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/crypto_chart/crypto.intent");
    let intent = IntentFile::parse_with_id_mode(&path, IdMode::Compatibility)
        .expect("legacy Constraint declarations must still lint");

    assert_eq!(intent.features.len(), 4);
    assert_eq!(
        intent
            .features
            .iter()
            .map(|feature| feature.scenarios.len())
            .sum::<usize>(),
        4
    );

    let error_handling = intent
        .features
        .iter()
        .find(|feature| feature.id.as_deref() == Some("feature.error_handling"))
        .expect("Error Handling feature");
    assert_eq!(error_handling.id.as_deref(), Some("feature.error_handling"));
    assert_eq!(error_handling.scenarios.len(), 1);

    let truth = VerificationTruth::from_intent(&intent).unwrap();
    let feature_truth = truth
        .features()
        .iter()
        .find(|feature| feature.id().as_str() == "feature.error_handling")
        .expect("Error Handling truth");
    assert!(feature_truth.is_unproven());
    assert!(feature_truth.obligation_ids().is_empty());
    assert!(truth
        .obligations()
        .iter()
        .all(|obligation| obligation.feature_id().as_str() != "feature.error_handling"));
}

#[test]
fn constraint_metadata_cannot_mutate_preceding_parent_metadata() {
    let feature_content = r#"Feature: Parent feature
  id: feature.parent
  description: original feature description
  test:
    - request: GET /original

Constraint: Constraint metadata
  description: constraint description
  request: GET /constraint

  Scenario: Constraint scenario
    When the constraint is illustrated
    → the illustration is visible
"#;
    let feature_intent = IntentFile::parse_content_with_id_mode(
        feature_content,
        "constraint-feature-metadata.intent".to_string(),
        IdMode::Strict,
    )
    .unwrap();
    assert_eq!(
        feature_intent.features[0].description.as_deref(),
        Some("original feature description")
    );
    assert_eq!(feature_intent.features[0].tests.len(), 1);
    assert_eq!(feature_intent.features[0].tests[0].path, "/original");

    let component_content = r#"Component: Parent component
  id: component.parent
  description: original component description
  parameters: [original]
  Inherent Behavior:
    → original inherent behavior

Constraint: Constraint metadata
  description: constraint description
  parameters: [constraint]
  Inherent Behavior:
    → constraint inherent behavior

  Scenario: Constraint scenario
    When the constraint is illustrated
    → the illustration is visible
"#;
    let component_intent = IntentFile::parse_content_with_id_mode(
        component_content,
        "constraint-component-metadata.intent".to_string(),
        IdMode::Compatibility,
    )
    .unwrap();
    let component = &component_intent.components[0];
    assert_eq!(
        component.description.as_deref(),
        Some("original component description")
    );
    assert_eq!(component.parameters, ["original"]);
    assert_eq!(component.inherent_behavior, ["original inherent behavior"]);
}

#[test]
fn strict_identity_policy_ignores_legacy_constraint_scenarios() {
    let content = r#"Feature: Behavioral feature
  id: feature.behavioral

Constraint: Legacy declaration

  Scenario: Constraint example
    When the constraint is illustrated
    → the illustration is visible
"#;

    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "strict-constraint.intent".to_string(),
        IdMode::Strict,
    )
    .expect("non-obligation Constraint scenarios must not require verification IDs");

    let truth = VerificationTruth::from_intent(&intent).unwrap();
    assert!(truth.features()[0].is_unproven());
    assert!(truth.obligations().is_empty());
    assert!(intent.verification_warnings().is_empty());
}

#[test]
fn documentation_only_feature_is_not_made_behavioral_by_following_constraint() {
    let content = r#"Feature: Shared terminology
  id: feature.shared-terminology
  verification: documentation-only
  rationale: Defines vocabulary without a behavioral claim

Constraint: Legacy declaration

  Scenario: Constraint example
    When the constraint is illustrated
    → the illustration is visible
"#;

    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "documentation-constraint.intent".to_string(),
        IdMode::Strict,
    )
    .expect("Constraint scenarios must not invalidate documentation-only features");

    let truth = VerificationTruth::from_intent(&intent).unwrap();
    assert_eq!(
        truth.features()[0].declaration(),
        DeclarationStatus::DocumentationOnly
    );
    assert!(!truth.features()[0].is_unproven());
    assert!(truth.obligations().is_empty());
}

#[test]
fn compatibility_outcome_starting_with_id_remains_prose_unless_it_declares_an_outcome_id() {
    let content = r#"Feature: Compatibility prose
  id: feature.compatibility-prose

  Scenario: Describe an id field
    id: scenario.compatibility-prose.id-field
    When the record is returned
    → id: field is returned; alternate key is omitted
"#;

    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "compatibility-id-prose.intent".to_string(),
        IdMode::Compatibility,
    )
    .expect("legacy outcome prose beginning with 'id:' must remain valid");

    let scenario = &intent.features[0].scenarios[0];
    assert_eq!(
        scenario.outcomes,
        ["id: field is returned; alternate key is omitted"]
    );
    assert_eq!(
        intent.outcome_stable_id(0, 0, 0).unwrap().origin(),
        IdOrigin::CompatibilityDerived
    );

    let strict_error = IntentFile::parse_content_with_id_mode(
        content,
        "compatibility-id-prose.intent".to_string(),
        IdMode::Strict,
    )
    .expect_err("strict mode still requires an explicit outcome identity")
    .to_string();
    assert!(
        strict_error.contains("missing outcome ID"),
        "{strict_error}"
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
        .verification_warnings()
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
    assert_eq!(intent.components[0].id, "component.reusable-check");
    assert_eq!(intent.components[0].scenarios.len(), 1);
    assert_eq!(
        intent.component_scenario_stable_id(0, 0).unwrap().as_str(),
        "scenario.component.existing-behavior"
    );
    assert_eq!(intent.components[0].scenarios[0].outcomes.len(), 1);
}

#[test]
fn constraint_metadata_cannot_change_feature_verification() {
    let content = r#"Feature: Behavioral feature
  id: feature.behavioral

  Scenario: Existing behavior
    id: scenario.behavioral.existing
    When the feature runs
    → id: outcome.behavioral.existing; result is valid

Constraint: Legacy boundary
  verification: documentation-only
  rationale: this belongs to the constraint
"#;

    let intent = IntentFile::parse_content_with_id_mode(
        content,
        "feature-constraint.intent".to_string(),
        IdMode::Strict,
    )
    .expect("constraint metadata must not alter feature verification");

    assert!(matches!(
        intent.feature_verification(0),
        Some(&FeatureVerification::Behavioral)
    ));
}
