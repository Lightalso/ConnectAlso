#![no_main]

use libfuzzer_sys::fuzz_target;
use connectalso_nat::stun;

fuzz_target!(|data: &[u8]| {
    // Fuzz the STUN response parser with arbitrary input.
    // Must never panic — all errors must be handled gracefully.
    stun::fuzz_stun_response(data);
});
