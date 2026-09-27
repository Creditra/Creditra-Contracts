// This binary is a host-side schema generator and must not be compiled for
// the `wasm32` target when building the contract artifact. Guard it so CI
// invocations that build the wasm target don't attempt to compile this file.
#![cfg(not(target_arch = "wasm32"))]

use cosmwasm_schema::write_api;
#[allow(unused_imports)]
use creditra_credit::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};

fn main() {
    write_api! {
        instantiate: InstantiateMsg,
        execute: ExecuteMsg,
        query: QueryMsg,
    }
}
