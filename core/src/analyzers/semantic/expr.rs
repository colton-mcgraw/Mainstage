//! Expression-level semantic analysis and type inference.
//!
//! This module analyzes expressions, assignments, calls and other expression
//! forms to infer kinds (`InferredKind`), perform basic type checks, and
//! update the `SymbolTable` with inferred types and usage information.

use super::kind::{InferredKind, Kind, Origin};
use super::symbol::Symbol;
pub(crate) fn analyze_assignment(
    node: &mut crate::ast::AstNode,
    tbl: &mut crate::analyzers::semantic::table::SymbolTable,
) -> Result<InferredKind, Box<dyn crate::error::MainstageErrorExt>> {
    if let crate::ast::AstNodeKind::Assignment { target, value } = &mut node.kind {
        // infer the value kind
        let value_kind =
            super::node::analyze_node(value, tbl)?.unwrap_or_else(InferredKind::dynamic);

        // target must be identifier for now
        if let crate::ast::AstNodeKind::Identifier { name } = &mut target.kind {
            // If we're inside an object declaration body, treat this assignment
            // as a property definition on the enclosing object instead of a
            // local variable.
            if let Some(obj_name) = tbl.current_object_name() {
                // Find the object symbol (should be in an outer/global scope)
                if let Some(obj_sym) = tbl.get_latest_symbol_mut(&obj_name) {
                    // Insert or update property on object
                    let prop_sym = Symbol::new_variable(
                        name.clone(),
                        Some(value_kind.clone()),
                        node.location.clone(),
                        node.span.clone(),
                    );
                    obj_sym.insert_property(name.clone(), prop_sym);
                    return Ok(value_kind);
                }
            }

            if let Some(sym) = tbl.get_latest_symbol_mut(name) {
                match &sym.inferred_type() {
                    Some(existing) => {
                        // existing is &InferredKind
                        let existing_clone = (*existing).clone();
                        // If existing is dynamic, overwrite. Otherwise check compatibility with implicit promotions.
                        if existing_clone.is_dynamic() {
                            sym.set_inferred_type(value_kind.clone());
                        } else {
                            // Use compatibility rules directly on InferredKind
                            if !existing_clone.is_compatible_with(&value_kind) {
                                return Err(Box::new(
                                    crate::analyzers::semantic::err::SemanticError::with(
                                        crate::error::Level::Error,
                                        format!(
                                            "Incompatible assignment: variable '{}' has type {} but assigned value has type {}",
                                            name, existing_clone, value_kind
                                        ),
                                        "mainstage.analyzers.semantic.expr.analyze_assignment"
                                            .to_string(),
                                        node.location.clone(),
                                        node.span.clone(),
                                    ),
                                ));
                            }
                            // Compatible: keep existing declared type (implicit promotion allowed)
                        }
                    }
                    None => {
                        sym.set_inferred_type(value_kind.clone());
                    }
                }
            } else {
                // No visible symbol — create a new variable in the current scope.
                tbl.insert_symbol(Symbol::new_variable(
                    name.clone(),
                    Some(value_kind.clone()),
                    node.location.clone(),
                    node.span.clone(),
                ));
            }

            Ok(value_kind)
        } else {
            Err(Box::new(
                crate::analyzers::semantic::err::SemanticError::with(
                    crate::error::Level::Error,
                    format!(
                        "Assignment target must be an identifier, found: {}",
                        target.kind
                    ),
                    "mainstage.analyzers.semantic.expr.analyze_assignment".to_string(),
                    node.location.clone(),
                    node.span.clone(),
                ),
            ))
        }
    } else {
        Err(Box::new(
            crate::analyzers::semantic::err::SemanticError::with(
                crate::error::Level::Error,
                format!("Expected Assignment node, found: {}", node.kind),
                "mainstage.analyzers.semantic.expr.analyze_assignment".to_string(),
                node.location.clone(),
                node.span.clone(),
            ),
        ))
    }
}

