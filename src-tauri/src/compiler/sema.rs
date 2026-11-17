//! Stage 3: semantic analysis and lowering (AST -> IR).
//!
//! The parser only checks *shape*; this stage checks *meaning*:
//!
//! * symbol tables: `let` variables, shape names, the built-in `width`,
//!   `height`, `fps`, `duration`
//! * type checking: every property value must match its schema type
//! * constant evaluation: all expressions (`width / 2 + 10`, `rgb(...)`,
//!   `cos(30)`) are folded to plain values, so the IR contains no expressions
//! * validation: unknown shapes/properties/easings (with "did you mean"),
//!   duplicates, ranges, keyframes after the end of the video, unused variables
//! * lowering: building the sorted per-property keyframe timelines of the IR
//!
//! It runs in two passes: first the `video` block (which defines the built-in
//! constants), then everything else in source order. A `let` must come before
//! its first use. Errors never abort the pass: it records a diagnostic and
//! keeps going so the user sees every problem at once.

use super::ast::*;
use super::builtins;
use super::diagnostics::{has_errors, suggest, Diagnostic};
use super::ir::{self, Easing, Key};
use super::schema::{self, PropDef, ShapeKind};
use super::span::Span;
use super::value::{Rgb, Ty, Value};
use std::collections::{HashMap, HashSet};

const EPS: f64 = 1e-9;

pub fn analyze(ast: &Program) -> (Option<ir::Program>, Vec<Diagnostic>) {
    let mut s = Sema {
        diags: Vec::new(),
        vars: HashMap::new(),
        bad_vars: HashSet::new(),
        shapes: Vec::new(),
        shape_ix: HashMap::new(),
        poisoned: HashSet::new(),
        duration: 0.0,
    };
    let program = s.run(ast);
    let mut diags = s.diags;
    diags.sort_by_key(|d| d.span.start);
    let program = if has_errors(&diags) { None } else { Some(program) };
    (program, diags)
}

struct Var {
    value: Value,
    span: Span,
    used: bool,
    builtin: bool,
}

struct ShapeInfo {
    name: String,
    kind: ShapeKind,
    base: HashMap<&'static str, Value>,
    keys: HashMap<&'static str, Vec<Key>>,
}

struct Sema {
    diags: Vec<Diagnostic>,
    vars: HashMap<String, Var>,
    /// `let`s whose value failed to evaluate: uses are silenced to avoid cascades.
    bad_vars: HashSet<String>,
    shapes: Vec<ShapeInfo>,
    shape_ix: HashMap<String, usize>,
    /// Shapes whose declaration failed: later `at` blocks on them are skipped silently.
    poisoned: HashSet<String>,
    duration: f64,
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
        _ => Rgb(0, 0, 0),
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

        // ---- pass 2: everything else, in order ------------------------------
        for item in &ast.items {
            match &item.kind {
                ItemKind::Video(_) => {}
                ItemKind::Let { name, value } => self.item_let(name, value),
                ItemKind::Shape { kind, name, body } => self.item_shape(kind, name, body),
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

    fn item_let(&mut self, name: &Ident, value: &Expr) {
        let v = self.eval(value);
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
                self.vars.insert(name.name.clone(), Var { value, span: name.span, used: false, builtin: false });
            }
            None => {
                self.bad_vars.insert(name.name.clone());
            }
        }
    }

    fn item_shape(&mut self, kind: &Ident, name: &Ident, body: &Block) {
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
        if self.shape_ix.contains_key(&name.name) {
            self.diags.push(Diagnostic::error(
                "S002",
                name.span,
                format!("a shape named '{}' already exists", name.name),
            ));
            return;
        }
        let mut info = ShapeInfo { name: name.name.clone(), kind: sk, base: HashMap::new(), keys: HashMap::new() };
        let mut seen = HashSet::new();
        for p in &body.props {
            self.reject_ease(p);
            if let Some((def, v)) = self.check_assign(sk, p, &mut seen) {
                info.base.insert(def.name, v);
            }
        }
        self.shape_ix.insert(name.name.clone(), self.shapes.len());
        self.shapes.push(info);
    }

