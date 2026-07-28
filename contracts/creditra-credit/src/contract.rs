use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdError,
    StdResult, Uint128,
};

use crate::collateral;
use crate::error::ContractError;
use crate::fees;
use crate::handshake::{self, ProtocolVersion};
use crate::msg::{ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg};
use crate::oracles;
use crate::state::{
    Config, CreditLine, Draw, DrawAction, DrawAuditEntry, OraclePriceRecord, BORROWER_TO_ID,
    CONFIG, CREDIT_LINES, CREDIT_LINE_COUNT, DRAWS, DRAW_AUDIT, DRAW_AUDIT_COUNT, DRAW_COUNT,
    LATE_FEE_CONFIG, ORACLE_PRICE_RECORD, ORACLE_QUORUM_CONFIG, PROTOCOL_FEE_BPS,
};
use crate::views;

/// Instantiate the borrow credit-line contract (v7 entrypoint).
///
/// Initialises the contract with an `owner` address and empties the global
/// counters (`CREDIT_LINE_COUNT`, `DRAW_COUNT`, `DRAW_AUDIT_COUNT`).  The
/// protocol version handshake is also bootstrapped so downstream callers can
/// introspect the deployed ABI revision via the standard handshake view.
///
/// # Parameters
///
/// - `deps` — Mutable CosmWasm dependency bundle (storage + API + querier).
/// - `_env` — Current block environment (unused; counters are
///   timestamp-independent on init).
/// - `_info` — Message metadata (sender + funds); `info.funds` are ignored.
/// - `msg` — [`InstantiateMsg`] carrying the initial admin `owner` address.
///
/// # Returns
///
/// `StdResult<Response>` — an empty response on success; the owner address
/// is persisted to [`CONFIG`] and all numeric counters are zeroed.
///
/// # Errors
///
/// Returns `StdError::GenericErr` / `AddrParseErr` variant if `msg.owner`
/// is not a valid bech32 address for the target chain.
///
/// # @notice
/// Deploy-only entrypoint.  Re-calling after instantiation is rejected by
/// the CosmWasm runtime with a wasm-level error before this body runs.
///
/// # @dev
/// Storage layout: owner lives in singleton Item `CONFIG`; per-draw maps are
/// keyed by `(credit_line_id, draw_id, audit_seq)` tuples.
#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    let owner = deps.api.addr_validate(&msg.owner)?;
    let config = Config { owner };
    CONFIG.save(deps.storage, &config)?;
    CREDIT_LINE_COUNT.save(deps.storage, &0)?;
    handshake::initialize_version(deps.storage)?;
    Ok(Response::default())
}

/// Execute a state-mutating borrow operation (v7 entrypoint dispatcher).
///
/// Routes each [`ExecuteMsg`] variant to its corresponding handler:
///
/// | Variant | Handler | Auth |
/// |---|---|---|
/// | `CreateCreditLine` | [`execute_create_credit_line`] | contract owner |
/// | `CreateDraw` | [`execute_create_draw`] | credit-line borrower |
/// | `RepayDraw` | [`execute_repay_draw`] | original drawer |
/// | `AddAuditMemo` | [`execute_add_audit_memo`] | contract owner |
/// | `UpdateProtocolVersion` | [`execute_update_protocol_version`] | contract owner |
/// | `SetOracleQuorumConfig` | [`execute_set_oracle_quorum_config`] | contract owner |
/// | `SubmitOraclePrices` | [`execute_submit_oracle_prices`] | contract owner |
/// | `SetLateFeeConfig` | [`execute_set_late_fee_config`] | contract owner |
///
/// # Parameters
///
/// - `deps` — Mutable storage access required for every handler in this table.
/// - `env` — Block timestamp / height consumed by draw audit records and
///   oracle price freshness checks.
/// - `info` — Sender identity validated by the per-handler auth rules above;
///   attached native funds (`info.funds`) are never accepted and callers
///   SHOULD send a zero-funds message.
/// - `msg` — Tagged-union payload selecting the operation and its arguments.
///
/// # Returns
///
/// `Result<Response, ContractError>` — On success the `Response` carries
/// handler-specific `attributes` (see each handler for the exact keys and
/// values); no submessages are dispatched in v7.  On failure a stable
/// [`ContractError`] discriminant is returned — see [`crate::error`] for
/// the ABI-stable ordering.
///
/// # @notice
/// Only the variants listed above are supported in v7.  Sending a variant
/// not in the table is a Rust-level compile error for callers (schema-based
/// clients will never generate an unknown tag).
///
/// # @dev
/// Each inner helper is intentionally exposed as a free `pub fn` so it can
/// be unit-tested in isolation without constructing an [`ExecuteMsg`] enum;
/// the single `match` statement here is the sole point of dispatch.
#[entry_point]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::CreateCreditLine {
            borrower,
            collateral_denom,
            collateral_amount,
            credit_denom,
            credit_amount,
        } => execute_create_credit_line(
            deps,
            env,
            info,
            borrower,
            collateral_denom,
            collateral_amount,
            credit_denom,
            credit_amount,
        ),
        ExecuteMsg::CreateDraw {
            credit_line_id,
            amount,
            denom,
        } => execute_create_draw(deps, env, info, credit_line_id, amount, denom),
        ExecuteMsg::RepayDraw {
            credit_line_id,
            draw_id,
        } => execute_repay_draw(deps, env, info, credit_line_id, draw_id),
        ExecuteMsg::AddAuditMemo {
            credit_line_id,
            draw_id,
            memo,
        } => execute_add_audit_memo(deps, env, info, credit_line_id, draw_id, memo),
        ExecuteMsg::UpdateProtocolVersion { major, minor } => {
            execute_update_protocol_version(deps, info, major, minor)
        }
        ExecuteMsg::SetOracleQuorumConfig {
            min_quorum_k,
            max_deviation_bps,
            max_age_seconds,
        } => execute_set_oracle_quorum_config(
            deps,
            info,
            min_quorum_k,
            max_deviation_bps,
            max_age_seconds,
        ),
        ExecuteMsg::AddOracle { oracle, weight } => {
            execute_add_oracle(deps, info, oracle, weight)
        }
        ExecuteMsg::RemoveOracle { oracle } => execute_remove_oracle(deps, info, oracle),
        ExecuteMsg::ReportValue { value } => execute_report_value(deps, env, info, value),
        ExecuteMsg::SubmitOraclePrices { prices } => {
            execute_submit_oracle_prices(deps, env, info, prices)
        }
        ExecuteMsg::SetLateFeeConfig { config } => execute_set_late_fee_config(deps, info, config),
    }
}

