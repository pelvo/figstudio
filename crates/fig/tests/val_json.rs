use figstudio_fig::{Guid, Obj, Val};

#[test]
fn values_use_natural_json_and_sorted_object_keys() {
    let value = Val::Obj(Obj::from([
        ("z", Val::Bytes(vec![0, 255])),
        ("a", Val::Enum("RECTANGLE")),
        ("n", Val::Float(-0.0)),
    ]));
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#"{"a":"RECTANGLE","n":-0.0,"z":[0,255]}"#
    );
    let decoded: Val = serde_json::from_str(r#"{"a":true,"z":[1,2.5]}"#).unwrap();
    assert_eq!(decoded.get("a"), Some(&Val::Bool(true)));
    assert_eq!(
        decoded.get("z"),
        Some(&Val::Arr(vec![Val::Int(1), Val::Float(2.5)]))
    );
    assert_eq!(serde_json::to_string(&Guid(0, 1)).unwrap(), "[0,1]");
    assert_eq!(serde_json::from_str::<Guid>("[0,1]").unwrap(), Guid(0, 1));
}

#[test]
fn ambiguous_types_normalize_and_invalid_numbers_do_not_silently_become_null() {
    for (value, normalized) in [
        (Val::Enum("X"), Val::Str("X".into())),
        (Val::Bytes(vec![1]), Val::Arr(vec![Val::Int(1)])),
    ] {
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(serde_json::from_str::<Val>(&json).unwrap(), normalized);
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let json = serde_json::to_string(&Val::Arr(vec![Val::Float(value)])).unwrap();
        assert!(json.contains("$float_bits"));
        let decoded = serde_json::from_str::<Val>(&json).unwrap();
        let Val::Arr(items) = decoded else {
            panic!("expected an array");
        };
        let Val::Float(back) = &items[0] else {
            panic!("expected a float");
        };
        assert_eq!(back.to_bits(), value.to_bits());
    }
    // Objects that merely contain a $float_bits key stay ordinary objects.
    for input in [
        r#"{"$float_bits":"123","other":1}"#,
        r#"{"$float_bits":"xyz"}"#,
        r#"{"$float_bits":"00112233445566zz"}"#,
    ] {
        assert!(matches!(
            serde_json::from_str::<Val>(input).unwrap(),
            Val::Obj(_)
        ));
    }
    assert!(serde_json::from_str::<Val>("null").is_err());
    assert!(serde_json::from_str::<Obj>(r#"{"same":1,"same":2}"#).is_err());
    assert!(serde_json::from_str::<Val>("18446744073709551615").is_err());
    for value in [Val::Int(i64::MIN), Val::Int(i64::MAX), Val::Float(-0.0)] {
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(serde_json::from_str::<Val>(&json).unwrap(), value);
    }
}

#[test]
fn deserializing_object_keys_obeys_the_process_intern_budget() {
    let key = "x".repeat(32 * 1024 * 1024 + 1);
    let input = format!("{{\"{key}\":true}}");
    assert!(
        serde_json::from_str::<Obj>(&input)
            .unwrap_err()
            .to_string()
            .contains("budget")
    );
}
