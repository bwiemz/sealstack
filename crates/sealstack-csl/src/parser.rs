//! Parser for the Context Schema Language.
//!
//! Implemented with `winnow` 0.6. The grammar is hand-written as a tree of parser
//! combinators following the EBNF in §2 of the CSL spec. Every AST node is annotated
//! with a [`Span`] via `winnow::Parser::with_span`.
//!
//! # Conventions
//!
//! * Parsers named `ws_*` accept a single lexical token followed by trailing whitespace.
//! * `cut_err` is used after the parser has committed to a rule; this produces better
//!   error messages by preventing backtracking through alternatives that we know cannot match.
//! * `ContextError` with `StrContext::Label` is attached at significant decision points.
//!
//! # Status
//!
//! **This parser has not been compile-verified by the author of this file.**
//! It follows the documented `winnow` 0.6 API, but specific combinator signatures
//! sometimes shift between minor releases. Expect 1–2 session(s) of minor fixes
//! against the compiler. The grammar coverage is complete for the v0.1 spec subset.

use winnow::{
    LocatingSlice, ModalResult, Parser,
    ascii::{digit1, multispace1},
    combinator::{alt, cut_err, delimited, opt, preceded, repeat, separated, terminated},
    error::ContextError,
    stream::Location,
    token::{one_of, take_until, take_while},
};

use crate::ast::{
    Action, BinaryOp, Cardinality, ContextBlock, ContextStmt, Decorator, DeletePolicy,
    DurationUnit, EnumDecl, EnumVariant, Expr, FieldDecl, File, ImportStmt, Literal, NamespaceDecl,
    Path, PolicyBlock, PolicyRule, PrimitiveType, RelationDecl, SchemaDecl, TopDecl, TypeExpr,
    UnaryOp,
};
use crate::error::{CslError, CslResult};
use crate::span::Span;

/// Input type for the parser: a byte-located slice over the source.
pub type Input<'s> = LocatingSlice<&'s str>;

/// Short alias for the parser result type.
type PR<T> = ModalResult<T, ContextError>;

// === Top-level entry points ===========================================================

/// Parse a CSL source string into a [`File`] AST.
///
/// # Errors
/// Returns a spanful parse error if the input does not match the grammar.
pub fn parse_file(source: &str) -> CslResult<File> {
    parse_file_named("<input>", source)
}

/// Parse a CSL source string with an explicit filename for diagnostics.
///
/// # Errors
/// Same as [`parse_file`].
pub fn parse_file_named(filename: &str, source: &str) -> CslResult<File> {
    let mut input = LocatingSlice::new(source);
    match file.parse_next(&mut input) {
        Ok(mut f) => {
            f.filename = Some(filename.to_owned());
            f.source = source.to_owned();
            Ok(f)
        }
        Err(winnow::error::ErrMode::Backtrack(e) | winnow::error::ErrMode::Cut(e)) => {
            // Location of the farthest point the parser reached.
            let offset = input.location();
            let span = Span::new(offset, (offset + 1).min(source.len()));
            let message = format_context_error(&e);
            Err(CslError::parse(Some(filename), source, span, message))
        }
        Err(winnow::error::ErrMode::Incomplete(_)) => Err(CslError::parse(
            Some(filename),
            source,
            Span::point(source.len()),
            "unexpected end of input",
        )),
    }
}

fn format_context_error(e: &ContextError) -> String {
    // ContextError's Display is decent; we lift it here for consistency.
    let msg = format!("{e}");
    if msg.is_empty() {
        "unexpected token".to_owned()
    } else {
        msg
    }
}

// === Whitespace and comments =========================================================

fn line_comment<'s>(i: &mut Input<'s>) -> PR<()> {
    ("//", take_while(0.., |c: char| c != '\n'))
        .void()
        .parse_next(i)
}

fn block_comment<'s>(i: &mut Input<'s>) -> PR<()> {
    ("/*", take_until(0.., "*/"), "*/").void().parse_next(i)
}

/// Skip any amount of whitespace or comment.
fn skip_ws<'s>(i: &mut Input<'s>) -> PR<()> {
    let _: Vec<()> =
        repeat(0.., alt((multispace1.void(), line_comment, block_comment))).parse_next(i)?;
    Ok(())
}

