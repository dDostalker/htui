//! Rust ↔ JS value conversion for host functions.
//!
//! The [`IntoJs`] / [`FromJs`] traits hide the Boa value types from
//! application code: a command returns a plain Rust value (or a `Result`)
//! and the conversion layer turns it into a `JsValue`, and vice versa for
//! arguments. The [`JsObjectBuilder`] returned by
//! [`HostCtx::object`](crate::script::HostCtx::object) builds JS objects
//! incrementally, so a host function that returns `{ host, os, cores }` needs
//! no `ObjectInitializer` boilerplate.
//!
//! All fallible conversions report a Boa [`JsError`], which the conversion
//! plumbing turns into a JS exception. [`HostError`] is the Boa-free
//! equivalent: `Result<T, HostError>` throws without the user ever naming an
//! engine type.

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

/// Boa-free error type for host functions.
///
/// Host code returns `Result<T, HostError>` and the engine converts it into
/// a JS exception (`Error`, `TypeError` or `RangeError`), so the caller can
/// `try/catch` it on the JS side. Construct it from a string, or use
/// [`HostError::type_error`] / [`HostError::range_error`] for typed errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostError {
    kind: HostErrorKind,
    message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostErrorKind {
    Error,
    TypeError,
    RangeError,
}

impl HostError {
    /// A generic `Error` exception.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            kind: HostErrorKind::Error,
            message: message.into(),
        }
    }

    /// A `TypeError` exception.
    pub fn type_error(message: impl Into<String>) -> Self {
        Self {
            kind: HostErrorKind::TypeError,
            message: message.into(),
        }
    }

    /// A `RangeError` exception.
    pub fn range_error(message: impl Into<String>) -> Self {
        Self {
            kind: HostErrorKind::RangeError,
            message: message.into(),
        }
    }

    /// The human-readable error message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HostError {}

impl From<String> for HostError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl From<&str> for HostError {
    fn from(message: &str) -> Self {
        Self::new(message.to_owned())
    }
}

/// Wraps a Boa error (e.g. from a [`FromJs`] conversion) so `?` works inside
/// `Result<_, HostError>` closures.
impl From<JsError> for HostError {
    fn from(e: JsError) -> Self {
        Self {
            kind: HostErrorKind::Error,
            message: e.to_string(),
        }
    }
}

impl IntoJsError for HostError {
    fn into_js_error(self) -> JsError {
        let native = match self.kind {
            HostErrorKind::Error => JsNativeError::error(),
            HostErrorKind::TypeError => JsNativeError::typ(),
            HostErrorKind::RangeError => JsNativeError::range(),
        };
        JsError::from_native(native.with_message(self.message))
    }
}

/// Errors host code can return from a `Result<_, E>` command.
///
/// `String` / `&str` become generic `Error` exceptions; [`HostError`]
/// preserves its typed kind (`Error` / `TypeError` / `RangeError`).
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
// JsObjectBuilder — incremental JS object construction.
// ---------------------------------------------------------------------------

/// Incremental JS object builder returned by
/// [`HostCtx::object`](crate::script::HostCtx::object), hiding
/// `ObjectInitializer`.
///
/// Owns its properties instead of borrowing a Boa [`Context`]: values are
/// converted lazily when the object is assembled. This makes the builder
/// [`IntoJs`], so host functions can return it directly — no engine type
/// appears in user code.
/// Lazily-converted object property: the boxed closure captures the raw
/// Rust value and produces a `JsValue` only when the object is assembled
/// inside a Boa [`Context`].
type LazyProp = Box<dyn FnOnce(&mut Context) -> JsValue>;

pub struct JsObjectBuilder {
    props: Vec<(String, LazyProp)>,
}

impl JsObjectBuilder {
    /// Start an empty object builder.
    pub fn new() -> Self {
        Self { props: Vec::new() }
    }

    /// Add one property. Conversion failures are logged and skipped so one bad
    /// value does not abort the whole object.
    pub fn set<V: IntoJs + 'static>(&mut self, key: impl Into<String>, val: V) {
        let key = key.into();
        self.props.push((
            key.clone(),
            Box::new(move |ctx| match val.into_js(ctx) {
                Ok(v) => v,
                Err(e) => {
                    log::warn!("JsObjectBuilder: property `{key}` failed to convert: {e}");
                    JsValue::undefined()
                }
            }),
        ));
    }

    /// Assemble the JS object, converting all stored values with `ctx`.
    pub fn build(self, ctx: &mut Context) -> JsValue {
        let props: Vec<(String, JsValue)> = self
            .props
            .into_iter()
            .map(|(key, conv)| (key, conv(ctx)))
            .collect();
        let mut init = ObjectInitializer::new(ctx);
        for (key, v) in props {
            init.property(js_string!(key), v, Attribute::all());
        }
        init.build().into()
    }
}

impl Default for JsObjectBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl IntoJs for JsObjectBuilder {
    fn into_js(self, ctx: &mut Context) -> Result<JsValue, JsError> {
        Ok(self.build(ctx))
    }
}
