use super::{MAX_FRAME, decode, read, write};
use serde_json::Value;

#[test]
fn chunked_messages_preserve_large_unicode_and_escaped_text() {
    let original = serde_json::json!({"text": "👩‍💻\n\"\\".repeat(MAX_FRAME / 6)});
    let mut wire = Vec::new();
    write(&mut wire, &original).unwrap();
    assert!(wire.len() > MAX_FRAME);
    assert!(
        wire.split_inclusive(|byte| *byte == b'\n')
            .all(|line| line.len() <= MAX_FRAME)
    );
    assert_eq!(
        read::<Value>(&mut std::io::Cursor::new(wire)).unwrap(),
        Some(original)
    );
}

#[test]
fn duplicate_keys_and_oversized_frames_are_rejected() {
    assert!(decode::<Value>(br#"{"outer":{"key":1,"key":2}}"#).is_err());
    let oversized = vec![b' '; MAX_FRAME + 1];
    assert!(read::<Value>(&mut std::io::Cursor::new(oversized)).is_err());
}

#[test]
fn incomplete_and_reordered_chunks_fail() {
    for wire in [
        r#"{"antislop_chunk":{"index":1,"text":"{}","done":true}}"#,
        r#"{"antislop_chunk":{"index":0,"text":"{","done":false}}"#,
    ] {
        assert!(read::<Value>(&mut std::io::Cursor::new(wire)).is_err());
    }
}