/// Wrap a parser so that trailing whitespace is consumed.
fn lex<'s, O, F>(f: F) -> impl Parser<Input<'s>, O, ContextError>
where
    F: Parser<Input<'s>, O, ContextError>,
{
    terminated(f, skip_ws)
}

/// Literal keyword/punctuation that must match exactly, followed by skipped whitespace.
fn kw<'s>(s: &'static str) -> impl Parser<Input<'s>, &'s str, ContextError> {
    lex(s)
}

// === Identifiers =====================================================================

/// Recognize an identifier by taking the slice that matches `(alpha, alphanumeric)*`.
fn ident_recognized<'s>(i: &mut Input<'s>) -> PR<&'s str> {
    (
        one_of(|c: char| c.is_alphabetic() || c == '_'),
        take_while(0.., |c: char| c.is_alphanumeric() || c == '_'),
    )
        .take()
        .parse_next(i)
}

/// Parse a lowercase-leading identifier. Rejects reserved keywords.
fn ident(i: &mut Input<'_>) -> PR<(String, Span)> {
    let (s, span) = ident_recognized.with_span().parse_next(i)?;
    if is_reserved(s) {
        return Err(winnow::error::ErrMode::Backtrack(ContextError::new()));
    }
    skip_ws(i)?;
    Ok((s.to_owned(), span.into()))
}

/// Parse an identifier that must begin with an uppercase letter (type names).
fn type_ident(i: &mut Input<'_>) -> PR<(String, Span)> {
    let (s, span) = ident_recognized.with_span().parse_next(i)?;
    if !s.chars().next().is_some_and(char::is_uppercase) {
        return Err(winnow::error::ErrMode::Backtrack(ContextError::new()));
    }
    skip_ws(i)?;
    Ok((s.to_owned(), span.into()))
}

fn is_reserved(s: &str) -> bool {
    matches!(
        s,
        "schema"
            | "entity"
            | "relation"
            | "relations"
            | "enum"
            | "policy"
            | "context"
            | "import"
            | "namespace"
            | "version"
            | "from"
            | "via"
            | "as"
            | "one"
            | "many"
            | "optional"
            | "required"
            | "true"
            | "false"
            | "null"
            | "and"
            | "or"
            | "not"
            | "in"
            | "on_delete"
            | "cascade"
            | "restrict"
            | "set_null"
            | "use"
            | "profile"
    )
}

fn path_parser<'s>(i: &mut Input<'s>) -> PR<Path> {
    let (segs, span) = separated::<_, &str, Vec<&str>, _, _, _, _>(1.., ident_recognized, ".")
        .with_span()
        .parse_next(i)?;
    skip_ws(i)?;
    Ok(Path {
        segments: segs.into_iter().map(str::to_owned).collect(),
        span: span.into(),
    })
}

// === Literals ========================================================================

fn string_literal<'s>(i: &mut Input<'s>) -> PR<(String, Span)> {
    let (s, span) = delimited(
        '"',
        // Very small escape-handling: \" \\ \n \t.
        repeat::<_, _, String, _, _>(
            0..,
            alt((
                preceded('\\', one_of(['"', '\\', 'n', 't'])).map(|c: char| match c {
                    'n' => '\n',
                    't' => '\t',
                    other => other,
                }),
                one_of(|c: char| c != '"' && c != '\\'),
            )),
        ),
        '"',
    )
    .with_span()
    .parse_next(i)?;
    skip_ws(i)?;
    Ok((s, span.into()))
}

fn integer_literal<'s>(i: &mut Input<'s>) -> PR<(i64, Span)> {
    let (s, span) = (opt('-'), digit1).take().with_span().parse_next(i)?;
    skip_ws(i)?;
    let v: i64 = s
        .parse()
        .map_err(|_| winnow::error::ErrMode::Cut(ContextError::new()))?;
    Ok((v, span.into()))
}

fn float_literal<'s>(i: &mut Input<'s>) -> PR<(f64, Span)> {
    let (s, span) = (opt('-'), digit1, '.', digit1)
        .take()
        .with_span()
        .parse_next(i)?;
    skip_ws(i)?;
    let v: f64 = s
        .parse()
        .map_err(|_| winnow::error::ErrMode::Cut(ContextError::new()))?;
    Ok((v, span.into()))
}

