# {{PROJECT_NAME}}
# {{DESCRIPTION}}
# Run: ntnt intent check {{FILENAME}}.tnt

## Overview
Brief description of what this project does and its purpose.

## Glossary

| Term | Means |
|------|-------|
| success response | status 2xx |
| they see {text} | body contains {text} |
| they don't see {text} | body not contains {text} |
| returns HTML | header "Content-Type" contains "text/html" |
| returns JSON | header "Content-Type" contains "application/json" |
| page loads successfully | status 200, returns HTML |

---

Component: {{COMPONENT_NAME}}
  id: component.{{component_id}}
  description: "Reusable behavior pattern"

  Inherent Behavior:
    → expected behavior 1
    → expected behavior 2

---

Feature: {{FEATURE_NAME}}
  id: feature.{{feature_id}}
  description: "What this feature does"

  Scenario: {{SCENARIO_NAME}}
    description: "What this scenario tests"
    When a user visits the homepage
    → page loads successfully
    → they see "Expected Content"

  Scenario: Another Scenario
    description: "Another test case"
    Given some precondition
    When a user performs an action
    → expected result

---

Feature: API Endpoint
  id: feature.api_endpoint
  description: "JSON API for data access"

  Scenario: Get data successfully
    description: "Happy path for data retrieval"
    When a user visits /api/data
    → status 200
    → returns JSON
    → they see "data"

  Scenario: Handle missing data
    description: "Error case when data not found"
    When a user visits /api/data/999
    → status 404
    → they see "not found"

---

Constraint: Responsive Design
  description: "Pages should be mobile-friendly with proper viewport meta"
  applies_to: [feature.{{feature_id}}]

Constraint: Accessible
  description: "Use semantic HTML and proper heading hierarchy"
  applies_to: [feature.{{feature_id}}]
