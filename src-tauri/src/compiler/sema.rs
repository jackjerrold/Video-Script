//! Stage 3: semantic analysis and lowering (AST -> IR).
//!
//! The parser only checks *shape*; this stage checks *meaning*:
//!
//! * symbol tables: `let` variables, node names, the built-in `width`,
//!   `height`, `fps`, `duration`
//! * type checking: every property value must match its schema type
//! * constant evaluation: expressions (`width / 2 + 10`, `rgb(...)`, `cos(30)`)
//!   are folded to plain values
//! * **relative keyframes**: inside an `at` block a property name means "this
//!   property's value before this keyframe", so `x: x + 100` moves by 100.
//!   Those expressions cannot be folded yet, so they stay symbolic ([`Sym`])
//!   until every keyframe is known and can be resolved in time order
//! * **groups**: `group` nodes form a tree; names are global
//! * validation: unknown names (with "did you mean"), duplicates, ranges,
//!   keyframes after the end of the video, unused or shadowed variables
//! * lowering: building the sorted per-property keyframe timelines of the IR
//!
//! Two passes: first the `video` block (it defines the built-in constants),
//! then everything else in source order. A `let` must come before its first
//! use. Errors never abort a pass: each records a diagnostic and moves on, so
//! the user sees every problem at once.

use super::ast::*;
use super::builtins;
use super::diagnostics::{has_errors, suggest, Diagnostic};
use super::ir::{self, Easing, Key};
use super::schema::{self, PropDef, ShapeKind};
use super::params::{self, Overrides, Param};
use super::span::Span;
use super::value::{Rgba, Ty, Value};
use std::collections::{HashMap, HashSet};

const EPS: f64 = 1e-9;

pub fn analyze(ast: &Program) -> (Option<ir::Program>, Vec<Diagnostic>) {
    analyze_with(ast, &Overrides::new())
}

/// Like [`analyze`], with inspector values substituted for `@` parameters.
pub fn analyze_with(ast: &Program, overrides: &Overrides) -> (Option<ir::Program>, Vec<Diagnostic>) {
    let mut s = Sema {
        overrides: overrides.clone(),
        params: Vec::new(),
        diags: Vec::new(),
        vars: HashMap::new(),
        bad_vars: HashSet::new(),
        nodes: Vec::new(),
        node_ix: HashMap::new(),
        poisoned: HashSet::new(),
        duration: 0.0,
        seq: 0,
    };
    let program = s.run(ast);
    let mut diags = s.diags;
    diags.sort_by_key(|d| d.span.start);
    let program = if has_errors(&diags) { None } else { Some(program) };
    (program, diags)
}

// ---- symbolic values ---------------------------------------------------------

