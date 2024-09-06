#![no_main]
#[macro_use] extern crate libfuzzer_sys;
extern crate serde_cbor;

use serde_cbor::Value;

fuzz_target!(|data: &[u8]| {
    let new = serde_cbor::from_slice::<Value>(data).map_err(|_| ());
    let original = serde_cbor_original::from_slice::<Value>(data).map_err(|_| ());
    assert_eq!(new, original);
    assert_eq!(
        new.and_then(|v| serde_cbor::to_vec(&v).map_err(|_| ())),
        original.and_then(|v| serde_cbor_original::to_vec(&v).map_err(|_| ())),
    );
});
