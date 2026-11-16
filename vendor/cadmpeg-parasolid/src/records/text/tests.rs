// SPDX-License-Identifier: Apache-2.0
use super::*;

fn header(embedded: bool) -> String {
    let description = ": TRANSMIT FILE created by modeller version 1400000";
    let schema = if embedded {
        "SCH_1400000_14000_13006"
    } else {
        "SCH_1300000_13006"
    };
    format!(
        "T{} {description}{} {schema}{}0 ",
        description.len(),
        schema.len(),
        if embedded { "250 " } else { "" }
    )
}

#[test]
fn analytic_point_preserves_signed_integer_null_pointer_and_vector() {
    let text = format!("{}29 1 -7 0 32767 0 0 1.25 -2 3 1 0", header(false));
    let actual = to_neutral_binary(text.as_bytes()).unwrap();
    let mut expected = b"PS".to_vec();
    let description = b": TRANSMIT FILE created by modeller version 1400000";
    let schema = b"SCH_1300000_13006";
    expected.extend(u32::try_from(description.len()).unwrap().to_be_bytes());
    expected.extend(description);
    expected.extend(u32::try_from(schema.len()).unwrap().to_be_bytes());
    expected.extend(schema);
    expected.extend(0u32.to_be_bytes());
    expected.extend([0, 29, 0, 2]);
    expected.extend((-7i32).to_be_bytes());
    expected.extend([0, 1, 255, 255, 0, 1, 0, 1, 0, 1]);
    for real in [1.25f64, -2.0, 3.0] {
        expected.extend(real.to_be_bytes());
    }
    expected.extend([0, 1, 0, 1]);
    assert_eq!(actual, expected);
    let records = super::super::PrimaryRecords::read(&actual)
        .unwrap()
        .unwrap();
    assert!(records.is_terminated());
    assert_eq!(
        records.nodes().next().unwrap().vector("pvec"),
        Some([1.25, -2.0, 3.0])
    );
}

#[test]
fn embedded_custom_types_preserve_escapes_and_null_vector() {
    let text = format!(
        "{}240 2 4 TEST4 Test4 text0 9 1 c3 vec0 0 1 v1 \\9?1 0",
        header(true)
    );
    let binary = to_neutral_binary(text.as_bytes()).unwrap();
    let records = super::super::PrimaryRecords::read(&binary)
        .unwrap()
        .unwrap();
    assert!(records.is_terminated());
    let node = records.nodes().next().unwrap();
    assert_eq!(
        binary.get(node.field_range("text").unwrap()).unwrap(),
        b"         "
    );
    assert_eq!(
        binary.get(node.field_range("vec").unwrap()).unwrap(),
        super::super::fields::NULL_REAL.repeat(3)
    );
}

#[test]
fn embedded_base_schema_and_wrapped_numeric_tokens() {
    let text = format!("{}29 255 1 ?0 0 0 0 1.\r\n25 -2 3 1 0", header(true));
    let binary = to_neutral_binary(text.as_bytes()).unwrap();
    let records = super::super::PrimaryRecords::read(&binary)
        .unwrap()
        .unwrap();
    assert!(records.is_terminated());
    assert_eq!(
        records.nodes().next().unwrap().vector("pvec"),
        Some([1.25, -2.0, 3.0])
    );
}

#[test]
fn malformed_and_oversized_text_is_rejected() {
    for suffix in [
        "29 1 0 0 0 0 0 NaN 0 0 1 0",
        "83 4000001 1 1 0",
        "1 7",
        "29 1 0",
        "1 0 junk",
    ] {
        assert!(to_neutral_binary(format!("{}{suffix}", header(false)).as_bytes()).is_err());
    }
    let text = format!("{}29 1 0 0 0 0 0 1 2 3 1 0", header(false));
    for end in 0..text.len() {
        let _ = to_neutral_binary(&text.as_bytes()[..end]);
    }
}
