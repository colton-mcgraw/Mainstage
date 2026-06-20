//! Phase 37 — Parameterized Stages / Build Matrix.
//!
//! Lowers `matrix { <dim>: [<values>] }` on a stage into N concrete stages, one per
//! combination of dimension values, *before* semantic analysis. The dependency graph,
//! change detection, and the parallel scheduler therefore only ever see ordinary stages
//! and need no matrix awareness of their own.
//!
//! A stage `bootloader` carrying `matrix { arch: ["x64", "arm64"] }` expands to two
//! stages named `bootloader[x64]` and `bootloader[arm64]`, each with the matching
//! `arch` binding exposed as a built-in string variable (alongside `platform`).
//!
//! References to a matrixed stage by its **base** name fan out to every variant: a base
//! name in a pipeline's `stages:` list or a stage's `depends_on:` is replaced by all of
//! its variants, and a `<base>.outputs` reference becomes the combined outputs of every
//! variant. This keeps the surface syntax writable — the bracketed generated names are
//! never typed by hand — while still letting a multi-target build wire stages together.

use std::collections::{HashMap, HashSet};

use crate::{
    ast::*,
    error::{Diagnostic, Error, Result},
};

/// Built-in variable names a matrix dimension may not shadow.
const RESERVED_NAMES: &[&str] = &["platform", "inputs", "outputs", "failed_stage"];

/// Expand every matrixed stage in `program` into its concrete variants, rewriting
/// references to matrixed base names so they fan out to the generated variants.
///
/// Returns the lowered [`Program`] on success, or [`Error::Semantic`] carrying every
/// matrix diagnostic (empty dimensions, duplicate dimensions or values, reserved-name
/// collisions, and name clashes between generated stages) found during lowering. A
/// program with no `matrix` blocks is returned structurally unchanged.
pub fn expand(program: &Program) -> Result<Program> {
    let mut errors: Vec<Diagnostic> = Vec::new();

    // Names of stages that exist after lowering — authored non-matrix stages plus every
    // generated variant — so a generated name colliding with either is reported.
    let mut all_names: HashSet<String> = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Stage(s) if s.matrix.is_empty() => Some(s.name.clone()),
            _ => None,
        })
        .collect();

    // Validate each matrixed stage and compute its variants. `expansions` maps a base
    // name to the ordered list of generated variant stages.
    let mut expansions: HashMap<String, Vec<StageBlock>> = HashMap::new();
    for item in &program.items {
        if let Item::Stage(stage) = item
            && !stage.matrix.is_empty()
            && let Some(variants) = expand_stage(stage, &mut all_names, &mut errors)
        {
            expansions.insert(stage.name.clone(), variants);
        }
    }

    if !errors.is_empty() {
        return Err(Error::Semantic(errors));
    }

    // The set of base names that were matrixed — reference rewriting keys off this.
    let bases: HashMap<String, Vec<String>> = expansions
        .iter()
        .map(|(base, variants)| (base.clone(), variants.iter().map(|s| s.name.clone()).collect()))
        .collect();

    // Rebuild the program, replacing each matrixed stage with its variants (every
    // variant already has its references rewritten) and rewriting references elsewhere.
    let mut items = Vec::with_capacity(program.items.len());
    for item in &program.items {
        match item {
            Item::Stage(s) if !s.matrix.is_empty() => {
                for variant in expansions.remove(&s.name).into_iter().flatten() {
                    items.push(Item::Stage(rewrite_stage(variant, &bases)));
                }
            }
            Item::Stage(s) => items.push(Item::Stage(rewrite_stage(s.clone(), &bases))),
            Item::Pipeline(p) => items.push(Item::Pipeline(rewrite_pipeline(p.clone(), &bases))),
            Item::Let(d) => {
                let mut d = d.clone();
                rewrite_expr(&mut d.value, &bases);
                items.push(Item::Let(d));
            }
            Item::Project(b) => {
                let mut b = b.clone();
                for field in &mut b.fields {
                    rewrite_expr(&mut field.value, &bases);
                }
                items.push(Item::Project(b));
            }
            Item::Import(d) => items.push(Item::Import(d.clone())),
            // Templates (Phase 46) and includes (Phase 48) are both lowered away before
            // matrix expansion runs (see `commands.rs::prepare`); these arms only keep the
            // match total.
            Item::Template(t) => items.push(Item::Template(t.clone())),
            Item::Include(d) => items.push(Item::Include(d.clone())),
        }
    }

    Ok(Program { items, span: program.span.clone() })
}

// ── Validation & variant generation ──────────────────────────────────────────────

