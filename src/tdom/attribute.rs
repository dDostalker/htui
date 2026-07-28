use crate::tdom::node::Node;

// ----------------------------------------------------------
// Attribute accessors
//
// Only the handful of attribute reads that carry parsing semantics
// (class tokenisation, numeric coercion, id lookup) live here. Plain
// `attributes.get("foo")` is preferred for one-off keys so we don't
// grow the surface to match every HTML standard attribute.
// ----------------------------------------------------------
impl Node {
    /// Look up the `id` attribute (used by CSS `#id` selectors).
    pub fn get_id(&self) -> Option<&String> {
        self.attributes.get("id")
    }

    /// Tokenise the `class` attribute by whitespace.
    pub fn get_class(&self) -> Option<Vec<String>> {
        self.attributes
            .get("class")
            .map(|s| s.split_whitespace().map(str::to_string).collect())
    }

    /// `true` iff `class` contains the given token (used by CSS `.class`
    /// selectors).
    pub fn has_class(&self, name: &str) -> bool {
        self.get_class()
            .is_some_and(|classes| classes.iter().any(|c| c == name))
    }

    /// Look up the `src` attribute (used by `<img>`).
    pub fn get_src(&self) -> Option<&String> {
        self.attributes.get("src")
    }

    /// Parse the `width` attribute as `u32` pixels (used by `<img>`).
    pub fn get_width(&self) -> Option<u32> {
        self.attributes.get("width").and_then(|v| v.parse().ok())
    }

    /// Parse the `height` attribute as `u32` pixels (used by `<img>`).
    pub fn get_height(&self) -> Option<u32> {
        self.attributes.get("height").and_then(|v| v.parse().ok())
    }
}