pub(crate) fn analyze_block(
    node: &mut crate::ast::AstNode,
    tbl: &mut crate::analyzers::semantic::table::SymbolTable,
) -> Result<(), Box<dyn crate::error::MainstageErrorExt>> {
    if let crate::ast::AstNodeKind::Block { statements } = &mut node.kind {
        for stmt in statements.iter_mut() {
            super::node::analyze_node(stmt, tbl)?;
        }
    } else {
        return Err(Box::new(
            crate::analyzers::semantic::err::SemanticError::with(
                crate::error::Level::Error,
                format!("Expected Block node, found: {}", node.kind),
                "mainstage.analyzers.semantic.expr.analyze_block".to_string(),
                node.location.clone(),
                node.span.clone(),
            ),
        ));
    }
    Ok(())
}
/// Analyze an identifier usage and return its inferred kind (or Dynamic if unknown).
pub(crate) fn analyze_identifier(
    node: &mut crate::ast::AstNode,
    tbl: &mut crate::analyzers::semantic::table::SymbolTable,
) -> Result<InferredKind, Box<dyn crate::error::MainstageErrorExt>> {
    if let crate::ast::AstNodeKind::Identifier { name } = &mut node.kind {
        if let Some(sym) = tbl.get_latest_symbol_mut(name) {
            // record this usage for ref-counting/diagnostics
            sym.increment_ref_count();
            // record the precise usage site (location/span) for diagnostics
            sym.record_usage(node.location.clone(), node.span.clone());
            if let Some(k) = sym.inferred_type() {
                let mut ik = k.clone();
                ik.origin = Origin::Expression;
                // update location/span to usage site if available
                if node.location.is_some() {
                    ik.location = node.location.clone();
                }
                if node.span.is_some() {
                    ik.span = node.span.clone();
                }
                return Ok(ik);
            }
        }
        // If not present, return dynamic; do not create a variable placeholder here.
        // This avoids creating spurious variable symbols for identifiers that may
        // actually be functions/objects declared later (which would otherwise be
        // reported as "declared but never used"). Assignments will create variables
        // in the current scope as needed.
        Ok(InferredKind::dynamic())
    } else {
        Err(Box::new(
            crate::analyzers::semantic::err::SemanticError::with(
                crate::error::Level::Error,
                format!("Expected Identifier node, found: {}", node.kind),
                "mainstage.analyzers.semantic.expr.analyze_identifier".to_string(),
                node.location.clone(),
                node.span.clone(),
            ),
        ))
    }
}

