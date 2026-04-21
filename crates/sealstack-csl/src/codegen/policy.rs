//! WASM policy bundle codegen. One bundle per schema; empty policies still
//! emit a bundle that denies all actions.

use sealstack_policy_ir::{IR_SECTION_BYTES, MAGIC, action_bit, op};

use crate::ast::{Action, BinaryOp, Expr, Literal, PolicyBlock, SchemaDecl, UnaryOp};
use crate::error::{CslError, CslResult};
use crate::types::TypedFile;

const RUNTIME_WASM: &[u8] = include_bytes!("../../assets/policy_runtime.wasm");

/// A compiled WASM policy bundle, ready to write as `<namespace>.<schema>.wasm`.
#[derive(Clone, Debug)]
pub struct PolicyBundle {
    /// CSL namespace (empty string becomes "default" in the filename).
    pub namespace: String,
    /// CSL schema name.
    pub schema: String,
    /// Raw WASM bytes.
    pub wasm: Vec<u8>,
}

/// Emit one bundle per schema.
///
/// # Errors
///
/// Returns [`CslError::Codegen`] if lowering produces IR that exceeds the
/// reserved data-section size or if the runtime asset cannot be patched.
pub fn emit_policy_bundles(typed: &TypedFile) -> CslResult<Vec<PolicyBundle>> {
    let mut out = Vec::with_capacity(typed.schemas.len());
    for name in &typed.decl_order {
        if !typed.schemas.contains_key(name) {
            continue;
        }
        let ir = lower_schema_to_ir(typed, name)?;
        let wasm = patch_runtime(&ir)?;
        out.push(PolicyBundle {
            namespace: if typed.namespace.is_empty() {
                "default".to_string()
            } else {
                typed.namespace.clone()
            },
            schema: name.clone(),
            wasm,
        });
    }
    Ok(out)
}

/// Patch the `.sealstack_predicate_ir` data segment of the runtime asset with
/// the given IR bytes (which already include the 8-byte "SLIR" + length header)
/// plus zero padding.
///
/// This function implements byte-level section surgery via `wasmparser`
/// scanning — no `wasm-encoder` re-encode — so the output differs from the
/// input only in the patched segment bytes.
pub(crate) fn patch_runtime(ir_with_header: &[u8]) -> CslResult<Vec<u8>> {
    if ir_with_header.len() > IR_SECTION_BYTES {
        return Err(CslError::Codegen {
            message: format!(
                "policy IR exceeds {} bytes (got {})",
                IR_SECTION_BYTES,
                ir_with_header.len()
            ),
        });
    }

    let mut padded = Vec::with_capacity(IR_SECTION_BYTES);
    padded.extend_from_slice(ir_with_header);
    padded.resize(IR_SECTION_BYTES, 0u8);

    // Scan for a contiguous zero-filled region of the exact target size in
    // the Data section. The runtime reserves `IR_SECTION_BYTES` of zeros via
    // `static PREDICATE_IR: [u8; IR_SECTION_BYTES] = [0; ...]` in a custom
    // link section, which the linker lays down as a single data segment
    // whose initial contents are all-zeros. We find that segment by scanning
    // every data segment for one whose length matches and whose bytes are
    // all zero (the compiler has not yet patched), and rewrite in place.
    let scan = locate_predicate_section(RUNTIME_WASM)?;

    let mut out = RUNTIME_WASM.to_vec();
    out[scan.start..scan.start + IR_SECTION_BYTES].copy_from_slice(&padded);
    Ok(out)
}

struct ScanResult {
    start: usize,
}

fn locate_predicate_section(wasm: &[u8]) -> CslResult<ScanResult> {
    use wasmparser::{Parser, Payload};

    for payload in Parser::new(0).parse_all(wasm) {
        let p = payload.map_err(|e| CslError::Codegen {
            message: format!("parse runtime wasm: {e}"),
        })?;
        if let Payload::DataSection(reader) = p {
            for item in reader {
                let data = item.map_err(|e| CslError::Codegen {
                    message: format!("parse data segment: {e}"),
                })?;
                if data.data.len() == IR_SECTION_BYTES && data.data.iter().all(|b| *b == 0) {
                    // `data.data` is a sub-slice of `wasm`; recover the offset.
                    let start = data.data.as_ptr() as usize - wasm.as_ptr() as usize;
                    return Ok(ScanResult { start });
                }
            }
        }
    }
    Err(CslError::Codegen {
        message: "could not find .sealstack_predicate_ir data segment in runtime wasm".into(),
    })
}