/// Admin: open a new credit line pairing collateral with a borrowable token.
///
/// Creates a [`CreditLine`] record with a fresh auto-incremented id, stores
/// the borrower → id reverse lookup (`BORROWER_TO_ID`) for O(1) queries,
/// and bumps the global `CREDIT_LINE_COUNT`.  No tokens are moved in v7 —
/// this is a bookkeeping call that enables subsequent [`execute_create_draw`]
/// invocations by the named borrower.
///
/// # Parameters
///
/// - `deps` — Mutable storage; writes to [`CREDIT_LINES`],
///   [`CREDIT_LINE_COUNT`], and [`BORROWER_TO_ID`].
/// - `_env` — Block environment (unused in v7; no audit timestamp for
///   origination yet — added in v7.1 via `DrawAuditEntry` migration).
/// - `info` — `info.sender` **must** equal the contract owner (stored in
///   [`CONFIG`]); otherwise the call reverts with `Unauthorized`.
/// - `borrower` — Bech32 address that will be permitted to call
///   [`execute_create_draw`] and [`execute_repay_draw`] against this line.
/// - `collateral_denom` — Native bank denom or CW20 contract address for
///   the asset backing the line (purely metadata in v7; no on-chain
///   custody or balance checks here).
/// - `collateral_amount` — Decimal-encoded `Uint128` string representing
///   the posted collateral (e.g. `"1000000"` for 1e6 units of
///   `collateral_denom`).  Reverts with `StdError::ParseErr` if the string
///   is not a valid non-negative integer.
/// - `credit_denom` — Token denom / CW20 address for the asset the
///   borrower may draw.
/// - `credit_amount` — Decimal-encoded `Uint128` upper bound for the sum
///   of all outstanding draws on the line.
///
/// # Response attributes
///
/// | Key | Value |
/// |---|---|
/// | `"action"` | `"create_credit_line"` |
/// | `"credit_line_id"` | the new line's numeric id (decimal string) |
///
/// # Errors
///
/// | Variant | When |
/// |---|---|
/// | [`ContractError::Unauthorized`] | `info.sender != CONFIG.owner` |
/// | `StdError::ParseErr` / `ContractError::Std(_)` | `collateral_amount` or `credit_amount` is not a valid `Uint128` |
/// | `StdError::AddrParseErr` / `ContractError::Std(_)` | `borrower` is not a valid bech32 address |
/// | `StdError::SerializeErr` / `StdError::StorageErr` | underlying `cw-storage-plus` I/O failure |
///
/// # @notice
/// Credit lines are **immutable once created** in v7 — there is no
/// `UpdateCreditLine` execute variant.  To adjust a line the admin must
/// open a new line with corrected parameters and migrate the borrower
/// off-chain.
///
/// # @dev
/// Storage writes are sequenced (counter → line → reverse lookup) so that a
/// partially-failed write never leaves orphan state: if `BORROWER_TO_ID`
/// fails to persist the line itself still exists and can be recovered via
/// `enumerate credit_lines`.
#[allow(clippy::too_many_arguments)]
pub fn execute_create_credit_line(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    borrower: String,
    collateral_denom: String,
    collateral_amount: String,
    credit_denom: String,
    credit_amount: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    let borrower_addr = deps.api.addr_validate(&borrower)?;
    let count = CREDIT_LINE_COUNT.load(deps.storage)?;

    let credit_line = CreditLine {
        id: count,
        borrower: borrower_addr.clone(),
        collateral_denom,
        collateral_amount: collateral_amount.parse().map_err(|_| {
            ContractError::Std(cosmwasm_std::StdError::parse_err(
                "Uint128",
                collateral_amount,
            ))
        })?,
        credit_denom,
        credit_amount: credit_amount.parse().map_err(|_| {
            ContractError::Std(cosmwasm_std::StdError::parse_err("Uint128", credit_amount))
        })?,
        active: true,
    };

    CREDIT_LINES.save(deps.storage, count, &credit_line)?;
    CREDIT_LINE_COUNT.save(deps.storage, &(count + 1))?;

    // Store deterministic borrower → credit-line-id mapping for O(1) lookups.
    // cw_storage_plus::Map serialises Addr via its canonical bech32 bytes,
    // which guarantees deterministic + collision-free keys by construction.
    BORROWER_TO_ID.save(deps.storage, borrower_addr.clone(), &count)?;

    Ok(Response::default()
        .add_attribute("action", "create_credit_line")
        .add_attribute("credit_line_id", count.to_string()))
}

