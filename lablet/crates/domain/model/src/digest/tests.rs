use super::*;

/// SHA-256 of the three bytes `abc`, from FIPS 180-2's first example.
const ABC: [u8; 32] = [
    0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
    0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
];
const ABC_HEX: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

#[test]
fn a_digest_is_its_bytes_in_lower_case_hex_high_digit_first() {
    let digest = ConfigDigest::from_sha256(ABC);

    assert_eq!(digest.as_str(), ABC_HEX);
    assert_eq!(digest.to_string(), ABC_HEX);
    assert_eq!(String::from(digest), ABC_HEX);
}

#[test]
fn every_digit_a_byte_can_hold_is_written_as_hex() {
    let mut bytes = [0; 32];
    for (place, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::try_from(place * 8).unwrap();
    }
    bytes[31] = 0xff;

    let digest = ConfigDigest::from_sha256(bytes);

    assert_eq!(
        digest.as_str(),
        "0008101820283038404850586068707880889098a0a8b0b8c0c8d0d8e0e8f0ff"
    );
    assert_eq!(digest.as_str().len(), ConfigDigest::LEN);
}

#[test]
fn the_text_of_a_digest_reads_back_as_the_same_digest() {
    assert_eq!(
        ConfigDigest::new(ABC_HEX).unwrap(),
        ConfigDigest::from_sha256(ABC)
    );
    assert_eq!(
        ConfigDigest::new("0123456789abcdef".repeat(4))
            .unwrap()
            .as_str(),
        "0123456789abcdef".repeat(4)
    );
}

#[test]
fn text_that_is_no_digest_is_refused_with_what_it_was() {
    let long = format!("{ABC_HEX}0");
    for refused in [
        "",
        &ABC_HEX[1..],
        long.as_str(),
        &ABC_HEX.to_uppercase(),
        &format!("g{}", &ABC_HEX[1..]),
        &format!("`{}", &ABC_HEX[1..]),
        &format!("/{}", &ABC_HEX[1..]),
        &format!(":{}", &ABC_HEX[1..]),
    ] {
        assert_eq!(
            ConfigDigest::new(refused),
            Err(DigestError {
                value: refused.to_owned()
            }),
            "{refused:?}"
        );
    }
    assert_eq!(
        DigestError {
            value: "9f2c".to_owned()
        }
        .to_string(),
        r#""9f2c" isn't a config digest, which is 64 lower-case hex digits"#
    );
}
