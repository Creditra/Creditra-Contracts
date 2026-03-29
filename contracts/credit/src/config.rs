use crate::auth::require_admin_auth;
use crate::storage::admin_key;
use crate::storage::DataKey;
use soroban_sdk::{Address, Env};

pub fn init(env: Env, admin: Address) {
        env.storage().instance().set(&admin_key(&env), &admin);
        env.storage()
            .instance()
            .set(&DataKey::LiquiditySource, &env.current_contract_address());
    }

    /// @notice Sets the token contract used for reserve/liquidity checks and draw transfers.
    /// @dev Admin-only.
    pub fn set_liquidity_token(env: Env, token_address: Address) {
        require_admin_auth(&env);
        env.storage()
            .instance()
            .set(&DataKey::LiquidityToken, &token_address);
    }

    /// @notice Sets the address that provides liquidity for draw operations.
    /// @dev Admin-only. If unset, init config uses the contract address.
    pub fn set_liquidity_source(env: Env, reserve_address: Address) {
        require_admin_auth(&env);
        env.storage()
            .instance()
            .set(&DataKey::LiquiditySource, &reserve_address);
    }
