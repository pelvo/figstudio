use figstudio_fig::Val;

#[test]
fn finite_float_values_round_trip_without_rounding() {
    for value in [
        51.248178375505404,
        2.0030397744267762e-253,
        7.101215824554616e260,
        f64::MAX,
        f64::MIN_POSITIVE,
        -0.0,
    ] {
        let json = serde_json::to_string(&Val::Float(value)).unwrap();
        let Val::Float(decoded) = serde_json::from_str::<Val>(&json).unwrap() else {
            panic!("float must remain a float");
        };
        assert_eq!(decoded.to_bits(), value.to_bits(), "{json}");
    }
}
