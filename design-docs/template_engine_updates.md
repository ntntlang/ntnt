# NTNT Template Engine Overhaul Plan

## Overview

This document outlines a comprehensive plan to transform NTNT's template engine from a basic embedded templating system into a world-class, elegant, and powerful templating solution that rivals and improves upon the best template engines (Jinja2, Handlebars, Liquid, Pug, Svelte, Vue templates) while maintaining NTNT's philosophy of simplicity and expressiveness.

## Current State Analysis

### What We Have Now

The current template engine uses triple-quoted strings (`"""..."""`) with these features:

| Feature | Syntax | Status |
|---------|--------|--------|
| Variable interpolation | `{{expr}}` | ✅ Working |
| CSS-safe single braces | `{ }` | ✅ Working |
| For loops | `{{#for x in arr}}...{{/for}}` | ✅ Working |
| If conditionals | `{{#if cond}}...{{/if}}` | ✅ Working |
| If-else | `{{#if cond}}...{{#else}}...{{/if}}` | ✅ Working |
| Escape braces | `\{{` and `\}}` | ✅ Working |

### Current Limitations

1. **No external template file imports** - Templates must be inline
2. **No template inheritance/layouts** - No `extend`, `block`, `include`
3. **No filters/pipes** - Can't do `{{name | uppercase | trim}}`
4. **No partial templates** - No reusable components
5. **No else-if chains** - Must nest if blocks
6. **No loop metadata** - No `index`, `first`, `last`, `length`
7. **No comments** - No way to add template comments
8. **No whitespace control** - Can't trim leading/trailing whitespace
9. **No named slots** - No component composition
10. **No raw/verbatim blocks** - Can't output literal `{{...}}`

### Code Locations

| Component | File | Lines |
|-----------|------|-------|
| TemplatePart AST | `src/ast.rs` | 380-396 |
| Lexer parsing | `src/lexer.rs` | 440-620 |
| Parser conversion | `src/parser.rs` | 1497-1560 |
| Interpreter evaluation | `src/interpreter.rs` | 2623-2710 |

---

## Design Principles

### 1. Elegant Simplicity
- Minimal syntax that feels natural
- Progressive disclosure - simple things are simple, complex things are possible
- No boilerplate required for common operations

### 2. Robust & Predictable
- Clear error messages with line numbers
- No silent failures
- Consistent behavior across all edge cases

### 3. Extensible
- User-defined filters
- Custom components
- Plugin architecture for future expansion

### 4. Clean Separation
- Templates in `.html` files (not embedded in `.tnt`)
- Clear data binding model
- IDE-friendly syntax

---

## Proposed Features

### Phase 1: Core Improvements (Priority: HIGH)

#### 1.1 External Template Loading via `template()` Function

**New stdlib function: `std/template`**

```ntnt
import { template, render } from "std/template"

// Load and render a template file
let html = template("views/home.html", map {
    "title": "Welcome",
    "users": users
})

// Or two-step for reuse
let tmpl = template("views/layout.html")
let html = render(tmpl, data)
```

Template files use the same `{{}}` syntax as inline templates.

#### 1.2 Template Inheritance (Layouts)

**views/base.html:**
```html
<!DOCTYPE html>
<html>
<head>
    <title>{{#block title}}Default Title{{/block}}</title>
    {{#block head}}{{/block}}
</head>
<body>
    {{#block content}}{{/block}}
</body>
</html>
```

**views/home.html:**
```html
{{#extends "base.html"}}

{{#block title}}Home Page{{/block}}

{{#block content}}
<h1>Welcome, {{user.name}}</h1>
{{/block}}
```

#### 1.3 Partials / Includes

```html
{{#include "partials/header.html"}}

<main>
    {{#for product in products}}
        {{#include "partials/product-card.html" with product}}
    {{/for}}
</main>

{{#include "partials/footer.html"}}
```

#### 1.4 Else-If Chains

```html
{{#if status == "active"}}
    <span class="badge green">Active</span>
{{#elif status == "pending"}}
    <span class="badge yellow">Pending</span>
{{#elif status == "archived"}}
    <span class="badge gray">Archived</span>
{{#else}}
    <span class="badge red">Unknown</span>
{{/if}}
```

#### 1.5 Loop Metadata

