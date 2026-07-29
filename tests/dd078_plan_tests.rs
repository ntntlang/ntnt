use std::collections::{BTreeMap, BTreeSet};

const PLAN: &str = include_str!("../plans/dd-078-intent-verification-implementation.md");

#[derive(Debug)]
struct SliceGraph {
    dependencies: BTreeMap<String, BTreeSet<String>>,
    scopes: BTreeMap<String, String>,
}

fn expand_id(value: &str) -> Result<Vec<String>, String> {
    let value = value.trim();
    let Some((start, end)) = value.split_once('–') else {
        return Ok(vec![value.to_string()]);
    };

    let prefix_len = start
        .char_indices()
        .find_map(|(index, ch)| ch.is_ascii_alphabetic().then_some(index))
        .unwrap_or(start.len());
    let (prefix, start_suffix) = start.split_at(prefix_len);
    let end_suffix = end.strip_prefix(prefix).unwrap_or(end);

    if start_suffix.len() == 1
        && end_suffix.len() == 1
        && start_suffix.as_bytes()[0].is_ascii_alphabetic()
        && end_suffix.as_bytes()[0].is_ascii_alphabetic()
    {
        let start_byte = start_suffix.as_bytes()[0];
        let end_byte = end_suffix.as_bytes()[0];
        if start_byte > end_byte {
            return Err(format!("reversed slice range {value}"));
        }
        return Ok((start_byte..=end_byte)
            .map(|suffix| format!("{prefix}{}", suffix as char))
            .collect());
    }

    Ok(vec![start.to_string(), end.to_string()])
}

fn dependency_tokens(value: &str) -> Result<BTreeSet<String>, String> {
    let mut tokens = BTreeSet::new();
    for item in value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        if item.starts_with("DD-") || item.starts_with("Task ") {
            tokens.insert(item.to_string());
        } else {
            tokens.extend(expand_id(item)?);
        }
    }
    Ok(tokens)
}

fn internal_dependencies(value: &BTreeSet<String>) -> Vec<String> {
    value
        .iter()
        .filter(|item| !item.starts_with("DD-") && !item.starts_with("Task "))
        .cloned()
        .collect()
}

fn depends_transitively(graph: &SliceGraph, slice: &str, target: &str) -> bool {
    let mut pending = vec![slice.to_string()];
    let mut seen = BTreeSet::new();
    while let Some(current) = pending.pop() {
        if !seen.insert(current.clone()) {
            continue;
        }
        for dependency in internal_dependencies(&graph.dependencies[&current]) {
            if dependency == target {
                return true;
            }
            pending.push(dependency);
        }
    }
    false
}

fn parse_graph(plan: &str) -> Result<SliceGraph, String> {
    let block = plan
        .split_once("### Dependency-closed DD-078 PR slices")
        .ok_or("missing DD-078 slice table marker")?
        .1
        .split_once("Each task below supplies")
        .ok_or("missing DD-078 slice table terminator")?
        .0;

    let mut dependencies = BTreeMap::new();
    let mut scopes = BTreeMap::new();
    for line in block.lines().filter(|line| line.starts_with("| ")) {
        if line.starts_with("| Slice") || line.starts_with("|---") {
            continue;
        }
        let columns: Vec<_> = line.trim_matches('|').split('|').map(str::trim).collect();
        if columns.len() != 3 {
            return Err(format!("malformed slice row: {line}"));
        }
        let row_dependencies = dependency_tokens(columns[2])?;
        for id in expand_id(columns[0])? {
            if dependencies
                .insert(id.clone(), row_dependencies.clone())
                .is_some()
            {
                return Err(format!("duplicate slice ID {id}"));
            }
            scopes.insert(id, columns[1].to_string());
        }
    }
    if dependencies.is_empty() {
        return Err("slice table must not be empty".to_string());
    }
    Ok(SliceGraph {
        dependencies,
        scopes,
    })
}

