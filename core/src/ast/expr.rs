//! file: core/src/ast/expr.rs
//! description: parsing helpers for expression-level rules.
//!
//! Contains functions that convert `pest` parse `Pair`s for expressions
//! into `AstNode` trees. These helpers preserve `Location` and `Span`
//! information for diagnostics and support nested/compound expressions.
//!
use crate::{
    ast::{AstNode, AstNodeKind, BinaryOperator, UnaryOperator,MainstageErrorExt, Rule, rules},
    script,
};

pub(crate) fn parse_expression_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (mut inner_pairs, location, span) = rules::get_data_from_rule(&pair, script);
    let eq_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
    match eq_pair.as_rule() {
        Rule::expression => parse_expression_rule(eq_pair, script),
        Rule::equality_expression => {
            super::expr::parse_equality_expression_rule(eq_pair, script)
        }
        _ => Err(Box::<dyn MainstageErrorExt>::from(Box::new(
            crate::ast::err::SyntaxError::with(
                crate::Level::Error,
                format!("Unexpected expression type. {:?}", eq_pair.as_rule()),
                "mainstage.expr.parse_expression_rule".into(),
                location,
                span,
            ),
        ))),
    }
}

fn parse_equality_expression_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (mut inner_pairs, location, span) = rules::get_data_from_rule(&pair, script);

    let left_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
    let mut node = parse_relational_expression_rule(left_pair, script)?;

    // Handle zero-or-more (op, right) repetitions
    while let Some(op_pair) = inner_pairs.next() {
        let op = match op_pair.as_str() {
            "==" => BinaryOperator::Eq,
            "!=" => BinaryOperator::Ne,
            _ => {
                return Err(Box::<dyn MainstageErrorExt>::from(Box::new(
                    crate::ast::err::SyntaxError::with(
                        crate::Level::Error,
                        "Invalid equality operator.".into(),
                        "mainstage.expr.parse_equality_expression_rule".into(),
                        location.clone(),
                        span.clone(),
                    ),
                )))
            }
        };
        let right_pair = match inner_pairs.next() {
            Some(rp) => rp,
            None => {
                return Err(Box::<dyn MainstageErrorExt>::from(Box::new(
                    crate::ast::err::SyntaxError::with(
                        crate::Level::Error,
                        "Missing right-hand operand for equality operator.".into(),
                        "mainstage.expr.parse_equality_expression_rule".into(),
                        location.clone(),
                        span.clone(),
                    ),
                )))
            }
        };
        let right_node = parse_relational_expression_rule(right_pair, script)?;

        node = AstNode::new(
            AstNodeKind::BinaryOp {
                left: Box::new(node),
                op,
                right: Box::new(right_node),
            },
            rules::get_location_from_pair(&op_pair, script),
            rules::get_span_from_pair(&op_pair, script),
        );
    }

    Ok(node)
}

fn parse_relational_expression_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (mut inner_pairs, location, span) = rules::get_data_from_rule(&pair, script);

    let left_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
    let mut node = parse_additive_expression_rule(left_pair, script)?;

    // Handle zero-or-more (op, right) repetitions
    while let Some(op_pair) = inner_pairs.next() {
        let op = match op_pair.as_str() {
            "<" => BinaryOperator::Lt,
            "<=" => BinaryOperator::Le,
            ">" => BinaryOperator::Gt,
            ">=" => BinaryOperator::Ge,
            _ => {
                return Err(Box::<dyn MainstageErrorExt>::from(Box::new(
                    crate::ast::err::SyntaxError::with(
                        crate::Level::Error,
                        "Invalid relational operator.".into(),
                        "mainstage.expr.parse_relational_expression_rule".into(),
                        location.clone(),
                        span.clone(),
                    ),
                )))
            }
        };

        let right_pair = match inner_pairs.next() {
            Some(rp) => rp,
            None => {
                return Err(Box::<dyn MainstageErrorExt>::from(Box::new(
                    crate::ast::err::SyntaxError::with(
                        crate::Level::Error,
                        "Missing right-hand operand for relational operator.".into(),
                        "mainstage.expr.parse_relational_expression_rule".into(),
                        location.clone(),
                        span.clone(),
                    ),
                )))
            }
        };
        let right_node = parse_additive_expression_rule(right_pair, script)?;

        node = AstNode::new(
            AstNodeKind::BinaryOp {
                left: Box::new(node),
                op,
                right: Box::new(right_node),
            },
            rules::get_location_from_pair(&op_pair, script),
            rules::get_span_from_pair(&op_pair, script),
        );
    }

    Ok(node)
}