/// Analyze expressions (literals, binary ops, unary ops, lists, etc.) and
/// return an inferred kind.
pub(crate) fn analyze_expression(
    node: &mut crate::ast::AstNode,
    tbl: &mut crate::analyzers::semantic::table::SymbolTable,
) -> Result<InferredKind, Box<dyn crate::error::MainstageErrorExt>> {
    match &mut node.kind {
        crate::ast::AstNodeKind::Integer { .. } => {
            let mut ik = InferredKind::integer();
            if let Some(loc) = node.location.clone() {
                ik = ik.with_location(loc);
            }
            if let Some(span) = node.span.clone() {
                ik = ik.with_span(span);
            }
            Ok(ik)
        }
        crate::ast::AstNodeKind::Float { .. } => {
            let mut ik = InferredKind::float();
            if let Some(loc) = node.location.clone() {
                ik = ik.with_location(loc);
            }
            if let Some(span) = node.span.clone() {
                ik = ik.with_span(span);
            }
            Ok(ik)
        }
        crate::ast::AstNodeKind::String { .. } => {
            let mut ik = InferredKind::string();
            if let Some(loc) = node.location.clone() {
                ik = ik.with_location(loc);
            }
            if let Some(span) = node.span.clone() {
                ik = ik.with_span(span);
            }
            Ok(ik)
        }
        crate::ast::AstNodeKind::Bool { .. } => {
            let mut ik = InferredKind::boolean();
            if let Some(loc) = node.location.clone() {
                ik = ik.with_location(loc);
            }
            if let Some(span) = node.span.clone() {
                ik = ik.with_span(span);
            }
            Ok(ik)
        }
        crate::ast::AstNodeKind::Null => Ok(InferredKind::new(
            Kind::Null,
            Origin::Expression,
            node.location.clone(),
            node.span.clone(),
        )),
        crate::ast::AstNodeKind::List { elements } => {
            // Infer element type: lists must be homogeneous (single element type).
            if elements.is_empty() {
                // empty list -> element type dynamic
                let out = InferredKind::new(
                    Kind::Array,
                    Origin::Expression,
                    node.location.clone(),
                    node.span.clone(),
                )
                .with_element(InferredKind::dynamic());
                return Ok(out);
            }

            let mut elem_type: Option<InferredKind> = None;
            for el in elements.iter_mut() {
                let k =
                    super::node::analyze_node(el, tbl)?.unwrap_or_else(InferredKind::dynamic);
                if let Some(prev) = elem_type {
                    let unified = prev.unify(&k);
                    // If unify is dynamic but both operands were concrete and incompatible -> error
                    if unified.is_dynamic() && !prev.is_dynamic() && !k.is_dynamic() {
                        return Err(Box::new(
                            crate::analyzers::semantic::err::SemanticError::with(
                                crate::error::Level::Error,
                                format!("Incompatible list element types: {} and {}", prev, k),
                                "mainstage.analyzers.semantic.expr.analyze_expression".to_string(),
                                node.location.clone(),
                                node.span.clone(),
                            ),
                        ));
                    }
                    elem_type = Some(unified);
                } else {
                    elem_type = Some(k);
                }
            }

            let element_kind = elem_type.unwrap_or_else(InferredKind::dynamic);
            let out = InferredKind::new(
                Kind::Array,
                Origin::Expression,
                node.location.clone(),
                node.span.clone(),
            )
            .with_element(element_kind);
            Ok(out)
        }
        crate::ast::AstNodeKind::Identifier { .. } => analyze_identifier(node, tbl),
        crate::ast::AstNodeKind::Call { callee, args } => {
            // Analyze argument expressions for side-effects and type-checking
            for a in args.iter_mut() {
                let _ = super::node::analyze_node(a, tbl)?;
            }

            // If the callee is a simple identifier, resolve it without creating
            // a variable placeholder. Treat it as a function/stage call.
            if let crate::ast::AstNodeKind::Identifier { name } = &mut callee.kind {
                if let Some(sym) = tbl.get_latest_symbol_mut(name) {
                    // record usage
                    sym.increment_ref_count();
                    sym.record_usage(node.location.clone(), node.span.clone());
                    // If the symbol has a declared return kind, use it
                    if let Some(r) = sym.returns() {
                        return Ok(r.clone());
                    }
                    // Otherwise, use the inferred type if present
                    if let Some(t) = sym.inferred_type() {
                        return Ok(t.clone());
                    }
                    return Ok(InferredKind::dynamic());
                } else {
                    // No symbol found: create a placeholder function symbol in the global scope
                    tbl.insert_symbol(Symbol::new(
                        name.clone(),
                        super::symbol::SymbolKind::Function,
                        None,
                        None,
                        node.location.clone(),
                        node.span.clone(),
                    ));
                    if let Some(sym) = tbl.get_latest_symbol_mut(name) {
                        sym.increment_ref_count();
                        sym.record_usage(node.location.clone(), node.span.clone());
                    }
                    return Ok(InferredKind::dynamic());
                }
            }

            // For non-identifier callees, analyze normally and return dynamic
            let _c =
                super::node::analyze_node(callee, tbl)?.unwrap_or_else(InferredKind::dynamic);
            Ok(InferredKind::dynamic())
        }
        crate::ast::AstNodeKind::UnaryOp { op, expr } => {
            let operand =
                super::node::analyze_node(expr, tbl)?.unwrap_or_else(InferredKind::dynamic);
            use crate::ast::kind::UnaryOperator;
            match op {
                UnaryOperator::Plus | UnaryOperator::Minus => {
                    if operand.is_numeric() || operand.is_dynamic() {
                        Ok(operand)
                    } else {
                        Err(Box::new(
                            crate::analyzers::semantic::err::SemanticError::with(
                                crate::error::Level::Error,
                                format!(
                                    "Unary operator requires numeric operand, found {}",
                                    operand
                                ),
                                "mainstage.analyzers.semantic.expr.analyze_expression".to_string(),
                                node.location.clone(),
                                node.span.clone(),
                            ),
                        ))
                    }
                }
                UnaryOperator::Not => Ok(InferredKind::boolean()),
                _ => Ok(InferredKind::dynamic()),
            }
        }
        crate::ast::AstNodeKind::BinaryOp { left, op, right } => {
            let l =
                super::node::analyze_node(left, tbl)?.unwrap_or_else(InferredKind::dynamic);
            let r =
                super::node::analyze_node(right, tbl)?.unwrap_or_else(InferredKind::dynamic);
            use crate::ast::kind::BinaryOperator;

            // If either side is dynamic, unify will return dynamic and allow
            // permissive behavior. Otherwise, compute unified kind and error
            // if incompatible for the operator.
            let unified = l.unify(&r);

            // Heuristic: if unify produced Dynamic but both operands were concrete
            // and operator is arithmetic, treat as error (e.g., Array + Integer)
            match op {
                BinaryOperator::Add
                | BinaryOperator::Sub
                | BinaryOperator::Mul
                | BinaryOperator::Div
                | BinaryOperator::Mod => {
                    if unified.is_dynamic() && !l.is_dynamic() && !r.is_dynamic() {
                        return Err(Box::new(
                            crate::analyzers::semantic::err::SemanticError::with(
                                crate::error::Level::Error,
                                format!(
                                    "Incompatible operands for binary operator {:?}: {} and {}",
                                    op, l, r
                                ),
                                "mainstage.analyzers.semantic.expr.analyze_expression".to_string(),
                                node.location.clone(),
                                node.span.clone(),
                            ),
                        ));
                    }
                    Ok(unified)
                }
                // Comparison operators yield boolean
                BinaryOperator::Eq
                | BinaryOperator::Ne
                | BinaryOperator::Lt
                | BinaryOperator::Le
                | BinaryOperator::Gt
                | BinaryOperator::Ge => Ok(InferredKind::boolean()),
            }
        }
        crate::ast::AstNodeKind::Member { object, property } => {
            // Evaluate object expression; if it's a simple identifier resolving to
            // an object symbol, look up the property.
            if let crate::ast::AstNodeKind::Identifier { name } = &mut object.kind
                && let Some(obj_sym) = tbl.get_latest_symbol_mut(name)
            {
                // Record that the object itself was referenced (e.g. `prj` in `prj.sources`).
                obj_sym.increment_ref_count();
                obj_sym.record_usage(object.location.clone(), object.span.clone());
                // Try to find the property on the object
                if let Some(prop_mut) = obj_sym.get_property_mut(property) {
                    // record usage on the property symbol
                    prop_mut.increment_ref_count();
                    prop_mut.record_usage(object.location.clone(), object.span.clone());
                    if let Some(t) = prop_mut.inferred_type() {
                        return Ok(t.clone());
                    }
                    return Ok(InferredKind::dynamic());
                } else {
                    // Property not present yet; create placeholder property symbol
                    let placeholder = Symbol::new_variable(
                        property.clone(),
                        Some(InferredKind::dynamic()),
                        node.location.clone(),
                        node.span.clone(),
                    );
                    obj_sym.insert_property(property.clone(), placeholder);
                    return Ok(InferredKind::dynamic());
                }
            }
            // For non-identifier objects or unresolved object, analyze the object
            // expression and conservatively return dynamic.
            let _ = super::node::analyze_node(object, tbl)?;
            Ok(InferredKind::dynamic())
        }
        crate::ast::AstNodeKind::Index { object, index } => {
            // If the object is a plain identifier, mark it referenced so parameters
            // like `prj` are not reported as unused when only member/index reads occur.
            if let crate::ast::AstNodeKind::Identifier { name } = &mut object.kind
                && let Some(obj_sym) = tbl.get_latest_symbol_mut(name)
            {
                obj_sym.increment_ref_count();
                obj_sym.record_usage(object.location.clone(), object.span.clone());
            }

            // Evaluate object expression and if it's an array, return element kind
            let obj_kind =
                super::node::analyze_node(object, tbl)?.unwrap_or_else(InferredKind::dynamic);
            if obj_kind.kind == crate::analyzers::semantic::kind::Kind::Array {
                if let Some(elem) = obj_kind.element_kind() {
                    return Ok(elem.clone());
                }
                return Ok(InferredKind::dynamic());
            }
            // Otherwise analyze index for side-effects and return dynamic
            let _ = super::node::analyze_node(index, tbl)?;
            Ok(InferredKind::dynamic())
        }
        _ => Err(Box::new(
            crate::analyzers::semantic::err::SemanticError::with(
                crate::error::Level::Error,
                format!(
                    "Unsupported expression node for analyze_expression: {}",
                    node.kind
                ),
                "mainstage.analyzers.semantic.expr.analyze_expression".to_string(),
                node.location.clone(),
                node.span.clone(),
            ),
        )),
    }
}

