//! Phase 48 — Multi-File Composition (`include`).
//!
//! Lowers `include "<path>";` top-level items by lexically merging the items of the
//! referenced `.ms` file into the program, *before* semantic analysis — the same
//! "lower before analysis" discipline [`crate::matrix`] and [`crate::templates`] follow.
//! After this pass no [`Item::Include`] remains: each include is replaced in place by the
//! (recursively expanded) items of the file it names, so the dependency graph, change
//! detection, and the parallel scheduler only ever see one ordinary, flat [`Program`] and
//! need no include awareness of their own.
//!
//! **Composition is lexical inclusion, not a package manager.** There is no cross-file
//! runtime and no namespacing: all included items share one *flat namespace*. Two files
//! that define the same `stage` / `let` / `template` / `pipeline` / `project` name
//! therefore collide — and that collision is reported by the existing duplicate-name
//! checks in [`crate::sema`], because every node keeps the span (and so the originating
//! file) it was parsed with. This module deliberately does **not** re-implement those
//! checks; it only flattens the item list.
//!
//! Three things are validated here, each with a source span pointing at the `include`:
//! an included file must be readable and parseable; the include graph must be acyclic
//! (a file may not transitively include itself); and a file pulled in more than once is
//! merged only once (de-duplicated — a repeated include is a no-op, not an error).
//!
//! **Path resolution.** An `include` path is resolved relative to the directory of the
//! file that contains it (so an include in `components/a.ms` of `"shared.ms"` reads
//! `components/shared.ms`). Because each included item keeps its own file on its spans, a
//! `glob` written in an included file resolves against *that* file's directory rather than
//! the root script's — see [`crate::eval`]'s glob handling.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use crate::{
    ast::*,
    error::{Diagnostic, Error, Result},
    parser::parse,
    source::Source,
};

/// Flatten every `include` in `program` by merging the referenced files' items in place,
/// resolved relative to the directory of the including file. Returns the merged
/// [`Program`] (with no [`Item::Include`] remaining), or [`Error::Semantic`] carrying
/// every diagnostic (unreadable/unparseable file, include cycle) collected while merging.
/// A program with no `include` items is returned structurally unchanged.
pub fn expand(program: &Program) -> Result<Program> {
    // Fast path: nothing to merge. Includes are a top-level item, so a shallow scan of the
    // root's items is enough to skip the work (and the clone of every sub-tree) entirely.
    if !program.items.iter().any(|i| matches!(i, Item::Include(_))) {
        return Ok(program.clone());
    }

    let root_file = program.span.file.clone();
    let mut errors: Vec<Diagnostic> = Vec::new();
    // Files already merged, keyed by canonical identity, so a second include of the same
    // file is de-duplicated rather than duplicated.
    let mut merged: HashSet<PathBuf> = HashSet::new();
    // The chain of files currently being merged, for cycle detection. The root is on the
    // stack throughout, so an include that transitively names the root is caught too.
    let mut stack: Vec<PathBuf> = vec![identity(&root_file)];

    let items = merge(program, &root_file, &mut stack, &mut merged, &mut errors);

    if errors.is_empty() {
        Ok(Program { items, span: program.span.clone() })
    } else {
        Err(Error::Semantic(errors))
    }
}

