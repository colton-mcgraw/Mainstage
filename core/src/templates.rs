//! Phase 46 — Reusable Step Templates.
//!
//! Lowers `template <name> { <step>* }` items and `use <name>;` steps into ordinary
//! steps, *before* semantic analysis. Each `use` is replaced in place by the named
//! template's steps; the `template` items themselves are then dropped. The dependency
//! graph, change detection, and the parallel scheduler therefore only ever see plain
//! stages full of plain steps and need no template awareness of their own — the same
//! "lower before analysis" discipline the Phase 37 `matrix` expansion follows.
//!
//! Templates may `use` other templates, so a template body is itself expanded (with the
//! nested `use` inlined) before it is spliced into a stage. Three things are validated,
//! each with a source span: template names are unique, every `use` names a declared
//! template, and there is no recursive `use` cycle.

use std::collections::HashMap;

use crate::{
    ast::*,
    error::{Diagnostic, Error, Result},
};

/// Inline every `use` step in `program` with the referenced template's steps and drop the
/// `template` items. Returns the lowered [`Program`], or [`Error::Semantic`] carrying every
/// diagnostic (duplicate template, unknown reference, recursive cycle) found while lowering.
/// A program with no templates and no `use` steps is returned structurally unchanged.
pub fn expand(program: &Program) -> Result<Program> {
    let mut errors: Vec<Diagnostic> = Vec::new();

    // Collect the declared templates, reporting any duplicate name. A later duplicate
    // shadows the earlier one in the map; we still report it and bail before inlining.
    let mut templates: HashMap<String, &TemplateBlock> = HashMap::new();
    for item in &program.items {
        if let Item::Template(t) = item
            && templates.insert(t.name.clone(), t).is_some()
        {
            errors.push(
                Diagnostic::new(format!("template '{}' is already defined", t.name))
                    .with_span(t.span.clone()),
            );
        }
    }

    // Fully expand each template body (inlining nested `use`), in source order so the
    // diagnostics for an unknown reference or a cycle are deterministic. `resolved` caches
    // each template's final, `use`-free step list.
    let mut resolved: HashMap<String, Vec<Step>> = HashMap::new();
    for item in &program.items {
        if let Item::Template(t) = item {
            let mut stack: Vec<String> = Vec::new();
            resolve_template(&t.name, &templates, &mut resolved, &mut stack, &mut errors);
        }
    }

    if !errors.is_empty() {
        return Err(Error::Semantic(errors));
    }

    // Rebuild the program: drop `template` items and inline `use` steps everywhere a step
    // block appears (stage `steps` / `on_failure`, pipeline `on_failure` / `on_success`).
    let mut items = Vec::with_capacity(program.items.len());
    for item in &program.items {
        match item {
            Item::Template(_) => {}
            Item::Stage(s) => {
                let mut s = s.clone();
                s.steps = inline_steps(&s.steps, &templates, &mut resolved, &mut errors);
                s.on_failure = inline_steps(&s.on_failure, &templates, &mut resolved, &mut errors);
                items.push(Item::Stage(s));
            }
            Item::Pipeline(p) => {
                let mut p = p.clone();
                p.on_failure = inline_steps(&p.on_failure, &templates, &mut resolved, &mut errors);
                p.on_success = inline_steps(&p.on_success, &templates, &mut resolved, &mut errors);
                items.push(Item::Pipeline(p));
            }
            other => items.push(other.clone()),
        }
    }

    if !errors.is_empty() {
        return Err(Error::Semantic(errors));
    }

    Ok(Program { items, span: program.span.clone() })
}