fn visit(
    id: &str,
    graph: &SliceGraph,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
) -> Result<(), String> {
    if visited.contains(id) {
        return Ok(());
    }
    if !visiting.insert(id.to_string()) {
        return Err(format!("dependency cycle at {id}"));
    }
    for dependency in internal_dependencies(&graph.dependencies[id]) {
        if !graph.dependencies.contains_key(&dependency) {
            return Err(format!(
                "slice {id} has unknown internal dependency {dependency}"
            ));
        }
        visit(&dependency, graph, visiting, visited)?;
    }
    visiting.remove(id);
    visited.insert(id.to_string());
    Ok(())
}

fn heading_ids(line: &str) -> Result<Option<Vec<String>>, String> {
    let declaration = if let Some(rest) = line.strip_prefix("## Task ") {
        if let Some((_, slice)) = rest.split_once(" / Slice ") {
            slice.split_once(':').ok_or("slice heading colon")?.0
        } else if let Some((_, slices)) = rest.split_once(" / Slices ") {
            slices.split_once(':').ok_or("slices heading colon")?.0
        } else {
            rest.split_once(':').ok_or("task heading colon")?.0
        }
    } else if let Some(rest) = line.strip_prefix("### Slice ") {
        rest.split_once(':').ok_or("slice heading colon")?.0
    } else {
        return Ok(None);
    };

    let mut ids = Vec::new();
    for part in declaration.replace(" and ", ",").split(',') {
        ids.extend(expand_id(part.trim())?);
    }
    Ok(Some(ids))
}

fn validate_owners(plan: &str, graph: &SliceGraph) -> Result<(), String> {
    let lines: Vec<_> = plan.lines().collect();
    let mut owners = BTreeMap::<String, usize>::new();
    let allowed_non_slice_tasks = ["0", "15", "16", "17"];

    for (index, line) in lines.iter().enumerate() {
        let Some(ids) = heading_ids(line)? else {
            continue;
        };
        let known_ids: Vec<_> = ids
            .iter()
            .filter(|id| graph.dependencies.contains_key(*id))
            .collect();
        for id in &ids {
            if id.chars().any(|ch| ch.is_ascii_alphabetic()) && !graph.dependencies.contains_key(id)
            {
                return Err(format!("unknown slice owner heading {id}"));
            }
            if !graph.dependencies.contains_key(id)
                && !allowed_non_slice_tasks.contains(&id.as_str())
            {
                return Err(format!("unknown task owner heading {id}"));
            }
        }
        if known_ids.is_empty() {
            continue;
        }
        let end = lines[index + 1..]
            .iter()
            .position(|candidate| {
                candidate.starts_with("## Task ") || candidate.starts_with("### Slice ")
            })
            .map(|offset| index + 1 + offset)
            .unwrap_or(lines.len());
        let dependency_lines: Vec<_> = lines[index + 1..end]
            .iter()
            .filter_map(|candidate| candidate.strip_prefix("**Table dependencies:** "))
            .collect();
        if dependency_lines.len() != 1 {
            return Err(format!(
                "owner heading {line} must have exactly one Table dependencies line"
            ));
        }
        let owner_dependencies = dependency_tokens(dependency_lines[0])?;
        for id in known_ids {
            *owners.entry(id.clone()).or_default() += 1;
            if owner_dependencies != graph.dependencies[id] {
                return Err(format!(
                    "slice {id} owner dependencies {owner_dependencies:?} do not match table {:?}",
                    graph.dependencies[id]
                ));
            }
        }
    }

    for id in graph.dependencies.keys() {
        if owners.get(id) != Some(&1) {
            return Err(format!(
                "slice {id} must have exactly one owning task/section heading"
            ));
        }
    }
    Ok(())
}