/// Produce the merged item list of `program` (located at `file`), recursively splicing in
/// the items of any file it includes. `stack` holds the active include chain (for cycle
/// detection), `merged` the set of files already pulled in (for de-duplication), and
/// `errors` accumulates every read/parse/cycle diagnostic.
fn merge(
    program: &Program,
    file: &Path,
    stack: &mut Vec<PathBuf>,
    merged: &mut HashSet<PathBuf>,
    errors: &mut Vec<Diagnostic>,
) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::with_capacity(program.items.len());
    for item in &program.items {
        let Item::Include(inc) = item else {
            out.push(item.clone());
            continue;
        };

        let resolved = resolve_include(file, &inc.path);
        let id = identity(&resolved);

        // A cycle: the target is somewhere on the active include chain (including the root).
        if stack.contains(&id) {
            errors.push(
                Diagnostic::new(format!(
                    "include cycle detected: '{}' eventually includes itself",
                    inc.path
                ))
                .with_span(inc.span.clone()),
            );
            continue;
        }
        // Already merged via an earlier include: de-duplicate silently.
        if !merged.insert(id.clone()) {
            continue;
        }

        let source = match Source::from_file(&resolved) {
            Ok(s) => s,
            Err(_) => {
                errors.push(
                    Diagnostic::new(format!("cannot read included file '{}'", resolved.display()))
                        .with_span(inc.span.clone()),
                );
                continue;
            }
        };
        let sub = match parse(&source) {
            Ok(p) => p,
            // Surface the included file's own parse diagnostics (they carry that file's
            // spans) so the error points at the real problem, not just the `include` site.
            Err(Error::Parse(diags)) | Err(Error::Semantic(diags)) | Err(Error::Eval(diags)) => {
                errors.extend(diags);
                continue;
            }
            Err(e) => {
                errors.push(Diagnostic::new(e.to_string()).with_span(inc.span.clone()));
                continue;
            }
        };

        stack.push(id);
        let sub_items = merge(&sub, &resolved, stack, merged, errors);
        stack.pop();
        out.extend(sub_items);
    }
    out
}

/// Resolve an `include` path written in `including_file` to the file it names: relative to
/// the including file's directory, then lexically normalized (so spans and downstream globs
/// see a clean path).
fn resolve_include(including_file: &Path, rel: &str) -> PathBuf {
    let base = including_file.parent().unwrap_or_else(|| Path::new(""));
    normalize(&base.join(rel))
}

/// A stable identity for a path, used for cycle detection and de-duplication. Prefers the
/// canonical (symlink-resolved, absolute) form; falls back to a lexically normalized path
/// when the file does not exist (a subsequent read then reports the missing file).
fn identity(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| normalize(path))
}

/// Lexically normalize a path: drop `.` components and collapse `..` against a preceding
/// normal component, without touching the filesystem. Leading `..` segments are kept. An
/// empty result becomes `.`.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                // Collapse `foo/..` but keep a leading `..` (nothing to pop, or the tail is
                // itself a `..`).
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                _ => out.push(".."),
            },
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