/// Return the fully expanded (`use`-free) steps of the template named `name`, memoizing the
/// result in `resolved`. `stack` holds the templates currently being expanded; encountering
/// `name` already on it is a recursive `use` cycle. Assumes `name` names a known template.
fn resolve_template(
    name: &str,
    templates: &HashMap<String, &TemplateBlock>,
    resolved: &mut HashMap<String, Vec<Step>>,
    stack: &mut Vec<String>,
    errors: &mut Vec<Diagnostic>,
) -> Vec<Step> {
    if let Some(steps) = resolved.get(name) {
        return steps.clone();
    }
    if stack.iter().any(|n| n == name) {
        errors.push(
            Diagnostic::new(format!("template '{name}' is part of a recursive `use` cycle"))
                .with_span(templates[name].span.clone()),
        );
        return Vec::new();
    }
    stack.push(name.to_string());
    let body = templates[name].steps.clone();
    let inlined = inline_steps_inner(&body, templates, resolved, stack, errors);
    stack.pop();
    resolved.insert(name.to_string(), inlined.clone());
    inlined
}

/// Inline every `use` step in `steps`, descending into nested step blocks. A `use` naming an
/// undefined template is reported and dropped; a valid one is replaced by the referenced
/// template's expanded steps.
fn inline_steps(
    steps: &[Step],
    templates: &HashMap<String, &TemplateBlock>,
    resolved: &mut HashMap<String, Vec<Step>>,
    errors: &mut Vec<Diagnostic>,
) -> Vec<Step> {
    let mut stack: Vec<String> = Vec::new();
    inline_steps_inner(steps, templates, resolved, &mut stack, errors)
}

/// The shared recursion behind [`inline_steps`] and [`resolve_template`]; `stack` carries the
/// active template-expansion chain so a cycle reached through a nested block is still caught.
fn inline_steps_inner(
    steps: &[Step],
    templates: &HashMap<String, &TemplateBlock>,
    resolved: &mut HashMap<String, Vec<Step>>,
    stack: &mut Vec<String>,
    errors: &mut Vec<Diagnostic>,
) -> Vec<Step> {
    let mut out: Vec<Step> = Vec::with_capacity(steps.len());
    for step in steps {
        match step {
            Step::Use(u) => match templates.get(u.name.as_str()) {
                None => errors.push(
                    Diagnostic::new(format!("use of undefined template '{}'", u.name))
                        .with_span(u.span.clone()),
                ),
                Some(_) => {
                    out.extend(resolve_template(&u.name, templates, resolved, stack, errors));
                }
            },
            // Block steps: inline within their bodies, preserving everything else.
            Step::If(s) => {
                let mut s = s.clone();
                s.then_steps =
                    inline_steps_inner(&s.then_steps, templates, resolved, stack, errors);
                s.else_steps =
                    inline_steps_inner(&s.else_steps, templates, resolved, stack, errors);
                out.push(Step::If(s));
            }
            Step::For(s) => {
                let mut s = s.clone();
                s.steps = inline_steps_inner(&s.steps, templates, resolved, stack, errors);
                out.push(Step::For(s));
            }
            Step::Try(s) => {
                let mut s = s.clone();
                s.steps = inline_steps_inner(&s.steps, templates, resolved, stack, errors);
                out.push(Step::Try(s));
            }
            Step::Workdir(s) => {
                let mut s = s.clone();
                s.steps = inline_steps_inner(&s.steps, templates, resolved, stack, errors);
                out.push(Step::Workdir(s));
            }
            Step::WithEnv(s) => {
                let mut s = s.clone();
                s.steps = inline_steps_inner(&s.steps, templates, resolved, stack, errors);
                out.push(Step::WithEnv(s));
            }
            other => out.push(other.clone()),
        }
    }
    out
}

