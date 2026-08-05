//! Rust ↔ JS value conversion for host functions.
//!
//! The [`IntoJs`] / [`FromJs`] traits hide the Boa value types from
//! application code: a command returns a plain Rust value (or a `Result`)
//! and the conversion layer turns it into a `JsValue`, and vice versa for
//! arguments. The `js_obj!` macro builds JS objects from literal pairs, so a
//! host function that returns `{ host, os, cores }` needs no
//! `ObjectInitializer` boilerplate.
//!
//! All fallible conversions report a Boa [`JsError`], which a host function
//! can return directly to throw a JS exception.

use boa_engine::{
    Context, JsError, JsNativeError, JsString, JsValue, js_string, object::ObjectInitializer,
    property::Attribute,
};

/// Convert a Rust value into a JS value.
pub trait IntoJs {
    /// Convert `self` into a JS value using `ctx`.
    fn into_js(self, ctx: &mut Context) -> Result<JsValue, JsError>;
}

/// Convert a JS value back into a Rust value.
pub trait FromJs: Sized {
    /// Parse `value` (a JS argument) into a Rust value.
    fn from_js(value: &JsValue, ctx: &mut Context) -> Result<Self, JsError>;
}

/// Errors host code can return from a `Result<_, E>` command.
///
/// `String` / `&str` become generic `Error` exceptions; `JsNativeError`
/// passes through typed.
pub trait IntoJsError {
    /// Convert into a JS error.
    fn into_js_error(self) -> JsError;
}
impl IntoJsError for String {
    fn into_js_error(self) -> JsError {
        JsError::from_native(JsNativeError::error().with_message(self))
    }
}
impl IntoJsError for &str {
    fn into_js_error(self) -> JsError {
        JsError::from_native(JsNativeError::error().with_message(self.to_owned()))
    }
}
impl IntoJsError for JsNativeError {
    fn into_js_error(self) -> JsError {
        JsError::from_native(self)
    }
}
impl IntoJsError for JsError {
    fn into_js_error(self) -> JsError {
        self
    }
}

fn type_err(msg: impl Into<String>) -> JsError {
    JsError::from_native(JsNativeError::typ().with_message(msg.into()))
}

// ---------------------------------------------------------------------------
// Scalar / string impls
// ---------------------------------------------------------------------------

impl IntoJs for String {
    fn into_js(self, _ctx: &mut Context) -> Result<JsValue, JsError> {
        Ok(js_string!(self).into())
    }
}
impl IntoJs for &str {
    fn into_js(self, _ctx: &mut Context) -> Result<JsValue, JsError> {
        Ok(js_string!(self).into())
    }
}
impl IntoJs for JsString {
    fn into_js(self, _ctx: &mut Context) -> Result<JsValue, JsError> {
        Ok(self.into())
    }
}
impl IntoJs for f64 {
    fn into_js(self, _ctx: &mut Context) -> Result<JsValue, JsError> {
        Ok(JsValue::from(self))
    }
}
impl IntoJs for f32 {
    fn into_js(self, _ctx: &mut Context) -> Result<JsValue, JsError> {
        Ok(JsValue::from(self as f64))
    }
}
macro_rules! int_into_js {
    ($($t:ty),*) => {
        $(
            impl IntoJs for $t {
                fn into_js(self, _ctx: &mut Context) -> Result<JsValue, JsError> {
                    Ok(JsValue::from(self))
                }
            }
        )*
    };
}
int_into_js!(i8, i16, i32, i64, u8, u16, u32, u64, isize, usize);
impl IntoJs for bool {
    fn into_js(self, _ctx: &mut Context) -> Result<JsValue, JsError> {
        Ok(JsValue::from(self))
    }
}
impl IntoJs for () {
    fn into_js(self, _ctx: &mut Context) -> Result<JsValue, JsError> {
        Ok(JsValue::undefined())
    }
}
impl IntoJs for JsValue {
    fn into_js(self, _ctx: &mut Context) -> Result<JsValue, JsError> {
        Ok(self)
    }
}

impl<T: IntoJs> IntoJs for Option<T> {
    fn into_js(self, ctx: &mut Context) -> Result<JsValue, JsError> {
        match self {
            Some(v) => v.into_js(ctx),
            None => Ok(JsValue::null()),
        }
    }
}

impl<T: IntoJs> IntoJs for Vec<T> {
    fn into_js(self, ctx: &mut Context) -> Result<JsValue, JsError> {
        let arr = boa_engine::object::builtins::JsArray::new(ctx);
        for item in self {
            arr.push(item.into_js(ctx)?, ctx)?;
        }
        Ok(arr.into())
    }
}