fn parse_additive_expression_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (mut inner_pairs, location, span) = rules::get_data_from_rule(&pair, script);

    let left_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
    let mut node = parse_multiplicative_expression_rule(left_pair, script)?;

    // Handle zero-or-more (op, right) repetitions
    while let Some(op_pair) = inner_pairs.next() {
        let op = match op_pair.as_str() {
            "+" => BinaryOperator::Add,
            "-" => BinaryOperator::Sub,
            _ => {
                return Err(Box::<dyn MainstageErrorExt>::from(Box::new(
                    crate::ast::err::SyntaxError::with(
                        crate::Level::Error,
                        "Invalid additive operator.".into(),
                        "mainstage.expr.parse_additive_expression_rule".into(),
                        location.clone(),
                        span.clone(),
                    ),
                )))
            }
        };

        let right_pair = match inner_pairs.next() {
            Some(rp) => rp,
            None => {
                return Err(Box::<dyn MainstageErrorExt>::from(Box::new(
                    crate::ast::err::SyntaxError::with(
                        crate::Level::Error,
                        "Missing right-hand operand for additive operator.".into(),
                        "mainstage.expr.parse_additive_expression_rule".into(),
                        location.clone(),
                        span.clone(),
                    ),
                )))
            }
        };
        let right_node = parse_multiplicative_expression_rule(right_pair, script)?;

        node = AstNode::new(
            AstNodeKind::BinaryOp {
                left: Box::new(node),
                op,
                right: Box::new(right_node),
            },
            rules::get_location_from_pair(&op_pair, script),
            rules::get_span_from_pair(&op_pair, script),
        );
    }

    Ok(node)
}

fn parse_multiplicative_expression_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (mut inner_pairs, location, span) = rules::get_data_from_rule(&pair, script);

    // First term is required
    let first_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
    let mut node = parse_unary_expression_rule(first_pair, script)?;

    // Handle zero-or-more (op, right) repetitions
    while let Some(op_pair) = inner_pairs.next() {
        // op_pair should be an operator; next() must provide the right-hand operand
        let op = match op_pair.as_str() {
            "*" => BinaryOperator::Mul,
            "/" => BinaryOperator::Div,
            "%" => BinaryOperator::Mod,
            _ => {
                return Err(Box::<dyn MainstageErrorExt>::from(Box::new(
                    crate::ast::err::SyntaxError::with(
                        crate::Level::Error,
                        "Invalid multiplicative operator.".into(),
                        "mainstage.expr.parse_multiplicative_expression_rule".into(),
                        location.clone(),
                        span.clone(),
                    ),
                )))
            }
        };

        let right_pair = match inner_pairs.next() {
            Some(rp) => rp,
            None => {
                return Err(Box::<dyn MainstageErrorExt>::from(Box::new(
                    crate::ast::err::SyntaxError::with(
                        crate::Level::Error,
                        "Missing right-hand operand for binary operator.".into(),
                        "mainstage.expr.parse_multiplicative_expression_rule".into(),
                        location.clone(),
                        span.clone(),
                    ),
                )))
            }
        };
        let right_node = parse_unary_expression_rule(right_pair, script)?;

        node = AstNode::new(
            AstNodeKind::BinaryOp {
                left: Box::new(node),
                op,
                right: Box::new(right_node),
            },
            rules::get_location_from_pair(&op_pair, script),
            rules::get_span_from_pair(&op_pair, script),
        );
    }

    Ok(node)
}

fn parse_unary_expression_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (mut inner_pair, location, span) = rules::get_data_from_rule(&pair, script);
    let next_rule = rules::fetch_next_pair(&mut inner_pair, &location, &span)?;
    match next_rule.as_rule() {
        Rule::unary_op => {
            let mut inner_pairs = next_rule.into_inner();
            let op_pair = inner_pairs.next().unwrap();
            let expr_pair = inner_pairs.next().unwrap();

            Ok(AstNode::new(
                AstNodeKind::UnaryOp {
                    op: match op_pair.as_str() {
                        "++" => UnaryOperator::Inc,
                        "--" => UnaryOperator::Dec,
                        "-" => UnaryOperator::Minus,
                        "+" => UnaryOperator::Plus,
                        "!" => UnaryOperator::Not,
                        _ => {
                            return Err(Box::<dyn MainstageErrorExt>::from(Box::new(
                                crate::ast::err::SyntaxError::with(
                                    crate::Level::Error,
                                    "Invalid unary operator.".into(),
                                    "mainstage.expr.parse_unary_expression_rule".into(),
                                    location.clone(),
                                    span.clone(),
                                ),
                            )))
                        }
                    },
                    expr: Box::new(parse_unary_expression_rule(expr_pair, script)?),
                },
                location,
                span,
            ))
        }
        Rule::postfix_expression => parse_postfix_expression_rule(next_rule, script),
        _ => Err(Box::<dyn MainstageErrorExt>::from(Box::new(
            crate::ast::err::SyntaxError::with(
                crate::Level::Error,
                "Unexpected unary expression type.".into(),
                "mainstage.expr.parse_unary_expression_rule".into(),
                location,
                span,
            ),
        ))),
    }
}

