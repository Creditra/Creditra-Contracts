// SPDX-License-Identifier: MIT

//! TTL bump regression tests for collateral persistent storage keys.
//!
//! The credit contract stores per-borrower collateral balances in persistent
//! storage entries (`CollateralBalance(borrower)` and
//! `CollateralBalanceV2(borrower, token)`).
//! These entries must have their TTL extended on every read/write path
//! (deposit, withdraw, partial release, get_collateral query) so that active
//! borrowers' collateral is not silently archived by the network.
//!
//! Tests here exercise the storage helpers directly via `env.as_contract` so
//! we can focus on TTL behaviour without requiring a fully configured token
//! transfer environment.

#[cfg(test)]
mod test {
    use crate::storage::{DataKey, LEDGER_BUMP_AMOUNT, LEDGER_BUMP_THRESHOLD};
    use crate::{Credit, CreditClient};
    use soroban_sdk::testutils::storage::Persistent as _;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{Address, Env};

    fn setup(env: &Env) -> (Address, CreditClient) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(env, &contract_id);
        client.init(&admin);
        (contract_id, client)
    }

    fn advance_ledgers(env: &Env, delta: u32) {
        env.ledger().with_mut(|li| {
            li.sequence_number = li.sequence_number.saturating_add(delta);
        });
    }

    fn ttl_for_key<K: soroban_sdk::IntoVal<Env, soroban_sdk::Val>>(
        env: &Env,
        contract_id: &Address,
        key: &K,
    ) -> u32 {
        env.as_contract(contract_id, || env.storage().persistent().get_ttl(key))
    }

    /// Helper: drain the TTL of a persistent key to just below the bump
    /// threshold so the next read/write path must perform a real bump.
    fn advance_past_ttl_threshold<K: soroban_sdk::IntoVal<Env, soroban_sdk::Val>>(
        env: &Env,
        contract_id: &Address,
        key: &K,
    ) {
        let initial = ttl_for_key(env, contract_id, key);
        let target = LEDGER_BUMP_THRESHOLD.saturating_sub(1);
        let delta = initial.saturating_sub(target);
        advance_ledgers(env, delta);
    }

    // ── Single-token collateral balance TTL tests ────────────────────────────

    #[test]
    fn set_collateral_balance_bumps_ttl_on_write() {
        let env = Env::default();
        let (contract_id, _client) = setup(&env);
        let borrower = Address::generate(&env);
        let balance_key = DataKey::CollateralBalance(borrower.clone());

        env.as_contract(&contract_id, || {
            crate::storage::set_collateral_balance(&env, &borrower, 500_i128);
        });

        let initial_ttl = ttl_for_key(&env, &contract_id, &balance_key);
        assert!(
            initial_ttl >= LEDGER_BUMP_AMOUNT,
            "first set must set TTL >= bump amount; got {initial_ttl}"
        );

        advance_past_ttl_threshold(&env, &contract_id, &balance_key);

        env.as_contract(&contract_id, || {
            crate::storage::set_collateral_balance(&env, &borrower, 700_i128);
        });

        let after_ttl = ttl_for_key(&env, &contract_id, &balance_key);
        assert!(
            after_ttl >= LEDGER_BUMP_AMOUNT,
            "set must extend TTL; after={after_ttl}"
        );
    }

    #[test]
    fn get_collateral_balance_bumps_ttl_on_read() {
        let env = Env::default();
        let (contract_id, _client) = setup(&env);
        let borrower = Address::generate(&env);
        let balance_key = DataKey::CollateralBalance(borrower.clone());

        env.as_contract(&contract_id, || {
            crate::storage::set_collateral_balance(&env, &borrower, 1_000_i128);
        });

        advance_past_ttl_threshold(&env, &contract_id, &balance_key);

        let balance = env.as_contract(&contract_id, || {
            crate::storage::get_collateral_balance(&env, &borrower)
        });
        assert_eq!(balance, 1_000_i128);

        let after_ttl = ttl_for_key(&env, &contract_id, &balance_key);
        assert!(
            after_ttl >= LEDGER_BUMP_AMOUNT,
            "get_collateral_balance read must extend TTL; after={after_ttl}"
        );
    }

    #[test]
    fn get_collateral_balance_absent_key_returns_zero_without_panic() {
        let env = Env::default();
        let (contract_id, _client) = setup(&env);
        let borrower = Address::generate(&env);

        let balance = env.as_contract(&contract_id, || {
            crate::storage::get_collateral_balance(&env, &borrower)
        });
        assert_eq!(balance, 0_i128);
    }

    #[test]
    fn set_collateral_balance_bumps_ttl_on_balance_reduction() {
        // set_collateral_balance reads the previous value directly from
        // storage (rather than calling get_collateral_balance) to avoid a
        // redundant TTL bump on the read side.  This test confirms the
        // function bumps TTL correctly even when reducing the balance.
        let env = Env::default();
        let (contract_id, _client) = setup(&env);
        let borrower = Address::generate(&env);
        let balance_key = DataKey::CollateralBalance(borrower.clone());

        env.as_contract(&contract_id, || {
            crate::storage::set_collateral_balance(&env, &borrower, 500_i128);
        });

        let balance = env.as_contract(&contract_id, || {
            crate::storage::get_collateral_balance(&env, &borrower)
        });
        assert_eq!(balance, 500_i128);

        advance_past_ttl_threshold(&env, &contract_id, &balance_key);
        env.as_contract(&contract_id, || {
            crate::storage::set_collateral_balance(&env, &borrower, 300_i128);
        });

        let after_ttl = ttl_for_key(&env, &contract_id, &balance_key);
        assert!(
            after_ttl >= LEDGER_BUMP_AMOUNT,
            "set must extend TTL even when reducing balance; after={after_ttl}"
        );

        let balance = env.as_contract(&contract_id, || {
            crate::storage::get_collateral_balance(&env, &borrower)
        });
        assert_eq!(balance, 300_i128);
    }

    #[test]
    fn set_collateral_balance_zero_adjusts_total_collateral_accumulator() {
        let env = Env::default();
        let (contract_id, _client) = setup(&env);
        let borrower = Address::generate(&env);

        env.as_contract(&contract_id, || {
            crate::storage::set_collateral_balance(&env, &borrower, 500_i128);
        });

        let total_before =
            env.as_contract(&contract_id, || crate::storage::get_total_collateral(&env));
        assert_eq!(total_before, 500_i128);

        env.as_contract(&contract_id, || {
            crate::storage::set_collateral_balance(&env, &borrower, 0_i128);
        });

        let total_after =
            env.as_contract(&contract_id, || crate::storage::get_total_collateral(&env));
        assert_eq!(total_after, 0_i128);
    }

    // ── Multi-collateral token balance TTL tests ─────────────────────────────

    #[test]
    fn set_collateral_balance_for_token_bumps_ttl_on_write() {
        let env = Env::default();
        let (contract_id, _client) = setup(&env);
        let borrower = Address::generate(&env);
        let token = Address::generate(&env);
        let balance_key = DataKey::CollateralBalanceV2(borrower.clone(), token.clone());

        env.as_contract(&contract_id, || {
            crate::storage::set_collateral_balance_for_token(&env, &borrower, &token, 500_i128);
        });

        let initial_ttl = ttl_for_key(&env, &contract_id, &balance_key);
        assert!(
            initial_ttl >= LEDGER_BUMP_AMOUNT,
            "first token set must set TTL >= bump amount; got {initial_ttl}"
        );

        advance_past_ttl_threshold(&env, &contract_id, &balance_key);

        env.as_contract(&contract_id, || {
            crate::storage::set_collateral_balance_for_token(&env, &borrower, &token, 700_i128);
        });

        let after_ttl = ttl_for_key(&env, &contract_id, &balance_key);
        assert!(
            after_ttl >= LEDGER_BUMP_AMOUNT,
            "token set must extend TTL; after={after_ttl}"
        );
    }

    #[test]
    fn get_collateral_balance_for_token_bumps_ttl_on_read() {
        let env = Env::default();
        let (contract_id, _client) = setup(&env);
        let borrower = Address::generate(&env);
        let token = Address::generate(&env);
        let balance_key = DataKey::CollateralBalanceV2(borrower.clone(), token.clone());

        env.as_contract(&contract_id, || {
            crate::storage::set_collateral_balance_for_token(&env, &borrower, &token, 1_000_i128);
        });

        advance_past_ttl_threshold(&env, &contract_id, &balance_key);

        let balance = env.as_contract(&contract_id, || {
            crate::storage::get_collateral_balance_for_token(&env, &borrower, &token)
        });
        assert_eq!(balance, 1_000_i128);

        let after_ttl = ttl_for_key(&env, &contract_id, &balance_key);
        assert!(
            after_ttl >= LEDGER_BUMP_AMOUNT,
            "get_collateral_balance_for_token read must extend TTL; after={after_ttl}"
        );
    }

    #[test]
    fn get_collateral_balance_for_token_absent_key_returns_zero() {
        let env = Env::default();
        let (contract_id, _client) = setup(&env);
        let borrower = Address::generate(&env);
        let token = Address::generate(&env);

        let balance = env.as_contract(&contract_id, || {
            crate::storage::get_collateral_balance_for_token(&env, &borrower, &token)
        });
        assert_eq!(balance, 0_i128);
    }
}
