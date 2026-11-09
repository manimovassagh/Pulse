//! Conversion of values to the text shown in the DOM.
//!
//! Numbers are formatted the way JavaScript's `String(n)` does for the common
//! cases (`1` rather than `1.0`, `NaN`, `Infinity`), so TSX code renders the
//! same text it would under a JavaScript framework.

/// Converts a value to its display text.
pub trait ToText {
    fn to_text(&self) -> String;
}

impl ToText for f64 {
    fn to_text(&self) -> String {
        let v = *self;
        if v.is_nan() {
            "NaN".into()
        } else if v.is_infinite() {
            if v > 0.0 { "Infinity" } else { "-Infinity" }.into()
        } else if v == 0.0 {
            // Covers -0, which JavaScript prints as "0".
            "0".into()
        } else {
            // Rust's `Display` for f64 yields the shortest round-trip
            // representation without a trailing ".0", like JavaScript.
            format!("{v}")
        }
    }
}

impl ToText for bool {
    fn to_text(&self) -> String {
        if *self { "true" } else { "false" }.into()
    }
}

impl ToText for String {
    fn to_text(&self) -> String {
        self.clone()
    }
}

impl ToText for str {
    fn to_text(&self) -> String {
        self.to_owned()
    }
}

impl<T: ToText + ?Sized> ToText for &T {
    fn to_text(&self) -> String {
        (**self).to_text()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_format_like_javascript() {
        assert_eq!(1.0.to_text(), "1");
        assert_eq!((-0.0).to_text(), "0");
        assert_eq!(2.5.to_text(), "2.5");
        assert_eq!((0.1 + 0.2).to_text(), "0.30000000000000004");
        assert_eq!(f64::NAN.to_text(), "NaN");
        assert_eq!(f64::NEG_INFINITY.to_text(), "-Infinity");
        assert_eq!((-42.0).to_text(), "-42");
    }

    #[test]
    fn other_values() {
        assert_eq!(true.to_text(), "true");
        assert_eq!("hi".to_text(), "hi");
        assert_eq!(String::from("x").to_text(), "x");
    }
}
