//! Runtime values and the world state.
//!
//! Values have a total canonical order so that every enumeration, effect set,
//! and event set in the machine is deterministic regardless of authoring order.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ast::{Literal, Model, TypeDef, TypeRef};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "t", content = "v", rename_all = "snake_case")]
pub enum Value {
    None,
    Bool(bool),
    Int(i64),
    Text(String),
    /// An enum variant, carried with its owning type name.
    Variant(String, String),
    /// An identity from an open domain: (type name, representation).
    /// Equality is by representation; the domain is never enumerated.
    Opaque(String, String),
    /// A tuple of values. Never stored in state; used while enumerating
    /// multi-name binders.
    Row(Vec<Value>),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::None => write!(f, "none"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(n) => write!(f, "{n}"),
            Value::Text(s) => write!(f, "{s:?}"),
            Value::Variant(_, v) => write!(f, "{v}"),
            Value::Opaque(t, r) => write!(f, "{t}({r:?})"),
            Value::Row(vs) => {
                let parts: Vec<String> = vs.iter().map(|v| v.to_string()).collect();
                write!(f, "({})", parts.join(", "))
            }
        }
    }
}

impl Value {
    pub fn type_name(&self) -> String {
        match self {
            Value::None => "none".into(),
            Value::Bool(_) => "Bool".into(),
            Value::Int(_) => "Int".into(),
            Value::Text(_) => "Text".into(),
            Value::Variant(t, _) | Value::Opaque(t, _) => t.clone(),
            Value::Row(_) => "row".into(),
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            _ => None,
        }
    }

    /// Does this value inhabit the given type? `none` inhabits nothing.
    pub fn inhabits(&self, ty: &TypeRef, model: &Model) -> bool {
        match (ty, self) {
            (TypeRef::Opt { .. }, Value::None) => true,
            (TypeRef::Opt { inner }, v) => v.inhabits(inner, model),
            (TypeRef::Int, Value::Int(_)) => true,
            (TypeRef::Bool, Value::Bool(_)) => true,
            (TypeRef::Text, Value::Text(_)) => true,
            (TypeRef::Range { lo, hi }, Value::Int(n)) => n >= lo && n <= hi,
            (TypeRef::Named { name }, v) => match model.type_decl(name).map(|t| &t.def) {
                Some(TypeDef::Enum { .. }) => matches!(v, Value::Variant(t, _) if t == name),
                Some(TypeDef::Range { lo, hi }) => matches!(v, Value::Int(n) if n >= lo && n <= hi),
                Some(TypeDef::Opaque) => matches!(v, Value::Opaque(t, _) if t == name),
                None => false,
            },
            _ => false,
        }
    }

    pub fn from_literal(lit: &Literal, model: &Model) -> Result<Value, String> {
        Ok(match lit {
            Literal::Int { value } => Value::Int(*value),
            Literal::Bool { value } => Value::Bool(*value),
            Literal::Text { value } => Value::Text(value.clone()),
            Literal::None => Value::None,
            Literal::Variant { name } => {
                let owner = model
                    .variant_owner(name)
                    .ok_or_else(|| format!("`{name}` is not a variant of exactly one enum type"))?;
                Value::Variant(owner.name.clone(), name.clone())
            }
        })
    }
}

/// Enumerate every inhabitant of a finite type in canonical order.
/// Returns `None` for unbounded types (`Int`, `Text`).
pub fn inhabitants(ty: &TypeRef, model: &Model) -> Option<Vec<Value>> {
    match ty {
        TypeRef::Int | TypeRef::Text | TypeRef::Opt { .. } => None,
        TypeRef::Bool => Some(vec![Value::Bool(false), Value::Bool(true)]),
        TypeRef::Range { lo, hi } => Some((*lo..=*hi).map(Value::Int).collect()),
        TypeRef::Named { name } => {
            let decl = model.type_decl(name)?;
            match &decl.def {
                TypeDef::Enum { variants } => {
                    Some(variants.iter().map(|v| Value::Variant(name.clone(), v.clone())).collect())
                }
                TypeDef::Range { lo, hi } => Some((*lo..=*hi).map(Value::Int).collect()),
                TypeDef::Opaque => None,
            }
        }
    }
}

pub fn type_display(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Int => "Int".into(),
        TypeRef::Bool => "Bool".into(),
        TypeRef::Text => "Text".into(),
        TypeRef::Named { name } => name.clone(),
        TypeRef::Range { lo, hi } => format!("{lo}..{hi}"),
        TypeRef::Opt { inner } => format!("{}?", type_display(inner)),
    }
}

/// A key tuple identifying one instance of a fact.
pub type Key = Vec<Value>;

/// The authoritative world state: for every fact schema, the map from key
/// tuple to value. Set-like facts store `Value::Bool(true)` as their value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub struct State {
    pub facts: BTreeMap<String, BTreeMap<Key, Value>>,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn lookup(&self, fact: &str, key: &Key) -> Option<&Value> {
        self.facts.get(fact).and_then(|m| m.get(key))
    }

    pub fn instances(&self, fact: &str) -> impl Iterator<Item = (&Key, &Value)> {
        self.facts.get(fact).into_iter().flat_map(|m| m.iter())
    }

    /// Every opaque identity of type `ty` present anywhere in the state, as a
    /// key or a value: the known population of an open domain.
    pub fn identities_of(&self, ty: &str) -> std::collections::BTreeSet<Value> {
        let mut out = std::collections::BTreeSet::new();
        for instances in self.facts.values() {
            for (key, value) in instances {
                for v in key.iter().chain(std::iter::once(value)) {
                    if let Value::Opaque(t, _) = v {
                        if t == ty {
                            out.insert(v.clone());
                        }
                    }
                }
            }
        }
        out
    }

    pub fn set(&mut self, fact: &str, key: Key, value: Value) {
        self.facts.entry(fact.to_string()).or_default().insert(key, value);
    }

    pub fn remove(&mut self, fact: &str, key: &Key) -> bool {
        let Some(m) = self.facts.get_mut(fact) else { return false };
        let removed = m.remove(key).is_some();
        if m.is_empty() {
            self.facts.remove(fact);
        }
        removed
    }

    /// A stable, human-readable rendering: one line per fact instance, in
    /// canonical order. Set-like facts (no value type) render without `= v`.
    pub fn render(&self, model: &Model) -> Vec<String> {
        let mut out = Vec::new();
        for (fact, instances) in &self.facts {
            let set_like = model.fact(fact).is_some_and(|f| f.value.is_none());
            for (key, value) in instances {
                let mut s = fact.clone();
                if !key.is_empty() {
                    let ks: Vec<String> = key.iter().map(|v| v.to_string()).collect();
                    s.push('(');
                    s.push_str(&ks.join(", "));
                    s.push(')');
                }
                if !set_like {
                    s.push_str(" = ");
                    s.push_str(&value.to_string());
                }
                out.push(s);
            }
        }
        out
    }
}
