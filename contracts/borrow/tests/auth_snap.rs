#![cfg(test)]
extern crate std;

use soroban_sdk::{testutils::Address as _, Address, Env, IntoVal, Symbol};
// Adjust the import paths if wrong
use crate::{BorrowContract, BorrowContractClient};

#[test]
fn test_borrow_auth_snapshot() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, BorrowContract);
    let client = BorrowContractClient::new(&env, &contract_id);

    let borrower = Address::generate(&env);
    let borrow_amount: i128 = 5_000;

    client.borrow(&borrower, &borrow_amount);

    let auths = env.auths();
    
    assert_eq!(
        auths,
        std::vec![(
            borrower.clone(),
            client.address.clone(),
            Symbol::new(&env, "borrow"),
            (&borrower, borrow_amount).into_val(&env)
        )],
        "Authorization snapshot mismatch: borrower auth was not required or payload is incorrect."
    );
}

#[test]
fn test_repay_auth_snapshot() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, BorrowContract);
    let client = BorrowContractClient::new(&env, &contract_id);

    let repayer = Address::generate(&env);
    let repay_amount: i128 = 2_500;

    client.repay(&repayer, &repay_amount);

    let auths = env.auths();
    
    assert_eq!(
        auths,
        std::vec![(
            repayer.clone(),
            client.address.clone(),
            Symbol::new(&env, "repay"),
            (&repayer, repay_amount).into_val(&env)
        )],
        "Authorization snapshot mismatch: repayer auth was not required or payload is incorrect."
    );
}