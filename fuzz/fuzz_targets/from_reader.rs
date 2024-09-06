#![no_main]
#[macro_use] extern crate libfuzzer_sys;
extern crate serde_cbor;

use serde_cbor::Value;

fuzz_target!(|data: &[u8]| {
    let mut data1 = data;
    let mut data2 = data;
    let new = serde_cbor::from_reader::<Value, _>(&mut data1).map_err(|_| ());
    let original = serde_cbor_original::from_reader::<Value, _>(&mut data2).map_err(|_| ());
    assert_eq!(new, original);
});