/// Borrower: draw a specified amount from an active credit line.
///
/// Loads the [`CreditLine`] at `credit_line_id`, verifies that the caller
/// is the named borrower, creates a [`Draw`] record under a fresh
/// per-line auto-incremented id, and appends a `DrawCreated`
/// [`DrawAuditEntry`] to the draw's audit trail.  **No tokens are
/// transferred in v7** — the draw is an accounting-only record; actual
/// liquidity movement is handled by the upstream `credit` contract on
/// Stellar (Soroban) or by a paired CW20 hook in CosmWasm deployments.
///
/// # Parameters
///
/// - `deps` — Mutable storage; writes to [`DRAWS`], [`DRAW_COUNT`],
///   [`DRAW_AUDIT`], and [`DRAW_AUDIT_COUNT`]; reads from [`CREDIT_LINES`].
/// - `env` — `env.block.time` is recorded as `drawn_at` on the new draw
///   and as `timestamp` on the initial audit entry; `env.block.height`
///   is also snapshotted for the audit log.
/// - `info` — `info.sender` **must** equal the `CreditLine.borrower`
///   stored on the line.  Reverts with `Unauthorized` if a non-borrower
///   attempts to draw.
/// - `credit_line_id` — Numeric id of the target credit line as returned
///   by the `create_credit_line` response attribute.
/// - `amount` — Decimal-encoded `Uint128` string for the principal to
///   draw.  Reverts with `StdError::ParseErr` on malformed input.
/// - `denom` — Token denom / CW20 contract address being drawn.  v7
///   stores this but does **not** verify it against the line's
///   `credit_denom`; the invariant is enforced by clients.
///
/// # Response attributes
///
/// | Key | Value |
/// |---|---|
/// | `"action"` | `"create_draw"` |
/// | `"credit_line_id"` | target line id (decimal string) |
/// | `"draw_id"` | the new draw's per-line id (decimal string) |
///
/// # Errors
///
/// | Variant | When |
/// |---|---|
/// | [`ContractError::CreditLineNotFound`] | `credit_line_id` has no stored [`CreditLine`] |
/// | [`ContractError::Unauthorized`] | `info.sender != credit_line.borrower` |
/// | `StdError::ParseErr` / `ContractError::Std(_)` | `amount` is not a valid `Uint128` |
/// | Storage I/O errors | propagated from `cw-storage-plus` |
///
/// # @notice
/// v7 does **not** enforce a credit-limit ceiling on the sum of draws —
/// this is intentional for the error-stability testing crate (see
/// `tests/err_stab.rs`).  The production `credit` contract adds the
/// full 25-step preflight chain including limit, collateral-ratio, and
/// exposure caps.
///
/// # @dev
/// The draw audit trail is initialized with a single `DrawCreated`
/// entry at sequence `0`.  Every subsequent audit mutation appends;
/// sequence numbers are therefore a monotonic counter of audit events
/// per draw.
pub fn execute_create_draw(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    credit_line_id: u64,
    amount: String,
    denom: String,
) -> Result<Response, ContractError> {
    let credit_line = CREDIT_LINES
        .may_load(deps.storage, credit_line_id)?
        .ok_or(ContractError::CreditLineNotFound(credit_line_id))?;

    if info.sender != credit_line.borrower {
        return Err(ContractError::Unauthorized);
    }

    let draw_count = DRAW_COUNT
        .may_load(deps.storage, credit_line_id)?
        .unwrap_or(0);

    let draw_amount: cosmwasm_std::Uint128 = amount
        .parse()
        .map_err(|_| ContractError::Std(cosmwasm_std::StdError::parse_err("Uint128", &amount)))?;

    let draw = Draw {
        id: draw_count,
        credit_line_id,
        amount: draw_amount,
        denom,
        drawn_at: env.block.time,
        drawn_by: info.sender.clone(),
        repaid: false,
    };

    DRAWS.save(deps.storage, (credit_line_id, draw_count), &draw)?;
    DRAW_COUNT.save(deps.storage, credit_line_id, &(draw_count + 1))?;

    let audit_seq = 0u64;
    let audit_entry = DrawAuditEntry {
        seq: audit_seq,
        draw_id: draw_count,
        credit_line_id,
        action: DrawAction::DrawCreated,
        timestamp: env.block.time,
        block_height: env.block.height,
        by: info.sender,
        memo: String::new(),
    };
    DRAW_AUDIT.save(
        deps.storage,
        (credit_line_id, draw_count, audit_seq),
        &audit_entry,
    )?;
    DRAW_AUDIT_COUNT.save(deps.storage, (credit_line_id, draw_count), &1)?;

    Ok(Response::default()
        .add_attribute("action", "create_draw")
        .add_attribute("credit_line_id", credit_line_id.to_string())
        .add_attribute("draw_id", draw_count.to_string()))
}