/// Analyze an `if` node: check condition is boolean-compatible and analyze the body.
pub(crate) fn analyze_if(
    node: &mut crate::ast::AstNode,
    tbl: &mut crate::analyzers::semantic::table::SymbolTable,
) -> Result<Option<InferredKind>, Box<dyn crate::error::MainstageErrorExt>> {
    if let crate::ast::AstNodeKind::If { condition, body } = &mut node.kind {
        let cond = super::node::analyze_node(condition, tbl)?.unwrap_or_else(InferredKind::dynamic);
        // condition must be boolean-compatible
        if !InferredKind::boolean().is_compatible_with(&cond) && !cond.is_dynamic() {
            return Err(Box::new(
                crate::analyzers::semantic::err::SemanticError::with(
                    crate::error::Level::Error,
                    format!("If condition must be boolean, found {}", cond),
                    "mainstage.analyzers.semantic.expr.analyze_if".to_string(),
                    node.location.clone(),
                    node.span.clone(),
                ),
            ));
        }

        // analyze body within its own scope
        tbl.enter_scope();
        super::node::analyze_node(body, tbl)?;
        tbl.exit_scope();
        Ok(None)
    } else {
        Err(Box::new(
            crate::analyzers::semantic::err::SemanticError::with(
                crate::error::Level::Error,
                format!("Expected If node, found: {}", node.kind),
                "mainstage.analyzers.semantic.expr.analyze_if".to_string(),
                node.location.clone(),
                node.span.clone(),
            ),
        ))
    }
}

