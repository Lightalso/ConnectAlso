#![no_main]

use libfuzzer_sys::fuzz_target;
use connectalso_relay_proto::RelayFrame;

fuzz_target!(|data: &[u8]| {
    // Attempt to decode arbitrary bytes as a relay frame.
    // Should never panic — invalid input must return Err, not crash.
    let _ = RelayFrame::decode(data);
});
