#![no_main]

use libfuzzer_sys::fuzz_target;
use connectalso_relay_proto::RelayFrame;

fuzz_target!(|data: &[u8]| {
    // Round-trip: decode then encode — should be idempotent for valid frames
    if let Ok(frame) = RelayFrame::decode(data) {
        if let Ok(encoded) = frame.encode() {
            let decoded2 = RelayFrame::decode(&encoded).unwrap();
            assert_eq!(frame.sender_id, decoded2.sender_id);
            assert_eq!(frame.target_id, decoded2.target_id);
            assert_eq!(frame.msg_type, decoded2.msg_type);
            assert_eq!(frame.payload, decoded2.payload);
        }
    }
});