fn validate_created_module_registration(plan: &str, graph: &SliceGraph) -> Result<(), String> {
    let lines: Vec<_> = plan.lines().collect();
    for (index, heading) in lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.starts_with("## Task ") || line.starts_with("### Slice "))
    {
        let owner = heading_ids(heading)?
            .unwrap_or_default()
            .into_iter()
            .find(|id| graph.dependencies.contains_key(id));
        let end = lines[index + 1..]
            .iter()
            .position(|candidate| {
                candidate.starts_with("## Task ") || candidate.starts_with("### Slice ")
            })
            .map(|offset| index + 1 + offset)
            .unwrap_or(lines.len());
        let section = lines[index..end].join("\n");
        let mut remainder = section.as_str();
        while let Some(create_start) = remainder.find("**Create") {
            let after_create = &remainder[create_start..];
            let create_end = [
                "\n**Create",
                "\n**Modify",
                "\n**RED",
                "\n**GREEN",
                "\n**REFACTOR",
                "\n**Gate",
                "\n**Verify",
                "\n**Acceptance",
            ]
            .iter()
            .filter_map(|marker| after_create.find(marker))
            .min()
            .unwrap_or(after_create.len());
            let create_block = &after_create[..create_end];
            for path in create_block
                .split('`')
                .filter(|part| part.starts_with("src/") && part.ends_with(".rs"))
            {
                let required = if path.starts_with("src/verification/provider/")
                    && path != "src/verification/provider/mod.rs"
                {
                    Some("src/verification/provider/mod.rs")
                } else if path.starts_with("src/verification/") && path != "src/verification/mod.rs"
                {
                    Some("src/verification/mod.rs")
                } else if path.starts_with("src/stdlib/test/") {
                    Some("src/stdlib/test.rs")
                } else if path.starts_with("src/stdlib/") && path != "src/stdlib/mod.rs" {
                    Some("src/stdlib/mod.rs")
                } else if path.starts_with("src/project_env/") && path != "src/project_env/mod.rs" {
                    Some("src/project_env/mod.rs")
                } else if path.matches('/').count() == 1
                    && path != "src/main.rs"
                    && !path.starts_with("src/bin/")
                {
                    Some("src/lib.rs")
                } else {
                    None
                };
                if let Some(required) = required {
                    if !section.contains(required) {
                        return Err(format!(
                            "owner {heading} creates {path} without parent registration {required}"
                        ));
                    }
                    let creator = match required {
                        "src/verification/mod.rs" => Some("1A"),
                        "src/stdlib/test.rs" => Some("3D"),
                        "src/verification/provider/mod.rs" => Some("5A"),
                        "src/project_env/mod.rs" => Some("14D"),
                        _ => None,
                    };
                    if let (Some(owner), Some(creator)) = (owner.as_deref(), creator) {
                        if owner != creator && !depends_transitively(graph, owner, creator) {
                            return Err(format!(
                                "owner {heading} creates {path}, but parent {required} is owned by non-dependency {creator}"
                            ));
                        }
                    }
                }
            }
            if create_end == after_create.len() {
                break;
            }
            remainder = &after_create[create_end..];
        }
    }
    Ok(())
}

fn validate_external_prerequisites(plan: &str, graph: &SliceGraph) -> Result<(), String> {
    let ledger = plan
        .split_once("### External prerequisite ledger")
        .ok_or("missing external prerequisite ledger")?
        .1
        .split_once("### Dependency-closed DD-078 PR slices")
        .ok_or("missing external prerequisite ledger terminator")?
        .0;
    let owners: BTreeSet<_> = ledger
        .lines()
        .filter(|line| {
            line.starts_with("| ")
                && !line.starts_with("| External owner")
                && !line.starts_with("|---")
        })
        .filter_map(|line| line.trim_matches('|').split('|').next())
        .map(str::trim)
        .map(str::to_string)
        .collect();

    for dependency in graph.dependencies.values().flatten() {
        if dependency.starts_with("DD-") && !owners.contains(dependency) {
            return Err(format!("unknown external prerequisite {dependency}"));
        }
    }
    for required in [
        "DD-077 PR 0A",
        "DD-077 Design spike 0B",
        "DD-077 PR 2C",
        "DD-077 PR 2D",
        "DD-077 PR 2E",
        "DD-077 PR 1B",
        "DD-077 PR 1C",
        "DD-047 Slice 1C",
        "DD-047 PR 2",
    ] {
        if !owners.contains(required) {
            return Err(format!("missing external ledger owner {required}"));
        }
    }
    for identity in [
        "f0132afcff984bb43305be39122d7e74a6850396",
        "31a6d82f79e6051a7f00bfb182c979e5e78f2c3f",
        "5a24c0cd1ff2f4d58e77ef263346cf6828cd28d6",
        "41b644195e2aaa81997f76631daa8bae5e5cb53c",
    ] {
        if !ledger.contains(identity) {
            return Err(format!("missing external source identity {identity}"));
        }
    }
    Ok(())
}

