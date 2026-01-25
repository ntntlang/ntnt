# NTNT Examples

Code examples demonstrating NTNT language features.

## Full Applications

| Example | Description |
|---------|-------------|
| [ntnt-lang-org/](ntnt-lang-org/) | Production website with file-based routing, templates, middleware |
| [website/](website/) | Simple website with hot-reloadable templates |
| [crypto_chart/](crypto_chart/) | Chart application with intent specification |
| [snowgauge/](snowgauge/) | Snow conditions dashboard |

## Intent-Driven Development

| Example | Description |
|---------|-------------|
| [intent_demo/](intent_demo/) | Basic IDD workflow demonstration |
| [ial_demo/](ial_demo/) | Intent Assertion Language with natural language scenarios |

## Quick Start

| File | Description |
|------|-------------|
| [hello.tnt](hello.tnt) | Hello World |
| [fibonacci.tnt](fibonacci.tnt) | Basic recursion |

## Language Features

| File | Description |
|------|-------------|
| [contracts.tnt](contracts.tnt) | Design by contract basics |
| [contracts_full.tnt](contracts_full.tnt) | Complete contracts example |
| [invariants.tnt](invariants.tnt) | Struct invariants |
| [pattern_matching.tnt](pattern_matching.tnt) | Match expressions and enums |
| [concurrent_demo.tnt](concurrent_demo.tnt) | Channels and concurrency |
| [modules.tnt](modules.tnt) | Module imports |

## Web Development

| File | Description |
|------|-------------|
| [http_server.tnt](http_server.tnt) | REST API with routing, middleware, contracts |
| [http_client.tnt](http_client.tnt) | HTTP client using fetch() |
| [postgres_demo.tnt](postgres_demo.tnt) | Database integration |
| [redirect_demo.tnt](redirect_demo.tnt) | HTTP redirects |

## Standard Library

| File | Description |
|------|-------------|
| [string_demo.tnt](string_demo.tnt) | String functions |
| [string_processing.tnt](string_processing.tnt) | String manipulation |
| [math_demo.tnt](math_demo.tnt) | Math functions |
| [math_science.tnt](math_science.tnt) | Scientific calculations |
| [time_demo.tnt](time_demo.tnt) | Date/time handling |
| [csv_test.tnt](csv_test.tnt) | CSV parsing |
| [data_processing.tnt](data_processing.tnt) | Data transformation |
| [environment.tnt](environment.tnt) | Environment variables |

## Running Examples

```bash
# Lint first (always!)
ntnt lint examples/hello.tnt

# Run the example
ntnt run examples/hello.tnt

# For web servers
ntnt run examples/http_server.tnt
# Then visit http://localhost:8080

# For IDD examples
ntnt intent check examples/intent_demo/server.tnt
```

## See Also

- [Documentation Index](../docs/INDEX.md)
- [Language Specification](../LANGUAGE_SPEC.md)