/// Lower a schema's `policy { ... }` block to the flat IR byte stream
/// (magic + length + action table + rule bodies).
///
/// # Errors
///
/// Returns [`CslError::Codegen`] on unsupported predicate shapes.
pub fn lower_schema_to_ir(typed: &TypedFile, schema_name: &str) -> CslResult<Vec<u8>> {
    let Some(schema) = typed.schemas.get(schema_name) else {
        return Err(CslError::Codegen {
            message: format!("schema `{schema_name}` not found"),
        });
    };
    let decl = &schema.decl;
    let mut body = lower_policy_block_body(decl)?;

    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&MAGIC);
    let len_u32 = u32::try_from(body.len()).map_err(|_| CslError::Codegen {
        message: "policy IR exceeds u32::MAX".into(),
    })?;
    out.extend_from_slice(&len_u32.to_le_bytes());
    out.append(&mut body);
    Ok(out)
}

fn lower_policy_block_body(decl: &SchemaDecl) -> CslResult<Vec<u8>> {
    let Some(block) = &decl.policy else {
        // Empty policy block → action_table_count=0 → runtime denies.
        return Ok(vec![0u8]);
    };
    build_action_table_and_rules(block)
}

fn build_action_table_and_rules(block: &PolicyBlock) -> CslResult<Vec<u8>> {
    // Pass 1: lower each rule to its straight-line bytecode. Each rule ends
    // in OP_RESULT which pops a Bool and returns it as the verdict.
    let mut rule_streams: Vec<(u8, Vec<u8>)> = Vec::with_capacity(block.rules.len());
    for rule in &block.rules {
        let mut stream = Vec::new();
        lower_expr(&rule.predicate, &mut stream)?;
        stream.push(op::RESULT);
        let mask = action_mask(&rule.actions);
        rule_streams.push((mask, stream));
    }

    // Pass 2: layout.
    //   count: u8
    //   entries: { mask: u8, offset: u16 LE } * count
    // Then concatenated rule streams.
    let count = u8::try_from(rule_streams.len()).map_err(|_| CslError::Codegen {
        message: "too many policy rules (max 255)".into(),
    })?;
    let mut out = Vec::with_capacity(1 + 3 * rule_streams.len());
    out.push(count);

    // Compute offsets relative to the start of the rule-bytecode region
    // (which starts right after the table).
    let mut running_offset: u16 = 0;
    let mut table_offsets = Vec::with_capacity(rule_streams.len());
    for (_mask, stream) in &rule_streams {
        table_offsets.push(running_offset);
        running_offset = running_offset
            .checked_add(u16::try_from(stream.len()).map_err(|_| CslError::Codegen {
                message: "rule bytecode exceeds 64 KiB".into(),
            })?)
            .ok_or_else(|| CslError::Codegen {
                message: "cumulative rule bytecode exceeds 64 KiB".into(),
            })?;
    }

    for ((mask, _), offset) in rule_streams.iter().zip(&table_offsets) {
        out.push(*mask);
        out.extend_from_slice(&offset.to_le_bytes());
    }
    for (_mask, stream) in rule_streams {
        out.extend(stream);
    }
    Ok(out)
}

fn action_mask(actions: &[Action]) -> u8 {
    let mut m = 0u8;
    for a in actions {
        m |= match a {
            Action::Read => action_bit::READ,
            Action::List => action_bit::LIST,
            Action::Write => action_bit::WRITE,
            Action::Delete => action_bit::DELETE,
        };
    }
    m
}

fn lower_expr(expr: &Expr, out: &mut Vec<u8>) -> CslResult<()> {
    match expr {
        Expr::Literal(lit, _) => lower_literal(lit, out),
        Expr::Path(p) => lower_path(p, out),
        Expr::Binary(bop, a, b, _) => lower_binary(*bop, a, b, out),
        Expr::Unary(uop, inner, _) => lower_unary(*uop, inner, out),
        Expr::Call(name, args, _) => lower_call(&name.joined(), args, out),
        Expr::List(_, _) => Err(CslError::Codegen {
            message: "inline list literals not supported in policy predicates yet".into(),
        }),
    }
}

