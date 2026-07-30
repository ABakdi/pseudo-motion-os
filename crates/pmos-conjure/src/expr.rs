//! The Conjure expression language (App DSL spec §6).
//!
//! Pure and side-effect-free by construction: no user loops, no recursion,
//! no environment access beyond an injected clock — evaluation cost is
//! statically boundable, which is the core sandbox guarantee.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fmt;

// ---------- values ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Bool(bool),
    Num(f64),
    Str(String),
    List(Vec<Value>),
    Map(BTreeMap<String, Value>),
}

impl Value {
    pub fn truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Num(n) => *n != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::List(l) => !l.is_empty(),
            Value::Map(m) => !m.is_empty(),
        }
    }

    pub fn as_num(&self) -> f64 {
        match self {
            Value::Num(n) => *n,
            Value::Bool(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Value::Str(s) => s.parse().unwrap_or(0.0),
            _ => 0.0,
        }
    }

    pub fn display(&self) -> String {
        match self {
            Value::Str(s) => s.clone(),
            Value::Num(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    format!("{}", *n as i64)
                } else {
                    format!("{n}")
                }
            }
            Value::Bool(b) => b.to_string(),
            Value::List(l) => {
                let inner: Vec<String> = l.iter().map(|v| v.display()).collect();
                format!("[{}]", inner.join(", "))
            }
            Value::Map(_) => "{…}".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExprError(pub String);

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ---------- AST ----------

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f64),
    Str(String),
    Bool(bool),
    Ident(String),
    List(Vec<Expr>),
    Not(Box<Expr>),
    Neg(Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    Index(Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

// ---------- tokenizer ----------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    Op(&'static str),
}

fn tokenize(src: &str) -> Result<Vec<Tok>, ExprError> {
    let mut out = Vec::new();
    let b: Vec<char> = src.chars().collect();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '0'..='9' | '.' if c != '.' || b.get(i + 1).is_some_and(|d| d.is_ascii_digit()) => {
                let start = i;
                while i < b.len() && (b[i].is_ascii_digit() || b[i] == '.') {
                    i += 1;
                }
                let s: String = b[start..i].iter().collect();
                out.push(Tok::Num(
                    s.parse()
                        .map_err(|_| ExprError(format!("bad number {s}")))?,
                ));
            }
            '\'' | '"' => {
                let quote = c;
                i += 1;
                let start = i;
                while i < b.len() && b[i] != quote {
                    i += 1;
                }
                if i >= b.len() {
                    return Err(ExprError("unterminated string".into()));
                }
                out.push(Tok::Str(b[start..i].iter().collect()));
                i += 1;
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < b.len() && (b[i].is_alphanumeric() || b[i] == '_' || b[i] == '.') {
                    i += 1;
                }
                out.push(Tok::Ident(b[start..i].iter().collect()));
            }
            _ => {
                let two: String = b[i..(i + 2).min(b.len())].iter().collect();
                let op = match two.as_str() {
                    "==" | "!=" | "<=" | ">=" | "&&" | "||" => {
                        i += 2;
                        match two.as_str() {
                            "==" => "==",
                            "!=" => "!=",
                            "<=" => "<=",
                            ">=" => ">=",
                            "&&" => "&&",
                            _ => "||",
                        }
                    }
                    _ => {
                        i += 1;
                        match c {
                            '+' => "+",
                            '-' => "-",
                            '*' => "*",
                            '/' => "/",
                            '%' => "%",
                            '<' => "<",
                            '>' => ">",
                            '!' => "!",
                            '(' => "(",
                            ')' => ")",
                            '[' => "[",
                            ']' => "]",
                            ',' => ",",
                            '?' => "?",
                            ':' => ":",
                            other => {
                                return Err(ExprError(format!("unexpected character `{other}`")))
                            }
                        }
                    }
                };
                out.push(Tok::Op(op));
            }
        }
    }
    Ok(out)
}