// ── Tests ────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;
    use crate::source::Source;

    fn lower(src: &str) -> Result<Program> {
        let program = parse(&Source::from_str("test.ms", src)).expect("parse should succeed");
        expand(&program)
    }

    /// The steps of the named stage in a lowered program.
    fn stage_steps<'a>(program: &'a Program, name: &str) -> &'a [Step] {
        program
            .items
            .iter()
            .find_map(|i| match i {
                Item::Stage(s) if s.name == name => Some(s.steps.as_slice()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no stage named '{name}'"))
    }

    fn exec_commands(steps: &[Step]) -> Vec<&str> {
        steps
            .iter()
            .filter_map(|s| match s {
                Step::Exec(e) => Some(e.command.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn no_templates_is_unchanged() {
        let program = lower("stage build {\n  steps {\n    $ make\n  }\n}\n").unwrap();
        assert_eq!(exec_commands(stage_steps(&program, "build")), vec!["make"]);
        // No residual template items remain.
        assert!(!program.items.iter().any(|i| matches!(i, Item::Template(_))));
    }

    #[test]
    fn use_inlines_template_steps() {
        let program = lower(
            "template setup {\n  $ checkout\n  $ restore-cache\n}\n\
             stage build {\n  steps {\n    use setup;\n    $ make\n  }\n}\n",
        )
        .unwrap();
        assert_eq!(
            exec_commands(stage_steps(&program, "build")),
            vec!["checkout", "restore-cache", "make"]
        );
        // The template item is dropped from the lowered program.
        assert!(!program.items.iter().any(|i| matches!(i, Item::Template(_))));
    }

    #[test]
    fn template_shared_across_two_stages() {
        let program = lower(
            "template teardown {\n  $ flush\n}\n\
             stage a {\n  steps {\n    $ work-a\n    use teardown;\n  }\n}\n\
             stage b {\n  steps {\n    $ work-b\n    use teardown;\n  }\n}\n",
        )
        .unwrap();
        assert_eq!(exec_commands(stage_steps(&program, "a")), vec!["work-a", "flush"]);
        assert_eq!(exec_commands(stage_steps(&program, "b")), vec!["work-b", "flush"]);
    }

    #[test]
    fn use_inside_a_nested_block_is_inlined() {
        let program = lower(
            "template note {\n  log \"hi\"\n}\n\
             stage s {\n  steps {\n    try {\n      use note;\n    }\n  }\n}\n",
        )
        .unwrap();
        // The `try` block now contains the template's inlined `log` step.
        match &stage_steps(&program, "s")[0] {
            Step::Try(t) => assert!(matches!(t.steps.as_slice(), [Step::Log(_)])),
            other => panic!("expected a try step, got {other:?}"),
        }
    }

    #[test]
    fn template_using_another_template_is_inlined() {
        let program = lower(
            "template inner {\n  $ deep\n}\n\
             template outer {\n  $ before\n  use inner;\n  $ after\n}\n\
             stage s {\n  steps { use outer; }\n}\n",
        )
        .unwrap();
        assert_eq!(exec_commands(stage_steps(&program, "s")), vec!["before", "deep", "after"]);
    }

    #[test]
    fn unknown_template_is_an_error() {
        let err = lower("stage s {\n  steps { use missing; }\n}\n").unwrap_err();
        assert!(
            matches!(err, Error::Semantic(diags) if diags[0].message.contains("undefined template 'missing'"))
        );
    }

    #[test]
    fn duplicate_template_is_an_error() {
        let err = lower("template t {\n  $ a\n}\ntemplate t {\n  $ b\n}\n").unwrap_err();
        assert!(
            matches!(err, Error::Semantic(diags) if diags.iter().any(|d| d.message.contains("already defined")))
        );
    }

    #[test]
    fn recursive_cycle_is_an_error() {
        let err = lower(
            "template a {\n  use b;\n}\n\
             template b {\n  use a;\n}\n\
             stage s {\n  steps { use a; }\n}\n",
        )
        .unwrap_err();
        assert!(
            matches!(err, Error::Semantic(diags) if diags.iter().any(|d| d.message.contains("recursive `use` cycle")))
        );
    }

    #[test]
    fn self_recursive_template_is_an_error() {
        let err =
            lower("template a {\n  use a;\n}\nstage s {\n  steps { use a; }\n}\n").unwrap_err();
        assert!(
            matches!(err, Error::Semantic(diags) if diags.iter().any(|d| d.message.contains("recursive `use` cycle")))
        );
    }
}