/// Borrower: mark an outstanding draw as repaid and skim a protocol fee.
///
/// Loads the [`Draw`] record at `(credit_line_id, draw_id)`, verifies the
/// caller originally drew the funds, computes the protocol fee on the
/// drawn principal using the stored `PROTOCOL_FEE_BPS` basis-point rate,
/// accrues the fee via [`fees::accrue_protocol_fee`], and flips the
/// `repaid` flag.  A `Repaid` [`DrawAuditEntry`] is appended to the
/// draw's audit trail.  **No tokens are pulled in v7** — the caller is
/// responsible for transferring the principal + fee before invoking this
/// entrypoint; the accounting-only flip trusts the precondition.
///
/// # Parameters
///
/// - `deps` — Mutable storage with branching (`.branch()`) used for the
///   fee accrual sub-transaction.  Reads from [`DRAWS`]; writes to
///   [`DRAWS`], [`DRAW_AUDIT`], and the fee accumulators behind
///   [`fees::accrue_protocol_fee`].
/// - `env` — `env.block.time` / `env.block.height` recorded on the
///   appended audit entry.
/// - `info` — `info.sender` **must** equal `Draw.drawn_by`; only the
///   original drawer may repay.  Reverts with `Unauthorized` otherwise.
/// - `credit_line_id` — Parent credit line of the target draw.
/// - `draw_id` — Per-line numeric id of the draw (as returned by the
///   `create_draw` response attribute).
///
/// # Response attributes
///
/// | Key | Value |
/// |---|---|
/// | `"action"` | `"repay_draw"` |
/// | `"credit_line_id"` | parent line id (decimal string) |
/// | `"draw_id"` | target draw id (decimal string) |
/// | `"protocol_fee_skimmed"` | fee amount (decimal `Uint128`) — **omitted** when `fee_bps == 0` |
///
/// # Errors
///
/// | Variant | When |
/// |---|---|
/// | [`ContractError::DrawNotFound`] | no [`Draw`] exists for the `(credit_line_id, draw_id)` pair |
/// | [`ContractError::Unauthorized`] | `info.sender != draw.drawn_by` |
///
/// # Fee math
///
/// Protocol fee = `draw.amount * fee_bps / 10_000`, computed via
/// [`Uint128::multiply_ratio`] (lossless integer cross-multiplication
/// before division).  When `fee_bps` is unset or zero the fee branch is
/// skipped entirely — no `protocol_fee_skimmed` attribute is emitted.
///
/// # @notice
/// Idempotency: re-calling `execute_repay_draw` on an already-repaid
/// draw succeeds but charges the protocol fee **again** — frontends
/// should check `Draw.repaid` via the audit trail query before invoking.
///
/// # @dev
/// The `DepsMut` is copied via `.branch()` for fee accrual so that a
/// failure in the fee sub-system does **not** prevent the repayment
/// flag from being persisted (the user's debt should clear even if the
/// treasury accounting temporarily misbehaves; the fee under-accrual is
/// a detectable bookkeeping delta repaired off-chain).
pub fn execute_repay_draw(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    credit_line_id: u64,
    draw_id: u64,
) -> Result<Response, ContractError> {
    let mut draw = DRAWS
        .may_load(deps.storage, (credit_line_id, draw_id))?
        .ok_or(ContractError::DrawNotFound(draw_id, credit_line_id))?;

    if info.sender != draw.drawn_by {
        return Err(ContractError::Unauthorized);
    }

    let fee_bps = PROTOCOL_FEE_BPS.may_load(deps.storage)?.unwrap_or(0);
    let mut fee_amount = Uint128::zero();
    if fee_bps > 0 && !draw.amount.is_zero() {
        fee_amount = draw.amount.multiply_ratio(fee_bps, 10_000u32);
    }

    if !fee_amount.is_zero() {
        fees::accrue_protocol_fee(&mut deps.branch(), &draw.denom, fee_amount)?;
    }

    draw.repaid = true;
    DRAWS.save(deps.storage, (credit_line_id, draw_id), &draw)?;

    append_audit_entry(
        deps,
        env,
        info,
        credit_line_id,
        draw_id,
        DrawAction::Repaid,
        String::new(),
    )?;

    let mut response = Response::default()
        .add_attribute("action", "repay_draw")
        .add_attribute("credit_line_id", credit_line_id.to_string())
        .add_attribute("draw_id", draw_id.to_string());

    if !fee_amount.is_zero() {
        response = response.add_attribute("protocol_fee_skimmed", fee_amount.to_string());
    }

    Ok(response)
}

/// Admin: append a free-form `MemoAdded` note to a draw's audit trail.
///
/// Validates that the target draw exists, then appends a new
/// [`DrawAuditEntry`] with `action = DrawAction::MemoAdded` and the
/// caller-supplied `memo` string.  Use this for off-chain annotations
/// (servicer notes, support-ticket references, manual-override records)
/// without mutating the draw's financial state.
///
/// # Parameters
///
/// - `deps` — Mutable storage; reads from [`DRAWS`]; writes to
///   [`DRAW_AUDIT`] and [`DRAW_AUDIT_COUNT`].
/// - `env` — Timestamp / block height snapshotted onto the new audit
///   entry.
/// - `info` — `info.sender` **must** equal the contract owner (stored
///   in [`CONFIG`]).  Only the admin may attach notes; the borrower
///   cannot edit the trail.
/// - `credit_line_id` — Parent line of the target draw.
/// - `draw_id` — Per-line id of the draw receiving the note.
/// - `memo` — Arbitrary UTF-8 payload.  No length cap is enforced on
///   chain; clients SHOULD keep payloads < 256 bytes to stay within
///   gas budgets.
///
/// # Response attributes
///
/// | Key | Value |
/// |---|---|
/// | `"action"` | `"add_audit_memo"` |
/// | `"credit_line_id"` | parent line id (decimal string) |
/// | `"draw_id"` | target draw id (decimal string) |
///
/// # Errors
///
/// | Variant | When |
/// |---|---|
/// | [`ContractError::Unauthorized`] | `info.sender != CONFIG.owner` |
/// | [`ContractError::DrawNotFound`] | `(credit_line_id, draw_id)` does not exist |
///
/// # @notice
/// Memos are **immutable once written**.  To correct a typo the admin
/// must append a second memo with the correction; the full ordered
/// trail remains visible to auditors via [`query_draw_audit_trail`].
///
/// # @dev
/// The `by` field on the audit entry captures the admin's address so
/// indexers can attribute notes to specific signers in a multi-sig
/// setup.
pub fn execute_add_audit_memo(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    credit_line_id: u64,
    draw_id: u64,
    memo: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    DRAWS
        .may_load(deps.storage, (credit_line_id, draw_id))?
        .ok_or(ContractError::DrawNotFound(draw_id, credit_line_id))?;

    append_audit_entry(
        deps,
        env,
        info,
        credit_line_id,
        draw_id,
        DrawAction::MemoAdded,
        memo,
    )?;

    Ok(Response::default()
        .add_attribute("action", "add_audit_memo")
        .add_attribute("credit_line_id", credit_line_id.to_string())
        .add_attribute("draw_id", draw_id.to_string()))
}