/// A command may return `Result<T, E>`: `Ok` becomes the value, `Err` is
/// thrown as a JS error (caught by `try/catch` on the JS side).
impl<T: IntoJs, E: IntoJsError> IntoJs for Result<T, E> {
    fn into_js(self, ctx: &mut Context) -> Result<JsValue, JsError> {
        match self {
            Ok(v) => v.into_js(ctx),
            Err(e) => Err(e.into_js_error()),
        }
    }
}

// ---------------------------------------------------------------------------
// FromJs impls
// ---------------------------------------------------------------------------

impl FromJs for String {
    fn from_js(value: &JsValue, ctx: &mut Context) -> Result<Self, JsError> {
        value
            .to_string(ctx)
            .map(|s| s.to_std_string_lossy())
            .map_err(|_| type_err("expected a string"))
    }
}
impl FromJs for f64 {
    fn from_js(value: &JsValue, ctx: &mut Context) -> Result<Self, JsError> {
        value
            .to_number(ctx)
            .map_err(|_| type_err("expected a number"))
    }
}
macro_rules! num_from_js {
    ($($t:ty),*) => {
        $(
            impl FromJs for $t {
                fn from_js(value: &JsValue, ctx: &mut Context) -> Result<Self, JsError> {
                    let n = f64::from_js(value, ctx)?;
                    Ok(n as $t)
                }
            }
        )*
    };
}
num_from_js!(f32, i8, i16, i32, i64, u8, u16, u32, u64, isize, usize);
impl FromJs for bool {
    fn from_js(value: &JsValue, _ctx: &mut Context) -> Result<Self, JsError> {
        value
            .as_boolean()
            .ok_or_else(|| type_err("expected a boolean"))
    }
}
impl FromJs for () {
    fn from_js(_value: &JsValue, _ctx: &mut Context) -> Result<Self, JsError> {
        Ok(())
    }
}
impl FromJs for JsValue {
    fn from_js(value: &JsValue, _ctx: &mut Context) -> Result<Self, JsError> {
        Ok(value.clone())
    }
}

impl<T: FromJs> FromJs for Option<T> {
    fn from_js(value: &JsValue, ctx: &mut Context) -> Result<Self, JsError> {
        if value.is_undefined() || value.is_null() {
            return Ok(None);
        }
        T::from_js(value, ctx).map(Some)
    }
}

impl<T: FromJs> FromJs for Vec<T> {
    fn from_js(value: &JsValue, ctx: &mut Context) -> Result<Self, JsError> {
        let arr = value
            .as_object()
            .ok_or_else(|| type_err("expected an array"))?;
        let mut out = Vec::new();
        let mut i = 0u32;
        loop {
            let item = arr.get(i, ctx)?;
            if item.is_undefined() {
                break;
            }
            out.push(T::from_js(&item, ctx)?);
            i += 1;
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// js_obj! — build a JS object from literal key/value pairs.
// ---------------------------------------------------------------------------

/// Incremental builder behind [`js_obj!`], hiding `ObjectInitializer`.
pub struct JsObjectBuilder<'a> {
    ctx: &'a mut Context,
    props: Vec<(String, JsValue)>,
}

impl<'a> JsObjectBuilder<'a> {
    /// Start an empty object builder over `ctx`.
    pub fn new(ctx: &'a mut Context) -> Self {
        Self {
            ctx,
            props: Vec::new(),
        }
    }

    /// Add one property. Conversion failures are logged and skipped so one bad
    /// value does not abort the whole object.
    pub fn set<V: IntoJs>(&mut self, key: impl Into<String>, val: V) {
        let key = key.into();
        match val.into_js(self.ctx) {
            Ok(v) => self.props.push((key, v)),
            Err(e) => log::warn!("js_obj!: property `{key}` failed to convert: {e}"),
        }
    }

    /// Assemble the JS object.
    pub fn build(self) -> JsValue {
        let mut init = ObjectInitializer::new(self.ctx);
        for (k, v) in self.props {
            init.property(js_string!(k), v, Attribute::all());
        }
        init.build().into()
    }
}

/// Build a JS object from literal key/value pairs.
///
/// ```ignore
/// let obj = htui::js_obj!(call.context,
///     "host"  => host_string,
///     "cores" => 16u64,
/// );
/// ```
#[macro_export]
macro_rules! js_obj {
    ($ctx:expr, $($key:expr => $val:expr),* $(,)?) => {{
        let mut __builder = $crate::script::JsObjectBuilder::new($ctx);
        $( __builder.set($key, $val); )*
        __builder.build()
    }};
}