fn validate_task_dependencies(plan: &str, graph: &SliceGraph) -> Result<(), String> {
    let allowed = BTreeMap::from([
        ("Task 0", "## Task 0:"),
        (
            "Task 16 DB conversion",
            "## Task 16: Larrimon Waves B–D — HTTP, database/jobs, browser",
        ),
    ]);
    for dependency in graph.dependencies.values().flatten() {
        if !dependency.starts_with("Task ") {
            continue;
        }
        let heading = allowed
            .get(dependency.as_str())
            .ok_or_else(|| format!("unknown task dependency {dependency}"))?;
        if !plan.contains(heading) {
            return Err(format!(
                "task dependency {dependency} has no exact owner {heading}"
            ));
        }
    }
    Ok(())
}

fn assert_release_closed(
    name: &str,
    members: &[&str],
    available: &mut BTreeSet<String>,
    graph: &SliceGraph,
) -> Result<(), String> {
    let group: BTreeSet<_> = members.iter().map(|id| id.to_string()).collect();
    for id in &group {
        if !graph.dependencies.contains_key(id) {
            return Err(format!("release {name} names unknown slice {id}"));
        }
        for dependency in internal_dependencies(&graph.dependencies[id]) {
            if !available.contains(&dependency) && !group.contains(&dependency) {
                return Err(format!(
                    "release {name} omits dependency {dependency} required by {id}"
                ));
            }
        }
    }
    available.extend(group);
    Ok(())
}

fn validate_releases(plan: &str, graph: &SliceGraph) -> Result<(), String> {
    let block = plan
        .split_once("## Release sequencing")
        .ok_or("missing release sequencing")?
        .1;
    let mut rows = BTreeMap::new();
    for line in block.lines().filter(|line| line.starts_with("| ")) {
        if line.starts_with("| Candidate feature boundary") || line.starts_with("|---") {
            continue;
        }
        let columns: Vec<_> = line.trim_matches('|').split('|').map(str::trim).collect();
        if columns.len() != 2 {
            return Err(format!("malformed release row: {line}"));
        }
        if rows.insert(columns[0], columns[1]).is_some() {
            return Err(format!("duplicate release row {}", columns[0]));
        }
    }
    let expected_rows = [
        "v0.6.0 foundation",
        "next feature release",
        "following feature release",
        "browser/project feature release",
        "project-environment feature release",
        "Larrimon pure-project deletion",
        "future monitoring/reliability releases",
    ];
    if rows.keys().copied().collect::<BTreeSet<_>>()
        != expected_rows.into_iter().collect::<BTreeSet<_>>()
    {
        return Err(format!("release row set drifted: {:?}", rows.keys()));
    }
    for (name, required) in [
        ("v0.6.0 foundation", "Slices 1A–1B, 2A–2G, 3A–3E"),
        ("next feature release", "Slice 4, 5A–5B, 6A–6B"),
        (
            "following feature release",
            "Slices 7P, 7A–7F, 8–9, 10P, 10A–10C, and 11A",
        ),
        (
            "browser/project feature release",
            "Slices 12P, 12A–12B, 13A–13E, and 14A–14B",
        ),
        ("project-environment feature release", "Slices 14C–14D"),
        (
            "Larrimon pure-project deletion",
            "Slices 14C–14D and migration compatibility Slice 16M before Task 17",
        ),
        (
            "future monitoring/reliability releases",
            "Slices 18P, 18A–18B, then 19A–19C, then 20P, 20A–20C",
        ),
    ] {
        if !rows[name].contains(required) {
            return Err(format!(
                "release {name} omits canonical membership {required}"
            ));
        }
    }

    let mut available = BTreeSet::new();
    assert_release_closed(
        "foundation",
        &[
            "1A", "1B", "2A", "2B", "2C", "2D", "2E", "2F", "2G", "3A", "3B", "3C", "3D", "3E",
        ],
        &mut available,
        graph,
    )?;
    assert_release_closed(
        "next",
        &["4", "5A", "5B", "6A", "6B"],
        &mut available,
        graph,
    )?;
    assert_release_closed(
        "providers",
        &[
            "7P", "7A", "7B", "7C", "7D", "7E", "7F", "8", "9", "10P", "10A", "10B", "10C", "11A",
        ],
        &mut available,
        graph,
    )?;
    assert_release_closed(
        "browser-project",
        &[
            "12P", "12A", "12B", "13A", "13B", "13C", "13D", "13E", "14A", "14B",
        ],
        &mut available,
        graph,
    )?;
    assert_release_closed(
        "project-environment",
        &["14C", "14D"],
        &mut available,
        graph,
    )?;
    assert_release_closed("larrimon-deletion", &["16M"], &mut available, graph)?;
    assert_release_closed("monitoring", &["18P", "18A", "18B"], &mut available, graph)?;
    assert_release_closed("reliability", &["19A", "19B", "19C"], &mut available, graph)?;
    assert_release_closed("ha", &["20P", "20A", "20B", "20C"], &mut available, graph)?;
    Ok(())
}