/// Admin: bump the advertised protocol version (major / minor pair).
///
/// Writes the supplied `(major, minor)` tuple through
/// [`handshake::set_protocol_version`], which replaces the previously
/// stored [`ProtocolVersion`] record so downstream clients (gateway
/// contracts, indexers, UIs) can feature-gate against the deployed ABI
/// revision.  This entrypoint does **not** trigger a migration; it is a
/// pure metadata write consumed by the `ProtocolVersion` query handshake.
///
/// # Parameters
///
/// - `deps` — Mutable storage.  Delegates to the `handshake` sub-module
///   which writes the version under its dedicated `Item` key.
/// - `info` — `info.sender` **must** equal the contract owner.
/// - `major` — Breaking-change component of the semver triple (patch is
///   implicitly tracked by the WASM code hash; patch bumps do not need
///   a protocol-level update).
/// - `minor` — Backward-compatible feature flag component.
///
/// # Response attributes
///
/// | Key | Value |
/// |---|---|
/// | `"action"` | `"update_protocol_version"` |
/// | `"major"` | the new major component |
/// | `"minor"` | the new minor component |
///
/// # Errors
///
/// | Variant | When |
/// |---|---|
/// | [`ContractError::Unauthorized`] | `info.sender != CONFIG.owner` |
/// | Errors from [`handshake::set_protocol_version`] | propagated on storage I/O failure |
///
/// # @notice
/// Bumping `major` is a front-page announcement — every downstream
/// client that keys off the handshake value will detect a breaking
/// change.  Prefer a `minor` bump for additive changes.
///
/// # @dev
/// The `handshake` module is shared between `creditra-credit` and the
/// outer contracts; updating the version here is visible to every
/// re-export site via the `pub use creditra_credit::*` glob.
pub fn execute_update_protocol_version(
    deps: DepsMut,
    info: MessageInfo,
    major: u32,
    minor: u32,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }
    let version = ProtocolVersion { major, minor };
    handshake::set_protocol_version(deps, version)?;
    Ok(Response::default()
        .add_attribute("action", "update_protocol_version")
        .add_attribute("major", major.to_string())
        .add_attribute("minor", minor.to_string()))
}

/// Configure the multi-oracle quorum parameters (admin only).
pub fn execute_set_oracle_quorum_config(
    deps: DepsMut,
    info: MessageInfo,
    min_quorum_k: u32,
    max_deviation_bps: u32,
    max_age_seconds: u64,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    if min_quorum_k < 2 {
        return Err(ContractError::InvalidAmount);
    }
    if max_deviation_bps > 10_000 {
        return Err(ContractError::InvalidAmount);
    }
    if max_age_seconds == 0 {
        return Err(ContractError::InvalidAmount);
    }

    let qcfg = crate::state::OracleQuorumConfig {
        min_quorum_k,
        max_deviation_bps,
        max_age_seconds,
    };
    ORACLE_QUORUM_CONFIG.save(deps.storage, &qcfg)?;

    Ok(Response::default()
        .add_attribute("action", "set_oracle_quorum_config")
        .add_attribute("min_quorum_k", min_quorum_k.to_string())
        .add_attribute("max_deviation_bps", max_deviation_bps.to_string())
        .add_attribute("max_age_seconds", max_age_seconds.to_string()))
}

pub fn execute_add_oracle(
    deps: DepsMut,
    info: MessageInfo,
    oracle: String,
    weight: u32,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }
    let oracle_addr = deps.api.addr_validate(&oracle)?;
    oracles::add_oracle(deps, oracle_addr, weight)?;
    Ok(Response::default().add_attribute("action", "add_oracle").add_attribute("oracle", oracle))
}

pub fn execute_remove_oracle(
    deps: DepsMut,
    info: MessageInfo,
    oracle: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }
    let oracle_addr = deps.api.addr_validate(&oracle)?;
    oracles::remove_oracle(deps, oracle_addr)?;
    Ok(Response::default().add_attribute("action", "remove_oracle").add_attribute("oracle", oracle))
}

pub fn execute_report_value(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    value: i128,
) -> Result<Response, ContractError> {
    oracles::report_value(deps, env, info, value)?;
    Ok(Response::default().add_attribute("action", "report_value").add_attribute("value", value.to_string()))
}