fn parse_postfix_expression_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (mut inner_pair, location, span) = rules::get_data_from_rule(&pair, script);
    let next_rule = rules::fetch_next_pair(&mut inner_pair, &location, &span)?;
    // Start with the primary expression, then apply zero-or-more postfix ops
    let mut node = parse_primary_expression_rule(next_rule, script)?;

    // Remaining inner pairs (if any) are postfix_op instances; apply them left-to-right.
    for op_pair in inner_pair {
        // Each op_pair is a postfix_op; inspect its inner contents to determine
        // whether it's a call, member access, index, or postfix inc/dec.
        let mut op_inner = op_pair.clone().into_inner();

        // If there are inner pairs, inspect the first one to decide.
        if let Some(first) = op_inner.next() {
            match first.as_rule() {
                Rule::arguments => {
                    // Call: build argument list
                    let mut args: Vec<AstNode> = Vec::new();
                    for arg_pair in first.into_inner() {
                        // each arg_pair is a parameter expression wrapper; extract inner expr
                        if let Some(expr_pair) = arg_pair.into_inner().next() {
                            let expr_node = parse_expression_rule(expr_pair, script)?;
                            args.push(expr_node);
                        }
                    }
                    node = AstNode::new(
                        AstNodeKind::Call { callee: Box::new(node), args },
                        rules::get_location_from_pair(&op_pair, script),
                        rules::get_span_from_pair(&op_pair, script),
                    );
                }
                Rule::identifier => {
                    // Member access like `.prop`
                    let prop_name = first.as_str().to_string();
                    node = AstNode::new(
                        AstNodeKind::Member { object: Box::new(node), property: prop_name },
                        rules::get_location_from_pair(&op_pair, script),
                        rules::get_span_from_pair(&op_pair, script),
                    );
                }
                Rule::expression => {
                    // Indexing like `[expr]`
                    let idx_node = parse_expression_rule(first, script)?;
                    node = AstNode::new(
                        AstNodeKind::Index { object: Box::new(node), index: Box::new(idx_node) },
                        rules::get_location_from_pair(&op_pair, script),
                        rules::get_span_from_pair(&op_pair, script),
                    );
                }
                _ => {
                    // Handle raw tokens such as postfix ++/-- which have no inner pair
                    let s = op_pair.as_str();
                    match s {
                        "++" => {
                            node = AstNode::new(
                                AstNodeKind::UnaryOp { op: UnaryOperator::Inc, expr: Box::new(node) },
                                rules::get_location_from_pair(&op_pair, script),
                                rules::get_span_from_pair(&op_pair, script),
                            );
                        }
                        "--" => {
                            node = AstNode::new(
                                AstNodeKind::UnaryOp { op: UnaryOperator::Dec, expr: Box::new(node) },
                                rules::get_location_from_pair(&op_pair, script),
                                rules::get_span_from_pair(&op_pair, script),
                            );
                        }
                        _ => {
                            return Err(Box::<dyn MainstageErrorExt>::from(Box::new(
                                crate::ast::err::SyntaxError::with(
                                    crate::Level::Error,
                                    "Unsupported postfix operator.".into(),
                                    "mainstage.expr.parse_postfix_expression_rule".into(),
                                    location.clone(),
                                    span.clone(),
                                ),
                            )));
                        }
                    }
                }
            }
        } else {
            // No inner pairs. This can happen for empty-call parentheses `()`
            // (e.g. `obj.fn()` with zero args). Treat `()` as a zero-argument
            // call; otherwise fall back to handling raw tokens like ++/--.
            let s = op_pair.as_str();
            if s.starts_with('(') {
                // Empty call: create Call node with no args
                node = AstNode::new(
                    AstNodeKind::Call { callee: Box::new(node), args: Vec::new() },
                    rules::get_location_from_pair(&op_pair, script),
                    rules::get_span_from_pair(&op_pair, script),
                );
            } else {
                match s {
                    "++" => {
                        node = AstNode::new(
                            AstNodeKind::UnaryOp { op: UnaryOperator::Inc, expr: Box::new(node) },
                            rules::get_location_from_pair(&op_pair, script),
                            rules::get_span_from_pair(&op_pair, script),
                        );
                    }
                    "--" => {
                        node = AstNode::new(
                            AstNodeKind::UnaryOp { op: UnaryOperator::Dec, expr: Box::new(node) },
                            rules::get_location_from_pair(&op_pair, script),
                            rules::get_span_from_pair(&op_pair, script),
                        );
                    }
                    _ => {
                        return Err(Box::<dyn MainstageErrorExt>::from(Box::new(
                            crate::ast::err::SyntaxError::with(
                                crate::Level::Error,
                                "Unsupported postfix operator with no inner rule.".into(),
                                "mainstage.expr.parse_postfix_expression_rule".into(),
                                location.clone(),
                                span.clone(),
                            ),
                        )));
                    }
                }
            }
        }
    }

    Ok(node)
}