/// Validate one matrixed stage and produce its concrete variants, or `None` if the
/// matrix is invalid (diagnostics are pushed onto `errors`). Newly generated names are
/// inserted into `all_names`; a collision there is reported.
fn expand_stage(
    stage: &StageBlock,
    all_names: &mut HashSet<String>,
    errors: &mut Vec<Diagnostic>,
) -> Option<Vec<StageBlock>> {
    let before = errors.len();

    // Dimension names: unique, non-reserved, non-empty value lists with unique values.
    let mut seen_dims: HashSet<&str> = HashSet::new();
    for dim in &stage.matrix {
        if RESERVED_NAMES.contains(&dim.name.as_str()) {
            errors.push(
                Diagnostic::new(format!(
                    "matrix dimension '{}' shadows the built-in variable of the same name",
                    dim.name
                ))
                .with_span(dim.span.clone()),
            );
        }
        if !seen_dims.insert(dim.name.as_str()) {
            errors.push(
                Diagnostic::new(format!(
                    "matrix dimension '{}' is declared more than once in stage '{}'",
                    dim.name, stage.name
                ))
                .with_span(dim.span.clone()),
            );
        }
        if dim.values.is_empty() {
            errors.push(
                Diagnostic::new(format!(
                    "matrix dimension '{}' has no values; an empty dimension produces no stages",
                    dim.name
                ))
                .with_span(dim.span.clone()),
            );
        }
        let mut seen_vals: HashSet<&str> = HashSet::new();
        for val in &dim.values {
            if !seen_vals.insert(val.value.as_str()) {
                errors.push(
                    Diagnostic::new(format!(
                        "matrix dimension '{}' repeats the value \"{}\"",
                        dim.name, val.value
                    ))
                    .with_span(val.span.clone()),
                );
            }
        }
    }

    if errors.len() != before {
        return None;
    }

    // Cartesian product of all dimensions, in declaration order. Each combination is a
    // list of (dimension, value) bindings.
    let combinations = cartesian(&stage.matrix);

    let mut variants = Vec::with_capacity(combinations.len());
    for bindings in combinations {
        let suffix = bindings.iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>().join(",");
        let name = format!("{}[{}]", stage.name, suffix);

        if !all_names.insert(name.clone()) {
            errors.push(
                Diagnostic::new(format!(
                    "matrix expansion of stage '{}' produces '{}', which already names another stage",
                    stage.name, name
                ))
                .with_span(stage.span.clone()),
            );
            continue;
        }

        let mut variant = stage.clone();
        variant.name = name;
        variant.matrix = Vec::new();
        variant.matrix_bindings =
            bindings.into_iter().map(|(n, v)| MatrixBinding { name: n, value: v }).collect();
        variants.push(variant);
    }

    if errors.len() != before { None } else { Some(variants) }
}

/// Compute the cartesian product of every dimension's values, yielding one binding list
/// per combination. Dimension order is preserved so generated names are deterministic.
fn cartesian(dims: &[MatrixDim]) -> Vec<Vec<(String, String)>> {
    let mut combos: Vec<Vec<(String, String)>> = vec![Vec::new()];
    for dim in dims {
        let mut next = Vec::with_capacity(combos.len() * dim.values.len());
        for combo in &combos {
            for val in &dim.values {
                let mut extended = combo.clone();
                extended.push((dim.name.clone(), val.value.clone()));
                next.push(extended);
            }
        }
        combos = next;
    }
    combos
}

// ── Reference rewriting ──────────────────────────────────────────────────────────

/// Rewrite every matrixed-base reference inside a stage: its `depends_on` edges and any
/// `<base>.outputs` references in its `inputs` / `outputs` expressions.
fn rewrite_stage(mut stage: StageBlock, bases: &HashMap<String, Vec<String>>) -> StageBlock {
    stage.depends_on = expand_deps(stage.depends_on, bases);
    if let Some(expr) = &mut stage.inputs {
        rewrite_expr(expr, bases);
    }
    if let Some(expr) = &mut stage.outputs {
        rewrite_expr(expr, bases);
    }
    stage
}

/// Replace each `depends_on` entry that names a matrixed base with that base's variants,
/// preserving the original reference's span and de-duplicating.
fn expand_deps(deps: Vec<StageDep>, bases: &HashMap<String, Vec<String>>) -> Vec<StageDep> {
    let mut out: Vec<StageDep> = Vec::new();
    let push = |out: &mut Vec<StageDep>, dep: StageDep| {
        if !out.iter().any(|d| d.name == dep.name) {
            out.push(dep);
        }
    };
    for dep in deps {
        match bases.get(&dep.name) {
            Some(variants) => {
                for v in variants {
                    push(&mut out, StageDep { name: v.clone(), span: dep.span.clone() });
                }
            }
            None => push(&mut out, dep),
        }
    }
    out
}