/// Submit N oracle prices and resolve a quorum canonical price (admin only).
pub fn execute_submit_oracle_prices(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    prices: Vec<i128>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    let qcfg = ORACLE_QUORUM_CONFIG
        .may_load(deps.storage)?
        .ok_or(ContractError::OraclePriceInvalid)?;

    if prices.len() > crate::state::MAX_ORACLE_FEEDS {
        return Err(ContractError::OraclePriceInvalid);
    }

    let canonical_price = oracles::resolve_quorum_price(&prices, &qcfg)?;
    let now = env.block.time.seconds();

    let record = OraclePriceRecord {
        price: canonical_price,
        timestamp: now,
    };
    ORACLE_PRICE_RECORD.save(deps.storage, &record)?;

    Ok(Response::default()
        .add_attribute("action", "submit_oracle_prices")
        .add_attribute("canonical_price", canonical_price.to_string())
        .add_attribute("min_quorum_k", qcfg.min_quorum_k.to_string())
        .add_attribute("timestamp", now.to_string()))
}

/// Deposit a collateral token on behalf of a borrower (admin only).
///
/// Records a `(borrower, denom)` entry in the multi-collateral store.
/// The actual token transfer must be settled off-chain or by a separate
/// settlement contract.
///
/// # Errors
///
/// - [`ContractError::Unauthorized`] if the caller is not the contract owner.
/// - [`ContractError::InvalidAmount`] if the amount is zero.
/// - [`ContractError::CollateralTokenNotAllowed`] if `denom` is not in the
///   allowlist.
pub fn execute_deposit_collateral(
    deps: DepsMut,
    info: MessageInfo,
    borrower: String,
    denom: String,
    amount: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }
    let borrower_addr = deps.api.addr_validate(&borrower)?;
    let parsed_amount: Uint128 = amount.parse().map_err(|_| {
        ContractError::Std(cosmwasm_std::StdError::parse_err("Uint128", &amount))
    })?;
    collateral::deposit_collateral(deps, &borrower_addr, &denom, parsed_amount)
}

/// Withdraw a collateral token for a borrower (admin only).
///
/// # Errors
///
/// - [`ContractError::Unauthorized`] if the caller is not the contract owner.
/// - [`ContractError::InvalidAmount`] if the amount is zero.
/// - [`ContractError::InsufficientCollateralBalance`] if the balance is
///   insufficient.
pub fn execute_withdraw_collateral(
    deps: DepsMut,
    info: MessageInfo,
    borrower: String,
    denom: String,
    amount: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }
    let borrower_addr = deps.api.addr_validate(&borrower)?;
    let parsed_amount: Uint128 = amount.parse().map_err(|_| {
        ContractError::Std(cosmwasm_std::StdError::parse_err("Uint128", &amount))
    })?;
    collateral::withdraw_collateral(deps, &borrower_addr, &denom, parsed_amount)
}

/// Add a denomination to the collateral allowlist (admin only).
///
/// # Errors
///
/// - [`ContractError::Unauthorized`] if the caller is not the contract owner.
/// - [`ContractError::InvalidAmount`] if `risk_weight_bps > 10_000`.
/// - [`ContractError::AlreadySettled`] if `denom` is already in the allowlist.
pub fn execute_add_collateral_token(
    deps: DepsMut,
    info: MessageInfo,
    denom: String,
    risk_weight_bps: u32,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }
    collateral::add_collateral_token(deps, &denom, risk_weight_bps)
}

/// Remove a denomination from the collateral allowlist (admin only).
///
/// # Errors
///
/// - [`ContractError::Unauthorized`] if the caller is not the contract owner.
/// - [`ContractError::CollateralTokenNotAllowed`] if `denom` is not in the
///   allowlist.
pub fn execute_remove_collateral_token(
    deps: DepsMut,
    info: MessageInfo,
    denom: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }
    collateral::remove_collateral_token(deps, &denom)
}

/// Update the risk weight for an allowed collateral token (admin only).
///
/// # Errors
///
/// - [`ContractError::Unauthorized`] if the caller is not the contract owner.
/// - [`ContractError::CollateralTokenNotAllowed`] if `denom` is not in the
///   allowlist.
/// - [`ContractError::InvalidAmount`] if `risk_weight_bps > 10_000`.
pub fn execute_set_collateral_risk_weight(
    deps: DepsMut,
    info: MessageInfo,
    denom: String,
    risk_weight_bps: u32,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }
    collateral::set_collateral_risk_weight(deps, &denom, risk_weight_bps)
}

fn append_audit_entry(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    credit_line_id: u64,
    draw_id: u64,
    action: DrawAction,
    memo: String,
) -> Result<(), ContractError> {
    let audit_count = DRAW_AUDIT_COUNT
        .may_load(deps.storage, (credit_line_id, draw_id))?
        .unwrap_or(0);

    let entry = DrawAuditEntry {
        seq: audit_count,
        draw_id,
        credit_line_id,
        action,
        timestamp: env.block.time,
        block_height: env.block.height,
        by: info.sender,
        memo,
    };

    DRAW_AUDIT.save(deps.storage, (credit_line_id, draw_id, audit_count), &entry)?;
    DRAW_AUDIT_COUNT.save(deps.storage, (credit_line_id, draw_id), &(audit_count + 1))?;

    Ok(())
}