fn duration_unit<'s>(i: &mut Input<'s>) -> PR<DurationUnit> {
    alt((
        "ns".value(DurationUnit::Ns),
        "us".value(DurationUnit::Us),
        "ms".value(DurationUnit::Ms),
        "mo".value(DurationUnit::Mo),
        "s".value(DurationUnit::S),
        "m".value(DurationUnit::M),
        "h".value(DurationUnit::H),
        "d".value(DurationUnit::D),
        "w".value(DurationUnit::W),
        "y".value(DurationUnit::Y),
    ))
    .parse_next(i)
}

fn duration_literal<'s>(i: &mut Input<'s>) -> PR<(Literal, Span)> {
    let ((n, unit), span) = (digit1.parse_to::<i64>(), duration_unit)
        .with_span()
        .parse_next(i)?;
    skip_ws(i)?;
    Ok((Literal::Duration(n, unit), span.into()))
}

fn bool_literal<'s>(i: &mut Input<'s>) -> PR<(bool, Span)> {
    let (b, span) = alt(("true".value(true), "false".value(false)))
        .with_span()
        .parse_next(i)?;
    skip_ws(i)?;
    Ok((b, span.into()))
}

// === Type expressions ================================================================

fn type_expr<'s>(i: &mut Input<'s>) -> PR<TypeExpr> {
    // Parse a non-optional type, then optionally wrap in Optional(...).
    let inner = type_expr_non_optional(i)?;
    if kw("?").parse_next(i).is_ok() {
        let span = inner.span();
        Ok(TypeExpr::Optional(Box::new(inner), span))
    } else {
        Ok(inner)
    }
}

fn type_expr_non_optional<'s>(i: &mut Input<'s>) -> PR<TypeExpr> {
    alt((
        type_ref,
        type_list,
        type_vector,
        type_map,
        type_named_or_primitive,
    ))
    .parse_next(i)
}

fn type_ref<'s>(i: &mut Input<'s>) -> PR<TypeExpr> {
    let ((target, _), span) = (
        preceded(kw("Ref"), cut_err(delimited(kw("<"), type_ident, kw(">")))),
        skip_ws,
    )
        .with_span()
        .parse_next(i)?;
    Ok(TypeExpr::Ref(target.0, span.into()))
}

fn type_list<'s>(i: &mut Input<'s>) -> PR<TypeExpr> {
    let (inner, span) = preceded(kw("List"), cut_err(delimited(kw("<"), type_expr, kw(">"))))
        .with_span()
        .parse_next(i)?;
    Ok(TypeExpr::List(Box::new(inner), span.into()))
}

fn type_vector<'s>(i: &mut Input<'s>) -> PR<TypeExpr> {
    let ((n, _), span) = (
        preceded(
            kw("Vector"),
            cut_err(delimited(kw("<"), integer_literal, kw(">"))),
        ),
        skip_ws,
    )
        .with_span()
        .parse_next(i)?;
    let n = u32::try_from(n.0).map_err(|_| winnow::error::ErrMode::Cut(ContextError::new()))?;
    Ok(TypeExpr::Vector(n, span.into()))
}

fn type_map<'s>(i: &mut Input<'s>) -> PR<TypeExpr> {
    let ((k, v), span) = preceded(
        kw("Map"),
        cut_err(delimited(
            kw("<"),
            (terminated(type_expr, kw(",")), type_expr),
            kw(">"),
        )),
    )
    .with_span()
    .parse_next(i)?;
    Ok(TypeExpr::Map(Box::new(k), Box::new(v), span.into()))
}

fn type_named_or_primitive<'s>(i: &mut Input<'s>) -> PR<TypeExpr> {
    let (name, span) = type_ident.parse_next(i)?;
    if let Some(p) = PrimitiveType::from_name(&name) {
        Ok(TypeExpr::Primitive(p, span))
    } else {
        Ok(TypeExpr::Named(name, span))
    }
}

// === Expressions (Pratt-style precedence) ============================================

fn expr<'s>(i: &mut Input<'s>) -> PR<Expr> {
    expr_or(i)
}