/// Rewrite a pipeline's `stages:` expression so matrixed base names fan out to variants.
fn rewrite_pipeline(
    mut pipeline: PipelineBlock,
    bases: &HashMap<String, Vec<String>>,
) -> PipelineBlock {
    if let Some(expr) = &mut pipeline.stages {
        rewrite_stage_list(expr, bases);
    }
    if let Some(expr) = &mut pipeline.input {
        rewrite_expr(expr, bases);
    }
    pipeline
}

/// Rewrite a `stages:` expression: within any list, a bare identifier naming a matrixed
/// base is spliced into its variant identifiers; a bare base identifier on its own
/// becomes a list of variant identifiers. Other expression forms are left to the
/// general `<base>.outputs` rewriter.
fn rewrite_stage_list(expr: &mut Expr, bases: &HashMap<String, Vec<String>>) {
    match expr {
        Expr::List(list) => {
            let mut items = Vec::with_capacity(list.items.len());
            for item in std::mem::take(&mut list.items) {
                match &item {
                    Expr::Ident(id) if bases.contains_key(&id.name) => {
                        for v in &bases[&id.name] {
                            items.push(Expr::Ident(IdentExpr {
                                name: v.clone(),
                                span: id.span.clone(),
                            }));
                        }
                    }
                    _ => {
                        let mut item = item;
                        rewrite_stage_list(&mut item, bases);
                        items.push(item);
                    }
                }
            }
            list.items = items;
        }
        Expr::Ident(id) if bases.contains_key(&id.name) => {
            let span = id.span.clone();
            let items = bases[&id.name]
                .iter()
                .map(|v| Expr::Ident(IdentExpr { name: v.clone(), span: span.clone() }))
                .collect();
            *expr = Expr::List(ListExpr { items, span });
        }
        Expr::If(if_expr) => {
            rewrite_stage_list(&mut if_expr.then_expr, bases);
            rewrite_stage_list(&mut if_expr.else_expr, bases);
        }
        _ => {}
    }
}

