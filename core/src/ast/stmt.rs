//! file: core/src/ast/stmt.rs
//! description: parsing helpers for top-level items and statements.
//!
//! This module contains functions that parse `item` and `statement` rules
//! from the `pest`-generated `RulesParser` into `AstNode` structures.
//! Parsing helpers attach `Location`/`Span` metadata using the `rules`
//! helpers to aid diagnostics.
//!
use crate::{
    ast::{AstNode, AstNodeKind, BinaryOperator, MainstageErrorExt, Rule, rules},
    script,
};

/// Parse a single top-level `item` rule into an `AstNode`.
///
/// This function consumes a `pest::iterators::Pair<Rule>` whose rule is
/// `item` and returns the corresponding AST node. An `item` in the grammar can
/// be either a `statement` or a `declaration`. The returned `AstNode` carries
/// the `location` and `span` information produced by the `rules` helpers so
/// error reporting can point back to precise positions in the source file.
///
/// # Arguments
///
/// - `pair`: The `pest::iterators::Pair<Rule>` representing the `item` rule.
/// - `script`: The script context used for source text, file name, and error
///   reporting.
///
/// # Returns
///
/// - `Ok(AstNode)` when parsing succeeds.
/// - `Err(Box<dyn MainstageErrorExt>)` when a parse or semantic error occurs.
///
/// # Errors
///
/// Returns a `SyntaxError` (wrapped in `MainstageErrorExt`) when the inner
/// rule is not one of the expected alternatives (`statement`, `declaration`,
/// or `EOI`). The error includes `Location` and `Span` where available.
///
pub(crate) fn parse_item_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (mut inner_pairs, location, span) = rules::get_data_from_rule(&pair, script);

    // Handle empty pairs, which may occur in the case of EOI
    if inner_pairs.peek().is_none() {
        return Ok(AstNode::new(AstNodeKind::Null, None, None));
    }

    let next_rule = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
    match next_rule.as_rule() {
        Rule::statement => parse_statement_rule(next_rule, script),
        Rule::declaration => parse_declaration_rule(next_rule, script),
        Rule::EOI => Ok(AstNode::new(AstNodeKind::Null, location, span)),
        _ => Err(Box::<dyn MainstageErrorExt>::from(Box::new(
            crate::ast::err::SyntaxError::with(
                crate::Level::Error,
                "Unexpected rule in item parsing.".into(),
                "mainstage.stmt.parse_item_rule".into(),
                location,
                span,
            ),
        )))?,
    }
}

fn parse_statement_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (mut inner_pairs, location, span) = rules::get_data_from_rule(&pair, script);
    let next_rule = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
    match next_rule.as_rule() {
        Rule::terminated_statement => parse_terminated_statement_rule(next_rule, script),
        Rule::loop_stmt => parse_loop_statement_rule(next_rule, script),
        Rule::conditional_stmt => parse_conditional_statement_rule(next_rule, script),
        Rule::block => parse_block_rule(next_rule, script),
        _ => Err(Box::<dyn MainstageErrorExt>::from(Box::new(
            crate::ast::err::SyntaxError::with(
                crate::Level::Error,
                format!("Unexpected statement type: {:?}", next_rule.as_rule()).into(),
                "mainstage.stmt.parse_statement_rule".into(),
                location,
                span,
            ),
        ))),
    }
}

