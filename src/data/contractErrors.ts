import { ContractErrorSpec } from '../types';

export const CONTRACT_ERRORS: ContractErrorSpec[] = [
  {
    code: 101,
    variant: 'Unauthorized',
    module: 'credit',
    summary: 'Caller lacks administrative or borrower authorization',
    description: 'Triggered when a function requiring admin authentication (require_admin) or borrower auth is invoked by an unauthorized principal.'
  },
  {
    code: 102,
    variant: 'AlreadyInitialized',
    module: 'credit',
    summary: 'Contract has already been initialized',
    description: 'The init entrypoint was called on a contract where DataKey::Admin is already populated.'
  },
  {
    code: 103,
    variant: 'CreditLineNotFound',
    module: 'credit',
    summary: 'Credit line does not exist for the specified borrower',
    description: 'Lookup for DataKey::CreditLine(borrower) returned None during draw, repay, or status lookup.'
  },
  {
    code: 104,
    variant: 'CreditLineAlreadyExists',
    module: 'credit',
    summary: 'Borrower already has an open credit line',
    description: 'open_credit_line called for a borrower who already possesses an active or suspended line under the duplicate open policy.'
  },
  {
    code: 105,
    variant: 'InsufficientAvailableLimit',
    module: 'borrow',
    summary: 'Requested draw amount exceeds available credit limit',
    description: 'draw_amount + utilized exceeds the computed credit_limit for the credit line.'
  },
  {
    code: 106,
    variant: 'DrawsFrozen',
    module: 'freeze',
    summary: 'Global draws are frozen by circuit breaker',
    description: 'draw_credit invoked while global draws_frozen flag is set to true in system config.'
  },
  {
    code: 107,
    variant: 'ProtocolPaused',
    module: 'credit',
    summary: 'Protocol operations are currently paused',
    description: 'Protocol-wide pause_protocol circuit breaker is active. Only repay_credit remains allowed.'
  },
  {
    code: 108,
    variant: 'MinCollateralViolation',
    module: 'collateral',
    summary: 'Collateral ratio drops below required minimum floor',
    description: 'Withdrawal or draw request would push the collateral-to-debt ratio below MinCollateralRatioBps (default 150%).'
  },
  {
    code: 109,
    variant: 'ExceedsUtilizationCap',
    module: 'credit',
    summary: 'Global or pool utilization exceeds the configured cap',
    description: 'Total protocol utilization would exceed UtilizationCapBps percentage of total exposure.'
  },
  {
    code: 110,
    variant: 'RateChangeExceedsLimit',
    module: 'risk',
    summary: 'Requested rate change violates maximum rate-delta guardrail',
    description: 'update_risk_parameters requested an interest rate shift exceeding MaxRateDeltaBps.'
  },
  {
    code: 111,
    variant: 'DelinquentNoDraw',
    module: 'borrow',
    summary: 'Borrower is delinquent and blocked from additional draws',
    description: 'Interest or repayment schedule is past due. Borrower must settle accrued debt before drawing further.'
  },
  {
    code: 112,
    variant: 'InvalidRiskScore',
    module: 'risk',
    summary: 'Provided risk score is out of valid bounds (0..1000)',
    description: 'Off-chain risk score input exceeds the mathematical domain expected by compute_rate_from_score.'
  },
  {
    code: 113,
    variant: 'ReentrancyLocked',
    module: 'credit',
    summary: 'Reentrancy guard active',
    description: 'Attempted recursive invocation of a protected entrypoint during cross-contract call execution.'
  },
  {
    code: 114,
    variant: 'OracleStale',
    module: 'credit',
    summary: 'Price or default oracle timestamp exceeds maximum staleness window',
    description: 'Oracle feed data is older than max_oracle_staleness_seconds, tripping the safety circuit breaker.'
  },
  {
    code: 115,
    variant: 'InvalidAuctionState',
    module: 'auction',
    summary: 'Auction is not in the required state for this operation',
    description: 'Bid or settlement attempted on an auction that is either expired, closed, or already settled.'
  },
  {
    code: 116,
    variant: 'BorrowerBlocked',
    module: 'credit',
    summary: 'Borrower address is explicitly blocked in security filter',
    description: 'Borrower principal is present in the DataKey::BlockedBorrower registry.'
  },
  {
    code: 117,
    variant: 'DrawCooldownActive',
    module: 'borrow',
    summary: 'Draw requested prior to draw_min_interval cooldown expiry',
    description: 'Borrower attempted consecutive draws before draw_min_interval_seconds elapsed since last draw.'
  },
  {
    code: 118,
    variant: 'ZeroAmountInvalid',
    module: 'borrow',
    summary: 'Requested draw or repayment amount must be strictly positive',
    description: 'draw_credit or repay_credit called with an amount <= 0.'
  },
  {
    code: 119,
    variant: 'InvalidStatusTransition',
    module: 'credit',
    summary: 'Requested state transition is prohibited by protocol lifecycle state machine',
    description: 'Attempted illegal transition, such as reinstating a Closed line or defaulting an already Closed line.'
  },
  {
    code: 120,
    variant: 'CrossContractHandshakeFailed',
    module: 'auction',
    summary: 'Cross-contract liquidation handoff replay protection rejected call',
    description: 'settle_default_liquidation failed verification between Credit and Auction contract instances.'
  },
  {
    code: 121,
    variant: 'ExceedsMaxTotalExposure',
    module: 'credit',
    summary: 'Operation would exceed global protocol exposure cap',
    description: 'Sum of all credit limits would breach max_total_exposure configured by risk governance.'
  },
  {
    code: 122,
    variant: 'BidTooLow',
    module: 'auction',
    summary: 'Placed bid does not meet minimum increment requirement',
    description: 'Bid amount is less than current_high_bid + min_increment_bps in English auction mode.'
  },
  {
    code: 123,
    variant: 'AuctionExpired',
    module: 'auction',
    summary: 'Auction duration has elapsed',
    description: 'Bid submitted after end_time timestamp in auction storage.'
  },
  {
    code: 124,
    variant: 'AlreadySettled',
    module: 'auction',
    summary: 'Auction has already been settled and debt written off',
    description: 'settle_default_liquidation called more than once for the same auction_id.'
  },
  {
    code: 125,
    variant: 'TreasuryBalanceInsufficient',
    module: 'credit',
    summary: 'Requested treasury withdrawal exceeds accrued fee reserves',
    description: 'withdraw_treasury called for an amount greater than DataKey::TreasuryBalance.'
  },
  {
    code: 126,
    variant: 'GracePeriodActive',
    module: 'credit',
    summary: 'Default cannot be declared while borrower is inside active grace period',
    description: 'default_credit_line rejected because current_timestamp < grace_period_ends_at.'
  },
  {
    code: 127,
    variant: 'BorrowerSelfSuspended',
    module: 'credit',
    summary: 'Draw rejected because borrower explicitly self-suspended line',
    description: 'Borrower triggered self_suspend_credit_line to halt further borrowing on their own line.'
  },
  {
    code: 128,
    variant: 'RateFloorViolation',
    module: 'risk',
    summary: 'Computed interest rate falls below strict protocol rate floor',
    description: 'r(k) result calculated below rate_floor_bps before clamp evaluation.'
  }
];
