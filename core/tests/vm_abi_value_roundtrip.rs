use mainstage_core::vm::value::Value as VmValue;

fn values_equal(a: &VmValue, b: &VmValue) -> bool {
    match (a, b) {
        (VmValue::Int(ai), VmValue::Int(bi)) => ai == bi,
        (VmValue::Float(af), VmValue::Float(bf)) => (af - bf).abs() < f64::EPSILON,
        (VmValue::Bool(ab), VmValue::Bool(bb)) => ab == bb,
        (VmValue::Str(asv), VmValue::Str(bsv)) => asv == bsv,
        (VmValue::Symbol(a2), VmValue::Symbol(b2)) => a2 == b2,
        (VmValue::Null, VmValue::Null) => true,
        (VmValue::Array(aa), VmValue::Array(ba)) => {
            if aa.len() != ba.len() { return false; }
            for i in 0..aa.len() { if !values_equal(&aa[i], &ba[i]) { return false; } }
            true
        }
        (VmValue::Object(ao), VmValue::Object(bo)) => {
            if ao.len() != bo.len() { return false; }
            for (k, v) in ao.iter() {
                match bo.get(k) { Some(v2) => if !values_equal(v, v2) { return false; }, None => return false }
            }
            true
        }
        _ => false,
    }
}

#[test]
fn vm_ir_value_roundtrip() {
    // build a nested vm value
    let original = VmValue::Array(vec![
        VmValue::Int(42),
        VmValue::Str("hello".to_string()),
        VmValue::Array(vec![VmValue::Bool(true), VmValue::Null]),
    ]);

    // Convert to IR value and back
    let irv = original.to_value();
    let round = VmValue::from(irv);

    assert!(values_equal(&original, &round), "Value roundtrip mismatch: original={:?} round={:?}", original, round);
}
