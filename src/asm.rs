//! The `.cdx` assembler: directives, symbol resolution, and instruction
//! encoding into an [`Object`] ready for `verify::admit`.

use std::collections::HashMap;

use crate::diag::{Code, Diag, Result, Span};
use crate::image::FuncMeta;
use crate::isa::{self, Op, Operand};
use crate::lex::{lex, Tok};

/// The unverified output of assembly: everything `verify::admit` needs.
#[derive(Clone, Debug, Default)]
pub struct Object {
    pub name: String,
    pub constants: Vec<Vec<u8>>,
    pub funcs: Vec<FuncMeta>,
    pub tools: Vec<String>,
    pub code: Vec<u8>,
    pub jump_tables: Vec<Vec<u32>>,
    pub entry: Option<String>,
}

/// Assemble a source string into an [`Object`].
pub fn assemble(name: &str, source: &str) -> Result<Object> {
    let toks = lex(source)?;
    let mut obj = Object::default();
    obj.name = name.to_string();

    let mut fn_name: Option<String> = None;
    let mut maxstack: Option<u16> = None;
    let mut args: u8 = 0;
    let mut returns: u8 = 1;
    let mut labels: HashMap<String, u32> = HashMap::new();
    let mut pending: Vec<(String, Span)> = Vec::new();
    let mut func_code_off = 0usize;
    let mut const_index: HashMap<String, u32> = HashMap::new();
    let mut tool_index: HashMap<String, u32> = HashMap::new();

    let mut i = 0;
    while i < toks.len() {
        let span = toks[i].span;
        match &toks[i].tok {
            Tok::Directive(d) => match d.as_str() {
                "isa" => {
                    i += 1;
                    while i < toks.len() && !matches!(toks[i].tok, Tok::Newline | Tok::End) {
                        i += 1;
                    }
                }
                "image" => {
                    i += 1;
                    if let Tok::Str(s) = &toks[i].tok {
                        obj.name = s.clone();
                        i += 1;
                    } else {
                        return Err(Diag::new(Code::BadOperand, "`.image` wants a string").at(span));
                    }
                }
                "fn" => {
                    i += 1;
                    let fname = match &toks[i].tok {
                        Tok::Ident(s) | Tok::Label(s) => s.clone(),
                        _ => return Err(Diag::new(Code::BadOperand, "`.fn` wants a name").at(span)),
                    };
                    i += 1;
                    args = 0;
                    returns = 1;
                    let mut in_args = false;
                    while i < toks.len() {
                        match &toks[i].tok {
                            Tok::Ident(s) if s == "(" => {
                                in_args = true;
                                i += 1;
                            }
                            Tok::Ident(s) if s == ")" => {
                                in_args = false;
                                i += 1;
                            }
                            Tok::Ident(s) if s == "->" => {
                                i += 1;
                                if let Tok::Ident(r) = &toks[i].tok {
                                    returns = if r == "void" { 0 } else { 1 };
                                    i += 1;
                                }
                            }
                            Tok::Ident(s)
                                if in_args
                                    && !matches!(
                                        s.as_str(),
                                        "i32" | "i64" | "str" | "void" | ","
                                    ) =>
                            {
                                args = args.saturating_add(1);
                                i += 1;
                            }
                            _ => break,
                        }
                    }
                    if fn_name.is_some() {
                        return Err(Diag::new(Code::DuplicateSymbol, "nested `.fn`").at(span));
                    }
                    fn_name = Some(fname);
                    maxstack = None;
                    labels.clear();
                    pending.clear();
                    func_code_off = obj.code.len();
                }
                "maxstack" => {
                    i += 1;
                    match &toks[i].tok {
                        Tok::Int(n) => {
                            maxstack = Some((*n).unsigned_abs().min(u16::MAX as u64) as u16);
                            i += 1;
                        }
                        _ => {
                            return Err(Diag::new(
                                Code::MissingMaxstack,
                                "`.maxstack` wants an integer",
                            )
                            .at(span));
                        }
                    }
                }
                "tool" => {
                    i += 1;
                    if let Tok::Ident(s) = &toks[i].tok {
                        let idx = obj.tools.len() as u32;
                        obj.tools.push(s.clone());
                        tool_index.insert(s.clone(), idx);
                        i += 1;
                    } else {
                        return Err(Diag::new(Code::BadOperand, "`.tool` wants a name").at(span));
                    }
                }
                "const" => {
                    i += 1;
                    let cname = match &toks[i].tok {
                        Tok::Ident(s) => s.clone(),
                        _ => {
                            return Err(
                                Diag::new(Code::BadOperand, "`.const` wants a name").at(span)
                            )
                        }
                    };
                    i += 1;
                    let bytes = match &toks[i].tok {
                        Tok::Str(s) => s.as_bytes().to_vec(),
                        Tok::Int(n) => n.to_le_bytes().to_vec(),
                        _ => {
                            return Err(Diag::new(
                                Code::BadOperand,
                                "`.const` wants a string or int",
                            )
                            .at(span))
                        }
                    };
                    let idx = obj.constants.len() as u32;
                    obj.constants.push(bytes);
                    const_index.insert(cname, idx);
                    i += 1;
                }
                other => {
                    return Err(Diag::new(Code::UnknownDirective, format!("`.{other}`")).at(span));
                }
            },
            Tok::Label(l) => {
                if fn_name.is_none() {
                    return Err(Diag::new(Code::BadOperand, "label outside a function").at(span));
                }
                if labels.contains_key(l) {
                    return Err(Diag::new(Code::DuplicateLabel, format!("`{l}`")).at(span));
                }
                let idx = (obj.code.len() - func_code_off) as u32 / 4;
                labels.insert(l.clone(), idx);
                i += 1;
            }
            Tok::Ident(m) => {
                if fn_name.is_none() {
                    return Err(
                        Diag::new(Code::BadOperand, "instruction outside a function").at(span),
                    );
                }
                let insn = isa::TABLE
                    .iter()
                    .find(|r| r.mnemonic == m)
                    .ok_or_else(|| Diag::new(Code::UnknownMnemonic, format!("`{m}`")).at(span))?;
                let op = insn.op;
                let a: u8 = 0;
                let mut b: u32 = 0;
                i += 1;
                match insn.operand {
                    Operand::None => {
                        if i < toks.len() && !matches!(toks[i].tok, Tok::Newline | Tok::End) {
                            return Err(Diag::new(
                                Code::UnexpectedOperand,
                                format!("`{m}` takes no operand"),
                            )
                            .at(span));
                        }
                    }
                    Operand::Imm => match &toks[i].tok {
                        Tok::Int(n) => {
                            if *n < 0 {
                                b = ((*n) as i16) as u16 as u32;
                            } else {
                                b = (*n).unsigned_abs().min(u32::MAX as u64) as u32;
                            }
                            i += 1;
                        }
                        Tok::Ident(s) => {
                            let idx = *const_index.get(s).ok_or_else(|| {
                                Diag::new(Code::UnknownSymbol, format!("`{s}`")).at(span)
                            })?;
                            b = idx;
                            i += 1;
                        }
                        _ => {
                            return Err(Diag::new(
                                Code::MissingOperand,
                                format!("`{m}` wants an immediate"),
                            )
                            .at(span));
                        }
                    },
                    Operand::Const => match &toks[i].tok {
                        Tok::Ident(s) => {
                            b = *const_index.get(s).ok_or_else(|| {
                                Diag::new(Code::UnknownSymbol, format!("`{s}`")).at(span)
                            })?;
                            i += 1;
                        }
                        Tok::Str(_) => {
                            // inline string constants get interned
                            let bytes = match &toks[i].tok {
                                Tok::Str(s) => s.as_bytes().to_vec(),
                                _ => unreachable!(),
                            };
                            let idx = obj.constants.len() as u32;
                            obj.constants.push(bytes);
                            b = idx;
                            i += 1;
                        }
                        _ => {
                            return Err(Diag::new(
                                Code::MissingOperand,
                                format!("`{m}` wants a constant"),
                            )
                            .at(span));
                        }
                    },
                    Operand::Tool => match &toks[i].tok {
                        Tok::Ident(s) => {
                            b = *tool_index.get(s).ok_or_else(|| {
                                Diag::new(Code::UnknownSymbol, format!("`{s}`")).at(span)
                            })?;
                            i += 1;
                        }
                        _ => {
                            return Err(Diag::new(
                                Code::MissingOperand,
                                format!("`{m}` wants a tool"),
                            )
                            .at(span));
                        }
                    },
                    Operand::Func => match &toks[i].tok {
                        Tok::Ident(s) => {
                            // function indices are assigned when the function
                            // table is closed; record the name to patch
                            pending.push((s.clone(), span));
                            b = 0;
                            i += 1;
                        }
                        _ => {
                            return Err(Diag::new(
                                Code::MissingOperand,
                                format!("`{m}` wants a function"),
                            )
                            .at(span));
                        }
                    },
                    Operand::Target => match &toks[i].tok {
                        Tok::Ident(s) => {
                            if let Some(&target) = labels.get(s) {
                                b = target;
                            } else {
                                pending.push((s.clone(), span));
                                b = 0;
                            }
                            i += 1;
                        }
                        _ => {
                            return Err(Diag::new(
                                Code::MissingOperand,
                                format!("`{m}` wants a label"),
                            )
                            .at(span));
                        }
                    },
                    Operand::Slot | Operand::Dimension | Operand::JumpTable => match &toks[i].tok {
                        Tok::Int(n) => {
                            b = (*n).unsigned_abs().min(u32::MAX as u64) as u32;
                            i += 1;
                        }
                        _ => {
                            return Err(Diag::new(
                                Code::MissingOperand,
                                format!("`{m}` wants an integer"),
                            )
                            .at(span));
                        }
                    },
                }
                if i < toks.len() && toks[i].tok == Tok::Newline {
                    i += 1;
                }
                if op == Op::Switch {
                    obj.jump_tables.push(vec![0]);
                }
                isa::encode(&mut obj.code, op, a, b);
            }
            Tok::Newline | Tok::End => {
                i += 1;
            }
            Tok::Str(_) | Tok::Int(_) => {
                return Err(Diag::new(Code::BadOperand, "unexpected value token").at(span));
            }
        }
    }