/// Analyze an IfElse node: check condition, analyze both branches.
pub(crate) fn analyze_ifelse(
    node: &mut crate::ast::AstNode,
    tbl: &mut crate::analyzers::semantic::table::SymbolTable,
) -> Result<Option<InferredKind>, Box<dyn crate::error::MainstageErrorExt>> {
    if let crate::ast::AstNodeKind::IfElse {
        condition,
        if_body,
        else_body,
    } = &mut node.kind
    {
        let cond = super::node::analyze_node(condition, tbl)?.unwrap_or_else(InferredKind::dynamic);
        if !InferredKind::boolean().is_compatible_with(&cond) && !cond.is_dynamic() {
            return Err(Box::new(
                crate::analyzers::semantic::err::SemanticError::with(
                    crate::error::Level::Error,
                    format!("If condition must be boolean, found {}", cond),
                    "mainstage.analyzers.semantic.expr.analyze_ifelse".to_string(),
                    node.location.clone(),
                    node.span.clone(),
                ),
            ));
        }

        tbl.enter_scope();
        super::node::analyze_node(if_body, tbl)?;
        tbl.exit_scope();

        tbl.enter_scope();
        super::node::analyze_node(else_body, tbl)?;
        tbl.exit_scope();

        Ok(None)
    } else {
        Err(Box::new(
            crate::analyzers::semantic::err::SemanticError::with(
                crate::error::Level::Error,
                format!("Expected IfElse node, found: {}", node.kind),
                "mainstage.analyzers.semantic.expr.analyze_ifelse".to_string(),
                node.location.clone(),
                node.span.clone(),
            ),
        ))
    }
}