/// A value that may depend on the *previous value of a property*.
///
/// Everything that does not mention a property folds to `Const` immediately.
#[derive(Debug, Clone)]
enum Sym {
    Const(Value),
    /// "The value of this property just before this keyframe".
    Prop(&'static str, Ty),
    Neg(Box<Sym>),
    Bin(BinOp, Box<Sym>, Box<Sym>),
    Call(&'static str, Vec<Sym>, Ty),
}

type ResolveError = (&'static str, String);

impl Sym {
    fn ty(&self) -> Ty {
        match self {
            Sym::Const(v) => v.ty(),
            Sym::Prop(_, t) | Sym::Call(_, _, t) => *t,
            Sym::Neg(_) | Sym::Bin(..) => Ty::Number,
        }
    }

    /// Compute the value given each property's current value.
    fn resolve(&self, env: &HashMap<&'static str, Value>) -> Result<Value, ResolveError> {
        match self {
            Sym::Const(v) => Ok(v.clone()),
            Sym::Prop(name, _) => env
                .get(name)
                .cloned()
                .ok_or(("S007", format!("no current value for property '{name}'"))),
            Sym::Neg(e) => match e.resolve(env)? {
                Value::Number(n) => Ok(Value::Number(-n)),
                other => Err(("S005", format!("cannot negate a {}", other.ty()))),
            },
            Sym::Bin(op, l, r) => match (l.resolve(env)?, r.resolve(env)?) {
                (Value::Number(a), Value::Number(b)) => {
                    let out = match op {
                        BinOp::Add => a + b,
                        BinOp::Sub => a - b,
                        BinOp::Mul => a * b,
                        BinOp::Div => {
                            if b == 0.0 {
                                return Err(("S008", "division by zero".into()));
                            }
                            a / b
                        }
                    };
                    if !out.is_finite() {
                        return Err(("S017", "result is not a finite number".into()));
                    }
                    Ok(Value::Number(out))
                }
                (a, b) => Err(("S005", format!("cannot apply '{}' to a {} and a {}", op.symbol(), a.ty(), b.ty()))),
            },
            Sym::Call(name, args, _) => {
                let vals = args.iter().map(|a| a.resolve(env)).collect::<Result<Vec<_>, _>>()?;
                builtins::call(name, &vals).map_err(|m| (if m.contains("finite") { "S017" } else { "S005" }, m))
            }
        }
    }
}

// ---- analysis state ------------------------------------------------------------

struct Var {
    value: Value,
    span: Span,
    used: bool,
    builtin: bool,
}

/// One property assignment inside an `at` block, not yet resolved.
struct PendingProp {
    def: &'static PropDef,
    value: Sym,
    span: Span,
    ease: Easing,
}

/// One `at` block. Resolved later, sorted by (time, source order).
struct PendingBlock {
    time: f64,
    seq: usize,
    props: Vec<PendingProp>,
}

struct NodeInfo {
    name: String,
    kind: ShapeKind,
    parent: Option<usize>,
    base: HashMap<&'static str, Value>,
    blocks: Vec<PendingBlock>,
    /// Times already keyed per property (duplicate detection).
    claimed: HashMap<&'static str, Vec<f64>>,
    /// Resolved keyframes, filled by `resolve_blocks`.
    keys: HashMap<&'static str, Vec<Key>>,
}

struct Sema {
    overrides: Overrides,
    params: Vec<Param>,
    diags: Vec<Diagnostic>,
    vars: HashMap<String, Var>,
    /// `let`s whose value failed to evaluate: uses are silenced to avoid cascades.
    bad_vars: HashSet<String>,
    /// Every shape and group, in declaration order (parents before children).
    nodes: Vec<NodeInfo>,
    node_ix: HashMap<String, usize>,
    /// Nodes whose declaration failed: later `at` blocks on them are skipped silently.
    poisoned: HashSet<String>,
    duration: f64,
    seq: usize,
}

fn fmt_num(v: f64) -> String {
    if v.is_finite() && v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn range_text(lo: f64, hi: f64) -> String {
    if hi.is_infinite() {
        format!("{} or more", fmt_num(lo))
    } else {
        format!("{} to {}", fmt_num(lo), fmt_num(hi))
    }
}

fn video_defaults() -> HashMap<&'static str, Value> {
    schema::VIDEO_PROPS.iter().map(|d| (d.name, d.default.value())).collect()
}

fn video_from(vals: &HashMap<&'static str, Value>) -> ir::Video {
    let n = |k: &str| match vals.get(k) {
        Some(Value::Number(n)) => *n,
        _ => 0.0,
    };
    let background = match vals.get("background") {
        Some(Value::Color(c)) => *c,
        _ => Rgba::rgb(0, 0, 0),
    };
    ir::Video {
        width: n("width") as u32,
        height: n("height") as u32,
        fps: n("fps") as u32,
        duration: n("duration"),
        background,
    }
}

impl Sema {
    fn run(&mut self, ast: &Program) -> ir::Program {
        // ---- pass 1: the video block --------------------------------------
        let mut videos = ast.items.iter().filter_map(|i| match &i.kind {
            ItemKind::Video(b) => Some((i, b)),
            _ => None,
        });
        let video = match videos.next() {
            Some((_, block)) => self.check_video(block),
            None => {
                self.diags.push(
                    Diagnostic::error("S009", Span::new(0, 0), "missing 'video { ... }' block")
                        .with_help("add: video { width: 640; height: 360; fps: 30; duration: 4s; }"),
                );
                video_from(&video_defaults())
            }
        };
        for (item, _) in videos {
            self.diags.push(Diagnostic::error("S010", item.span, "only one 'video' block is allowed"));
        }

        self.duration = video.duration;
        for (name, val) in [
            ("width", video.width as f64),
            ("height", video.height as f64),
            ("fps", video.fps as f64),
            ("duration", video.duration),
        ] {
            self.vars.insert(
                name.to_string(),
                Var { value: Value::Number(val), span: Span::new(0, 0), used: true, builtin: true },
            );
        }

        self.vars.insert(
            "pi".to_string(),
            Var { value: Value::Number(std::f64::consts::PI), span: Span::new(0, 0), used: true, builtin: true },
        );

        // `none` is a fully transparent color: `fill: none;` removes the fill.
        self.vars.insert(
            "none".to_string(),
            Var { value: Value::Color(Rgba::NONE), span: Span::new(0, 0), used: true, builtin: true },
        );

        // ---- pass 2: everything else, in order ------------------------------
        for item in &ast.items {
            match &item.kind {
                ItemKind::Video(_) => {}
                ItemKind::Let { name, value, attr } => self.item_let(name, value, attr.as_ref()),
                ItemKind::Shape { kind, name, body } => self.item_shape(kind, name, body, None),
                ItemKind::Group { name, body } => self.item_group(name, body, None),
                ItemKind::At { time, target, ease, body } => self.item_at(time, target, ease.as_ref(), body),
            }
        }

        // ---- lints ----------------------------------------------------------
        for (name, v) in &self.vars {
            if !v.used && !v.builtin && !name.starts_with('_') {
                self.diags.push(
                    Diagnostic::warning("W001", v.span, format!("variable '{name}' is never used"))
                        .with_help("prefix the name with '_' to silence this warning"),
                );
            }
        }

        // ---- resolve keyframes in time order, then lower ----------------------
        self.resolve_blocks();
        self.lower(video)
    }

    // ---- items --------------------------------------------------------------

    fn check_video(&mut self, block: &Block) -> ir::Video {
        let mut vals = video_defaults();
        let mut seen = HashSet::new();
        for p in &block.props {
            let Some(def) = schema::VIDEO_PROPS.iter().find(|d| d.name == p.name.name) else {
                let names: Vec<&str> = schema::VIDEO_PROPS.iter().map(|d| d.name).collect();
                self.unknown_prop(&p.name, "video", &names);
                continue;
            };
            if !seen.insert(def.name) {
                self.dup_prop(&p.name);
                continue;
            }
            self.reject_ease(p);
            let Some(v) = self.eval_as(&p.value, def.ty) else { continue };
            if let (Some((lo, hi)), Value::Number(n)) = (def.range, &v) {
                if *n < lo || *n > hi {
                    self.diags.push(Diagnostic::error(
                        "S011",
                        p.value.span,
                        format!("'{}' must be between {} and {}", def.name, fmt_num(lo), fmt_num(hi)),
                    ));
                    continue;
                }
                if matches!(def.name, "width" | "height" | "fps") && n.fract() != 0.0 {
                    self.diags.push(Diagnostic::error(
                        "S011",
                        p.value.span,
                        format!("'{}' must be a whole number", def.name),
                    ));
                    continue;
                }
            }
            vals.insert(def.name, v);
        }
        video_from(&vals)
    }

    fn item_let(&mut self, name: &Ident, expr: &Expr, attr: Option<&Attr>) {
        let v = self.eval(expr);
        if let Some(existing) = self.vars.get(&name.name) {
            let msg = if existing.builtin {
                format!("'{}' is a built-in and cannot be redefined", name.name)
            } else {
                format!("variable '{}' is already defined", name.name)
            };
            self.diags.push(Diagnostic::error("S012", name.span, msg));
            return;
        }
        match v {
            Some(value) => {
                let value = match attr {
                    Some(a) => self.apply_attr(a, name, expr, value),
                    None => value,
                };
                if schema::is_prop_name(&name.name) {
                    self.diags.push(
                        Diagnostic::warning(
                            "W004",
                            name.span,
                            format!("variable '{}' is hidden by the property '{}' inside 'at' blocks", name.name, name.name),
                        )
                        .with_help("inside 'at' blocks that name means the property's previous value; rename the variable to use it there"),
                    );
                }
                self.vars.insert(name.name.clone(), Var { value, span: name.span, used: false, builtin: false });
            }
            None => {
                self.bad_vars.insert(name.name.clone());
            }
        }
    }

    /// Validate an `@attribute` on a `let` and register the inspector
    /// parameter. Returns the variable's value, with the inspector override
    /// applied. On any problem it reports a diagnostic and returns `value` as is.
    fn apply_attr(&mut self, attr: &Attr, name: &Ident, expr: &Expr, value: Value) -> Value {
        let Some(&kind) = params::ATTRIBUTES.iter().find(|k| **k == attr.name.name) else {
            let mut d = Diagnostic::error("S021", attr.name.span, format!("unknown attribute '@{}'", attr.name.name));
            d = match suggest(&attr.name.name, params::ATTRIBUTES.iter().copied()) {
                Some(s) => d.with_help(format!("did you mean '@{s}'?")),
                None => d.with_help(format!("available: {}", params::ATTRIBUTES.iter().map(|a| format!("@{a}")).collect::<Vec<_>>().join(", "))),
            };
            self.diags.push(d);
            return value;
        };
        let args: Vec<Value> = attr.args.iter().filter_map(|a| self.eval(a)).collect();
        if args.len() != attr.args.len() {
            return value; // the argument's own error was already reported
        }
        let (mut min, mut max, mut step, mut choices) = (None, None, None, Vec::new());
        let problem: Option<String> = match kind {
            "slider" => {
                let nums: Vec<f64> = args.iter().filter_map(|a| if let Value::Number(n) = a { Some(*n) } else { None }).collect();
                if nums.len() != args.len() || !(2..=3).contains(&nums.len()) {
                    Some("@slider takes numbers: @slider(min, max) or @slider(min, max, step)".into())
                } else if nums[0] >= nums[1] {
                    Some("@slider needs min < max".into())
                } else if nums.get(2).is_some_and(|s| *s <= 0.0) {
                    Some("the @slider step must be positive".into())
                } else if !matches!(value, Value::Number(_)) {
                    Some("@slider needs a number variable".into())
                } else {
                    (min, max, step) = (Some(nums[0]), Some(nums[1]), nums.get(2).copied());
                    None
                }
            }
            "color" | "text" => {
                let want = if kind == "color" { Ty::Color } else { Ty::Str };
                if !args.is_empty() {
                    Some(format!("@{kind} takes no arguments"))
                } else if value.ty() != want {
                    Some(format!("@{kind} needs a {} variable", if kind == "color" { "color" } else { "string" }))
                } else {
                    None
                }
            }
            _ => {
                let strs: Vec<String> = args.iter().filter_map(|a| if let Value::Str(s) = a { Some(s.clone()) } else { None }).collect();
                if strs.len() != args.len() || strs.len() < 2 {
                    Some("@choice takes at least two strings: @choice(\"a\", \"b\")".into())
                } else if !matches!(&value, Value::Str(s) if strs.contains(s)) {
                    Some("the @choice variable must be set to one of the listed strings".into())
                } else {
                    choices = strs;
                    None
                }
            }
        };
        if let Some(msg) = problem {
            self.diags.push(Diagnostic::error("S022", attr.span, msg));
            return value;
        }
        let Some(default) = params::to_json(&value) else { return value };
        let current = self
            .overrides
            .get(&name.name)
            .and_then(|ov| params::from_override(kind, &choices, ov))
            .unwrap_or(value);
        let shown = params::to_json(&current).unwrap_or_else(|| default.clone());
        self.params.push(Param {
            name: name.name.clone(),
            kind,
            value: shown,
            default,
            min,
            max,
            step,
            choices,
            start: expr.span.start,
            end: expr.span.end,
        });
        current
    }

    fn add_node(&mut self, name: &Ident, kind: ShapeKind, parent: Option<usize>) -> usize {
        let ix = self.nodes.len();
        self.nodes.push(NodeInfo {
            name: name.name.clone(),
            kind,
            parent,
            base: HashMap::new(),
            blocks: Vec::new(),
            claimed: HashMap::new(),
            keys: HashMap::new(),
        });
        self.node_ix.insert(name.name.clone(), ix);
        ix
    }

    fn item_shape(&mut self, kind: &Ident, name: &Ident, body: &Block, parent: Option<usize>) {
        let Some(sk) = schema::find_shape(&kind.name) else {
            let names: Vec<&str> = schema::SHAPES.iter().map(|s| s.0).collect();
            let help = match suggest(&kind.name, names.iter().copied()) {
                Some(s) => format!("did you mean '{s}'?"),
                None => format!("available shapes: {}", names.join(", ")),
            };
            self.diags.push(
                Diagnostic::error("S001", kind.span, format!("unknown shape kind '{}'", kind.name)).with_help(help),
            );
            self.poisoned.insert(name.name.clone());
            return;
        };
        if self.node_ix.contains_key(&name.name) {
            self.diags.push(Diagnostic::error("S002", name.span, format!("a shape named '{}' already exists", name.name)));
            return;
        }
        let ix = self.add_node(name, sk, parent);
        let mut seen = HashSet::new();
        for p in &body.props {
            self.reject_ease(p);
            if let Some((def, v)) = self.check_assign(sk, p, &mut seen) {
                self.nodes[ix].base.insert(def.name, v);
            }
        }
    }

    fn item_group(&mut self, name: &Ident, body: &GroupBody, parent: Option<usize>) {
        if self.node_ix.contains_key(&name.name) {
            self.diags.push(Diagnostic::error("S002", name.span, format!("a shape named '{}' already exists", name.name)));
            self.poison_children(&body.children);
            return;
        }
        let ix = self.add_node(name, ShapeKind::Group, parent);
        let mut seen = HashSet::new();
        for p in &body.props {
            self.reject_ease(p);
            if let Some((def, v)) = self.check_assign(ShapeKind::Group, p, &mut seen) {
                self.nodes[ix].base.insert(def.name, v);
            }
        }
        for child in &body.children {
            match &child.kind {
                ItemKind::Shape { kind, name, body } => self.item_shape(kind, name, body, Some(ix)),
                ItemKind::Group { name, body } => self.item_group(name, body, Some(ix)),
                _ => {}
            }
        }
    }

    /// When a group is rejected its children never get declared; remember
    /// their names so later `at` blocks on them do not pile on more errors.
    fn poison_children(&mut self, items: &[Item]) {
        for item in items {
            match &item.kind {
                ItemKind::Shape { name, .. } => {
                    self.poisoned.insert(name.name.clone());
                }
                ItemKind::Group { name, body } => {
                    self.poisoned.insert(name.name.clone());
                    self.poison_children(&body.children);
                }
                _ => {}
            }
        }
    }

    fn item_at(&mut self, time: &Expr, target: &Ident, block_ease: Option<&Ident>, body: &Block) {
        let t = self.eval_as(time, Ty::Number);
        let default_ease = block_ease.and_then(|e| self.lookup_ease(e));

        let found = self.node_ix.get(&target.name).copied();
        if found.is_none() && !self.poisoned.contains(&target.name) {
            let names: Vec<&str> = self.nodes.iter().map(|s| s.name.as_str()).collect();
            let help = match suggest(&target.name, names.iter().copied()) {
                Some(s) => format!("did you mean '{s}'?"),
                None if names.is_empty() => "declare a shape first, e.g. rect box { ... }".to_string(),
                None => format!("declared shapes: {}", names.join(", ")),
            };
            self.diags.push(
                Diagnostic::error("S003", target.span, format!("unknown shape '{}'", target.name)).with_help(help),
            );
        }
        let (Some(Value::Number(t)), Some(ix)) = (t, found) else { return };

        if t < 0.0 {
            self.diags.push(Diagnostic::error("S015", time.span, "keyframe time cannot be negative"));
            return;
        }
        if t > self.duration + EPS {
            self.diags.push(Diagnostic::warning(
                "W002",
                time.span,
                format!(
                    "keyframe at {}s is after the end of the video ({}s); the animation will not finish",
                    fmt_num(t),
                    fmt_num(self.duration)
                ),
            ));
        }

        let kind = self.nodes[ix].kind;
        self.seq += 1;
        let seq = self.seq;
        let mut seen = HashSet::new();
        let mut props = Vec::new();
        for p in &body.props {
            let ease = match &p.ease {
                Some(e) => self.lookup_ease(e).unwrap_or(Easing::Linear),
                None => default_ease.unwrap_or(Easing::Linear),
            };
            let Some(def) = self.lookup_def(kind, p, &mut seen) else { continue };
            // Inside `at`, property names are allowed: they mean "previous value".
            let Some(sym) = self.eval_sym(&p.value, Some(kind)) else { continue };
            if !def.ty.accepts(sym.ty()) {
                self.diags.push(Diagnostic::error(
                    "S005",
                    p.value.span,
                    format!("expected a {}, found a {}", def.ty, sym.ty()),
                ));
                continue;
            }
            if let Sym::Const(v) = &sym {
                if !self.check_choice(def, v, p.value.span) {
                    continue;
                }
            }
            let duplicate = self.nodes[ix]
                .claimed
                .get(def.name)
                .map_or(false, |ts| ts.iter().any(|x| (x - t).abs() < EPS));
            if duplicate {
                self.diags.push(Diagnostic::error(
                    "S006",
                    p.name.span,
                    format!("'{}.{}' already has a keyframe at {}s", self.nodes[ix].name, def.name, fmt_num(t)),
                ));
                continue;
            }
            self.nodes[ix].claimed.entry(def.name).or_default().push(t);
            props.push(PendingProp { def, value: sym, span: p.value.span, ease });
        }
        self.nodes[ix].blocks.push(PendingBlock { time: t, seq, props });
    }

    // ---- shared checks ----------------------------------------------------------

    /// Find a property in a node kind's schema, rejecting unknown and repeated ones.
    fn lookup_def(
        &mut self,
        kind: ShapeKind,
        p: &PropAssign,
        seen: &mut HashSet<&'static str>,
    ) -> Option<&'static PropDef> {
        let Some(def) = schema::find_prop(kind, &p.name.name) else {
            let names: Vec<&str> = schema::shape_props(kind).map(|d| d.name).collect();
            self.unknown_prop(&p.name, kind.name(), &names);
            return None;
        };
        if !seen.insert(def.name) {
            self.dup_prop(&p.name);
            return None;
        }
        Some(def)
    }

    /// A constant `prop: value` in a declaration block.
    fn check_assign(
        &mut self,
        kind: ShapeKind,
        p: &PropAssign,
        seen: &mut HashSet<&'static str>,
    ) -> Option<(&'static PropDef, Value)> {
        let def = self.lookup_def(kind, p, seen)?;
        let v = self.eval_as(&p.value, def.ty)?;
        if !self.check_choice(def, &v, p.value.span) {
            return None;
        }
        self.range_warn(def, &v, p.value.span);
        Some((def, v))
    }

    /// For string properties with a fixed set of values (like `align`).
    fn check_choice(&mut self, def: &PropDef, v: &Value, span: Span) -> bool {
        if let (Some(choices), Value::Str(s)) = (def.choices, v) {
            if !choices.contains(&s.as_str()) {
                let help = match suggest(s, choices.iter().copied()) {
                    Some(x) => format!("did you mean \"{x}\"?"),
                    None => format!(
                        "expected one of: {}",
                        choices.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(", ")
                    ),
                };
                self.diags.push(
                    Diagnostic::error("S020", span, format!("invalid value \"{s}\" for '{}'", def.name)).with_help(help),
                );
                return false;
            }
        }
        true
    }

    fn range_warn(&mut self, def: &PropDef, v: &Value, span: Span) {
        if let (Some((lo, hi)), Value::Number(n)) = (def.range, v) {
            if *n < lo || *n > hi {
                self.diags.push(Diagnostic::warning(
                    "W003",
                    span,
                    format!("'{}' is {}, outside the expected range ({})", def.name, fmt_num(*n), range_text(lo, hi)),
                ));
            }
        }
    }

    fn unknown_prop(&mut self, id: &Ident, owner: &str, names: &[&str]) {
        let help = match suggest(&id.name, names.iter().copied()) {
            Some(s) => format!("did you mean '{s}'?"),
            None => format!("available properties: {}", names.join(", ")),
        };
        self.diags.push(
            Diagnostic::error("S004", id.span, format!("unknown property '{}' for {owner}", id.name)).with_help(help),
        );
    }

    fn dup_prop(&mut self, id: &Ident) {
        self.diags.push(Diagnostic::error(
            "S013",
            id.span,
            format!("property '{}' is set more than once in this block", id.name),
        ));
    }

    fn reject_ease(&mut self, p: &PropAssign) {
        if let Some(e) = &p.ease {
            self.diags.push(
                Diagnostic::error("S014", e.span, "'ease' only applies to keyframes in an 'at' block")
                    .with_help("remove it here, or move this value into an 'at' block"),
            );
        }
    }

    fn lookup_ease(&mut self, id: &Ident) -> Option<Easing> {
        if let Some(e) = schema::find_easing(&id.name) {
            return Some(e);
        }
        let names: Vec<&str> = schema::EASINGS.iter().map(|e| e.0).collect();
        let help = match suggest(&id.name, names.iter().copied()) {
            Some(s) => format!("did you mean '{s}'?"),
            None => format!("available easings: {}", names.join(", ")),
        };
        self.diags.push(Diagnostic::error("S016", id.span, format!("unknown easing '{}'", id.name)).with_help(help));
        None
    }

    // ---- expression evaluation ----------------------------------------------------

    /// Evaluate an expression that must be a compile-time constant.
    fn eval(&mut self, e: &Expr) -> Option<Value> {
        match self.eval_sym(e, None)? {
            Sym::Const(v) => Some(v),
            _ => None, // unreachable: without a target nothing can be symbolic
        }
    }

    fn eval_as(&mut self, e: &Expr, ty: Ty) -> Option<Value> {
        let v = self.eval(e)?;
        if !ty.accepts(v.ty()) {
            self.diags.push(Diagnostic::error("S005", e.span, format!("expected a {ty}, found a {}", v.ty())));
            return None;
        }
        Some(v)
    }

    /// Evaluate an expression. With `target = Some(kind)` (inside an `at`
    /// block) the target's property names are in scope and make the result
    /// symbolic; with `None` every name must be a constant.
    ///
    /// Returns `None` after recording a diagnostic when it cannot.
    fn eval_sym(&mut self, e: &Expr, target: Option<ShapeKind>) -> Option<Sym> {
        match &e.kind {
            ExprKind::Number(n) => Some(Sym::Const(Value::Number(*n))),
            ExprKind::Color(c) => Some(Sym::Const(Value::Color(*c))),
            ExprKind::Str(s) => Some(Sym::Const(Value::Str(s.clone()))),
            ExprKind::Ident(name) => {
                // Property names win over variables inside `at` blocks.
                if let Some(kind) = target {
                    if let Some(def) = schema::find_prop(kind, name) {
                        return Some(Sym::Prop(def.name, def.ty));
                    }
                }
                if let Some(v) = self.vars.get_mut(name) {
                    v.used = true;
                    return Some(Sym::Const(v.value.clone()));
                }
                if self.bad_vars.contains(name) {
                    return None;
                }
                let mut d = Diagnostic::error("S007", e.span, format!("unknown identifier '{name}'"));
                if target.is_none() && schema::is_prop_name(name) {
                    d = d.with_help(format!(
                        "'{name}' is a property; property names can only be used inside 'at' blocks, where they mean the value before that keyframe"
                    ));
                } else {
                    let mut candidates: Vec<&str> = self.vars.keys().map(|k| k.as_str()).collect();
                    if let Some(kind) = target {
                        candidates.extend(schema::shape_props(kind).map(|p| p.name));
                    }
                    if let Some(s) = suggest(name, candidates) {
                        d = d.with_help(format!("did you mean '{s}'?"));
                    }
                }
                self.diags.push(d);
                None
            }
            ExprKind::Unary { expr, .. } => match self.eval_sym(expr, target)? {
                Sym::Const(Value::Number(n)) => Some(Sym::Const(Value::Number(-n))),
                s if s.ty() == Ty::Number => Some(Sym::Neg(Box::new(s))),
                other => {
                    self.diags.push(Diagnostic::error("S005", e.span, format!("cannot negate a {}", other.ty())));
                    None
                }
            },
            ExprKind::Binary { op, lhs, rhs } => {
                let l = self.eval_sym(lhs, target);
                let r = self.eval_sym(rhs, target);
                let (l, r) = (l?, r?);
                if l.ty() != Ty::Number || r.ty() != Ty::Number {
                    self.diags.push(Diagnostic::error(
                        "S005",
                        e.span,
                        format!("cannot apply '{}' to a {} and a {}", op.symbol(), l.ty(), r.ty()),
                    ));
                    return None;
                }
                if matches!(op, BinOp::Div) {
                    if let Sym::Const(Value::Number(b)) = &r {
                        if *b == 0.0 {
                            self.diags.push(Diagnostic::error("S008", rhs.span, "division by zero"));
                            return None;
                        }
                    }
                }
                // Both sides known: fold now. Otherwise keep it symbolic.
                if let (Sym::Const(_), Sym::Const(_)) = (&l, &r) {
                    return match Sym::Bin(*op, Box::new(l), Box::new(r)).resolve(&HashMap::new()) {
                        Ok(v) => Some(Sym::Const(v)),
                        Err((code, msg)) => {
                            self.diags.push(Diagnostic::error(code, e.span, msg));
                            None
                        }
                    };
                }
                Some(Sym::Bin(*op, Box::new(l), Box::new(r)))
            }
            ExprKind::Call { callee, args } => {
                let Some(b) = builtins::find(&callee.name) else {
                    let names: Vec<&str> = builtins::BUILTINS.iter().map(|b| b.name).collect();
                    let help = match suggest(&callee.name, names.iter().copied()) {
                        Some(s) => format!("did you mean '{s}'?"),
                        None => format!("available functions: {}", names.join(", ")),
                    };
                    self.diags.push(
                        Diagnostic::error("S018", callee.span, format!("unknown function '{}'", callee.name))
                            .with_help(help),
                    );
                    return None;
                };
                if !b.params.accepts_count(args.len()) {
                    self.diags.push(
                        Diagnostic::error(
                            "S019",
                            e.span,
                            format!("'{}' takes {}, found {}", b.name, b.params.describe(), args.len()),
                        )
                        .with_help(b.doc),
                    );
                    return None;
                }
                let syms: Vec<Option<Sym>> = args.iter().map(|a| self.eval_sym(a, target)).collect();
                let syms: Vec<Sym> = syms.into_iter().collect::<Option<_>>()?;
                for (i, (arg, s)) in args.iter().zip(&syms).enumerate() {
                    let want = b.params.expected(i).unwrap_or(Ty::Number);
                    if !want.accepts(s.ty()) {
                        self.diags.push(
                            Diagnostic::error(
                                "S005",
                                arg.span,
                                format!("argument {} of '{}' must be a {}, found a {}", i + 1, b.name, want, s.ty()),
                            )
                            .with_help(b.doc),
                        );
                        return None;
                    }
                }
                let sym = Sym::Call(b.name, syms, b.ret);
                // All arguments known: fold now.
                if let Sym::Call(_, args, _) = &sym {
                    if args.iter().all(|a| matches!(a, Sym::Const(_))) {
                        return match sym.resolve(&HashMap::new()) {
                            Ok(v) => Some(Sym::Const(v)),
                            Err((code, msg)) => {
                                self.diags.push(Diagnostic::error(code, e.span, msg));
                                None
                            }
                        };
                    }
                }
                Some(sym)
            }
        }
    }

    // ---- keyframe resolution ----------------------------------------------------------

    /// Turn every `at` block into concrete keys.
    ///
    /// For each node, blocks are processed in (time, source order). Each
    /// block's expressions see the property values left by the blocks before
    /// it (initially the declared values), and all of a block's assignments
    /// are evaluated before any is applied, so `x: y; y: x;` swaps.
    fn resolve_blocks(&mut self) {
        for ix in 0..self.nodes.len() {
            let kind = self.nodes[ix].kind;
            let mut blocks = std::mem::take(&mut self.nodes[ix].blocks);
            blocks.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap().then(a.seq.cmp(&b.seq)));

            let mut current: HashMap<&'static str, Value> = self.nodes[ix].base.clone();
            for def in schema::shape_props(kind) {
                current.entry(def.name).or_insert_with(|| def.default.value());
            }

            for block in blocks {
                let mut updates = Vec::new();
                for p in &block.props {
                    match p.value.resolve(&current) {
                        Ok(v) => {
                            self.range_warn(p.def, &v, p.span);
                            updates.push((p.def.name, v, p.ease));
                        }
                        Err((code, msg)) => self.diags.push(Diagnostic::error(code, p.span, msg)),
                    }
                }
                for (name, value, ease) in updates {
                    current.insert(name, value.clone());
                    self.nodes[ix].keys.entry(name).or_default().push(Key { time: block.time, value, ease });
                }
            }
        }
    }

    // ---- lowering --------------------------------------------------------------------

    /// Build the IR tree. Every node gets one `Prop` per schema property; each
    /// prop's keys are sorted by time, with a synthetic t=0 key holding the
    /// declared (or default) value when the first user keyframe is later.
    fn lower(&self, video: ir::Video) -> ir::Program {
        let mut children_of: Vec<Vec<usize>> = vec![Vec::new(); self.nodes.len()];
        let mut roots = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            match n.parent {
                Some(p) => children_of[p].push(i),
                None => roots.push(i),
            }
        }
        let shapes = roots.iter().map(|&i| self.lower_node(i, &children_of)).collect();
        ir::Program { video, shapes, params: self.params.clone() }
    }

    fn lower_node(&self, i: usize, children_of: &[Vec<usize>]) -> ir::Shape {
        let info = &self.nodes[i];
        let mut props = Vec::new();
        for def in schema::shape_props(info.kind) {
            let base = info.base.get(def.name).cloned().unwrap_or_else(|| def.default.value());
            let mut keys = info.keys.get(def.name).cloned().unwrap_or_default();
            keys.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap());
            if let Some(first) = keys.first() {
                if first.time > EPS {
                    keys.insert(0, Key { time: 0.0, value: base.clone(), ease: Easing::Linear });
                }
            }
            props.push(ir::Prop { name: def.name, base, keys });
        }
        ir::Shape {
            name: info.name.clone(),
            kind: info.kind,
            props,
            children: children_of[i].iter().map(|&c| self.lower_node(c, children_of)).collect(),
        }
    }
}