```html
{{#for item in items}}
    <div class="item {{#if @first}}first{{/if}} {{#if @last}}last{{/if}}">
        <span class="index">{{@index + 1}} of {{@length}}</span>
        <span class="name">{{item.name}}</span>
    </div>
{{#empty}}
    <p>No items found.</p>
{{/for}}
```

Loop variables:
- `@index` - Zero-based index
- `@index1` - One-based index  
- `@first` - Boolean, true if first iteration
- `@last` - Boolean, true if last iteration
- `@length` - Total number of items
- `@even` / `@odd` - Boolean for even/odd indices

### Phase 2: Filters & Transformations (Priority: HIGH)

#### 2.1 Filter/Pipe Syntax

```html
{{name | uppercase}}
{{price | currency("USD")}}
{{description | truncate(100) | escape}}
{{items | length}}
{{date | format("YYYY-MM-DD")}}
```

#### 2.2 Built-in Filters

| Filter | Example | Description |
|--------|---------|-------------|
| `uppercase` | `{{s \| uppercase}}` | Convert to uppercase |
| `lowercase` | `{{s \| lowercase}}` | Convert to lowercase |
| `capitalize` | `{{s \| capitalize}}` | Capitalize first letter |
| `trim` | `{{s \| trim}}` | Remove leading/trailing whitespace |
| `escape` | `{{s \| escape}}` | HTML escape |
| `raw` | `{{s \| raw}}` | Output without escaping |
| `truncate(n)` | `{{s \| truncate(50)}}` | Truncate with ellipsis |
| `default(v)` | `{{s \| default("N/A")}}` | Fallback value |
| `length` | `{{arr \| length}}` | Array/string length |
| `first` | `{{arr \| first}}` | First element |
| `last` | `{{arr \| last}}` | Last element |
| `reverse` | `{{arr \| reverse}}` | Reverse array/string |
| `sort` | `{{arr \| sort}}` | Sort array |
| `sort_by(key)` | `{{arr \| sort_by("name")}}` | Sort by key |
| `join(sep)` | `{{arr \| join(", ")}}` | Join array |
| `split(sep)` | `{{s \| split(",")}}` | Split string |
| `json` | `{{obj \| json}}` | JSON stringify |
| `format(fmt)` | `{{date \| format("MM/DD")}}` | Format date/number |
| `number(dec)` | `{{n \| number(2)}}` | Format with decimals |
| `currency(c)` | `{{n \| currency("USD")}}` | Format as currency |
| `pluralize(s,p)` | `{{n \| pluralize("item","items")}}` | Singular/plural |
| `url_encode` | `{{s \| url_encode}}` | URL encode |
| `base64` | `{{s \| base64}}` | Base64 encode |
| `md5` | `{{s \| md5}}` | MD5 hash |
| `replace(a,b)` | `{{s \| replace("a","b")}}` | Replace substring |
| `slice(s,e)` | `{{arr \| slice(0,5)}}` | Slice array/string |

#### 2.3 Custom Filter Registration

```ntnt
import { register_filter, template } from "std/template"

// Register a custom filter
register_filter("gravatar", fn(email) {
    let hash = md5(lowercase(trim(email)))
    return "https://gravatar.com/avatar/" + hash
})

// Use in templates
// {{user.email | gravatar}}
```

### Phase 3: Components & Slots (Priority: MEDIUM)

#### 3.1 Component Definition

**components/card.html:**
```html
{{#component card}}
<div class="card {{@class}}">
    <div class="card-header">
        {{#slot header}}Default Header{{/slot}}
    </div>
    <div class="card-body">
        {{#slot}}{{/slot}}  <!-- Default slot -->
    </div>
    {{#if @footer}}
    <div class="card-footer">
        {{#slot footer}}{{/slot}}
    </div>
    {{/if}}
</div>
{{/component}}
```

#### 3.2 Component Usage

```html
{{#card class="featured" footer=true}}
    {{#fill header}}
        <h2>{{product.name}}</h2>
    {{/fill}}
    
    <p>{{product.description}}</p>
    
    {{#fill footer}}
        <button>Buy Now - ${{product.price}}</button>
    {{/fill}}
{{/card}}
```

#### 3.3 Component Library in NTNT

