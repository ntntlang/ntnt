# docs/learn/ — Agent Configuration Source Files

These files are embedded into the `ntnt` binary at compile time via `include_str!`
and used by the `ntnt learn` command to generate platform-specific AI agent
configuration files.

## Files

- **critical-rules.md** — Hand-curated critical syntax rules (~80 lines). These are
  the rules that agents get wrong most often. Embedded as `CRITICAL_RULES` in
  `src/main.rs`.

## How it works

1. `ntnt learn <platform>` generates config files for a specific AI coding agent
2. The critical rules are included in all generated files
3. For Claude Code, the full `AI_AGENT_GUIDE.md` is also included as a rules file
4. Generated files include a version header for `ntnt learn --check` to detect staleness

## Updating

Edit `critical-rules.md` directly — it's curated content, not auto-generated.
After editing, rebuild (`cargo build --profile dev-release`) to embed the changes.

The full guide (`AI_AGENT_GUIDE.md`) is also embedded and used for platforms that
support larger config files (Claude Code `.claude/rules/`).

## Validation

`ntnt docs --validate` checks that `docs/learn/critical-rules.md` exists.