/// Analyze a ForIn loop: create iterator variable in new scope and infer its type from iterable.
pub(crate) fn analyze_forin(
    node: &mut crate::ast::AstNode,
    tbl: &mut crate::analyzers::semantic::table::SymbolTable,
) -> Result<Option<InferredKind>, Box<dyn crate::error::MainstageErrorExt>> {
    if let crate::ast::AstNodeKind::ForIn {
        iterator,
        iterable,
        body,
    } = &mut node.kind
    {
        // evaluate iterable
        let iter_kind =
            super::node::analyze_node(iterable, tbl)?.unwrap_or_else(InferredKind::dynamic);

        // infer element kind
        let elem_kind = if iter_kind.kind == crate::analyzers::semantic::kind::Kind::Array {
            if let Some(e) = iter_kind.element_kind() {
                e.clone()
            } else {
                InferredKind::dynamic()
            }
        } else {
            // unknown iterable -> dynamic element
            InferredKind::dynamic()
        };

        // analyze body in a new scope with iterator variable inserted
        tbl.enter_scope();
        tbl.insert_symbol(Symbol::new_variable(
            iterator.clone(),
            Some(elem_kind),
            node.location.clone(),
            node.span.clone(),
        ));

        super::node::analyze_node(body, tbl)?;
        tbl.exit_scope();
        Ok(None)
    } else {
        Err(Box::new(
            crate::analyzers::semantic::err::SemanticError::with(
                crate::error::Level::Error,
                format!("Expected ForIn node, found: {}", node.kind),
                "mainstage.analyzers.semantic.expr.analyze_forin".to_string(),
                node.location.clone(),
                node.span.clone(),
            ),
        ))
    }
}

/// Analyze a ForTo loop: run initializer in a new scope so loop variable is local, check numeric types.
pub(crate) fn analyze_forto(
    node: &mut crate::ast::AstNode,
    tbl: &mut crate::analyzers::semantic::table::SymbolTable,
) -> Result<Option<InferredKind>, Box<dyn crate::error::MainstageErrorExt>> {
    if let crate::ast::AstNodeKind::ForTo {
        initializer,
        limit,
        body,
    } = &mut node.kind
    {
        tbl.enter_scope();
        // initializer may create the loop variable
        super::node::analyze_node(initializer, tbl)?;

        let limit_kind =
            super::node::analyze_node(limit, tbl)?.unwrap_or_else(InferredKind::dynamic);
        if !limit_kind.is_numeric() && !limit_kind.is_dynamic() {
            return Err(Box::new(
                crate::analyzers::semantic::err::SemanticError::with(
                    crate::error::Level::Error,
                    format!("For-to loop limit must be numeric, found {}", limit_kind),
                    "mainstage.analyzers.semantic.expr.analyze_forto".to_string(),
                    node.location.clone(),
                    node.span.clone(),
                ),
            ));
        }

        super::node::analyze_node(body, tbl)?;
        tbl.exit_scope();
        Ok(None)
    } else {
        Err(Box::new(
            crate::analyzers::semantic::err::SemanticError::with(
                crate::error::Level::Error,
                format!("Expected ForTo node, found: {}", node.kind),
                "mainstage.analyzers.semantic.expr.analyze_forto".to_string(),
                node.location.clone(),
                node.span.clone(),
            ),
        ))
    }
}

/// Analyze a While loop: check condition and analyze body in new scope.
pub(crate) fn analyze_while(
    node: &mut crate::ast::AstNode,
    tbl: &mut crate::analyzers::semantic::table::SymbolTable,
) -> Result<Option<InferredKind>, Box<dyn crate::error::MainstageErrorExt>> {
    if let crate::ast::AstNodeKind::While { condition, body } = &mut node.kind {
        let cond = super::node::analyze_node(condition, tbl)?.unwrap_or_else(InferredKind::dynamic);
        if !InferredKind::boolean().is_compatible_with(&cond) && !cond.is_dynamic() {
            return Err(Box::new(
                crate::analyzers::semantic::err::SemanticError::with(
                    crate::error::Level::Error,
                    format!("While condition must be boolean, found {}", cond),
                    "mainstage.analyzers.semantic.expr.analyze_while".to_string(),
                    node.location.clone(),
                    node.span.clone(),
                ),
            ));
        }

        tbl.enter_scope();
        super::node::analyze_node(body, tbl)?;
        tbl.exit_scope();
        Ok(None)
    } else {
        Err(Box::new(
            crate::analyzers::semantic::err::SemanticError::with(
                crate::error::Level::Error,
                format!("Expected While node, found: {}", node.kind),
                "mainstage.analyzers.semantic.expr.analyze_while".to_string(),
                node.location.clone(),
                node.span.clone(),
            ),
        ))
    }
}