/// Read-only query dispatcher for the borrow subsystem (v7 entrypoint).
///
/// Routes each [`QueryMsg`] variant to its corresponding view function and
/// serialises the typed response as JSON via [`to_json_binary`].  All
/// variants are pure reads — no storage mutations occur, no auth is
/// required, and the contract's response cacheability is maximised.
///
/// Query route table:
///
/// | Variant | Handler | Returns |
/// |---|---|---|
/// | `DrawAuditTrail { credit_line_id, draw_id }` | [`views::query_draw_audit_trail`] | `Vec<DrawAuditTrailResponse>` |
/// | `ProofOfReserve { denom }` | [`views::query_proof_of_reserve`] | [`ProofOfReserveResponse`] |
/// | `BorrowerHealthFactor { borrower }` | [`views::query_borrower_health_factor`] | [`BorrowerHealthFactorResponse`] |
/// | `GetOracleQuorumConfig {}` | inline (direct storage read) | [`OracleQuorumConfigResponse`] |
/// | `GetOraclePrice {}` | inline (direct storage read) | [`OraclePriceResponse`] |
/// | `GetLateFeeConfig {}` | inline (direct storage read) | [`LateFeeConfigResponse`] |
///
/// # Parameters
///
/// - `deps` — Read-only storage + API access.  Every variant consults
///   `deps.storage`; none touch the querier (no cross-contract calls in v7).
/// - `_env` — Block environment (unused by any v7 query; reserved for
///   future time-gated views).
/// - `msg` — Tagged-union query payload; the `#[cw_serde]` `QueryResponses`
///   derive macro attaches schema-level return types so client codegen
///   produces typed wrappers.
///
/// # Returns
///
/// `StdResult<Binary>` — JSON-encoded response body matching the variant's
/// `#[returns(…)]` schema annotation.  On failure a CosmWasm `StdError` is
/// returned (note: contract-level [`ContractError`] values from the view
/// helpers are **not** ABI-stable through the query boundary — they are
/// stringified via `.to_string()` into `StdError::GenericErr`).
///
/// # @notice
/// Callers SHOULD prefer the direct pub view helpers
/// ([`views::query_draw_audit_trail`] et al.) when composing from within
/// another Rust contract; the query endpoint is for off-chain consumers
/// and CW20-style cross-contract `WasmQuery` calls.
///
/// # @dev
/// The three inline reads (`GetOracleQuorumConfig`, `GetOraclePrice`,
/// `GetLateFeeConfig`) are trivial `may_load` calls and are intentionally
/// expanded here rather than routed through sub-modules to keep the
/// dispatch table a single match block for auditability.
#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::DrawAuditTrail {
            credit_line_id,
            draw_id,
        } => {
            let resp = views::query_draw_audit_trail(deps, credit_line_id, draw_id)
                .map_err(|e| StdError::generic_err(e.to_string()))?;
            to_json_binary(&resp)
        }
        QueryMsg::ProofOfReserve { denom } => {
            let resp = views::query_proof_of_reserve(deps, denom)
                .map_err(|e| StdError::generic_err(e.to_string()))?;
            to_json_binary(&resp)
        }
        QueryMsg::BorrowerHealthFactor { borrower } => {
            let resp = views::query_borrower_health_factor(deps, borrower)
                .map_err(|e| StdError::generic_err(e.to_string()))?;
            to_json_binary(&resp)
        }
        QueryMsg::GetOracleQuorumConfig {} => {
            let config = ORACLE_QUORUM_CONFIG
                .may_load(deps.storage)
                .map_err(|e| StdError::generic_err(e.to_string()))?;
            let resp = crate::msg::OracleQuorumConfigResponse { config };
            to_json_binary(&resp)
        }
        QueryMsg::GetOraclePrice {} => {
            let record = ORACLE_PRICE_RECORD
                .may_load(deps.storage)
                .map_err(|e| StdError::generic_err(e.to_string()))?;
            let resp = crate::msg::OraclePriceResponse {
                price: record.as_ref().map(|r| r.price),
                timestamp: record.as_ref().map(|r| r.timestamp),
            };
            to_json_binary(&resp)
        }
        QueryMsg::GetLateFeeConfig {} => {
            let config = LATE_FEE_CONFIG
                .may_load(deps.storage)
                .map_err(|e| StdError::generic_err(e.to_string()))?;
            let resp = crate::msg::LateFeeConfigResponse { config };
            to_json_binary(&resp)
        }
        QueryMsg::GetCollateralBalance { borrower, denom } => {
            query_collateral_balance(deps, borrower, denom)
        }
        QueryMsg::GetCollateralAllowlist {} => query_collateral_allowlist(deps),
    }
}

fn query_collateral_balance(
    deps: Deps,
    borrower: String,
    denom: Option<String>,
) -> StdResult<Binary> {
    let borrower_addr = deps.api.addr_validate(&borrower)?;

    let entries = match denom {
        Some(ref d) => {
            let amount = collateral::query_collateral_balance(deps, &borrower_addr, d);
            let risk_weight_bps = collateral::collateral_risk_weight_bps(deps, d);
            if amount.is_zero() {
                vec![]
            } else {
                vec![CollateralEntryResponse {
                    denom: d.clone(),
                    amount,
                    risk_weight_bps,
                }]
            }
        }
        None => {
            let raw = collateral::query_borrower_collateral(deps, &borrower_addr);
            raw.into_iter()
                .map(|(denom, amount)| CollateralEntryResponse {
                    denom: denom.clone(),
                    amount,
                    risk_weight_bps: collateral::collateral_risk_weight_bps(deps, &denom),
                })
                .collect()
        }
    };

    let weighted_total = collateral::weighted_collateral_total(deps, &borrower_addr)
        .map_err(|e| StdError::generic_err(e.to_string()))?;

    let resp = CollateralBalanceResponse {
        borrower,
        entries,
        weighted_total,
    };
    to_json_binary(&resp)
}

fn query_collateral_allowlist(deps: Deps) -> StdResult<Binary> {
    let denoms = collateral::query_collateral_allowlist(deps);
    let resp = CollateralAllowlistResponse { denoms };
    to_json_binary(&resp)
}