// ---------- parser (precedence climbing) ----------

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    depth: u32,
}

const MAX_DEPTH: u32 = 32;

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn eat_op(&mut self, op: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Op(o)) if *o == op) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_op(&mut self, op: &'static str) -> Result<(), ExprError> {
        match self.peek() {
            Some(Tok::Op(o)) if *o == op => {
                self.pos += 1;
                Ok(())
            }
            other => Err(ExprError(format!("expected `{op}`, found {other:?}"))),
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, ExprError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(ExprError("expression too deep".into()));
        }
        let cond = self.parse_or()?;
        let out = if self.eat_op("?") {
            let a = self.parse_expr()?;
            self.expect_op(":")?;
            let b = self.parse_expr()?;
            Expr::Ternary(Box::new(cond), Box::new(a), Box::new(b))
        } else {
            cond
        };
        self.depth -= 1;
        Ok(out)
    }

    fn parse_or(&mut self) -> Result<Expr, ExprError> {
        let mut lhs = self.parse_and()?;
        while self.eat_op("||") {
            let rhs = self.parse_and()?;
            lhs = Expr::Bin(BinOp::Or, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<Expr, ExprError> {
        let mut lhs = self.parse_cmp()?;
        while self.eat_op("&&") {
            let rhs = self.parse_cmp()?;
            lhs = Expr::Bin(BinOp::And, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_cmp(&mut self) -> Result<Expr, ExprError> {
        let lhs = self.parse_add()?;
        for (op, bo) in [
            ("==", BinOp::Eq),
            ("!=", BinOp::Ne),
            ("<=", BinOp::Le),
            (">=", BinOp::Ge),
            ("<", BinOp::Lt),
            (">", BinOp::Gt),
        ] {
            if self.eat_op(op) {
                let rhs = self.parse_add()?;
                return Ok(Expr::Bin(bo, Box::new(lhs), Box::new(rhs)));
            }
        }
        Ok(lhs)
    }

    fn parse_add(&mut self) -> Result<Expr, ExprError> {
        let mut lhs = self.parse_mul()?;
        loop {
            if self.eat_op("+") {
                let rhs = self.parse_mul()?;
                lhs = Expr::Bin(BinOp::Add, Box::new(lhs), Box::new(rhs));
            } else if self.eat_op("-") {
                let rhs = self.parse_mul()?;
                lhs = Expr::Bin(BinOp::Sub, Box::new(lhs), Box::new(rhs));
            } else {
                return Ok(lhs);
            }
        }
    }

    fn parse_mul(&mut self) -> Result<Expr, ExprError> {
        let mut lhs = self.parse_unary()?;
        loop {
            if self.eat_op("*") {
                let rhs = self.parse_unary()?;
                lhs = Expr::Bin(BinOp::Mul, Box::new(lhs), Box::new(rhs));
            } else if self.eat_op("/") {
                let rhs = self.parse_unary()?;
                lhs = Expr::Bin(BinOp::Div, Box::new(lhs), Box::new(rhs));
            } else if self.eat_op("%") {
                let rhs = self.parse_unary()?;
                lhs = Expr::Bin(BinOp::Mod, Box::new(lhs), Box::new(rhs));
            } else {
                return Ok(lhs);
            }
        }
    }

    fn parse_unary(&mut self) -> Result<Expr, ExprError> {
        if self.eat_op("!") {
            return Ok(Expr::Not(Box::new(self.parse_unary()?)));
        }
        if self.eat_op("-") {
            return Ok(Expr::Neg(Box::new(self.parse_unary()?)));
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, ExprError> {
        let mut e = self.parse_primary()?;
        loop {
            if self.eat_op("[") {
                let idx = self.parse_expr()?;
                self.expect_op("]")?;
                e = Expr::Index(Box::new(e), Box::new(idx));
            } else {
                return Ok(e);
            }
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, ExprError> {
        match self.peek().cloned() {
            Some(Tok::Num(n)) => {
                self.pos += 1;
                Ok(Expr::Num(n))
            }
            Some(Tok::Str(s)) => {
                self.pos += 1;
                Ok(Expr::Str(s))
            }
            Some(Tok::Ident(name)) => {
                self.pos += 1;
                match name.as_str() {
                    "true" => return Ok(Expr::Bool(true)),
                    "false" => return Ok(Expr::Bool(false)),
                    _ => {}
                }
                if self.eat_op("(") {
                    let mut args = Vec::new();
                    if !self.eat_op(")") {
                        loop {
                            args.push(self.parse_expr()?);
                            if self.eat_op(")") {
                                break;
                            }
                            self.expect_op(",")?;
                        }
                    }
                    Ok(Expr::Call(name, args))
                } else {
                    Ok(Expr::Ident(name))
                }
            }
            Some(Tok::Op("(")) => {
                self.pos += 1;
                let e = self.parse_expr()?;
                self.expect_op(")")?;
                Ok(e)
            }
            Some(Tok::Op("[")) => {
                self.pos += 1;
                let mut items = Vec::new();
                if !self.eat_op("]") {
                    loop {
                        items.push(self.parse_expr()?);
                        if self.eat_op("]") {
                            break;
                        }
                        self.expect_op(",")?;
                    }
                }
                Ok(Expr::List(items))
            }
            other => Err(ExprError(format!("unexpected token {other:?}"))),
        }
    }
}

pub fn parse(src: &str) -> Result<Expr, ExprError> {
    let toks = tokenize(src)?;
    let mut p = Parser {
        toks,
        pos: 0,
        depth: 0,
    };
    let e = p.parse_expr()?;
    if p.pos != p.toks.len() {
        return Err(ExprError(format!(
            "trailing tokens after expression: {:?}",
            &p.toks[p.pos..]
        )));
    }
    Ok(e)
}

// ---------- evaluation ----------

pub struct Env<'a> {
    pub state: &'a HashMap<String, Value>,
    pub locals: &'a HashMap<String, Value>,
    /// Milliseconds since epoch/boot — the only environment read (`time.now()`).
    pub now_ms: f64,
}

pub fn eval(e: &Expr, env: &Env) -> Result<Value, ExprError> {
    Ok(match e {
        Expr::Num(n) => Value::Num(*n),
        Expr::Str(s) => Value::Str(s.clone()),
        Expr::Bool(b) => Value::Bool(*b),
        Expr::List(items) => Value::List(
            items
                .iter()
                .map(|i| eval(i, env))
                .collect::<Result<_, _>>()?,
        ),
        Expr::Ident(name) => env
            .locals
            .get(name)
            .or_else(|| env.state.get(name))
            .cloned()
            .ok_or_else(|| ExprError(format!("unknown identifier `{name}`")))?,
        Expr::Not(inner) => Value::Bool(!eval(inner, env)?.truthy()),
        Expr::Neg(inner) => Value::Num(-eval(inner, env)?.as_num()),
        Expr::Ternary(c, a, b) => {
            if eval(c, env)?.truthy() {
                eval(a, env)?
            } else {
                eval(b, env)?
            }
        }
        Expr::Index(base, idx) => {
            let b = eval(base, env)?;
            let i = eval(idx, env)?;
            match b {
                Value::List(l) => l
                    .get(i.as_num() as usize)
                    .cloned()
                    .unwrap_or(Value::Num(0.0)),
                Value::Map(m) => m.get(&i.display()).cloned().unwrap_or(Value::Num(0.0)),
                _ => return Err(ExprError("cannot index this value".into())),
            }
        }
        Expr::Bin(op, l, r) => {
            let a = eval(l, env)?;
            // Short-circuit logic ops.
            match op {
                BinOp::And => return Ok(Value::Bool(a.truthy() && eval(r, env)?.truthy())),
                BinOp::Or => return Ok(Value::Bool(a.truthy() || eval(r, env)?.truthy())),
                _ => {}
            }
            let b = eval(r, env)?;
            match op {
                BinOp::Add => match (&a, &b) {
                    (Value::Str(_), _) | (_, Value::Str(_)) => {
                        Value::Str(format!("{}{}", a.display(), b.display()))
                    }
                    _ => Value::Num(a.as_num() + b.as_num()),
                },
                BinOp::Sub => Value::Num(a.as_num() - b.as_num()),
                BinOp::Mul => Value::Num(a.as_num() * b.as_num()),
                BinOp::Div => {
                    let d = b.as_num();
                    // Divide-by-zero yields 0: generated math must not crash apps.
                    Value::Num(if d == 0.0 { 0.0 } else { a.as_num() / d })
                }
                BinOp::Mod => {
                    let d = b.as_num();
                    Value::Num(if d == 0.0 { 0.0 } else { a.as_num() % d })
                }
                BinOp::Eq => Value::Bool(a == b),
                BinOp::Ne => Value::Bool(a != b),
                BinOp::Lt => Value::Bool(a.as_num() < b.as_num()),
                BinOp::Le => Value::Bool(a.as_num() <= b.as_num()),
                BinOp::Gt => Value::Bool(a.as_num() > b.as_num()),
                BinOp::Ge => Value::Bool(a.as_num() >= b.as_num()),
                BinOp::And | BinOp::Or => unreachable!(),
            }
        }
        Expr::Call(name, args) => {
            let vals: Vec<Value> = args
                .iter()
                .map(|a| eval(a, env))
                .collect::<Result<_, _>>()?;
            builtin(name, &vals, env)?
        }
    })
}

fn builtin(name: &str, a: &[Value], env: &Env) -> Result<Value, ExprError> {
    let n = |i: usize| a.get(i).map(|v| v.as_num()).unwrap_or(0.0);
    let s = |i: usize| a.get(i).map(|v| v.display()).unwrap_or_default();
    Ok(match name {
        "math.abs" => Value::Num(n(0).abs()),
        "math.min" => Value::Num(n(0).min(n(1))),
        "math.max" => Value::Num(n(0).max(n(1))),
        "math.floor" => Value::Num(n(0).floor()),
        "math.ceil" => Value::Num(n(0).ceil()),
        "math.round" => Value::Num(n(0).round()),
        "math.sqrt" => Value::Num(n(0).max(0.0).sqrt()),
        "math.pow" => Value::Num(n(0).powf(n(1))),
        "math.clamp" => Value::Num(n(0).clamp(n(1), n(2))),
        "str.len" => Value::Num(s(0).chars().count() as f64),
        "str.upper" => Value::Str(s(0).to_uppercase()),
        "str.lower" => Value::Str(s(0).to_lowercase()),
        "str.trim" => Value::Str(s(0).trim().to_string()),
        "str.contains" => Value::Bool(s(0).contains(&s(1))),
        "list.len" => match a.first() {
            Some(Value::List(l)) => Value::Num(l.len() as f64),
            _ => Value::Num(0.0),
        },
        "list.get" => match a.first() {
            Some(Value::List(l)) => l.get(n(1) as usize).cloned().unwrap_or(Value::Num(0.0)),
            _ => Value::Num(0.0),
        },
        "list.sum" => match a.first() {
            Some(Value::List(l)) => Value::Num(l.iter().map(|v| v.as_num()).sum()),
            _ => Value::Num(0.0),
        },
        "list.contains" => match a.first() {
            Some(Value::List(l)) => Value::Bool(l.iter().any(|v| v.display() == s(1))),
            _ => Value::Bool(false),
        },
        "map.get" => match a.first() {
            Some(Value::Map(m)) => m
                .get(&s(1))
                .cloned()
                .unwrap_or_else(|| a.get(2).cloned().unwrap_or(Value::Num(0.0))),
            _ => Value::Num(0.0),
        },
        "map.has" => match a.first() {
            Some(Value::Map(m)) => Value::Bool(m.contains_key(&s(1))),
            _ => Value::Bool(false),
        },
        "fmt.mmss" => {
            let total = n(0).max(0.0) as u64;
            Value::Str(format!("{:02}:{:02}", total / 60, total % 60))
        }
        "fmt.num" => Value::Str(format!("{:.*}", n(1).clamp(0.0, 9.0) as usize, n(0))),
        "time.now" => Value::Num(env.now_ms),
        other => return Err(ExprError(format!("unknown function `{other}`"))),
    })
}

// ---------- string templates ("Time left: ${fmt.mmss(remaining)}") ----------

#[derive(Debug, Clone)]
pub enum TemplatePart {
    Text(String),
    Expr(Expr),
}

pub fn parse_template(src: &str) -> Result<Vec<TemplatePart>, ExprError> {
    let mut parts = Vec::new();
    let mut rest = src;
    while let Some(start) = rest.find("${") {
        if start > 0 {
            parts.push(TemplatePart::Text(rest[..start].to_string()));
        }
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .ok_or_else(|| ExprError("unterminated ${…} in template".into()))?;
        parts.push(TemplatePart::Expr(parse(&after[..end])?));
        rest = &after[end + 1..];
    }
    if !rest.is_empty() {
        parts.push(TemplatePart::Text(rest.to_string()));
    }
    Ok(parts)
}

pub fn eval_template(parts: &[TemplatePart], env: &Env) -> Result<String, ExprError> {
    let mut out = String::new();
    for p in parts {
        match p {
            TemplatePart::Text(t) => out.push_str(t),
            TemplatePart::Expr(e) => out.push_str(&eval(e, env)?.display()),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(src: &str, state: &[(&str, Value)]) -> Value {
        let st: HashMap<String, Value> = state
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        let locals = HashMap::new();
        let env = Env {
            state: &st,
            locals: &locals,
            now_ms: 1000.0,
        };
        eval(&parse(src).unwrap(), &env).unwrap()
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(ev("1 + 2 * 3", &[]), Value::Num(7.0));
        assert_eq!(ev("(1 + 2) * 3", &[]), Value::Num(9.0));
        assert_eq!(ev("10 / 0", &[]), Value::Num(0.0)); // never crashes
    }

    #[test]
    fn state_and_ternary() {
        assert_eq!(
            ev(
                "running ? 'Pause' : 'Start'",
                &[("running", Value::Bool(true))]
            ),
            Value::Str("Pause".into())
        );
    }

    #[test]
    fn builtins() {
        assert_eq!(ev("fmt.mmss(90)", &[]), Value::Str("01:30".into()));
        assert_eq!(ev("math.clamp(15, 0, 10)", &[]), Value::Num(10.0));
        assert_eq!(ev("str.upper('abc')", &[]), Value::Str("ABC".into()));
        assert_eq!(
            ev(
                "list.sum(items)",
                &[("items", Value::List(vec![Value::Num(1.0), Value::Num(2.0)]))]
            ),
            Value::Num(3.0)
        );
    }

    #[test]
    fn string_concat_and_compare() {
        assert_eq!(ev("'a' + 1 + true", &[]), Value::Str("a1true".into()));
        assert_eq!(ev("2 >= 2 && 1 < 2", &[]), Value::Bool(true));
    }

    #[test]
    fn templates() {
        let st: HashMap<String, Value> = [("n".to_string(), Value::Num(90.0))].into();
        let locals = HashMap::new();
        let env = Env {
            state: &st,
            locals: &locals,
            now_ms: 0.0,
        };
        let t = parse_template("left: ${fmt.mmss(n)}!").unwrap();
        assert_eq!(eval_template(&t, &env).unwrap(), "left: 01:30!");
    }

    #[test]
    fn depth_limit() {
        let deep = format!("{}1{}", "(".repeat(40), ")".repeat(40));
        assert!(parse(&deep).is_err());
    }
}