fn parse_terminated_statement_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (mut inner_pairs, location, span) = rules::get_data_from_rule(&pair, script);
    let next_rule = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
    match next_rule.as_rule() {
        Rule::include_stmt => Ok(AstNode::new(
            AstNodeKind::Include {
                file: next_rule.as_str().to_string(),
            },
            location,
            span,
        )),
        Rule::import_stmt => {
            let mut inner = next_rule.into_inner();
            let module_pair = inner.next().unwrap();
            let alias_pair = inner.next();
            // alias can be identifier or literal "*" per grammar
            let alias = match alias_pair {
                Some(ap) => ap.as_str().to_string(),
                None => module_pair.as_str().to_string(),
            };
            Ok(AstNode::new(
                AstNodeKind::Import {
                    module: module_pair.as_str().to_string(),
                    alias,
                },
                location,
                span,
            ))
        }
        Rule::assignment_stmt => parse_assignment_statement_rule(next_rule, script),
        Rule::expression_stmt => super::expr::parse_expression_rule(next_rule, script),
        Rule::return_stmt => {
            // Parse inner expression for return value
            let mut inner = next_rule.into_inner();
            // The grammar for return_stmt is: "return" ~ expression ~ ";"
            // The first inner pair should be the expression
            if let Some(expr_pair) = inner.next() {
                let expr_node = super::expr::parse_expression_rule(expr_pair, script)?;
                Ok(AstNode::new(
                    AstNodeKind::Return {
                        value: Some(Box::new(expr_node)),
                    },
                    location,
                    span,
                ))
            } else {
                // No expression found: treat as returning nothing
                Ok(AstNode::new(
                    AstNodeKind::Return { value: None },
                    location,
                    span,
                ))
            }
        }
        _ => Err(Box::<dyn MainstageErrorExt>::from(Box::new(
            crate::ast::err::SyntaxError::with(
                crate::Level::Error,
                "Unexpected terminated statement type.".into(),
                "mainstage.stmt.parse_terminated_statement_rule".into(),
                location,
                span,
            ),
        ))),
    }
}

fn parse_assignment_statement_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (mut inner_pairs, location, span) = rules::get_data_from_rule(&pair, script);
    let identifier_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
    let op_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
    let expr_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;

    // Parse target and value once so we can reuse/cloned for compound ops.
    let target_node = super::expr::parse_identifier_rule(identifier_pair, script)?;
    let value_node = super::expr::parse_expression_rule(expr_pair, script)?;

    match op_pair.as_str() {
        "=" => Ok(AstNode::new(
            AstNodeKind::Assignment {
                target: Box::new(target_node),
                value: Box::new(value_node),
            },
            location,
            span,
        )),

        // Compound assignments become `target = target <op> value`
        "+=" | "-=" | "*=" | "/=" | "%=" => {
            let op = match op_pair.as_str() {
                "+=" => BinaryOperator::Add,
                "-=" => BinaryOperator::Sub,
                "*=" => BinaryOperator::Mul,
                "/=" => BinaryOperator::Div,
                "%=" => BinaryOperator::Mod,
                _ => unreachable!(),
            };

            // left side of binary op uses a clone of the identifier node
            let left_clone = target_node.clone();
            let binary_node = AstNode::new(
                AstNodeKind::BinaryOp {
                    left: Box::new(left_clone),
                    op,
                    right: Box::new(value_node),
                },
                location.clone(),
                span.clone(),
            );

            Ok(AstNode::new(
                AstNodeKind::Assignment {
                    target: Box::new(target_node),
                    value: Box::new(binary_node),
                },
                location,
                span,
            ))
        }

        _ => {
            return Err(Box::<dyn MainstageErrorExt>::from(Box::new(
                crate::ast::err::SyntaxError::with(
                    crate::Level::Error,
                    "Expected assignment operator.".into(),
                    "mainstage.stmt.parse_assignment_statement_rule".into(),
                    location,
                    span,
                ),
            )));
        }
    }
}

