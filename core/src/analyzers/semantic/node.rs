//! Node-level dispatcher for semantic analysis.
//!
//! Provides `analyze_node` which routes AST nodes to the appropriate
//! expression/statement analyzers and returns inferred kinds where relevant.

use super::kind::InferredKind;
use crate::ast::{AstNodeKind};

pub(crate) fn analyze_node(
    node: &mut crate::ast::AstNode,
    tbl: &mut crate::analyzers::semantic::table::SymbolTable,
) -> Result<Option<InferredKind>, Box<dyn crate::error::MainstageErrorExt>> {
    match &mut node.kind {
        AstNodeKind::Identifier { .. } => super::expr::analyze_identifier(node, tbl).map(Some),
        AstNodeKind::Assignment { .. } => super::expr::analyze_assignment(node, tbl).map(Some),
        AstNodeKind::Block { .. } => {
            super::expr::analyze_block(node, tbl)?;
            Ok(None)
        }
        AstNodeKind::Integer { .. }
        | AstNodeKind::Float { .. }
        | AstNodeKind::String { .. }
        | AstNodeKind::Bool { .. }
        | AstNodeKind::List { .. }
        | AstNodeKind::Null
        | AstNodeKind::UnaryOp { .. }
        | AstNodeKind::BinaryOp { .. }
        | AstNodeKind::Call { .. }
        | AstNodeKind::Member { .. }
        | AstNodeKind::Index { .. } => super::expr::analyze_expression(node, tbl).map(Some),
        AstNodeKind::If { .. } => super::expr::analyze_if(node, tbl),
        AstNodeKind::IfElse { .. } => super::expr::analyze_ifelse(node, tbl),
        AstNodeKind::ForIn { .. } => super::expr::analyze_forin(node, tbl),
        AstNodeKind::ForTo { .. } => super::expr::analyze_forto(node, tbl),
        AstNodeKind::While { .. } => super::expr::analyze_while(node, tbl),
        AstNodeKind::Return { .. } => super::expr::analyze_return(node, tbl),
        _ => {
            Err(Box::new(
                crate::analyzers::semantic::err::SemanticError::with(
                    crate::error::Level::Error,
                    format!("Unsupported node kind for analyze_node: {}", node.kind),
                    "mainstage.analyzers.semantic.node.analyze_node".to_string(),
                    node.location.clone(),
                    node.span.clone(),
                ),
            ))
        }
    }
}