    fn item_at(&mut self, time: &Expr, target: &Ident, block_ease: Option<&Ident>, body: &Block) {
        let t = self.eval_as(time, Ty::Number);
        let default_ease = block_ease.and_then(|e| self.lookup_ease(e));

        let found = self.shape_ix.get(&target.name).copied();
        if found.is_none() && !self.poisoned.contains(&target.name) {
            let names: Vec<&str> = self.shapes.iter().map(|s| s.name.as_str()).collect();
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

        let kind = self.shapes[ix].kind;
        let mut seen = HashSet::new();
        for p in &body.props {
            let ease = match &p.ease {
                Some(e) => self.lookup_ease(e).unwrap_or(Easing::Linear),
                None => default_ease.unwrap_or(Easing::Linear),
            };
            let Some((def, v)) = self.check_assign(kind, p, &mut seen) else { continue };

            let duplicate = self.shapes[ix]
                .keys
                .get(def.name)
                .map_or(false, |ks| ks.iter().any(|k| (k.time - t).abs() < EPS));
            if duplicate {
                self.diags.push(Diagnostic::error(
                    "S006",
                    p.name.span,
                    format!("'{}.{}' already has a keyframe at {}s", self.shapes[ix].name, def.name, fmt_num(t)),
                ));
                continue;
            }
            self.shapes[ix].keys.entry(def.name).or_default().push(Key { time: t, value: v, ease });
        }
    }

    // ---- shared checks ----------------------------------------------------------

    /// Check one `prop: value` assignment against a shape's schema.
    fn check_assign(
        &mut self,
        kind: ShapeKind,
        p: &PropAssign,
        seen: &mut HashSet<&'static str>,
    ) -> Option<(&'static PropDef, Value)> {
        let Some(def) = schema::find_prop(kind, &p.name.name) else {
            let names: Vec<&str> = schema::shape_props(kind).map(|d| d.name).collect();
            self.unknown_prop(&p.name, kind.name(), &names);
            return None;
        };
        if !seen.insert(def.name) {
            self.dup_prop(&p.name);
            return None;
        }
        let v = self.eval_as(&p.value, def.ty)?;
        if let (Some((lo, hi)), Value::Number(n)) = (def.range, &v) {
            if *n < lo || *n > hi {
                self.diags.push(Diagnostic::warning(
                    "W003",
                    p.value.span,
                    format!("'{}' is {}, outside the expected range ({})", def.name, fmt_num(*n), range_text(lo, hi)),
                ));
            }
        }
        Some((def, v))
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

    // ---- constant evaluation ------------------------------------------------------

    fn eval_as(&mut self, e: &Expr, ty: Ty) -> Option<Value> {
        let v = self.eval(e)?;
        if v.ty() != ty {
            self.diags.push(Diagnostic::error(
                "S005",
                e.span,
                format!("expected a {ty}, found a {}", v.ty()),
            ));
            return None;
        }
        Some(v)
    }

    /// Evaluate an expression to a value. Returns `None` after recording a
    /// diagnostic when it cannot.
    fn eval(&mut self, e: &Expr) -> Option<Value> {
        match &e.kind {
            ExprKind::Number(n) => Some(Value::Number(*n)),
            ExprKind::Color(c) => Some(Value::Color(*c)),
            ExprKind::Str(s) => Some(Value::Str(s.clone())),
            ExprKind::Ident(name) => {
                if let Some(v) = self.vars.get_mut(name) {
                    v.used = true;
                    return Some(v.value.clone());
                }
                if self.bad_vars.contains(name) {
                    return None;
                }
                let mut d = Diagnostic::error("S007", e.span, format!("unknown identifier '{name}'"));
                if let Some(s) = suggest(name, self.vars.keys().map(|k| k.as_str())) {
                    d = d.with_help(format!("did you mean '{s}'?"));
                }
                self.diags.push(d);
                None
            }
            ExprKind::Unary { expr, .. } => match self.eval(expr)? {
                Value::Number(n) => Some(Value::Number(-n)),
                other => {
                    self.diags.push(Diagnostic::error("S005", e.span, format!("cannot negate a {}", other.ty())));
                    None
                }
            },
            ExprKind::Binary { op, lhs, rhs } => {
                let l = self.eval(lhs);
                let r = self.eval(rhs);
                let (l, r) = (l?, r?);
                match (&l, &r) {
                    (Value::Number(a), Value::Number(b)) => {
                        let out = match op {
                            BinOp::Add => a + b,
                            BinOp::Sub => a - b,
                            BinOp::Mul => a * b,
                            BinOp::Div => {
                                if *b == 0.0 {
                                    self.diags.push(Diagnostic::error("S008", rhs.span, "division by zero"));
                                    return None;
                                }
                                a / b
                            }
                        };
                        if !out.is_finite() {
                            self.diags.push(Diagnostic::error("S017", e.span, "result is not a finite number"));
                            return None;
                        }
                        Some(Value::Number(out))
                    }
                    _ => {
                        self.diags.push(Diagnostic::error(
                            "S005",
                            e.span,
                            format!("cannot apply '{}' to a {} and a {}", op.symbol(), l.ty(), r.ty()),
                        ));
                        None
                    }
                }
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
                if args.len() != b.arity {
                    self.diags.push(
                        Diagnostic::error(
                            "S019",
                            e.span,
                            format!("'{}' takes {} argument(s), found {}", b.name, b.arity, args.len()),
                        )
                        .with_help(b.doc),
                    );
                    return None;
                }
                let vals: Vec<Option<Value>> = args.iter().map(|a| self.eval(a)).collect();
                let vals: Vec<Value> = vals.into_iter().collect::<Option<_>>()?;
                match builtins::call(&callee.name, &vals) {
                    Ok(v) => Some(v),
                    Err(msg) => {
                        self.diags.push(Diagnostic::error("S005", e.span, msg));
                        None
                    }
                }
            }
        }
    }

    // ---- lowering --------------------------------------------------------------------

    /// Build the IR. Every shape gets one `Prop` per schema property; each
    /// prop's keys are sorted by time, with a synthetic t=0 key holding the
    /// declared (or default) value when the first user keyframe is later.
    fn lower(&self, video: ir::Video) -> ir::Program {
        let mut shapes = Vec::new();
        for info in &self.shapes {
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
            shapes.push(ir::Shape { name: info.name.clone(), kind: info.kind, props });
        }
        ir::Program { video, shapes }
    }
}