```ntnt
import { component, template } from "std/template"

// Define component programmatically
component("avatar", fn(props, slots) {
    let size = props["size"] ?? 40
    let src = props["src"] ?? "/default-avatar.png"
    return """
    <img class="avatar" 
         src="{{src}}" 
         width="{{size}}" 
         height="{{size}}"
         alt="{{props["alt"] ?? "Avatar"}}">
    """
})
```

### Phase 4: Advanced Control Flow (Priority: MEDIUM)

#### 4.1 Match/Switch

```html
{{#match user.role}}
    {{#when "admin"}}
        <span class="badge admin">Administrator</span>
    {{/when}}
    {{#when "mod"}}
        <span class="badge mod">Moderator</span>
    {{/when}}
    {{#default}}
        <span class="badge user">User</span>
    {{/default}}
{{/match}}
```

#### 4.2 With Block (Scoping)

```html
{{#with user.profile as profile}}
    <div class="profile">
        <h2>{{profile.name}}</h2>
        <p>{{profile.bio}}</p>
    </div>
{{/with}}
```

#### 4.3 Let Block (Local Variables)

```html
{{#let fullName = user.first + " " + user.last}}
    <h1>Welcome, {{fullName}}</h1>
    <meta name="author" content="{{fullName}}">
{{/let}}
```

### Phase 5: Whitespace & Comments (Priority: MEDIUM)

#### 5.1 Whitespace Control

```html
<!-- Trim left whitespace -->
{{- name}}

<!-- Trim right whitespace -->
{{name -}}

<!-- Trim both -->
{{- name -}}

<!-- Applied to blocks -->
{{#for item in items -}}
    {{item.name}}
{{- /for}}
```

#### 5.2 Comments

```html
{{! This is a comment - not rendered }}

{{!--
    This is a 
    multi-line comment
--}}
```

#### 5.3 Raw/Verbatim Blocks

```html
{{#raw}}
    This {{will not}} be interpolated.
    Useful for showing template syntax in docs.
{{/raw}}
```

### Phase 6: Auto-Escaping & Security (Priority: HIGH)

#### 6.1 Context-Aware Auto-Escaping

```html
<!-- HTML context - auto HTML-escaped -->
<p>{{user.bio}}</p>

<!-- Attribute context - auto attribute-escaped -->
<div title="{{user.name}}">

<!-- URL context - auto URL-encoded -->
<a href="/users/{{user.id | url_encode}}">

<!-- JavaScript context - auto JSON-encoded -->
<script>
    const user = {{user | json}};
</script>

<!-- CSS context - auto CSS-escaped -->
<style>
    .user-{{user.id}} { color: {{user.color | css}}; }
</style>
```

#### 6.2 Explicit Escaping Control

```html
<!-- Force raw output (trusted content only!) -->
{{user.bio | raw}}

<!-- Force HTML escape (even in safe contexts) -->
{{content | escape}}
```

### Phase 7: Performance & Caching (Priority: LOW)

#### 7.1 Template Compilation Cache

```ntnt
import { template, enable_cache, clear_cache } from "std/template"

// Enable caching (on by default in production)
enable_cache(true)

// Templates are compiled once, reused on subsequent calls
let html1 = template("views/home.html", data1)
let html2 = template("views/home.html", data2)  // Uses cached AST

// Clear cache if needed (e.g., in development)
clear_cache()
```

#### 7.2 Pre-compilation

```ntnt
import { compile, render } from "std/template"

// Pre-compile for maximum performance
let compiled = compile("views/home.html")

// Render multiple times with different data
for user in users {
    let html = render(compiled, map { "user": user })
    // ...
}
```

---

## Implementation Plan

The implementation is organized into batches that deliver value incrementally, starting with enhancements to the existing template system before adding new modules.

### Batch 1: Quick Wins on Existing System ✅ COMPLETED

**Goal:** Enhance the existing inline template system with high-value features. No new modules required.

**Tasks:**
1. [x] Add `{{#elif}}` support to lexer and parser
2. [x] Add loop metadata variables (`@index`, `@first`, `@last`, `@length`, `@even`, `@odd`)
3. [x] Add `{{#empty}}` block for empty loops
4. [x] Add comments `{{! comment }}`
5. [x] Write tests for all new features

**Files modified:**
- `src/lexer.rs` - Added `Elif`, `Empty`, loop metadata tokens, comment handling
- `src/ast.rs` - Added new TemplatePart variants
- `src/parser.rs` - Parse new directives
- `src/interpreter.rs` - Evaluate new constructs