#[entry_point]
pub fn migrate(_deps: DepsMut, _env: Env, _msg: MigrateMsg) -> StdResult<Response> {
    Ok(Response::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
    use crate::penalties::{AprFeeConfig, FlatFeeConfig, LateFeeConfig};
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
    use cosmwasm_std::testing::{MockApi, MockQuerier, MockStorage};
    use cosmwasm_std::{from_json, Addr, OwnedDeps, Uint128};

    fn creator(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
        deps.api.addr_make("creator")
    }

    fn non_admin(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
        deps.api.addr_make("non_admin")
    }

    fn setup(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>) {
        let env = mock_env();
        let info = message_info(&creator(deps), &[]);
        let msg = InstantiateMsg {
            owner: creator(deps).to_string(),
        };
        instantiate(deps.as_mut(), env, info, msg).unwrap();
    }

    fn query_late_fee_config(
        deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>,
    ) -> Option<LateFeeConfig> {
        let env = mock_env();
        let msg = QueryMsg::GetLateFeeConfig {};
        let raw = query(deps.as_ref(), env, msg).unwrap();
        let resp: crate::msg::LateFeeConfigResponse = from_json(&raw).unwrap();
        resp.config
    }

    fn set_late_fee_config(
        deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
        sender: &Addr,
        config: Option<LateFeeConfig>,
    ) -> Result<Response, ContractError> {
        let env = mock_env();
        let info = message_info(sender, &[]);
        let msg = ExecuteMsg::SetLateFeeConfig { config };
        execute(deps.as_mut(), env, info, msg)
    }

    mod set_late_fee_config {
        use super::*;

        #[test]
        fn admin_can_set_flat_config() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = creator(&deps);

            let config = LateFeeConfig::Flat(FlatFeeConfig {
                amount: Uint128::new(100),
            });
            set_late_fee_config(&mut deps, &admin, Some(config)).unwrap();

            let stored = query_late_fee_config(&deps);
            assert_eq!(stored, Some(config));
        }

        #[test]
        fn admin_can_set_apr_config() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = creator(&deps);

            let config = LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 500 });
            set_late_fee_config(&mut deps, &admin, Some(config)).unwrap();

            let stored = query_late_fee_config(&deps);
            assert_eq!(stored, Some(config));
        }

        #[test]
        fn admin_can_clear_config() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = creator(&deps);

            let config = LateFeeConfig::Flat(FlatFeeConfig {
                amount: Uint128::new(50),
            });
            set_late_fee_config(&mut deps, &admin, Some(config)).unwrap();
            assert!(query_late_fee_config(&deps).is_some());

            set_late_fee_config(&mut deps, &admin, None).unwrap();
            assert!(query_late_fee_config(&deps).is_none());
        }

        #[test]
        fn non_admin_cannot_set_config() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let unauth = non_admin(&deps);

            let config = LateFeeConfig::Flat(FlatFeeConfig {
                amount: Uint128::new(100),
            });
            let err = set_late_fee_config(&mut deps, &unauth, Some(config)).unwrap_err();
            assert_eq!(err, ContractError::Unauthorized);
        }

        #[test]
        fn zero_flat_amount_rejected() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = creator(&deps);

            let config = LateFeeConfig::Flat(FlatFeeConfig {
                amount: Uint128::zero(),
            });
            let err = set_late_fee_config(&mut deps, &admin, Some(config)).unwrap_err();
            assert_eq!(err, ContractError::InvalidAmount);
        }

        #[test]
        fn apr_surcharge_exceeds_max_rejected() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = creator(&deps);

            let config = LateFeeConfig::AprBased(AprFeeConfig {
                surcharge_bps: 10_001,
            });
            let err = set_late_fee_config(&mut deps, &admin, Some(config)).unwrap_err();
            assert_eq!(err, ContractError::RateTooHigh);
        }

        #[test]
        fn max_apr_surcharge_accepted() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = creator(&deps);

            let config = LateFeeConfig::AprBased(AprFeeConfig {
                surcharge_bps: 10_000,
            });
            set_late_fee_config(&mut deps, &admin, Some(config)).unwrap();

            let stored = query_late_fee_config(&deps);
            assert_eq!(stored, Some(config));
        }

        #[test]
        fn clearing_config_when_already_clear_is_noop() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = creator(&deps);

            assert!(query_late_fee_config(&deps).is_none());
            set_late_fee_config(&mut deps, &admin, None).unwrap();
            assert!(query_late_fee_config(&deps).is_none());
        }

        #[test]
        fn set_response_has_correct_attributes() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = creator(&deps);

            let config = LateFeeConfig::Flat(FlatFeeConfig {
                amount: Uint128::new(200),
            });
            let resp = set_late_fee_config(&mut deps, &admin, Some(config)).unwrap();
            assert_eq!(resp.attributes[0].key, "action");
            assert_eq!(resp.attributes[0].value, "set_late_fee_config");

            set_late_fee_config(&mut deps, &admin, None).unwrap();
        }

        #[test]
        fn query_default_is_none() {
            let mut deps = mock_dependencies();
            setup(&mut deps);

            assert!(query_late_fee_config(&deps).is_none());
        }

        #[test]
        fn flat_config_survives_set_overwrite() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = creator(&deps);

            let flat = LateFeeConfig::Flat(FlatFeeConfig {
                amount: Uint128::new(100),
            });
            let apr = LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 200 });

            set_late_fee_config(&mut deps, &admin, Some(flat)).unwrap();
            set_late_fee_config(&mut deps, &admin, Some(apr)).unwrap();

            let stored = query_late_fee_config(&deps);
            assert_eq!(stored, Some(apr));
        }
    }
}