fn expr_or<'s>(i: &mut Input<'s>) -> PR<Expr> {
    let mut lhs = expr_and(i)?;
    while kw("or").parse_next(i).is_ok() {
        let rhs = expr_and(i)?;
        let span = lhs.span().merge(rhs.span());
        lhs = Expr::Binary(BinaryOp::Or, Box::new(lhs), Box::new(rhs), span);
    }
    Ok(lhs)
}

fn expr_and<'s>(i: &mut Input<'s>) -> PR<Expr> {
    let mut lhs = expr_not(i)?;
    while kw("and").parse_next(i).is_ok() {
        let rhs = expr_not(i)?;
        let span = lhs.span().merge(rhs.span());
        lhs = Expr::Binary(BinaryOp::And, Box::new(lhs), Box::new(rhs), span);
    }
    Ok(lhs)
}

fn expr_not<'s>(i: &mut Input<'s>) -> PR<Expr> {
    if kw("not").parse_next(i).is_ok() {
        let inner = expr_cmp(i)?;
        let span = inner.span();
        Ok(Expr::Unary(UnaryOp::Not, Box::new(inner), span))
    } else {
        expr_cmp(i)
    }
}

fn expr_cmp<'s>(i: &mut Input<'s>) -> PR<Expr> {
    let lhs = expr_add(i)?;
    let op = alt((
        kw("==").value(BinaryOp::Eq),
        kw("!=").value(BinaryOp::Ne),
        kw("<=").value(BinaryOp::Le),
        kw(">=").value(BinaryOp::Ge),
        kw("<").value(BinaryOp::Lt),
        kw(">").value(BinaryOp::Gt),
        (kw("not"), kw("in")).value(BinaryOp::NotIn),
        kw("in").value(BinaryOp::In),
    ))
    .parse_next(i);
    match op {
        Ok(op) => {
            let rhs = expr_add(i)?;
            let span = lhs.span().merge(rhs.span());
            Ok(Expr::Binary(op, Box::new(lhs), Box::new(rhs), span))
        }
        Err(_) => Ok(lhs),
    }
}

fn expr_add<'s>(i: &mut Input<'s>) -> PR<Expr> {
    let mut lhs = expr_mul(i)?;
    loop {
        let op = alt((kw("+").value(BinaryOp::Add), kw("-").value(BinaryOp::Sub))).parse_next(i);
        match op {
            Ok(op) => {
                let rhs = expr_mul(i)?;
                let span = lhs.span().merge(rhs.span());
                lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs), span);
            }
            Err(_) => return Ok(lhs),
        }
    }
}

fn expr_mul<'s>(i: &mut Input<'s>) -> PR<Expr> {
    let mut lhs = expr_unary(i)?;
    loop {
        let op = alt((kw("*").value(BinaryOp::Mul), kw("/").value(BinaryOp::Div))).parse_next(i);
        match op {
            Ok(op) => {
                let rhs = expr_unary(i)?;
                let span = lhs.span().merge(rhs.span());
                lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs), span);
            }
            Err(_) => return Ok(lhs),
        }
    }
}

fn expr_unary<'s>(i: &mut Input<'s>) -> PR<Expr> {
    if kw("-").parse_next(i).is_ok() {
        let inner = expr_atom(i)?;
        let span = inner.span();
        Ok(Expr::Unary(UnaryOp::Neg, Box::new(inner), span))
    } else {
        expr_atom(i)
    }
}

fn expr_atom<'s>(i: &mut Input<'s>) -> PR<Expr> {
    // `expr_literal` must come before `call_or_path` so that reserved words
    // like `true`, `false`, and `null` bind as boolean / null literals rather
    // than being eagerly consumed by `path_parser` (which uses the
    // permissive `ident_recognized` that doesn't reject reserved words).
    alt((parenthesized_expr, list_literal, expr_literal, call_or_path)).parse_next(i)
}

fn parenthesized_expr<'s>(i: &mut Input<'s>) -> PR<Expr> {
    delimited(kw("("), expr, kw(")")).parse_next(i)
}

fn list_literal<'s>(i: &mut Input<'s>) -> PR<Expr> {
    let (items, span) = delimited(
        kw("["),
        separated::<_, Expr, Vec<Expr>, _, _, _, _>(0.., expr, kw(",")),
        kw("]"),
    )
    .with_span()
    .parse_next(i)?;
    Ok(Expr::List(items, span.into()))
}