fn lower_literal(lit: &Literal, out: &mut Vec<u8>) -> CslResult<()> {
    match lit {
        Literal::Null => out.push(op::LIT_NULL),
        Literal::Bool(b) => {
            out.push(op::LIT_BOOL);
            out.push(u8::from(*b));
        }
        Literal::Integer(i) => {
            out.push(op::LIT_I64);
            out.extend_from_slice(&i.to_le_bytes());
        }
        Literal::Float(f) => {
            out.push(op::LIT_F64);
            out.extend_from_slice(&f.to_le_bytes());
        }
        Literal::String(s) => {
            out.push(op::LIT_STR);
            let len = u16::try_from(s.len()).map_err(|_| CslError::Codegen {
                message: "string literal exceeds 65535 bytes".into(),
            })?;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(s.as_bytes());
        }
        Literal::Duration(_, _) => {
            return Err(CslError::Codegen {
                message: "duration literals not supported in policy predicates yet".into(),
            });
        }
    }
    Ok(())
}

fn lower_path(path: &crate::ast::Path, out: &mut Vec<u8>) -> CslResult<()> {
    let segments = &path.segments;
    if segments.is_empty() {
        return Err(CslError::Codegen {
            message: "empty path".into(),
        });
    }
    let (load_op, rest) = match segments[0].as_str() {
        "caller" => (op::LOAD_CALLER, &segments[1..]),
        "self" => (op::LOAD_SELF, &segments[1..]),
        other => {
            return Err(CslError::Codegen {
                message: format!("unsupported path root `{other}` in policy predicate"),
            });
        }
    };
    out.push(load_op);
    let n_seg = u8::try_from(rest.len()).map_err(|_| CslError::Codegen {
        message: "path too deep (max 255 segments)".into(),
    })?;
    out.push(n_seg);
    for seg in rest {
        let len = u16::try_from(seg.len()).map_err(|_| CslError::Codegen {
            message: "path segment exceeds 65535 bytes".into(),
        })?;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(seg.as_bytes());
    }
    Ok(())
}

fn lower_binary(bop: BinaryOp, a: &Expr, b: &Expr, out: &mut Vec<u8>) -> CslResult<()> {
    lower_expr(a, out)?;
    lower_expr(b, out)?;
    let tag = match bop {
        BinaryOp::Eq => op::EQ,
        BinaryOp::Ne => op::NE,
        BinaryOp::Lt => op::LT,
        BinaryOp::Le => op::LE,
        BinaryOp::Gt => op::GT,
        BinaryOp::Ge => op::GE,
        BinaryOp::And => op::AND,
        BinaryOp::Or => op::OR,
        BinaryOp::In => op::IN,
        BinaryOp::NotIn => op::NOT_IN,
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
            return Err(CslError::Codegen {
                message: "arithmetic operators not supported in policy predicates".into(),
            });
        }
    };
    out.push(tag);
    Ok(())
}

fn lower_unary(uop: UnaryOp, inner: &Expr, out: &mut Vec<u8>) -> CslResult<()> {
    lower_expr(inner, out)?;
    match uop {
        UnaryOp::Not => out.push(op::NOT),
        UnaryOp::Neg => {
            return Err(CslError::Codegen {
                message: "unary minus not supported in policy predicates".into(),
            });
        }
    }
    Ok(())
}

fn lower_call(name: &str, args: &[Expr], out: &mut Vec<u8>) -> CslResult<()> {
    match name {
        "has_role" => {
            if args.len() != 2 {
                return Err(CslError::Codegen {
                    message: "has_role takes exactly 2 arguments".into(),
                });
            }
            lower_expr(&args[0], out)?;
            lower_expr(&args[1], out)?;
            out.push(op::CALL_HAS_ROLE);
            Ok(())
        }
        "tenant_match" => {
            if args.len() != 2 {
                return Err(CslError::Codegen {
                    message: "tenant_match takes exactly 2 arguments".into(),
                });
            }
            lower_expr(&args[0], out)?;
            lower_expr(&args[1], out)?;
            out.push(op::CALL_TENANT_MATCH);
            Ok(())
        }
        _ => Err(CslError::Codegen {
            message: format!("unknown built-in `{name}` in policy predicate"),
        }),
    }
}

// Used for tests only.
pub(crate) const _IR_VERSION: u8 = 1;