fn parse_primary_expression_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (mut inner_pair, location, span) = rules::get_data_from_rule(&pair, script);
    let next_rule = rules::fetch_next_pair(&mut inner_pair, &location, &span)?;
    match next_rule.as_rule() {
        Rule::value => parse_value_rule(next_rule, script),
        Rule::expression => parse_expression_rule(next_rule, script),
        Rule::identifier => parse_identifier_rule(next_rule, script),
        _ => Err(Box::<dyn MainstageErrorExt>::from(Box::new(
            crate::ast::err::SyntaxError::with(
                crate::Level::Error,
                "Unexpected primary expression type.".into(),
                "mainstage.expr.parse_primary_expression_rule".into(),
                location,
                span,
            ),
        ))),
    }
}

fn parse_value_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (mut inner_pair, location, span) = rules::get_data_from_rule(&pair, script);
    let next_rule = rules::fetch_next_pair(&mut inner_pair, &location, &span)?;
    match next_rule.as_rule() {
        Rule::string => Ok(AstNode::new(
            AstNodeKind::String {
                value: next_rule.as_str()[1..next_rule.as_str().len()-1].to_string()
            },
            location,
            span,
        )),
        Rule::number => {
            let num_str = next_rule.as_str();
            if let Ok(int_value) = num_str.parse::<i64>() {
                Ok(AstNode::new(
                    AstNodeKind::Integer { value: int_value },
                    location,
                    span,
                ))
            } else if let Ok(float_value) = num_str.parse::<f64>() {
                Ok(AstNode::new(
                    AstNodeKind::Float { value: float_value },
                    location,
                    span,
                ))
            } else {
                Err(Box::<dyn MainstageErrorExt>::from(Box::new(
                    crate::ast::err::SyntaxError::with(
                        crate::Level::Error,
                        "Invalid number format.".into(),
                "mainstage.expr.parse_value_rule".into(),
                        location,
                        span,
                    ),
                )))
            }
        }
        Rule::boolean => {
            let bool_value = match next_rule.as_str() {
                "true" => true,
                "false" => false,
                _ => {
                    return Err(Box::<dyn MainstageErrorExt>::from(Box::new(
                        crate::ast::err::SyntaxError::with(
                            crate::Level::Error,
                            "Invalid boolean value.".into(),
                    "mainstage.expr.parse_value_rule".into(),
                            location,
                            span,
                        ),
                    )))
                }
            };
            Ok(AstNode::new(
                AstNodeKind::Bool { value: bool_value },
                location,
                span,
            ))
        }
        Rule::null => Ok(AstNode::new(
            AstNodeKind::Null,
            location,
            span,
        )),
        Rule::array => {
            let elements = next_rule
                .into_inner()
                .map(|elem_pair| parse_expression_rule(elem_pair, script))
                .collect::<Result<Vec<AstNode>, Box<dyn MainstageErrorExt>>>()?;
            Ok(AstNode::new(
                AstNodeKind::List { elements },
                location,
                span,
            ))
        }
        Rule::shell_string => {
            let mut inner_pairs = next_rule.into_inner();

            let shell_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;
            let content_pair = rules::fetch_next_pair(&mut inner_pairs, &location, &span)?;

            Ok(AstNode::new(
                AstNodeKind::Command {
                    name: shell_pair.as_str().to_string(),
                    arg: content_pair.as_str().to_string(),
                },
                location,
                span,
            ))
        }
        _ => Err(Box::<dyn MainstageErrorExt>::from(Box::new(
            crate::ast::err::SyntaxError::with(
                crate::Level::Error,
                "Unexpected value type.".into(),
                "mainstage.expr.parse_value_rule".into(),
                location,
                span,
            ),
        ))),
    }
}

pub(crate) fn parse_identifier_rule(
    pair: pest::iterators::Pair<Rule>,
    script: &script::Script,
) -> Result<AstNode, Box<dyn MainstageErrorExt>> {
    let (_, location, span) = rules::get_data_from_rule(&pair, script);
    Ok(AstNode::new(
        AstNodeKind::Identifier {
            name: pair.as_str().to_string(),
        },
        location,
        span,
    ))
}