**Completed:** January 2026

---

### Batch 2: Filters ✅ COMPLETED

**Goal:** Add the powerful filter/pipe syntax that makes templates expressive.

**Tasks:**
1. [x] Implement filter/pipe parsing in lexer (`{{name | uppercase | trim}}`)
2. [x] Add filter AST representation
3. [x] Implement core built-in filters:
   - String: `uppercase`, `lowercase`, `capitalize`, `trim`, `truncate(n)`, `replace(a,b)`
   - Safety: `escape`, `raw`, `default(v)`
   - Collections: `length`, `first`, `last`, `reverse`, `join(sep)`, `slice(s,e)`
   - Formatting: `json`, `number(dec)`, `url_encode`
4. [x] Write filter tests

**Files modified:**
- `src/lexer.rs` - Parse pipe syntax within `{{}}`
- `src/ast.rs` - TemplateFilter AST node
- `src/interpreter.rs` - Filter evaluation logic (`apply_template_filter`)

**Completed:** January 2026

---

### Batch 3: External Templates ✅ COMPLETED

**Goal:** Enable templates to live in separate `.html` files.

**Tasks:**
1. [x] Create `src/stdlib/template.rs` module
2. [x] Implement `template(path, data)` function
3. [x] Implement `compile(path)` for pre-compilation
4. [x] Implement `render(compiled, data)` for reuse
5. [x] Add path resolution relative to calling `.tnt` file
6. [x] Register module in `src/stdlib/mod.rs`
7. [x] Write tests

**Files created/modified:**
- NEW: `src/stdlib/template.rs` - Template loading and rendering
- `src/stdlib/mod.rs` - Register template module
- `src/interpreter.rs` - Special handling for template(), compile(), render() functions

**API:**
```ntnt
import { template, compile, render } from "std/template"

// One-step: load and render
let html = template("views/home.html", map { "title": "Welcome" })

// Two-step: compile once, render many times
let tmpl = compile("views/home.html")
let html1 = render(tmpl, data1)
let html2 = render(tmpl, data2)
```

**Completed:** January 2026

---

### Batch 4: Inheritance & Includes (2-3 days)

**Goal:** Enable template composition with layouts and partials.

**Tasks:**
1. [ ] Implement `{{#extends "base.html"}}` directive
2. [ ] Implement `{{#block name}}...{{/block}}` directive
3. [ ] Implement `{{#include "partial.html"}}` directive
4. [ ] Implement `{{#include "partial.html" with data}}` variant
5. [ ] Write tests

**Files to modify:**
- `src/lexer.rs` - Add extends, block, include tokens
- `src/ast.rs` - Add TemplatePart variants
- `src/parser.rs` - Parse new directives
- `src/stdlib/template.rs` - Inheritance resolution logic

---

### Batch 5: Advanced Control Flow (Future)

**Goal:** Additional control flow constructs for complex templates.

**Tasks:**
1. [ ] Implement `{{#match}}` / `{{#when}}` / `{{#default}}`
2. [ ] Implement `{{#with expr as var}}` scoping
3. [ ] Implement `{{#let var = expr}}` local variables
4. [ ] Implement `{{#raw}}...{{/raw}}` verbatim blocks

**Priority:** Medium - nice to have but not essential for most use cases.

---

### Batch 6: Whitespace Control (Future)

**Goal:** Fine-grained control over whitespace in output.

**Tasks:**
1. [ ] Implement `{{- expr}}` (trim left)
2. [ ] Implement `{{expr -}}` (trim right)
3. [ ] Implement `{{- expr -}}` (trim both)
4. [ ] Apply to block directives: `{{#if cond -}}`, `{{- /if}}`

**Priority:** Medium - useful for minified output.

---

### Batch 7: Components & Slots (Future)

**Goal:** Reusable UI components with named slots.

**Tasks:**
1. [ ] Implement `{{#component name}}` definition
2. [ ] Implement `{{#slot name}}` and default slot
3. [ ] Implement component usage with `{{#fill slot}}`
4. [ ] Add component registry
5. [ ] Add `component()` function for programmatic definition

**Priority:** Low - powerful but adds complexity. Defer until there's clear demand.

---

### Batch 8: Security & Caching (Future)

