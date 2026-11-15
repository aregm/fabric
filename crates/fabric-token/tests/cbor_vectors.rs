//! Test vectors for the deterministic CBOR profile (task T3.1): the examples of RFC 8949
//! appendix A that the profile covers, its section 4.2.1 key order, the claims set of RFC 8392
//! appendix A.1, and the encodings the profile must refuse.

use fabric_policy::{Action, CloseReason};
use fabric_token::cbor::{CborError, Value, decode, encode, int, to_vec};
use fabric_token::{action_digest, hex};

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn bytes(value: &str) -> Value {
    Value::Bytes(hex::decode(value).unwrap())
}

fn array(items: Vec<Value>) -> Value {
    Value::Array(items)
}

fn tag(number: u64, value: Value) -> Value {
    Value::Tag(number, Box::new(value))
}

fn big(value: i128) -> Value {
    Value::Integer(value.try_into().unwrap())
}

/// `value` must list map entries in deterministic order, since decoding keeps the wire order.
fn check(value: &Value, expected: &str) {
    let encoded = encode(value).unwrap();
    assert_eq!(hex::encode(&encoded), expected, "encoding of {value:?}");
    assert_eq!(&decode(&encoded).unwrap(), value, "decoding of {expected}");
}

#[test]
fn rfc_8949_appendix_a_examples() {
    let integers: [(i128, &str); 16] = [
        (0, "00"),
        (1, "01"),
        (10, "0a"),
        (23, "17"),
        (24, "1818"),
        (25, "1819"),
        (100, "1864"),
        (1000, "1903e8"),
        (1_000_000, "1a000f4240"),
        (1_000_000_000_000, "1b000000e8d4a51000"),
        (18_446_744_073_709_551_615, "1bffffffffffffffff"),
        (-18_446_744_073_709_551_616, "3bffffffffffffffff"),
        (-1, "20"),
        (-10, "29"),
        (-100, "3863"),
        (-1000, "3903e7"),
    ];
    for (value, expected) in integers {
        check(&big(value), expected);
    }

    check(&Value::Bool(false), "f4");
    check(&Value::Bool(true), "f5");
    check(&Value::Null, "f6");

    check(
        &tag(0, text("2013-03-21T20:04:00Z")),
        "c074323031332d30332d32315432303a30343a30305a",
    );
    check(&tag(1, int(1_363_896_240)), "c11a514b67b0");
    check(&tag(23, bytes("01020304")), "d74401020304");
    check(&tag(24, bytes("6449455446")), "d818456449455446");
    check(
        &tag(32, text("http://www.example.com")),
        "d82076687474703a2f2f7777772e6578616d706c652e636f6d",
    );

    check(&bytes(""), "40");
    check(&bytes("01020304"), "4401020304");
    check(&text(""), "60");
    check(&text("a"), "6161");
    check(&text("IETF"), "6449455446");
    check(&text("\"\\"), "62225c");
    check(&text("\u{00fc}"), "62c3bc");
    check(&text("\u{6c34}"), "63e6b0b4");
    check(&text("\u{10151}"), "64f0908591");

    check(&array(vec![]), "80");
    check(&array(vec![int(1), int(2), int(3)]), "83010203");
    check(
        &array(vec![
            int(1),
            array(vec![int(2), int(3)]),
            array(vec![int(4), int(5)]),
        ]),
        "8301820203820405",
    );
    check(
        &array((1..=25).map(int).collect()),
        "98190102030405060708090a0b0c0d0e0f101112131415161718181819",
    );
    check(&Value::Map(vec![]), "a0");
    check(
        &Value::Map(vec![(int(1), int(2)), (int(3), int(4))]),
        "a201020304",
    );
    check(
        &Value::Map(vec![
            (text("a"), int(1)),
            (text("b"), array(vec![int(2), int(3)])),
        ]),
        "a26161016162820203",
    );
    check(
        &array(vec![text("a"), Value::Map(vec![(text("b"), text("c"))])]),
        "826161a161626163",
    );
    check(
        &Value::Map(
            ["a", "b", "c", "d", "e"]
                .into_iter()
                .map(|key| (text(key), text(&key.to_uppercase())))
                .collect(),
        ),
        "a56161614161626142616361436164614461656145",
    );
}

/// RFC 8949 section 4.2.1 lists these keys in core deterministic order.
#[test]
fn map_keys_follow_the_bytewise_order_of_their_encodings() {
    let keys = [
        int(10),
        int(100),
        int(-1),
        text("z"),
        text("aa"),
        array(vec![int(100)]),
        array(vec![int(-1)]),
        Value::Bool(false),
    ];
    let shuffled = Value::Map(
        keys.iter()
            .rev()
            .zip(0..)
            .map(|(key, value)| (key.clone(), int(value)))
            .collect(),
    );
    let encoded = encode(&shuffled).unwrap();
    assert_eq!(
        hex::encode(&encoded),
        "a80a071864062005617a0462616103811864028120\
         01f400"
    );
    let Value::Map(decoded) = decode(&encoded).unwrap() else {
        panic!("a map decodes as a map");
    };
    let decoded_keys: Vec<Value> = decoded.into_iter().map(|(key, _)| key).collect();
    assert_eq!(decoded_keys, keys);

    // Bytewise order puts the shorter "b" before "aa", unlike the order of the strings
    // themselves; `other_encodings_are_refused` checks that the string order is refused.
    check(
        &Value::Map(vec![(text("b"), int(0)), (text("aa"), int(0))]),
        "a261620062616100",
    );
    assert_eq!(
        encode(&Value::Map(vec![(text("aa"), int(0)), (text("b"), int(0))])).unwrap(),
        hex::decode("a261620062616100").unwrap()
    );
}

