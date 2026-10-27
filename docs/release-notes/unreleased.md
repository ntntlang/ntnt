# Unreleased

## Relative imports keep their declaring-file origin (#296)

Explicit `./` and `../` imports now resolve relative to the file containing the import, including inside ordinary functions, returned callbacks, job bodies and concurrent tasks. They no longer change meaning based on the caller or worker CWD. Bare file paths, absolute imports and standard-library imports are unchanged, as are lazy loading and ordinary template/filesystem resource paths.

This is a semantic correction: code intentionally relying on the old caller/CWD behavior needs a downstream path migration before upgrading. For a deliberate CWD lookup, `"./lib/helper.tnt"` can become `"lib/helper.tnt"`; caller-relative imports and `../` paths require an intentional path choice. Existing absolute-path workarounds remain valid. There is no compatibility mode or fallback directory search.

Rust embedders that construct `Statement::Import` directly must supply the new `source_file` field (`None` retains the interpreter-base/CWD fallback). `Parser::new` remains source-less; file-backed embedders can use `Parser::with_source_file`. AST JSON remains unchanged because runtime origins are not serialized.