**Goal:** Production-ready security and performance.

**Tasks:**
1. [ ] Implement context-aware auto-escaping (HTML, URL, JS, CSS contexts)
2. [ ] Add template compilation cache
3. [ ] Add `enable_cache()` / `clear_cache()` API
4. [ ] Performance optimization
5. [ ] Add `register_filter()` for custom filters

**Priority:** Low - optimization phase after core features are stable.

---

## Documentation Updates Required

### Files to Update

| File | Updates Needed |
|------|----------------|
| `LANGUAGE_SPEC.md` | New Template Strings section with all features |
| `docs/AI_AGENT_GUIDE.md` | Template syntax rules, examples, common patterns |
| `.github/copilot-instructions.md` | Template syntax for Copilot |
| `CLAUDE.md` | Template best practices |
| `README.md` | Feature highlights, quick examples |
| `ROADMAP.md` | Mark template features as completed |

### New Documentation to Create

| File | Content |
|------|---------|
| `docs/TEMPLATE_GUIDE.md` | Comprehensive template documentation |
| `docs/TEMPLATE_FILTERS.md` | Complete filter reference |
| `docs/TEMPLATE_COMPONENTS.md` | Component authoring guide |

---

## Example Updates Required

### Existing Examples to Update

| Example | Changes |
|---------|---------|
| `examples/template_string_test.tnt` | Add all new features |
| `examples/website.tnt` | Migrate to external templates |
| `examples/website/` | Add template files |
| `examples/crypto_chart/crypto.tnt` | Demonstrate filters |
| `examples/ntnt-lang-org/server.tnt` | Full component usage |

### New Examples to Create

| Example | Purpose |
|---------|---------|
| `examples/templates/` | Template-focused demos |
| `examples/templates/basic.tnt` | Basic template loading |
| `examples/templates/layouts.tnt` | Template inheritance |
| `examples/templates/components.tnt` | Component system |
| `examples/templates/filters.tnt` | Filter showcase |
| `examples/templates/views/` | Example .html templates |

---

## API Reference (Final Design)

### Module: `std/template`

```ntnt
import { 
    template,           // Load and render template file
    render,             // Render pre-compiled template
    compile,            // Pre-compile template
    register_filter,    // Add custom filter
    component,          // Define component programmatically
    enable_cache,       // Toggle template caching
    clear_cache         // Clear template cache
} from "std/template"
```

### Function Signatures

```ntnt
// Load template file and render with data
fn template(path: String, data: Map) -> String

// Render pre-compiled template
fn render(compiled: Template, data: Map) -> String

// Pre-compile template for reuse
fn compile(path: String) -> Template

// Register custom filter
fn register_filter(name: String, func: fn(value: Any, ...args) -> Any)

// Define component
fn component(name: String, renderer: fn(props: Map, slots: Map) -> String)

// Cache control
fn enable_cache(enabled: Bool)
fn clear_cache()
```

---

## Template Syntax Quick Reference

```html
{{! Variables }}
{{name}}
{{user.profile.name}}
{{items[0]}}

{{! Filters }}
{{name | uppercase | trim}}
{{price | currency("USD")}}

{{! Control Flow }}
{{#if condition}}...{{/if}}
{{#if condition}}...{{#elif other}}...{{#else}}...{{/if}}
{{#for item in items}}...{{#empty}}No items{{/for}}
{{#match value}}{{#when "a"}}...{{/when}}{{#default}}...{{/default}}{{/match}}

{{! Scoping }}
{{#with expr as var}}...{{/with}}
{{#let var = expr}}...{{/let}}

{{! Loop Metadata }}
{{@index}}  {{@index1}}  {{@first}}  {{@last}}  {{@length}}  {{@even}}  {{@odd}}

{{! Templates }}
{{#extends "base.html"}}
{{#block name}}...{{/block}}
{{#include "partial.html"}}
{{#include "partial.html" with data}}

{{! Components }}
{{#component name}}...{{#slot name}}...{{/slot}}...{{/component}}
{{#componentName prop=value}}{{#fill slotName}}...{{/fill}}{{/componentName}}

{{! Whitespace Control }}
{{- expr}}  {{expr -}}  {{- expr -}}

{{! Comments }}
{{! single line comment }}
{{!-- multi-line comment --}}

{{! Raw Output }}
{{#raw}}...{{/raw}}
{{expr | raw}}
```

