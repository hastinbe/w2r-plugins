//! `color_profiles.json` reading and writing over `w2r_formats::json` (strict: no duplicate keys,
//! no lone surrogates, at most 32 nested containers). Only what it lacks is here: a leading BOM is
//! dropped before parsing (Notepad adds one to files saved by hand), and the file is written
//! indented, since `Value::to_json` is compact.

pub use w2r_formats::json::{Number, Value};

/// `text` as JSON, after dropping one leading byte order mark. The error is the parser's reason
/// code (`not_json`, `duplicate_key`).
pub fn parse(text: &str) -> Result<Value, &'static str> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    w2r_formats::json::parse(text.as_bytes()).map_err(|r| r.code())
}

/// A whole number as a JSON number.
pub fn int(n: u64) -> Value {
    Value::Number(Number {
        text: n.to_string(),
    })
}

/// `v` as JSON indented by two spaces, ending in a line break: the layout the profile file is
/// written in.
pub fn pretty(v: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, v, 0);
    out.push('\n');
    out
}

fn write_value(out: &mut String, v: &Value, indent: usize) {
    let pad = |out: &mut String, n: usize| out.push_str(&"  ".repeat(n));
    match v {
        Value::Array(items) if !items.is_empty() => {
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                pad(out, indent + 1);
                write_value(out, item, indent + 1);
                out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
            }
            pad(out, indent);
            out.push(']');
        }
        Value::Object(fields) if !fields.is_empty() => {
            out.push_str("{\n");
            for (i, (k, item)) in fields.iter().enumerate() {
                pad(out, indent + 1);
                out.push_str(&Value::String(k.clone()).to_json());
                out.push_str(": ");
                write_value(out, item, indent + 1);
                out.push_str(if i + 1 < fields.len() { ",\n" } else { "\n" });
            }
            pad(out, indent);
            out.push('}');
        }
        // Scalars, `[]` and `{}`: the compact form is already the pretty one.
        other => out.push_str(&other.to_json()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_byte_order_mark_is_dropped() {
        let v = parse("\u{feff}{\"a\": \"\\ud83d\\ude00\"}").unwrap();
        assert_eq!(v.get("a").and_then(Value::as_str), Some("\u{1f600}"));
        assert!(parse("\u{feff}\u{feff}{}").is_err());
    }

    #[test]
    fn strict_documents_are_refused() {
        assert_eq!(parse("{\"a\": 1, \"a\": 2}"), Err("duplicate_key"));
        assert_eq!(parse("\"\\ud83d\""), Err("not_json"));
        assert_eq!(parse("[1,]"), Err("not_json"));
        assert_eq!(parse(&("[".repeat(40) + &"]".repeat(40))), Err("not_json"));
    }

    #[test]
    fn pretty_output_reads_back() {
        let text = r#"{"a": [1, 2.5, {"b": "x\"y"}], "c": {}, "d": [], "e": null, "f": true}"#;
        let v = parse(text).unwrap();
        let out = pretty(&v);
        assert_eq!(parse(&out), Ok(v));
        assert!(out.contains("\n  \"a\": [\n    1,\n    2.5,"));
        assert!(out.ends_with("}\n"));
    }

    #[test]
    fn whole_numbers_are_written_plainly() {
        assert_eq!(pretty(&int(3)), "3\n");
        assert_eq!(
            parse("3").unwrap().as_number().and_then(Number::as_u64),
            Some(3)
        );
    }
}
