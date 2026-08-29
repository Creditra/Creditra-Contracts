import { CreditLine, RiskFormulaConfig, ProtocolState, Auction, ContractEvent } from '../types';

export const INITIAL_RISK_CONFIG: RiskFormulaConfig = {
  baseRateBps: 450, // 4.50% base rate (b)
  riskSensitivityBps: 12, // 0.12% increase per risk point (s)
  rateFloorBps: 300, // 3.00% floor (r_min)
  rateCeilingBps: 2500, // 25.00% ceiling (r_max)
};

export const INITIAL_PROTOCOL_STATE: ProtocolState = {
  isPaused: false,
  isDrawsFrozen: false,
  maxTotalExposure: 5000000, // $5,000,000 max total credit
  totalExposure: 1850000,
  totalUtilized: 820000,
  utilizationCapBps: 8500, // 85%
  minCollateralRatioBps: 13000, // 130%
  protocolFeeBps: 1000, // 10% protocol fee on interest accrued
  treasuryBalance: 24580, // $24,580 collected fees
  drawMinIntervalSeconds: 300, // 5 min draw cooldown
};

const now = Math.floor(Date.now() / 1000);

export const INITIAL_CREDIT_LINES: CreditLine[] = [
  {
    borrower: 'GAC3...9K21',
    borrowerName: 'Stellar FinTech Corp',
    limit: 500000,
    utilized: 210000,
    interestRateBps: 690, // 6.90%
    riskScore: 200, // Low risk
    collateral: 350000, // 166% collateralized
    collateralRatioBps: 16666,
    status: 'Active',
    openTimestamp: now - 86400 * 45,
    lastAccrualTimestamp: now - 3600 * 4,
    accruedInterest: 1240,
    isDelinquent: false,
    isBlocked: false,
  },
  {
    borrower: 'GDR4...1M88',
    borrowerName: 'Mercurial Liquidity Pool',
    limit: 750000,
    utilized: 450000,
    interestRateBps: 870, // 8.70%
    riskScore: 350,
    collateral: 600000, // 133% collateralized
    collateralRatioBps: 13333,
    status: 'Active',
    openTimestamp: now - 86400 * 30,
    lastAccrualTimestamp: now - 3600 * 12,
    accruedInterest: 3820,
    isDelinquent: false,
    isBlocked: false,
  },
  {
    borrower: 'GBX9...4P02',
    borrowerName: 'Apex Capital Fund',
    limit: 400000,
    utilized: 160000,
    interestRateBps: 1170, // 11.70%
    riskScore: 600, // Higher risk
    collateral: 240000,
    collateralRatioBps: 15000,
    status: 'Active',
    openTimestamp: now - 86400 * 15,
    lastAccrualTimestamp: now - 3600 * 2,
    accruedInterest: 890,
    isDelinquent: false,
    isBlocked: false,
  },
  {
    borrower: 'GKT7...8W44',
    borrowerName: 'Horizon Micro-Lending',
    limit: 200000,
    utilized: 0,
    interestRateBps: 510, // 5.10%
    riskScore: 50, // Ultra safe
    collateral: 100000,
    collateralRatioBps: 0,
    status: 'Suspended',
    openTimestamp: now - 86400 * 60,
    lastAccrualTimestamp: now - 86400 * 10,
    accruedInterest: 0,
    isDelinquent: false,
    isBlocked: false,
  },
  {
    borrower: 'GZZ1...2Q99',
    borrowerName: 'Vanguard Credit DAO',
    limit: 300000,
    utilized: 280000,
    interestRateBps: 1890, // 18.90%
    riskScore: 820, // Critical delinquency risk
    collateral: 250000,
    collateralRatioBps: 8928, // Undercollateralized (< 130%)
    status: 'Defaulted',
    openTimestamp: now - 86400 * 90,
    lastAccrualTimestamp: now - 86400 * 5,
    accruedInterest: 14200,
    gracePeriodEndsAt: now - 86400 * 2,
    isDelinquent: true,
    isBlocked: true,
  }
];

export const INITIAL_AUCTIONS: Auction[] = [
  {
    id: 'AUC-2026-001',
    borrower: 'GZZ1...2Q99',
    mode: 'English',
    collateralAmount: 250000,
    debtToCover: 294200, // 280k principal + 14.2k interest
    startTime: now - 3600 * 6,
    endTime: now + 3600 * 18,
    minBid: 200000,
    currentHighBid: 245000,
    currentHighBidder: 'GBIDDER...88',
    status: 'Active',
    dutchStartPrice: 310000,
    dutchFloorPrice: 200000,
    dutchDecayRateSec: 10,
  },
  {
    id: 'AUC-2026-002',
    borrower: 'GOLD-OLD...44',
    mode: 'Dutch',
    collateralAmount: 180000,
    debtToCover: 160000,
    startTime: now - 3600 * 2,
    endTime: now + 3600 * 10,
    minBid: 150000,
    currentHighBid: 172000,
    currentHighBidder: 'GVAL...990',
    status: 'Active',
    dutchStartPrice: 220000,
    dutchFloorPrice: 150000,
    dutchDecayRateSec: 15,
  }
];

export const INITIAL_EVENTS: ContractEvent[] = [
  {
    id: 'evt-001',
    timestamp: now - 3600 * 12,
    topic: 'draw_credit',
    borrower: 'GDR4...1M88',
    payload: { drawAmount: 50000, totalUtilized: 450000, interestRateBps: 870 }
  },
  {
    id: 'evt-002',
    timestamp: now - 3600 * 8,
    topic: 'repay_credit',
    borrower: 'GAC3...9K21',
    payload: { repayAmount: 25000, totalUtilized: 210000, principalPaid: 24000, feePaid: 1000 }
  },
  {
    id: 'evt-003',
    timestamp: now - 3600 * 5,
    topic: 'update_risk_parameters',
    payload: { baseRateBps: 450, riskSensitivityBps: 12, rateFloorBps: 300, rateCeilingBps: 2500 }
  },
  {
    id: 'evt-004',
    timestamp: now - 3600 * 3,
    topic: 'default_credit_line',
    borrower: 'GZZ1...2Q99',
    payload: { defaultedAt: now - 3600 * 3, totalDebt: 294200, collateralSeized: 250000 }
  }
];