fn validate_spikes(plan: &str, graph: &SliceGraph) -> Result<(), String> {
    for (spike, implementation) in [
        ("6A", "6B"),
        ("7P", "7A"),
        ("10P", "10B"),
        ("12P", "12A"),
        ("18P", "18A"),
        ("20P", "20A"),
    ] {
        if !graph.scopes[spike].contains("spike") {
            return Err(format!("{spike} is not table-classified as a spike"));
        }
        if !graph.dependencies[implementation].contains(spike) {
            return Err(format!("{implementation} does not depend on spike {spike}"));
        }
        let marker = format!("## Task {spike}:");
        let body = plan
            .split_once(&marker)
            .ok_or_else(|| format!("missing spike owner {spike}"))?
            .1;
        let end = [body.find("\n## Task "), body.find("\n### Slice ")]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(body.len());
        let section = &body[..end];
        if !section.contains("**Artifact:**")
            || !(section.contains("no public") || section.contains("no production"))
            || section.contains("**Create:** `src/")
        {
            return Err(format!(
                "spike {spike} must remain artifact-only with no public/production API"
            ));
        }
    }
    if !graph.dependencies["20B"].contains("20P") {
        return Err("20B does not depend on spike 20P".to_string());
    }
    Ok(())
}

fn validate_plan(plan: &str) -> Result<(), String> {
    let graph = parse_graph(plan)?;
    for (id, dependencies) in &graph.dependencies {
        for dependency in internal_dependencies(dependencies) {
            if !graph.dependencies.contains_key(&dependency) {
                return Err(format!(
                    "slice {id} has unknown internal dependency {dependency}"
                ));
            }
        }
    }
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for id in graph.dependencies.keys() {
        visit(id, &graph, &mut visiting, &mut visited)?;
    }
    validate_owners(plan, &graph)?;
    validate_created_module_registration(plan, &graph)?;
    validate_external_prerequisites(plan, &graph)?;
    validate_task_dependencies(plan, &graph)?;
    validate_releases(plan, &graph)?;
    validate_spikes(plan, &graph)?;
    if !plan.contains("f0132afcff984bb43305be39122d7e74a6850396") || plan.contains("DD-077 Slice 0")
    {
        return Err("DD-077 immutable identity/owner naming drift".to_string());
    }
    Ok(())
}

#[test]
fn dd078_plan_is_dependency_closed_and_owner_consistent() {
    validate_plan(PLAN).unwrap();
}