fn parse_declaration_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (mut inner_pairs, location, span) = rules::get_data_from_rule(&pair, script);
    let next_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
    let mut inner_pairs = next_pair.clone().into_inner();
    match next_pair.as_rule() {
        Rule::workspace_decl => {
            // attributes are optional; only consume them if present
            let attributes = if inner_pairs
                .peek()
                .map(|p| p.as_rule() == Rule::attributes)
                .unwrap_or(false)
            {
                let attrs_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
                parse_attributes_rule(attrs_pair, script)
            } else {
                vec![]
            };
            let identifier_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
            let body_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
            Ok(AstNode::new(
                AstNodeKind::Workspace {
                    name: identifier_pair.as_str().to_string(),
                    body: Box::new(parse_block_rule(body_pair, script)?),
                },
                location,
                span,
            )
            .with_attributes(attributes))
        }
        Rule::project_decl => {
            // attributes are optional; only consume them if present
            let attributes = if inner_pairs
                .peek()
                .map(|p| p.as_rule() == Rule::attributes)
                .unwrap_or(false)
            {
                let attrs_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
                parse_attributes_rule(attrs_pair, script)
            } else {
                vec![]
            };
            let identifier_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
            let body_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
            Ok(AstNode::new(
                AstNodeKind::Project {
                    name: identifier_pair.as_str().to_string(),
                    body: Box::new(parse_block_rule(body_pair, script)?),
                },
                location,
                span,
            )
            .with_attributes(attributes))
        }
        Rule::stage_decl => {
            // attributes are optional; only consume them if present
            let attributes = if inner_pairs
                .peek()
                .map(|p| p.as_rule() == Rule::attributes)
                .unwrap_or(false)
            {
                let attrs_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
                parse_attributes_rule(attrs_pair, script)
            } else {
                vec![]
            };
            let identifier_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
            let mut args_pair = None;
            let mut body_pair = None;
            while let Some(pair) = inner_pairs.next() {
                match pair.as_rule() {
                    Rule::arguments => {
                        args_pair = Some(pair);
                    }
                    Rule::block => {
                        body_pair = Some(pair);
                    }
                    _ => {}
                }
            }
            let args = match args_pair {
                Some(pair) => Some(Box::new(parse_arguments_rule(pair, script)?)),
                None => None,
            };
            let body = match body_pair {
                Some(pair) => Some(Box::new(parse_block_rule(pair, script)?)),
                None => None,
            };
            Ok(AstNode::new(
                AstNodeKind::Stage {
                    name: identifier_pair.as_str().to_string(),
                    args,
                    body: body.expect("Stage declaration must have a body"),
                },
                location,
                span,
            )
            .with_attributes(attributes))
        }
        _ => Err(Box::<dyn MainstageErrorExt>::from(Box::new(
            crate::ast::err::SyntaxError::with(
                crate::Level::Error,
                "Unexpected declaration type.".into(),
                "mainstage.stmt.parse_declaration_rule".into(),
                location,
                span,
            ),
        ))),
    }
}

fn parse_attributes_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Vec<String> {
    // If the provided pair is not an attributes node, return empty.
    if pair.as_rule() != Rule::attributes {
        return vec![];
    }

    let (inner_pairs, _location, _span) = rules::get_data_from_rule(&pair, script);
    // Collect identifier text for each attribute; join with commas.
    let mut attrs: Vec<String> = Vec::new();
    for p in inner_pairs {
        // attribute -> identifier, but be defensive and inspect inner
        if p.as_rule() == Rule::attribute {
            if let Some(id_pair) = p.into_inner().next() {
                attrs.push(id_pair.as_str().to_string());
            }
        } else if p.as_rule() == Rule::identifier {
            attrs.push(p.as_str().to_string());
        }
    }
    attrs
}

fn parse_arguments_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (inner_pairs, location, span) = rules::get_data_from_rule(&pair, script);
    let mut args = Vec::new();

    for arg_pair in inner_pairs {
        // Inner pairs are parameter expressions. Unwrap into expression nodes, then parse.
        let expr_pair = arg_pair.into_inner().next().unwrap();
        let expr_node = super::expr::parse_expression_rule(expr_pair, script)?;
        args.push(expr_node);
    }

    Ok(AstNode::new(
        AstNodeKind::Arguments { args },
        location,
        span,
    ))
}

fn parse_block_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (inner_pairs, _location, _span) = rules::get_data_from_rule(&pair, script);
    let mut body = Vec::new();

    for stmt_pair in inner_pairs {
        let stmt_node = parse_statement_rule(stmt_pair, script)?;
        body.push(stmt_node);
    }

    Ok(AstNode::new(
        AstNodeKind::Block { statements: body },
        None,
        None,
    ))
}

