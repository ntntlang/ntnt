# Agent Documentation Index

Documentation specifically designed for AI agents (Claude, Copilot, etc.) working with NTNT code.

## Primary References

| Document | Purpose | Use When |
|----------|---------|----------|
| [CLAUDE.md](../CLAUDE.md) | Claude Code instructions | Working in Claude Code CLI |
| [AI Agent Guide](AI_AGENT_GUIDE.md) | Complete syntax and patterns reference | Need detailed syntax/patterns |
| [Copilot Instructions](../.github/copilot-instructions.md) | GitHub Copilot context | Using Copilot in VS Code |

## Auto-Generated References

| Document | Content |
|----------|---------|
| [STDLIB_REFERENCE.md](STDLIB_REFERENCE.md) | All stdlib functions and builtins |
| [SYNTAX_REFERENCE.md](SYNTAX_REFERENCE.md) | Keywords, operators, types, templates |
| [IAL_REFERENCE.md](IAL_REFERENCE.md) | Intent Assertion Language primitives |

Source files: [stdlib.toml](stdlib.toml), [syntax.toml](syntax.toml), [ial.toml](ial.toml)

Regenerate: `ntnt docs --generate`

## Specialized Agents

| Document | Purpose |
|----------|---------|
| [NTNT Dev Agent](../.github/agents/ntnt-dev.agent.md) | Compiler/runtime development |

## Claude Code Skills

| Skill | Location | Purpose |
|-------|----------|---------|
| IDD Workflow | [.claude/skills/idd.md](../.claude/skills/idd.md) | Intent-Driven Development |

## Key Information for Agents

### Critical Syntax Rules

1. **Map literals require `map` keyword**: `map { "key": "value" }` not `{ "key": "value" }`
2. **String interpolation uses `{expr}`** not `${expr}`
3. **Route patterns require raw strings**: `get(r"/users/{id}", handler)`
4. **Functions not methods**: `len(str)` not `str.len()`
5. **HTTP routes are global builtins**: Don't import `get`, `post`, `listen`

### IDD Workflow

1. Draft `.intent` file → present to user for approval
2. Run `ntnt intent init` to generate scaffolding
3. Implement with `@implements` annotations
4. Verify with `ntnt intent check`

### Always Lint Before Run

```bash
ntnt lint file.tnt    # Check for errors
ntnt run file.tnt     # Only after lint passes
```

## Related Documentation

- [Language Guide](../LANGUAGE_GUIDE.md) - Learning guide with examples
- [Design Documents](../design-docs/) - Planning docs (may not reflect current implementation)
