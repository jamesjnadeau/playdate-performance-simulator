//! Argument handling shared by the bindings.

use crate::ctx::CtxRef;
use mlua::{IntoLuaMulti, Lua, MultiValue, Result, Table, Value, Variadic};

/// Hidden field holding a native handle in object tables.
pub const ID: &str = "__pd";

/// Positional arguments with geometry objects expanded to numbers.
pub struct Args(pub Vec<Value>);

impl Args {
    /// Expands rect/point/lineSegment/arc/size/vector tables into their
    /// numbers, so `fillRect(r)` and `fillRect(x, y, w, h)` look the same.
    pub fn flat(v: Variadic<Value>) -> Args {
        let mut out = Vec::with_capacity(v.len());
        for a in v {
            if let Value::Table(t) = &a {
                if let Some(kind) = kind_of(t) {
                    let g = |k: &str| t.raw_get::<Value>(k).unwrap_or(Value::Nil);
                    match kind.as_str() {
                        "rect" => out.extend([g("x"), g("y"), g("width"), g("height")]),
                        "point" => out.extend([g("x"), g("y")]),
                        "vector2D" => out.extend([g("dx"), g("dy")]),
                        "size" => out.extend([g("width"), g("height")]),
                        "lineSegment" => out.extend([g("x1"), g("y1"), g("x2"), g("y2")]),
                        "arc" => out.extend([g("x"), g("y"), g("radius"), g("startAngle"), g("endAngle")]),
                        _ => out.push(a.clone()),
                    }
                    continue;
                }
            }
            out.push(a);
        }
        Args(out)
    }

    /// Arguments as given.
    pub fn raw(v: Variadic<Value>) -> Args {
        Args(v.into_iter().collect())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, i: usize) -> &Value {
        self.0.get(i).unwrap_or(&Value::Nil)
    }

    pub fn is_nil(&self, i: usize) -> bool {
        self.get(i).is_nil()
    }

    pub fn num(&self, i: usize) -> Option<f64> {
        match self.get(i) {
            Value::Integer(n) => Some(*n as f64),
            Value::Number(n) => Some(*n),
            Value::String(s) => s.to_str().ok().and_then(|s| s.trim().parse().ok()),
            _ => None,
        }
    }

    /// Number or `def`.
    pub fn f(&self, i: usize, def: f64) -> f64 {
        self.num(i).unwrap_or(def)
    }

    /// Number truncated to an integer, or `def`.
    pub fn i(&self, i: usize, def: i32) -> i32 {
        self.num(i).map_or(def, |n| n as i32)
    }

    pub fn bool(&self, i: usize, def: bool) -> bool {
        match self.get(i) {
            Value::Nil => def,
            Value::Boolean(b) => *b,
            _ => true,
        }
    }

    pub fn str(&self, i: usize) -> Option<String> {
        match self.get(i) {
            Value::String(s) => Some(s.to_string_lossy()),
            Value::Integer(n) => Some(n.to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }

    pub fn table(&self, i: usize) -> Option<Table> {
        match self.get(i) {
            Value::Table(t) => Some(t.clone()),
            _ => None,
        }
    }

    /// Handle id of an object table argument.
    pub fn handle(&self, i: usize) -> Option<u32> {
        self.table(i)
            .and_then(|t| t.raw_get::<Option<u32>>(ID).ok().flatten())
    }

    /// The arguments from `i` on.
    pub fn rest(&self, i: usize) -> Args {
        Args(self.0.get(i..).map(|s| s.to_vec()).unwrap_or_default())
    }
}

/// The geometry kind of a table (`rect`, `point`, ...), from its class.
pub fn kind_of(t: &Table) -> Option<String> {
    let mt = t.metatable()?;
    mt.raw_get::<Option<String>>("__kind").ok().flatten()
}

/// Creates an object table with a native handle and a class metatable.
pub fn new_handle(lua: &Lua, class: &Table, id: u32) -> Result<Table> {
    let t = lua.create_table()?;
    t.raw_set(ID, id)?;
    t.set_metatable(Some(class.clone()))?;
    Ok(t)
}

/// Native handle of `self`.
pub fn self_id(args: &Args) -> Result<u32> {
    args.handle(0)
        .ok_or_else(|| mlua::Error::runtime("expected an object (did you call with '.' instead of ':'?)"))
}

/// Registers a function in `t`.
pub fn reg<F, R>(lua: &Lua, t: &Table, name: &str, ctx: &CtxRef, f: F) -> Result<()>
where
    F: Fn(&Lua, &CtxRef, Args) -> Result<R> + 'static,
    R: IntoLuaMulti,
{
    let ctx = ctx.clone();
    let fun = lua.create_function(move |lua, v: Variadic<Value>| f(lua, &ctx, Args::flat(v)))?;
    t.raw_set(name, fun)
}

/// Like `reg` but the arguments are not expanded.
pub fn reg_raw<F, R>(lua: &Lua, t: &Table, name: &str, ctx: &CtxRef, f: F) -> Result<()>
where
    F: Fn(&Lua, &CtxRef, Args) -> Result<R> + 'static,
    R: IntoLuaMulti,
{
    let ctx = ctx.clone();
    let fun = lua.create_function(move |lua, v: Variadic<Value>| f(lua, &ctx, Args::raw(v)))?;
    t.raw_set(name, fun)
}

/// Shorthand for "no return values".
pub fn none() -> MultiValue {
    MultiValue::new()
}

/// Gets or creates a sub-table.
pub fn subtable(lua: &Lua, t: &Table, name: &str) -> Result<Table> {
    match t.raw_get::<Value>(name)? {
        Value::Table(s) => Ok(s),
        _ => {
            let s = lua.create_table()?;
            t.raw_set(name, s.clone())?;
            Ok(s)
        }
    }
}

/// Creates a class table whose instances index into it.
pub fn class(lua: &Lua, parent: &Table, name: &str, kind: Option<&str>) -> Result<Table> {
    let c = subtable(lua, parent, name)?;
    c.raw_set("__index", c.clone())?;
    if let Some(k) = kind {
        c.raw_set("__kind", k)?;
    }
    Ok(c)
}
