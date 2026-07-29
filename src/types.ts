export type CreditStatus = 'Active' | 'Suspended' | 'Closed' | 'Defaulted';

export interface CreditLine {
  borrower: string;
  borrowerName: string;
  limit: number; // in XLM/USDC units (i128)
  utilized: number; // in XLM/USDC units
  interestRateBps: number; // e.g., 850 = 8.50%
  riskScore: number; // 0 to 1000 score
  collateral: number; // deposited collateral
  collateralRatioBps: number; // e.g. 15000 = 150%
  status: CreditStatus;
  openTimestamp: number; // unix timestamp in seconds
  lastAccrualTimestamp: number; // unix timestamp
  accruedInterest: number;
  gracePeriodEndsAt?: number;
  isDelinquent: boolean;
  isBlocked: boolean;
}

export interface RiskFormulaConfig {
  baseRateBps: number; // b
  riskSensitivityBps: number; // s
  rateFloorBps: number; // r_min
  rateCeilingBps: number; // r_max
}

export interface ProtocolState {
  isPaused: boolean;
  isDrawsFrozen: boolean;
  maxTotalExposure: number;
  totalExposure: number;
  totalUtilized: number;
  utilizationCapBps: number; // e.g. 8000 = 80%
  minCollateralRatioBps: number; // e.g. 12000 = 120%
  protocolFeeBps: number; // e.g. 1000 = 10% on interest
  treasuryBalance: number;
  drawMinIntervalSeconds: number;
}

export type AuctionMode = 'English' | 'Dutch';
export type AuctionStatus = 'Active' | 'Closed' | 'Settled' | 'Claimed';

export interface Auction {
  id: string;
  borrower: string;
  mode: AuctionMode;
  collateralAmount: number;
  debtToCover: number;
  startTime: number;
  endTime: number;
  minBid: number;
  currentHighBid: number;
  currentHighBidder: string | null;
  status: AuctionStatus;
  dutchStartPrice: number;
  dutchFloorPrice: number;
  dutchDecayRateSec: number;
  settledAmount?: number;
}

export interface ContractEvent {
  id: string;
  timestamp: number;
  topic: string; // e.g., "draw_credit", "repay_credit", "settle_default_liquidation"
  borrower?: string;
  payload: Record<string, any>;
}

export interface ContractErrorSpec {
  code: number;
  variant: string;
  module: 'credit' | 'borrow' | 'collateral' | 'risk' | 'auction' | 'freeze';
  summary: string;
  description: string;
}