fn call_or_path<'s>(i: &mut Input<'s>) -> PR<Expr> {
    let path = path_parser(i)?;
    if kw("(").parse_next(i).is_ok() {
        let args: Vec<Expr> = separated(0.., call_arg, kw(",")).parse_next(i)?;
        let _ = kw(")").parse_next(i)?;
        let span = path.span.merge(Span::new(path.span.end, path.span.end + 1));
        Ok(Expr::Call(path, args, span))
    } else {
        Ok(Expr::Path(path))
    }
}

/// A single argument to a call expression.
///
/// Accepts both positional (`expr`) and keyword (`ident = expr`) forms. The
/// keyword name is currently discarded — the CSL spec references keyword args
/// for strategies like `semantic(max_tokens = 512)`, and a future pass will
/// preserve the label via a `Kwarg` AST variant. Discarding it here lets the
/// parser accept the spec's notation without widening the AST yet.
fn call_arg<'s>(i: &mut Input<'s>) -> PR<Expr> {
    use winnow::stream::Stream;
    let checkpoint = i.checkpoint();
    if (ident, kw("=")).parse_next(i).is_ok() {
        return expr(i);
    }
    i.reset(&checkpoint);
    expr(i)
}

fn expr_literal<'s>(i: &mut Input<'s>) -> PR<Expr> {
    // Order matters: duration before integer, float before integer, bool before path.
    alt((
        duration_literal.map(|(lit, span)| Expr::Literal(lit, span)),
        float_literal.map(|(n, span)| Expr::Literal(Literal::Float(n), span)),
        integer_literal.map(|(n, span)| Expr::Literal(Literal::Integer(n), span)),
        string_literal.map(|(s, span)| Expr::Literal(Literal::String(s), span)),
        bool_literal.map(|(b, span)| Expr::Literal(Literal::Bool(b), span)),
        kw("null")
            .with_span()
            .map(|(_, span)| Expr::Literal(Literal::Null, span.into())),
    ))
    .parse_next(i)
}

// === Decorators ======================================================================

fn decorator<'s>(i: &mut Input<'s>) -> PR<Decorator> {
    let (_, start) = kw("@").with_span().parse_next(i)?;
    let path = path_parser(i)?;
    let args = if kw("(").parse_next(i).is_ok() {
        let a: Vec<Expr> = separated(0.., expr, kw(",")).parse_next(i)?;
        let _ = kw(")").parse_next(i)?;
        a
    } else {
        Vec::new()
    };
    let assign = if kw("=").parse_next(i).is_ok() {
        Some(expr(i)?)
    } else {
        None
    };
    let span_end = assign
        .as_ref()
        .map(Expr::span)
        .or_else(|| args.last().map(Expr::span))
        .unwrap_or(path.span);
    let span = Span::new(start.start, span_end.end);
    Ok(Decorator {
        path,
        args,
        assign,
        span,
    })
}

// === Field declaration ===============================================================

fn field_decl<'s>(i: &mut Input<'s>) -> PR<FieldDecl> {
    let ((name, name_span), _, ty, decorators) = (
        ident,
        kw(":"),
        type_expr,
        repeat::<_, _, Vec<_>, _, _>(0.., decorator),
    )
        .parse_next(i)?;
    let _ = opt(kw(";")).parse_next(i)?;
    let span_end = decorators.last().map(|d| d.span).unwrap_or(ty.span());
    Ok(FieldDecl {
        name,
        ty,
        decorators,
        span: Span::new(name_span.start, span_end.end),
    })
}

// === Relations =======================================================================

fn cardinality<'s>(i: &mut Input<'s>) -> PR<Cardinality> {
    alt((
        kw("one").value(Cardinality::One),
        kw("many").value(Cardinality::Many),
    ))
    .parse_next(i)
}

fn delete_policy<'s>(i: &mut Input<'s>) -> PR<DeletePolicy> {
    alt((
        kw("cascade").value(DeletePolicy::Cascade),
        kw("restrict").value(DeletePolicy::Restrict),
        kw("set_null").value(DeletePolicy::SetNull),
    ))
    .parse_next(i)
}

