//! The Nomic semantic IR.
//!
//! This is the canonical machine representation of a model. The authoring
//! syntax parses into it, and it serializes to JSON so that other tools and
//! other implementations can consume models without sharing a parser. Nothing
//! in the evaluator depends on surface syntax; it only sees these structures.

use serde::{Deserialize, Serialize};

use crate::lexer::Pos;

/// Epistemic status of a declaration, per the production-modeling note.
/// Every declaration is `Required` unless it says otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Required,
    Observed,
    Expected,
    Assumed,
}

impl Status {
    pub fn keyword(self) -> &'static str {
        match self {
            Status::Required => "required",
            Status::Observed => "observed",
            Status::Expected => "expected",
            Status::Assumed => "assumed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypeRef {
    Int,
    Bool,
    Text,
    /// A user-declared type by name (enum or range alias).
    Named { name: String },
    /// An inline inclusive integer range such as `0..3`.
    Range { lo: i64, hi: i64 },
    /// `T?`: a value of `T` or `none`. Only meaningful for derived results and
    /// parameters; facts are already optional by absence.
    Opt { inner: Box<TypeRef> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Param {
    pub name: String,
    pub ty: TypeRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypeDef {
    Enum { variants: Vec<String> },
    Range { lo: i64, hi: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeDecl {
    pub name: String,
    pub doc: Option<String>,
    pub def: TypeDef,
    pub pos: Pos,
}

/// A fact schema. Facts are functional relations: a key tuple maps to at most
/// one value. A fact with no value type is a set of key tuples (present or
/// absent). A fact with no keys and a value type is a single variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactDecl {
    pub name: String,
    pub doc: Option<String>,
    pub status: Status,
    pub keys: Vec<Param>,
    pub value: Option<TypeRef>,
    pub pos: Pos,
}

/// A pure derived value: a function of authoritative state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeriveDecl {
    pub name: String,
    pub doc: Option<String>,
    pub status: Status,
    pub params: Vec<Param>,
    pub result: TypeRef,
    pub body: Expr,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionDecl {
    pub name: String,
    pub doc: Option<String>,
    pub params: Vec<Param>,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventDecl {
    pub name: String,
    pub doc: Option<String>,
    pub status: Status,
    pub params: Vec<Param>,
    /// Condition-backed events are edge-triggered on this predicate.
    pub when: Option<Expr>,
    pub pos: Pos,
}

/// The occurrence a rule matches: an action or event name with a pattern
/// argument per parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pattern {
    pub name: String,
    pub args: Vec<PatArg>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PatArg {
    Bind { name: String },
    Wildcard,
    Literal { value: Literal },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleDecl {
    pub name: String,
    pub doc: Option<String>,
    pub status: Status,
    pub on: Pattern,
    /// Applicability guard evaluated against the current state with the
    /// pattern's bindings. False means the rule does not match at all, as
    /// opposed to `require`, which means the occurrence is forbidden.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Expr>,
    pub body: Vec<Stmt>,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvariantDecl {
    pub name: String,
    pub doc: Option<String>,
    pub status: Status,
    pub body: Expr,
    pub pos: Pos,
}

/// A named, first-class departure from an invariant. When the invariant fails
/// in a state where the exception's condition holds, the state is accepted and
/// the exception is reported as exercised.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExceptionDecl {
    pub name: String,
    pub doc: Option<String>,
    #[serde(default)]
    pub status: Status,
    pub invariant: String,
    pub when: Expr,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioDecl {
    pub name: String,
    pub doc: Option<String>,
    pub steps: Vec<Step>,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Step {
    /// Directly establish a fact in the starting state, bypassing rules.
    Given { fact: String, keys: Vec<Expr>, value: Option<Expr>, pos: Pos },
    /// `given nothing`: discard the initial state and start empty.
    Clear { pos: Pos },
    /// Attempt an action and require the given outcome.
    Act { action: String, args: Vec<Expr>, outcome: Outcome, pos: Pos },
    /// Assert a proposition about the current state.
    Expect { expr: Expr, pos: Pos },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    /// The action must be accepted. If `emits` is present, the set of events
    /// emitted over the whole transition must equal it exactly.
    Accepted { emits: Option<Vec<EventRef>> },
    /// The action must be rejected, optionally by a named rule.
    Rejected { by: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRef {
    pub name: String,
    pub args: Vec<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Stmt {
    /// `require e ["reason"]`: deny when `e` is false.
    Require { cond: Expr, reason: Option<String>, pos: Pos },
    /// `deny [if e] ["reason"]`: deny (when `e` is true).
    Deny { cond: Option<Expr>, reason: Option<String>, pos: Pos },
    /// `allow`: explicit no-op disposition.
    Allow { pos: Pos },
    /// `assert F(k...) [= v]`.
    Assert { fact: String, keys: Vec<Expr>, value: Option<Expr>, pos: Pos },
    /// `retract F(k...)`.
    Retract { fact: String, keys: Vec<Expr>, pos: Pos },
    /// `emit E(args...)`.
    Emit { event: String, args: Vec<Expr>, pos: Pos },
    /// `let x = e`.
    Let { name: String, value: Expr, pos: Pos },
    /// `if e { ... } [else { ... }]`.
    If { cond: Expr, then: Vec<Stmt>, els: Vec<Stmt>, pos: Pos },
    /// `for (x: T, ... [where e]) { ... }`: bounded effect comprehension.
    For { binders: Vec<Binder>, filter: Option<Expr>, body: Vec<Stmt>, pos: Pos },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Literal {
    Int { value: i64 },
    Bool { value: bool },
    Text { value: String },
    None,
    /// An enum variant. The owning type is resolved by the checker.
    Variant { name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BinOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    And,
    Or,
    /// `a ?? b`: `a` unless it is `none`, else `b`.
    Coalesce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quantifier {
    All,
    Exists,
    Count,
    Sum,
    /// Deterministic selection: the first binding in canonical order that
    /// satisfies the filter, or `none`.
    First,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binder {
    pub name: String,
    pub ty: TypeRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchArm {
    pub pattern: PatArg,
    pub body: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Expr {
    Lit { value: Literal, pos: Pos },
    /// A bare name: local binding, zero-arity fact/derive, or enum variant.
    Name { name: String, pos: Pos },
    /// `F(args)`: a fact lookup or derived value application.
    Call { name: String, args: Vec<Expr>, pos: Pos },
    Unary { op: UnOp, expr: Box<Expr>, pos: Pos },
    Binary { op: BinOp, left: Box<Expr>, right: Box<Expr>, pos: Pos },
    Ternary { cond: Box<Expr>, then: Box<Expr>, els: Box<Expr>, pos: Pos },
    Quant {
        q: Quantifier,
        binders: Vec<Binder>,
        filter: Option<Box<Expr>>,
        body: Box<Expr>,
        pos: Pos,
    },
    Match { subject: Box<Expr>, arms: Vec<MatchArm>, pos: Pos },
    /// `legal(A(args))`: would action `A` be accepted from the current state?
    /// Considers rule dispositions and `ensure` postconditions over the
    /// action's own effects; reactions are not simulated.
    Legal { action: String, args: Vec<Expr>, pos: Pos },
}

impl Expr {
    pub fn pos(&self) -> Pos {
        match self {
            Expr::Lit { pos, .. }
            | Expr::Name { pos, .. }
            | Expr::Call { pos, .. }
            | Expr::Unary { pos, .. }
            | Expr::Binary { pos, .. }
            | Expr::Ternary { pos, .. }
            | Expr::Quant { pos, .. }
            | Expr::Match { pos, .. }
            | Expr::Legal { pos, .. } => *pos,
        }
    }
}

/// How cited code relates to a declaration (see the source-citations note).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    /// The cited code implements the declaration; the formal channel should agree with it.
    Realizes,
    /// The declaration was reverse-engineered from the cited code and may simplify it.
    DerivesFrom,
    /// The cited artifact supports the claim (a test, a log, a fixture).
    Evidences,
    /// The cited code disagrees with the declaration's intent.
    Contradicts,
    /// The cited code binds a configuration value the declaration refers to.
    Configures,
    /// Prose that informed the description channel.
    Documents,
}

impl Relation {
    pub fn keyword(self) -> &'static str {
        match self {
            Relation::Realizes => "realizes",
            Relation::DerivesFrom => "derives_from",
            Relation::Evidences => "evidences",
            Relation::Contradicts => "contradicts",
            Relation::Configures => "configures",
            Relation::Documents => "documents",
        }
    }
    pub fn from_keyword(s: &str) -> Option<Relation> {
        Some(match s {
            "realizes" => Relation::Realizes,
            "derives_from" => Relation::DerivesFrom,
            "evidences" => Relation::Evidences,
            "contradicts" => Relation::Contradicts,
            "configures" => Relation::Configures,
            "documents" => Relation::Documents,
            _ => return None,
        })
    }
}

/// A citation grounding a declaration in a location in a repository.
/// Locator syntax: `path[#Lstart[-Lend]][@pin]` where `pin` is the content
/// hash of the cited lines as computed by `nomic cite --pin`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Citation {
    /// Name of the declaration (or the model) this citation grounds.
    pub target: String,
    pub relation: Relation,
    pub path: String,
    pub lines: Option<(u32, u32)>,
    pub pin: Option<String>,
    pub note: Option<String>,
    /// The locator exactly as written, for in-place re-pinning.
    pub raw: String,
    pub pos: Pos,
}

/// `import "path" { Name [as Alias], ... }`: bring named declarations (and
/// what they depend on) from another file. Paths are relative to the
/// importing file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportDecl {
    pub path: String,
    pub names: Vec<ImportName>,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportName {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

/// `include "path"`: bring a whole module's declarations and behavior
/// (rules, invariants, ensures, exceptions, init) into this model.
/// Scenarios are not included; they are the included module's own claims.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncludeDecl {
    pub path: String,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Model {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<ImportDecl>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub includes: Vec<IncludeDecl>,
    pub name: Option<String>,
    pub doc: Option<String>,
    pub types: Vec<TypeDecl>,
    pub facts: Vec<FactDecl>,
    pub derives: Vec<DeriveDecl>,
    pub actions: Vec<ActionDecl>,
    pub events: Vec<EventDecl>,
    pub rules: Vec<RuleDecl>,
    pub invariants: Vec<InvariantDecl>,
    /// Postconditions on an action's own effects. Unlike invariants, a failed
    /// `ensure` rejects the action instead of reporting a model error.
    pub ensures: Vec<InvariantDecl>,
    pub exceptions: Vec<ExceptionDecl>,
    /// Effects establishing the initial state.
    pub init: Vec<Stmt>,
    /// Where the `init` block starts, for tools that reproduce layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init_pos: Option<Pos>,
    pub scenarios: Vec<ScenarioDecl>,
    pub citations: Vec<Citation>,
}

impl Model {
    pub fn fact(&self, name: &str) -> Option<&FactDecl> {
        self.facts.iter().find(|f| f.name == name)
    }
    pub fn derive(&self, name: &str) -> Option<&DeriveDecl> {
        self.derives.iter().find(|d| d.name == name)
    }
    pub fn action(&self, name: &str) -> Option<&ActionDecl> {
        self.actions.iter().find(|a| a.name == name)
    }
    pub fn event(&self, name: &str) -> Option<&EventDecl> {
        self.events.iter().find(|e| e.name == name)
    }
    pub fn type_decl(&self, name: &str) -> Option<&TypeDecl> {
        self.types.iter().find(|t| t.name == name)
    }
    pub fn invariant(&self, name: &str) -> Option<&InvariantDecl> {
        self.invariants.iter().find(|i| i.name == name)
    }
    /// The enum type owning a variant name, if exactly one does.
    pub fn variant_owner(&self, variant: &str) -> Option<&TypeDecl> {
        let mut found = None;
        for t in &self.types {
            if let TypeDef::Enum { variants } = &t.def {
                if variants.iter().any(|v| v == variant) {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(t);
                }
            }
        }
        found
    }
}