/// Analyze a Return node: evaluate the optional value and return its inferred kind
/// up the analyzer chain so enclosing scopes (functions/stages) can inspect it.
pub(crate) fn analyze_return(
    node: &mut crate::ast::AstNode,
    tbl: &mut crate::analyzers::semantic::table::SymbolTable,
) -> Result<Option<InferredKind>, Box<dyn crate::error::MainstageErrorExt>> {
    if let crate::ast::AstNodeKind::Return { value } = &mut node.kind {
        if let Some(v) = value {
            let k = super::node::analyze_node(v, tbl)?.unwrap_or_else(InferredKind::dynamic);
            return Ok(Some(k));
        } else {
            // explicit return with no value -> Void
            return Ok(Some(InferredKind::new(
                Kind::Void,
                Origin::Expression,
                node.location.clone(),
                node.span.clone(),
            )));
        }
    }

    Err(Box::new(
        crate::analyzers::semantic::err::SemanticError::with(
            crate::error::Level::Error,
            format!("Expected Return node, found: {}", node.kind),
            "mainstage.analyzers.semantic.expr.analyze_return".to_string(),
            node.location.clone(),
            node.span.clone(),
        ),
    ))
}

/// Collect return kinds from a node (recursively). Returns unified InferredKind if any returns found.
pub(crate) fn collect_returns(
    node: &mut crate::ast::AstNode,
    tbl: &mut crate::analyzers::semantic::table::SymbolTable,
) -> Result<Option<InferredKind>, Box<dyn crate::error::MainstageErrorExt>> {
    use crate::ast::AstNodeKind;

    match &mut node.kind {
        AstNodeKind::Return { .. } => {
            // analyze_return returns Ok(Some(kind)) on success
            let k = analyze_return(node, tbl)?;
            Ok(k)
        }
        AstNodeKind::Block { statements } => {
            let mut ret_kind: Option<InferredKind> = None;
            for stmt in statements.iter_mut() {
                if let Some(k) = collect_returns(stmt, tbl)? {
                    if let Some(existing) = ret_kind {
                        let unified = existing.unify(&k);
                        if unified.is_dynamic() && !existing.is_dynamic() && !k.is_dynamic() {
                            return Err(Box::new(
                                crate::analyzers::semantic::err::SemanticError::with(
                                    crate::error::Level::Error,
                                    format!(
                                        "Conflicting return types in block: {} vs {}",
                                        existing, k
                                    ),
                                    "mainstage.analyzers.semantic.expr.collect_returns".to_string(),
                                    node.location.clone(),
                                    node.span.clone(),
                                ),
                            ));
                        }
                        ret_kind = Some(unified);
                    } else {
                        ret_kind = Some(k);
                    }
                }
            }
            Ok(ret_kind)
        }
        AstNodeKind::If { condition: _, body } => collect_returns(body, tbl),
        AstNodeKind::IfElse {
            condition: _,
            if_body,
            else_body,
        } => {
            let a = collect_returns(if_body, tbl)?;
            let b = collect_returns(else_body, tbl)?;
            match (a, b) {
                (None, None) => Ok(None),
                (Some(x), None) => Ok(Some(x)),
                (None, Some(y)) => Ok(Some(y)),
                (Some(x), Some(y)) => {
                    let unified = x.unify(&y);
                    if unified.is_dynamic() && !x.is_dynamic() && !y.is_dynamic() {
                        return Err(Box::new(
                            crate::analyzers::semantic::err::SemanticError::with(
                                crate::error::Level::Error,
                                format!("Conflicting return types in branches: {} vs {}", x, y),
                                "mainstage.analyzers.semantic.expr.collect_returns".to_string(),
                                node.location.clone(),
                                node.span.clone(),
                            ),
                        ));
                    }
                    Ok(Some(unified))
                }
            }
        }
        AstNodeKind::ForIn { body, .. }
        | AstNodeKind::ForTo { body, .. }
        | AstNodeKind::While { condition: _, body } => collect_returns(body, tbl),
        // For other nodes, we don't collect returns here
        _ => Ok(None),
    }
}