fn relation_decl<'s>(i: &mut Input<'s>) -> PR<RelationDecl> {
    let ((name, name_span), _, card, (target, _), _, via, on_delete) = (
        ident,
        kw(":"),
        cardinality,
        type_ident,
        kw("via"),
        path_parser,
        opt(preceded(kw("on_delete"), delete_policy)),
    )
        .parse_next(i)?;
    let _ = opt(kw(";")).parse_next(i)?;
    let span = Span::new(name_span.start, via.span.end);
    Ok(RelationDecl {
        name,
        cardinality: card,
        target,
        via,
        on_delete,
        span,
    })
}

fn relations_block<'s>(i: &mut Input<'s>) -> PR<Vec<RelationDecl>> {
    preceded(
        kw("relations"),
        cut_err(delimited(kw("{"), repeat(0.., relation_decl), kw("}"))),
    )
    .parse_next(i)
}

// === Context block ===================================================================

fn context_stmt<'s>(i: &mut Input<'s>) -> PR<ContextStmt> {
    let ((key, key_span), _, value) = (ident, kw("="), expr).parse_next(i)?;
    let _ = opt(kw(";")).parse_next(i)?;
    let span = Span::new(key_span.start, value.span().end);
    Ok(ContextStmt { key, value, span })
}

fn context_block<'s>(i: &mut Input<'s>) -> PR<ContextBlock> {
    let (stmts, span) = preceded(
        kw("context"),
        cut_err(delimited(kw("{"), repeat(0.., context_stmt), kw("}"))),
    )
    .with_span()
    .parse_next(i)?;
    Ok(ContextBlock {
        stmts,
        span: span.into(),
    })
}

// === Policy block ====================================================================

fn action<'s>(i: &mut Input<'s>) -> PR<Action> {
    alt((
        kw("read").value(Action::Read),
        kw("list").value(Action::List),
        kw("write").value(Action::Write),
        kw("delete").value(Action::Delete),
    ))
    .parse_next(i)
}

fn policy_rule<'s>(i: &mut Input<'s>) -> PR<PolicyRule> {
    // Optional name label: `my_rule : read: expr` is rarely used; default to anonymous.
    let actions: Vec<Action> = separated(1.., action, kw("|")).parse_next(i)?;
    let _ = kw(":").parse_next(i)?;
    let predicate = expr(i)?;
    let _ = opt(kw(";")).parse_next(i)?;
    let span = predicate.span();
    Ok(PolicyRule {
        name: None,
        actions,
        predicate,
        span,
    })
}

fn policy_block<'s>(i: &mut Input<'s>) -> PR<PolicyBlock> {
    let (rules, span) = preceded(
        kw("policy"),
        cut_err(delimited(kw("{"), repeat(0.., policy_rule), kw("}"))),
    )
    .with_span()
    .parse_next(i)?;
    Ok(PolicyBlock {
        rules,
        span: span.into(),
    })
}

// === Schema ==========================================================================

fn schema_version<'s>(i: &mut Input<'s>) -> PR<u32> {
    preceded(kw("version"), cut_err(integer_literal))
        .map(|(n, _)| u32::try_from(n).unwrap_or(1))
        .parse_next(i)
}

fn schema_decl<'s>(i: &mut Input<'s>) -> PR<SchemaDecl> {
    let (_, (name, name_span)) = (kw("schema"), cut_err(type_ident)).parse_next(i)?;
    let version = opt(schema_version).parse_next(i)?;
    let _ = kw("{").parse_next(i)?;

    let mut fields: Vec<FieldDecl> = Vec::new();
    let mut relations: Vec<RelationDecl> = Vec::new();
    let mut context: Option<ContextBlock> = None;
    let mut policy: Option<PolicyBlock> = None;
    let mut decorators: Vec<Decorator> = Vec::new();

    loop {
        // End of schema body.
        if kw("}").parse_next(i).is_ok() {
            break;
        }

        // Schema-level decorators (e.g., `@audit`).
        if let Ok(d) = decorator.parse_next(i) {
            decorators.push(d);
            continue;
        }

        // relations block.
        if let Ok(r) = relations_block.parse_next(i) {
            relations.extend(r);
            continue;
        }

        // context block.
        if let Ok(cb) = context_block.parse_next(i) {
            context = Some(cb);
            continue;
        }

        // policy block.
        if let Ok(pb) = policy_block.parse_next(i) {
            policy = Some(pb);
            continue;
        }

        // Otherwise, assume a field declaration.
        let f = field_decl(i)?;
        fields.push(f);
    }

    let end = i.location();
    Ok(SchemaDecl {
        name,
        version,
        fields,
        relations,
        context,
        policy,
        decorators,
        span: Span::new(name_span.start, end),
    })
}