---

## Success Criteria

1. **Simplicity**: Basic templating should feel natural and require no documentation
2. **Power**: Complex layouts and components are possible without escaping to code
3. **Performance**: Templates compile once, render fast
4. **Security**: Auto-escaping prevents XSS by default
5. **Developer Experience**: Clear errors, IDE-friendly syntax
6. **Compatibility**: Existing `"""..."""` templates continue to work

---

## Migration Path

Existing code using inline `"""..."""` templates will continue to work unchanged. The new features are additive:

1. **Inline templates** - Still work, now with more features (filters, elif, loop metadata)
2. **External templates** - New `template()` function for file-based templates
3. **Components** - New optional feature for reusable UI pieces

No breaking changes to existing code.

---

## Timeline Summary

### Core Implementation (Recommended)

| Batch | Duration | Focus | Priority |
|-------|----------|-------|----------|
| Batch 1 | 1-2 days | elif, loop metadata, empty blocks, comments | **HIGH** |
| Batch 2 | 2-3 days | Filters and pipes | **HIGH** |
| Batch 3 | 3-4 days | External template loading | **HIGH** |
| Batch 4 | 2-3 days | Inheritance (extends, block, include) | **HIGH** |

**Core features: ~2 weeks**

### Future Enhancements (Deferred)

| Batch | Duration | Focus | Priority |
|-------|----------|-------|----------|
| Batch 5 | 2-3 days | match/when, with, let, raw blocks | Medium |
| Batch 6 | 1-2 days | Whitespace control | Medium |
| Batch 7 | 3-4 days | Components and slots | Low |
| Batch 8 | 2-3 days | Auto-escaping, caching | Low |

**Full feature set: ~4 weeks total**

### Documentation & Examples

| Task | Duration | When |
|------|----------|------|
| Update docs for Batch 1-2 | 1 day | After Batch 2 |
| Update docs for Batch 3-4 | 1 day | After Batch 4 |
| New template examples | 1 day | After Batch 4 |

---

## References & Inspiration

### Template Engines Studied

| Engine | Language | Key Ideas Borrowed |
|--------|----------|-------------------|
| Jinja2 | Python | Filters, inheritance, macros |
| Liquid | Ruby | Simple syntax, filters |
| Handlebars | JavaScript | Helpers, partials, `{{#each}}` |
| Mustache | Multi | Logic-less simplicity |
| Pug | JavaScript | Clean syntax, mixins |
| EJS | JavaScript | Embedded JS |
| Svelte | JavaScript | Components, slots |
| Vue | JavaScript | Directives, slots |
| Blade | PHP | Directives, components |
| Twig | PHP | Sandbox, inheritance |

### Design Decisions

1. **`{{}}` not `{% %}`** - Single delimiter style is cleaner
2. **`#directive` not `:directive`** - More readable in HTML context  
3. **Pipe `|` for filters** - Universal convention
4. **`@` for loop variables** - Clear scoping, avoids conflicts
5. **`{{/directive}}`** - Explicit closing is more readable than `{{end}}`

---

## Appendix: Syntax Comparison

| Feature | NTNT | Jinja2 | Handlebars | Liquid |
|---------|------|--------|------------|--------|
| Variable | `{{x}}` | `{{x}}` | `{{x}}` | `{{x}}` |
| Filter | `{{x \| f}}` | `{{x \| f}}` | N/A | `{{x \| f}}` |
| If | `{{#if x}}` | `{% if x %}` | `{{#if x}}` | `{% if x %}` |
| For | `{{#for x in y}}` | `{% for x in y %}` | `{{#each y}}` | `{% for x in y %}` |
| Include | `{{#include "f"}}` | `{% include "f" %}` | `{{> f}}` | `{% include "f" %}` |
| Extend | `{{#extends "f"}}` | `{% extends "f" %}` | N/A | N/A |
| Block | `{{#block x}}` | `{% block x %}` | N/A | N/A |
| Comment | `{{! x }}` | `{# x #}` | `{{! x }}` | `{% comment %}` |
| Raw | `{{#raw}}` | `{% raw %}` | `{{{{raw}}}}` | `{% raw %}` |

NTNT's syntax is intentionally closest to Handlebars for HTML templates, with Jinja2's power features.