/// Recursively rewrite `<base>.outputs` references to a matrixed base into the combined
/// outputs of every variant, anywhere an expression appears. A standalone reference
/// becomes a `[<v1>.outputs, <v2>.outputs, …]` list; nested lists flatten for path
/// collection, so this composes with `inputs: [<base>.outputs, assets]`.
fn rewrite_expr(expr: &mut Expr, bases: &HashMap<String, Vec<String>>) {
    match expr {
        Expr::StageRef(r) if bases.contains_key(&r.stage) => {
            let span = r.span.clone();
            let items = bases[&r.stage]
                .iter()
                .map(|v| Expr::StageRef(StageRefExpr { stage: v.clone(), span: span.clone() }))
                .collect();
            *expr = Expr::List(ListExpr { items, span });
        }
        Expr::List(list) => {
            for item in &mut list.items {
                rewrite_expr(item, bases);
            }
        }
        Expr::If(if_expr) => {
            rewrite_expr(&mut if_expr.then_expr, bases);
            rewrite_expr(&mut if_expr.else_expr, bases);
        }
        Expr::String(s) => {
            for part in &mut s.parts {
                if let StringPart::Interpolation(inner) = part {
                    rewrite_expr(inner, bases);
                }
            }
        }
        Expr::ModuleCall(c) => {
            for arg in &mut c.args {
                rewrite_expr(&mut arg.value, bases);
            }
        }
        _ => {}
    }
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
    fn no_matrix_is_unchanged() {
        let program = lower("stage build {}\n").unwrap();
        assert_eq!(stage_names(&program), vec!["build"]);
    }

    #[test]
    fn single_dimension_expands_to_variants() {
        let program =
            lower("stage bootloader {\n  matrix { arch: [\"x64\", \"arm64\"] }\n}\n").unwrap();
        assert_eq!(stage_names(&program), vec!["bootloader[x64]", "bootloader[arm64]"]);
        // Each variant carries its binding and no residual matrix.
        for item in &program.items {
            if let Item::Stage(s) = item {
                assert!(s.matrix.is_empty());
                assert_eq!(s.matrix_bindings.len(), 1);
                assert_eq!(s.matrix_bindings[0].name, "arch");
            }
        }
    }

    #[test]
    fn two_dimensions_form_cartesian_product() {
        let program = lower(
            "stage k {\n  matrix { arch: [\"x64\", \"arm64\"]\n  mode: [\"debug\", \"release\"] }\n}\n",
        )
        .unwrap();
        assert_eq!(
            stage_names(&program),
            vec!["k[x64,debug]", "k[x64,release]", "k[arm64,debug]", "k[arm64,release]"]
        );
    }

    #[test]
    fn pipeline_base_reference_fans_out() {
        let program = lower(
            "default pipeline dev { stages: [boot, link] }\n\
             stage boot { matrix { arch: [\"x64\", \"arm64\"] } }\n\
             stage link {}\n",
        )
        .unwrap();
        let pipeline = program.items.iter().find_map(|i| match i {
            Item::Pipeline(p) => Some(p),
            _ => None,
        });
        let stages = pipeline.unwrap().stages.as_ref().unwrap();
        if let Expr::List(l) = stages {
            let names: Vec<&str> = l
                .items
                .iter()
                .map(|e| match e {
                    Expr::Ident(id) => id.name.as_str(),
                    _ => "?",
                })
                .collect();
            assert_eq!(names, vec!["boot[x64]", "boot[arm64]", "link"]);
        } else {
            panic!("expected a list");
        }
    }

    #[test]
    fn depends_on_base_reference_fans_out() {
        let program = lower(
            "stage boot { matrix { arch: [\"x64\", \"arm64\"] } }\n\
             stage run { depends_on: [boot] }\n",
        )
        .unwrap();
        let run = program.items.iter().find_map(|i| match i {
            Item::Stage(s) if s.name == "run" => Some(s),
            _ => None,
        });
        let deps: Vec<&str> = run.unwrap().depends_on.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(deps, vec!["boot[x64]", "boot[arm64]"]);
    }

    #[test]
    fn stage_outputs_reference_combines_variants() {
        let program = lower(
            "stage boot { matrix { arch: [\"x64\", \"arm64\"] }\n outputs: [\"b/${arch}\"] }\n\
             stage pkg { inputs: [boot.outputs] }\n",
        )
        .unwrap();
        let pkg = program.items.iter().find_map(|i| match i {
            Item::Stage(s) if s.name == "pkg" => Some(s),
            _ => None,
        });
        // inputs: [ [boot[x64].outputs, boot[arm64].outputs] ] — nested list, flattens.
        if let Some(Expr::List(outer)) = &pkg.unwrap().inputs
            && let Expr::List(inner) = &outer.items[0]
        {
            let refs: Vec<&str> = inner
                .items
                .iter()
                .map(|e| match e {
                    Expr::StageRef(r) => r.stage.as_str(),
                    _ => "?",
                })
                .collect();
            assert_eq!(refs, vec!["boot[x64]", "boot[arm64]"]);
            return;
        }
        panic!("expected nested stage-ref list");
    }

    #[test]
    fn empty_dimension_is_an_error() {
        let err = lower("stage s { matrix { arch: [] } }\n").unwrap_err();
        assert!(matches!(err, Error::Semantic(diags) if diags[0].message.contains("no values")));
    }

    #[test]
    fn reserved_dimension_name_is_an_error() {
        let err = lower("stage s { matrix { platform: [\"x\"] } }\n").unwrap_err();
        assert!(matches!(err, Error::Semantic(diags) if diags[0].message.contains("shadows")));
    }

    #[test]
    fn duplicate_dimension_is_an_error() {
        let err = lower("stage s { matrix { arch: [\"x\"]\n arch: [\"y\"] } }\n").unwrap_err();
        assert!(
            matches!(err, Error::Semantic(diags) if diags.iter().any(|d| d.message.contains("more than once")))
        );
    }

    #[test]
    fn duplicate_value_is_an_error() {
        let err = lower("stage s { matrix { arch: [\"x\", \"x\"] } }\n").unwrap_err();
        assert!(matches!(err, Error::Semantic(diags) if diags[0].message.contains("repeats")));
    }

    #[test]
    fn generated_name_collision_is_an_error() {
        // Two matrixed stages sharing a base name expand to the same variant name; the
        // bracketed generated names cannot be hand-written, so this is the reachable
        // collision case.
        let err = lower(
            "stage a { matrix { x: [\"1\"] } }\n\
             stage a { matrix { x: [\"1\"] } }\n",
        )
        .unwrap_err();
        assert!(
            matches!(err, Error::Semantic(diags) if diags.iter().any(|d| d.message.contains("already names another stage")))
        );
    }
}