// ── Tests ────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A unique temp directory keyed by nanos + a counter, matching the repo's test idiom.
    fn temp_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("mainstage-include-{tag}-{nanos}-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Parse a file from disk and run the include expansion over it.
    fn expand_file(path: &Path) -> Result<Program> {
        let source = Source::from_file(path).expect("root file should exist");
        let program = parse(&source).expect("root should parse");
        expand(&program)
    }

    /// The names of every stage in a program, in order.
    fn stage_names(program: &Program) -> Vec<String> {
        program
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Stage(s) => Some(s.name.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn no_includes_is_unchanged() {
        let src = "stage build {\n  steps {\n    $ make\n  }\n}\n";
        let program = parse(&Source::from_str("test.ms", src)).unwrap();
        let merged = expand(&program).unwrap();
        assert_eq!(stage_names(&merged), vec!["build"]);
        assert!(!merged.items.iter().any(|i| matches!(i, Item::Include(_))));
    }

    #[test]
    fn include_merges_items_in_place() {
        let dir = temp_dir("merge");
        std::fs::write(dir.join("build.ms"), "stage compile {\n  steps {\n    $ cc\n  }\n}\n")
            .unwrap();
        std::fs::write(
            dir.join("main.ms"),
            "include \"build.ms\";\nstage test {\n  steps {\n    $ run\n  }\n}\n",
        )
        .unwrap();

        let merged = expand_file(&dir.join("main.ms")).unwrap();
        // The included `compile` lands where the `include` was — before `test`.
        assert_eq!(stage_names(&merged), vec!["compile", "test"]);
        assert!(!merged.items.iter().any(|i| matches!(i, Item::Include(_))));
    }

    #[test]
    fn include_resolves_relative_to_including_file() {
        let dir = temp_dir("relative");
        std::fs::create_dir_all(dir.join("components")).unwrap();
        // a.ms (in components/) includes a sibling by a bare name.
        std::fs::write(
            dir.join("components/a.ms"),
            "include \"shared.ms\";\nstage a {\n  steps {\n    $ a\n  }\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("components/shared.ms"),
            "stage shared {\n  steps {\n    $ s\n  }\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("main.ms"), "include \"components/a.ms\";\n").unwrap();

        let merged = expand_file(&dir.join("main.ms")).unwrap();
        assert_eq!(stage_names(&merged), vec!["shared", "a"]);
    }

    #[test]
    fn duplicate_include_is_merged_once() {
        let dir = temp_dir("dedup");
        std::fs::write(dir.join("lib.ms"), "stage lib {\n  steps {\n    $ l\n  }\n}\n").unwrap();
        // Both the root and `a.ms` include lib.ms; it must appear exactly once.
        std::fs::write(
            dir.join("a.ms"),
            "include \"lib.ms\";\nstage a {\n  steps {\n    $ a\n  }\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("main.ms"), "include \"lib.ms\";\ninclude \"a.ms\";\n").unwrap();

        let merged = expand_file(&dir.join("main.ms")).unwrap();
        assert_eq!(stage_names(&merged), vec!["lib", "a"], "lib is included only once");
    }

    #[test]
    fn include_cycle_is_an_error() {
        let dir = temp_dir("cycle");
        std::fs::write(dir.join("a.ms"), "include \"b.ms\";\n").unwrap();
        std::fs::write(dir.join("b.ms"), "include \"a.ms\";\n").unwrap();
        std::fs::write(dir.join("main.ms"), "include \"a.ms\";\n").unwrap();

        let err = expand_file(&dir.join("main.ms")).unwrap_err();
        assert!(
            matches!(err, Error::Semantic(diags) if diags.iter().any(|d| d.message.contains("cycle"))),
            "expected an include-cycle diagnostic"
        );
    }

    #[test]
    fn self_include_is_a_cycle() {
        let dir = temp_dir("self-cycle");
        std::fs::write(dir.join("main.ms"), "include \"main.ms\";\n").unwrap();
        let err = expand_file(&dir.join("main.ms")).unwrap_err();
        assert!(
            matches!(err, Error::Semantic(diags) if diags.iter().any(|d| d.message.contains("cycle")))
        );
    }

    #[test]
    fn missing_included_file_is_an_error() {
        let dir = temp_dir("missing");
        std::fs::write(dir.join("main.ms"), "include \"nope.ms\";\n").unwrap();
        let err = expand_file(&dir.join("main.ms")).unwrap_err();
        assert!(
            matches!(err, Error::Semantic(diags) if diags.iter().any(|d| d.message.contains("cannot read")))
        );
    }

    #[test]
    fn parse_error_in_included_file_is_reported() {
        let dir = temp_dir("badparse");
        std::fs::write(dir.join("broken.ms"), "stage {{{ not valid\n").unwrap();
        std::fs::write(dir.join("main.ms"), "include \"broken.ms\";\n").unwrap();
        let err = expand_file(&dir.join("main.ms")).unwrap_err();
        assert!(matches!(err, Error::Semantic(_) | Error::Parse(_)));
    }

    #[test]
    fn included_items_keep_their_own_file_on_spans() {
        let dir = temp_dir("spans");
        std::fs::write(dir.join("build.ms"), "stage compile {\n  steps {\n    $ cc\n  }\n}\n")
            .unwrap();
        std::fs::write(dir.join("main.ms"), "include \"build.ms\";\n").unwrap();

        let merged = expand_file(&dir.join("main.ms")).unwrap();
        let compile = merged
            .items
            .iter()
            .find_map(|i| match i {
                Item::Stage(s) if s.name == "compile" => Some(s),
                _ => None,
            })
            .unwrap();
        // The merged stage carries the *included* file on its span, so glob resolution and
        // cross-file diagnostics point at build.ms rather than main.ms.
        assert!(compile.span.file.ends_with("build.ms"));
    }

    #[test]
    fn normalize_collapses_dot_and_parent_components() {
        assert_eq!(normalize(Path::new("a/./b")), PathBuf::from("a/b"));
        assert_eq!(normalize(Path::new("a/b/../c")), PathBuf::from("a/c"));
        assert_eq!(normalize(Path::new("../a")), PathBuf::from("../a"));
        assert_eq!(normalize(Path::new("../../a")), PathBuf::from("../../a"));
        assert_eq!(normalize(Path::new("./")), PathBuf::from("."));
    }
}
