# Language support 0.1.0

All providers use real Tree-sitter grammars. Files are read as bounded UTF-8 text without executing repository code or installing dependencies. Syntax errors yield partial facts/diagnostics instead of parser crashes. AST depth and parse deadlines are bounded.

| Family | Extensions | Verified fixtures |
|---|---|---|
| Rust | rs | Functions, structs/enums/traits/modules, use aliases, local and module-qualified calls, impl qualified names, syntax errors |
| Python | py, pyi | Functions/classes, imports/aliases/relative modules, nested lexical owners, local binding/import shadowing, calls, duplicate ambiguity |
| TypeScript/JavaScript | ts, tsx, js, jsx, mjs, cjs | Functions/classes/interfaces/type aliases, arrow-function declarations, relative named/namespace/default ESM imports, export visibility and parameter shadowing |
| Go | go | Functions/type specs/method declarations, packages/import aliases, go.mod imports, same-directory functions, direct calls |

A .cjs extension selects the JavaScript grammar, not a promise of CommonJS resolution. Implemented relationships include declarations, imports, direct calls, identifier/property references and named type uses. Common locals/parameters, named fields/properties and Go struct/interface kinds are extracted with conservative lexical/type-value resolution. Exhaustive declaration/reference coverage, type propagation, inheritance/traits/interfaces/receivers, CommonJS, tsconfig aliases, Rust cfg/macros/glob/reexports, Python star/dynamic imports and richer package layouts are incomplete. Unsupported calls remain unresolved; dynamic receiver dispatch is not guessed from name matches. Inline module and framework semantics require additional fixtures. Test recognition currently uses test_ naming and Rust test attributes, not framework-complete discovery.

Go dependency resolution uses only locally indexed files under the declared go.mod module; it never fetches absent dependencies. ESM resolution is limited to confined relative paths. Rust external crates are not invented as local dependencies. Java/C#/C/C++/Ruby/PHP and embedding retrieval are unimplemented P1 scope.

Small original fixture assertions pass; broad labeled precision/recall, relevance ranking and real-world language coverage gates are not yet complete. Consult TEST_RESULTS.md rather than interpreting this table as compiler-grade support.