// === Enum ===========================================================================

fn enum_variant<'s>(i: &mut Input<'s>) -> PR<EnumVariant> {
    let ((name, name_span), wire) =
        (type_ident, opt(delimited(kw("("), string_literal, kw(")")))).parse_next(i)?;
    let wire_val = wire.map(|(s, _)| s);
    Ok(EnumVariant {
        name,
        wire: wire_val,
        span: name_span,
    })
}

fn enum_decl<'s>(i: &mut Input<'s>) -> PR<EnumDecl> {
    let (_, (name, name_span), _, variants, _) = (
        kw("enum"),
        cut_err(type_ident),
        kw("{"),
        separated::<_, EnumVariant, Vec<EnumVariant>, _, _, _, _>(1.., enum_variant, kw(",")),
        opt(kw(",")),
    )
        .parse_next(i)?;
    let _ = kw("}").parse_next(i)?;
    let _ = opt(kw(";")).parse_next(i)?;
    let span = Span::new(name_span.start, i.location());
    Ok(EnumDecl {
        name,
        variants,
        span,
    })
}

// === Top-level declarations ==========================================================

fn namespace_decl<'s>(i: &mut Input<'s>) -> PR<NamespaceDecl> {
    let (_, path) = (kw("namespace"), cut_err(path_parser)).parse_next(i)?;
    let _ = opt(kw(";")).parse_next(i)?;
    let span = path.span;
    Ok(NamespaceDecl { path, span })
}

fn import_stmt<'s>(i: &mut Input<'s>) -> PR<ImportStmt> {
    let (_, (path, _), alias) = (
        kw("import"),
        cut_err(string_literal),
        opt(preceded(kw("as"), cut_err(ident))),
    )
        .parse_next(i)?;
    let _ = opt(kw(";")).parse_next(i)?;
    let start = i.location();
    Ok(ImportStmt {
        path,
        alias: alias.map(|(n, _)| n),
        span: Span::new(start.saturating_sub(1), start),
    })
}

fn top_decl<'s>(i: &mut Input<'s>) -> PR<TopDecl> {
    alt((
        schema_decl.map(TopDecl::Schema),
        enum_decl.map(TopDecl::Enum),
        // TODO: top-level `policy Foo { ... }` and `context profile Foo { ... }`.
    ))
    .parse_next(i)
}

fn file<'s>(i: &mut Input<'s>) -> PR<File> {
    skip_ws(i)?;
    let namespace = opt(namespace_decl).parse_next(i)?;
    let imports: Vec<ImportStmt> = repeat(0.., import_stmt).parse_next(i)?;
    let decls: Vec<TopDecl> = repeat(0.., top_decl).parse_next(i)?;
    skip_ws(i)?;
    Ok(File {
        namespace,
        imports,
        decls,
        filename: None,
        source: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_empty_file() {
        let f = parse_file("").expect("ok");
        assert!(f.decls.is_empty());
    }

    #[test]
    fn parses_namespace_only() {
        let f = parse_file("namespace acme.crm;").expect("ok");
        assert_eq!(f.namespace.unwrap().path.joined(), "acme.crm");
    }

    #[test]
    fn parses_simple_schema() {
        let src = r#"
            schema Note {
                id:    Ulid    @primary
                title: String  @searchable
                body:  Text    @chunked
            }
        "#;
        let f = parse_file(src).expect("ok");
        let TopDecl::Schema(s) = &f.decls[0] else {
            panic!("not a schema");
        };
        assert_eq!(s.name, "Note");
        assert_eq!(s.fields.len(), 3);
        assert!(s.fields[0].decorators.iter().any(|d| d.is("primary")));
    }

    #[test]
    fn parses_hello_fixture() {
        let src = include_str!("../tests/fixtures/hello.csl");
        let f = parse_file(src).expect("parses hello fixture");
        assert_eq!(f.decls.len(), 1);
    }
}
