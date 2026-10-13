//! The fixed YAML options of `armadai_model::yaml`, and how `native` values are written.

use std::collections::BTreeMap;

use armadai_model::yaml::{from_str, to_string};
use armadai_model::{Manifest, NativeValue};
use serde_json::{Value, json};

#[test]
fn yaml_1_1_booleans_stay_strings() {
    let m: BTreeMap<String, Value> = from_str("a: yes\nb: on\nc: No\nd: true\n").unwrap();
    assert_eq!(
        Value::from_iter(m),
        json!({"a": "yes", "b": "on", "c": "No", "d": true})
    );
}

#[test]
fn merge_key_is_an_ordinary_key() {
    let m: BTreeMap<String, Value> = from_str("base: &b {x: 1}\n<<: *b\n").unwrap();
    assert_eq!(m.get("<<"), Some(&json!({"x": 1})));
    assert!(!m.contains_key("x"), "{m:?}");
}

#[test]
fn unknown_tag_is_refused_with_its_position() {
    let e = from_str::<Manifest>("schema_version: 1\nnative:\n  claude:\n    k: !foo bar\n")
        .unwrap_err();
    assert_eq!(e.location().map(|l| l.line()), Some(4), "{e}");
    assert!(e.to_string().contains("!foo"), "{e}");
}

#[test]
fn duplicate_key_in_a_free_map_is_refused_on_its_second_occurrence() {
    let e =
        from_str::<BTreeMap<String, BTreeMap<String, String>>>("a:\n  k: x\n  k: y\n").unwrap_err();
    assert_eq!(e.location().map(|l| (l.line(), l.column())), Some((3, 3)));
}

#[test]
fn error_message_is_one_line_without_snippet() {
    let e = from_str::<Manifest>("schema_version: 1\nbogus: 1\n").unwrap_err();
    assert!(!e.to_string().contains('\n'), "{e}");
}

#[test]
fn long_strings_are_never_folded() {
    let long = "word ".repeat(60).trim_end().to_string();
    let out = to_string(&BTreeMap::from([("k", long.clone())])).unwrap();
    assert_eq!(out, format!("k: {long}\n"));
}

#[test]
fn native_numbers_are_written_as_plain_yaml_numbers() {
    let src = "schema_version: 1\nnative:\n  claude:\n    zeta: 1\n    big: 18446744073709551615\n    neg: -9223372036854775808\n    huge: 123456789012345678901234567890\n    f: 0.1\n    list: [1, two, 3.5]\n";
    let m: Manifest = from_str(src).unwrap();
    let out = to_string(&m).unwrap();
    assert_eq!(
        out,
        "schema_version: 1\nnative:\n  claude:\n    zeta: 1\n    big: 18446744073709551615\n    neg: -9223372036854775808\n    huge: 1.2345678901234568e+29\n    f: 0.1\n    list:\n    - 1\n    - two\n    - 3.5\n"
    );
    assert_eq!(from_str::<Manifest>(&out).unwrap().native, m.native);
}

#[test]
fn native_keeps_the_author_key_order() {
    let m: Manifest =
        from_str("schema_version: 1\nnative:\n  claude:\n    z: 1\n    a: 2\n").unwrap();
    let keys: Vec<&String> = m.native["claude"].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["z", "a"]);
    assert_eq!(m.native["claude"], NativeValue(json!({"z": 1, "a": 2})));
}
