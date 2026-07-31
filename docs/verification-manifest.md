# Verification manifest

DD-078 Slice 2A uses the closest ancestor `ntnt.toml` as the canonical project marker. Project paths are canonical absolute paths; paths exposed by discovery are project-relative and sorted lexicographically. Verification discovery does not execute resources or assign policy.

## Schema version 1

Verification configuration lives in the project manifest:

```toml
[verification]
version = 1

[[verification.files]]
path = "verification/http_contract.tnt"
class = "verification"

[[verification.files]]
path = "server.tnt"
class = "application"
```

`[verification]` accepts exactly `version` and `files`. Each file entry accepts exactly `path` and `class`. Unknown fields and unsupported versions are errors.

The exhaustive file classes are:

- `application`: configured `.tnt` application source;
- `intent`: `.intent` requirements, discovered recursively;
- `verification`: configured `.tnt` verification cases;
- `support`: configured `.tnt` development/support logic;
- `product-assets`: non-ntnt production assets;
- `migrations`: production migration artifacts;
- `project-metadata`: project configuration and documentation.

`.intent` paths are discovered recursively as `intent`; listing the same path explicitly is a duplicate. Executable project `.tnt` files are configured explicitly as exactly one of `application`, `verification`, or `support`, so verification cases are not mistaken for application source. Those four executable/declarative classes require their matching `.tnt` or `.intent` extension. Configured resources must be existing regular files. Discovery combines recursive Intent discovery with the configured exhaustive inventory and returns one globally path-sorted list.

## Root and path rules

- The closest ancestor `ntnt.toml` is the project root for existing config and secret lookup compatibility. A non-regular nearest marker is an error rather than permission to inherit an outer project.
- Inputs that resolve to different roots are rejected as ambiguous. Lexical and canonical roots must agree when a source is reached through a symlink.
- A loaded verification manifest is bound to that canonical root and cannot be reused with another project root.
- A nested `ntnt.toml` encountered during project discovery is ambiguous and is rejected.
- Configured paths are non-empty project-relative paths made only of normal components. Absolute paths, `.` components, and `..` traversal are rejected.
- Configured paths beneath `target`, `build`, or `dist` are rejected as build-output ambiguity. Recursive discovery skips those directories, hidden directories, and `node_modules`.
- Canonical resource targets must stay beneath the canonical project root, and canonical aliases into `target`, `build`, or `dist` remain build-output errors.
- Symlink escapes and multiply linked regular files are rejected. Safe aliases that resolve two paths to one file are duplicate resources and are rejected.
- Duplicate paths are rejected, including automatic/configured duplicates. Assigning one path to different classes is an overlapping-class error.

These checks define inventory only. This slice does not add execution profiles, provider behavior, trust or authority policy, protected contracts, snapshots, resource dependency graphs, or verification verdicts.