/// RFC 8392 appendix A.1: a claims set whose integer keys are already in deterministic order.
#[test]
fn rfc_8392_example_claims_set() {
    let claims = Value::Map(vec![
        (int(1), text("coap://as.example.com")),
        (int(2), text("erikw")),
        (int(3), text("coap://light.example.com")),
        (int(4), int(1_444_064_944)),
        (int(5), int(1_443_944_944)),
        (int(6), int(1_443_944_944)),
        (int(7), bytes("0b71")),
    ]);
    check(
        &claims,
        "a70175636f61703a2f2f61732e6578616d706c652e636f6d02656572696b77037818636f61703a2f2f6c\
         696768742e6578616d706c652e636f6d041a5612aeb0051a5610d9f0061a5610d9f007420b71",
    );
}

/// Each refused input is well-formed CBOR in a form other than the deterministic one, or an
/// item outside the profile, or not well-formed at all.
#[test]
fn other_encodings_are_refused() {
    let not_deterministic = [
        // RFC 8949 appendix A, indefinite-length items.
        "5f42010243030405ff",
        "7f657374726561646d696e67ff",
        "9fff",
        "9f018202039f0405ffff",
        "9f01820203820405ff",
        "83018202039f0405ff",
        "83019f0203ff820405",
        "9f0102030405060708090a0b0c0d0e0f101112131415161718181819ff",
        "bf61610161629f0203ffff",
        "826161bf61626163ff",
        "bf6346756ef563416d7421ff",
        // Arguments longer than needed.
        "1817",
        "190017",
        "1a00000017",
        "1b0000000000000017",
        "3817",
        "580100",
        "780161",
        "9800",
        "b800",
        "d81200",
        // Map keys out of order.
        "a203040102",
        "a2626161006162 00",
    ];
    for input in not_deterministic {
        let input = hex::decode(&input.replace(' ', "")).unwrap();
        assert_eq!(
            decode(&input),
            Err(CborError::NotDeterministic),
            "{}",
            hex::encode(&input)
        );
    }

    assert_eq!(
        decode(&hex::decode("a201000100").unwrap()),
        Err(CborError::DuplicateKey)
    );

    let outside_the_profile = [
        "f90000",
        "f93c00",
        "fa47c35000",
        "fb3ff199999999999a",
        "c249010000000000000000",
        "c24101",
        "c349010000000000000000",
    ];
    for input in outside_the_profile {
        let result = decode(&hex::decode(input).unwrap());
        assert!(
            matches!(
                result,
                Err(CborError::Unsupported(_) | CborError::NotDeterministic)
            ),
            "{input}: {result:?}"
        );
    }

    let not_accepted = [
        "f7", "f0", "f8ff", // simple values other than false, true and null
        "0000", // two items
        "18",   // truncated
        "6261", // truncated text
        "61ff", // text that is not UTF-8
        "",
    ];
    for input in not_accepted {
        assert!(
            decode(&hex::decode(input).unwrap()).is_err(),
            "{input} was accepted"
        );
    }
}

/// The `args` digest covers the action's JSON field names and its kind. The expected bytes are
/// assembled by hand, and the digest was computed independently of this crate.
#[test]
fn actions_encode_as_maps_of_their_json_fields() {
    let meeting = Action::AddMeeting {
        negotiation: "n1".to_owned(),
        start_ms: 1_799_762_400_000, // 2027-01-12T14:00:00Z
        length_ms: 1_800_000,
    };
    let expected = concat!(
        "a4",
        "646b696e64",
        "6b6164645f6d656574696e67",
        "6873746172745f6d73",
        "1b000001a30a32d300",
        "696c656e6774685f6d73",
        "1a001b7740",
        "6b6e65676f74696174696f6e",
        "626e31",
    );
    assert_eq!(hex::encode(&to_vec(&meeting).unwrap()), expected);
    assert_eq!(
        hex::encode(&action_digest(&meeting).unwrap()),
        "2e7468c2766e9bdee098c1bb0ecdacafa19c8c26360f957890cea752d957b394"
    );

    let close = Action::SendClose {
        negotiation: "n1".to_owned(),
        reason: CloseReason::Agreed,
        note: "thanks".to_owned(),
    };
    assert_eq!(
        hex::encode(&to_vec(&close).unwrap()),
        concat!(
            "a4",
            "646b696e64",
            "6a73656e645f636c6f7365",
            "646e6f7465",
            "667468616e6b73",
            "66726561736f6e",
            "66616772656564",
            "6b6e65676f74696174696f6e",
            "626e31",
        )
    );
}