fn parse_loop_statement_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (mut inner_pairs, location, span) = rules::get_data_from_rule(&pair, script);
    let next_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
    match next_pair.as_rule() {
        Rule::for_in_stmt => parse_for_in_statement_rule(next_pair, script),
        Rule::for_to_stmt => parse_for_to_statement_rule(next_pair, script),
        Rule::while_stmt => parse_while_statement_rule(next_pair, script),
        _ => Err(Box::<dyn MainstageErrorExt>::from(Box::new(
            crate::ast::err::SyntaxError::with(
                crate::Level::Error,
                "Unexpected loop statement type.".into(),
                "mainstage.stmt.parse_loop_statement_rule".into(),
                location,
                span,
            ),
        ))),
    }
}

fn parse_for_in_statement_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (mut inner_pairs, location, span) = rules::get_data_from_rule(&pair, script);
    let iterator_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
    let iterable_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
    let body_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;

    let iterable_node = super::expr::parse_expression_rule(iterable_pair, script)?;
    let body_node = parse_block_rule(body_pair, script)?;

    Ok(AstNode::new(
        AstNodeKind::ForIn {
            iterator: iterator_pair.as_str().to_string(),
            iterable: Box::new(iterable_node),
            body: Box::new(body_node),
        },
        location,
        span,
    ))
}

fn parse_for_to_statement_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    // Placeholder implementation
    let (mut inner_pairs, location, span) = rules::get_data_from_rule(&pair, script);
    let initializer_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
    let limit_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
    let body_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;

    let initializer_node = super::expr::parse_expression_rule(initializer_pair, script)?;
    let limit_node = super::expr::parse_expression_rule(limit_pair, script)?;
    let body_node = parse_block_rule(body_pair, script)?;

    Ok(AstNode::new(
        AstNodeKind::ForTo {
            initializer: Box::new(initializer_node),
            limit: Box::new(limit_node),
            body: Box::new(body_node),
        },
        location,
        span,
    ))
}

fn parse_while_statement_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (mut inner_pairs, location, span) = rules::get_data_from_rule(&pair, script);
    let condition_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
    let body_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;

    let condition_node = super::expr::parse_expression_rule(condition_pair, script)?;
    let body_node = parse_block_rule(body_pair, script)?;

    Ok(AstNode::new(
        AstNodeKind::While {
            condition: Box::new(condition_node),
            body: Box::new(body_node),
        },
        location,
        span,
    ))
}

fn parse_conditional_statement_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (mut inner_pairs, location, span) = rules::get_data_from_rule(&pair, script);
    let next_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
    match next_pair.as_rule() {
        Rule::if_stmt => parse_if_statement_rule(next_pair, script),
        Rule::if_else_stmt => parse_if_else_statement_rule(next_pair, script),
        _ => Err(Box::<dyn MainstageErrorExt>::from(Box::new(
            crate::ast::err::SyntaxError::with(
                crate::Level::Error,
                "Unexpected conditional statement type.".into(),
                "mainstage.stmt.parse_conditional_statement_rule".into(),
                location,
                span,
            ),
        ))),
    }
}

fn parse_if_statement_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (mut inner_pairs, location, span) = rules::get_data_from_rule(&pair, script);
    let condition_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
    let body_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;

    let condition_node = super::expr::parse_expression_rule(condition_pair, script)?;
    let body_node = parse_block_rule(body_pair, script)?;

    Ok(AstNode::new(
        AstNodeKind::If {
            condition: Box::new(condition_node),
            body: Box::new(body_node),
        },
        location,
        span,
    ))
}

fn parse_if_else_statement_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (mut inner_pairs, location, span) = rules::get_data_from_rule(&pair, script);
    let condition_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
    let if_body_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
    let else_body_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;

    let condition_node = super::expr::parse_expression_rule(condition_pair, script)?;
    let if_body_node = parse_block_rule(if_body_pair, script)?;
    let else_body_node = parse_block_rule(else_body_pair, script)?;

    Ok(AstNode::new(
        AstNodeKind::IfElse {
            condition: Box::new(condition_node),
            if_body: Box::new(if_body_node),
            else_body: Box::new(else_body_node),
        },
        location,
        span,
    ))
}