#[test]
fn dd078_plan_validator_rejects_representative_drift() {
    let owner_drift = PLAN.replacen(
        "**Table dependencies:** 1A",
        "**Table dependencies:** 2A",
        1,
    );
    assert!(validate_plan(&owner_drift).is_err());

    let unknown_dependency = PLAN.replacen(
        "| 1B | JSON/human report schema and exit parity | 1A |",
        "| 1B | JSON/human report schema and exit parity | ZZ |",
        1,
    );
    assert!(validate_plan(&unknown_dependency).is_err());

    let duplicate_id = PLAN.replacen(
        "| 1A | status algebra, stable IDs, false-pass fixes | Task 0 |",
        "| 1A | status algebra, stable IDs, false-pass fixes | Task 0 |\n| 1A | duplicate | Task 0 |",
        1,
    );
    assert!(validate_plan(&duplicate_id).is_err());

    let production_spike = PLAN.replacen(
        "no public API or production supervisor",
        "production supervisor",
        1,
    );
    assert!(validate_plan(&production_spike).is_err());

    let release_drift = PLAN.replacen(
        "| project-environment feature release | Slices 14C–14D",
        "| project-environment feature release | Slice 14D",
        1,
    );
    assert!(validate_plan(&release_drift).is_err());

    let task_owner_drift = PLAN.replace("Task 16 DB conversion", "Task 999 DB conversion");
    assert!(validate_plan(&task_owner_drift).is_err());

    let external_owner_drift = PLAN
        .replace(
            "| 18B | monitoring protocol, catalog, and inventory acceptance profiles | 18A, 13A, DD-047 Slice 1C, DD-047 PR 2 |",
            "| 18B | monitoring protocol, catalog, and inventory acceptance profiles | 18A, 13A, DD-999 Slice 1C, DD-047 PR 2 |",
        )
        .replace(
            "**Table dependencies:** 13A, 18A, DD-047 Slice 1C, DD-047 PR 2",
            "**Table dependencies:** 13A, 18A, DD-999 Slice 1C, DD-047 PR 2",
        );
    assert!(validate_plan(&external_owner_drift).is_err());

    let larrimon_boundary_drift = PLAN.replace(
        "All relevant slices above plus Slices 14C–14D and migration compatibility Slice 16M before Task 17",
        "All relevant slices above plus Slices 14C–14D before Task 17",
    );
    assert!(validate_plan(&larrimon_boundary_drift).is_err());

    let module_registration_drift = PLAN.replace(
        "**Modify:** `src/verification/mod.rs` registration and report claim-scope/input-identity fields only;",
        "**Modify:** report claim-scope/input-identity fields only;",
    );
    assert!(validate_plan(&module_registration_drift).is_err());

    let parent_creator_dependency_drift = PLAN
        .replace(
            "| 13B | core ntnt AST/import/route/effect/project facts | 2G, 3D, 13A |",
            "| 13B | core ntnt AST/import/route/effect/project facts | 2G, 13A |",
        )
        .replace(
            "**Table dependencies:** 2G, 3D, 13A",
            "**Table dependencies:** 2G, 13A",
        );
    assert!(validate_plan(&parent_creator_dependency_drift).is_err());

    let reversed_range = PLAN.replacen("6B, 8, 10A–10B", "6B, 8, 10B–10A", 1);
    assert!(validate_plan(&reversed_range)
        .unwrap_err()
        .contains("reversed slice range 10B–10A"));

    let nested_slice_after_spike = PLAN.replace(
        "## Task 7A: Frozen out-of-process provider protocol",
        "### Slice 16M: synthetic spike-boundary fixture\n\n**Create:** `src/not-part-of-spike.rs`\n\n## Task 7A: Frozen out-of-process provider protocol",
    );
    let nested_graph = parse_graph(&nested_slice_after_spike).unwrap();
    assert!(validate_spikes(&nested_slice_after_spike, &nested_graph).is_ok());

    let second_create_block = PLAN.replace(
        "**Gate:** focused scanner parity/root-escape tests, existing inspector/Studio tests, fmt/clippy, and immutable review.",
        "**Create:** `src/verification/provider/late.rs`\n\n**Gate:** focused scanner parity/root-escape tests, existing inspector/Studio tests, fmt/clippy, and immutable review.",
    );
    assert!(validate_plan(&second_create_block)
        .unwrap_err()
        .contains("without parent registration src/verification/provider/mod.rs"));
}
