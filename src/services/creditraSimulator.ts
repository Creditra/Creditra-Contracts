import { RiskFormulaConfig, CreditLine, ProtocolState, Auction, ContractEvent } from '../types';

/**
 * Computes interest rate (in basis points) using Creditra's exact risk-pricing formula:
 * r(k) = clamp(b + k * s, r_min, min(r_max, 10000))
 * 
 * @param score Risk score (0..1000)
 * @param config Rate formula configuration parameters
 */
export function computeRateFromScore(score: number, config: RiskFormulaConfig): number {
  const clampedScore = Math.max(0, Math.min(1000, score));
  const rawRate = config.baseRateBps + clampedScore * config.riskSensitivityBps;
  const effectiveCeiling = Math.min(config.rateCeilingBps, 10000);
  return Math.max(config.rateFloorBps, Math.min(rawRate, effectiveCeiling));
}

/**
 * Calculates interest accrued on a credit line based on elapsed seconds:
 * Accrued = (Utilized * RateBps * ElapsedSeconds) / (10000 * 31536000)
 */
export function calculateAccruedInterest(utilized: number, rateBps: number, elapsedSeconds: number): number {
  if (utilized <= 0 || rateBps <= 0 || elapsedSeconds <= 0) return 0;
  const SECONDS_PER_YEAR = 31536000;
  const accrued = (utilized * rateBps * elapsedSeconds) / (10000 * SECONDS_PER_YEAR);
  return Math.round(accrued * 100) / 100;
}

/**
 * Calculates Dutch Auction current price based on elapsed time decay
 */
export function calculateDutchAuctionPrice(
  startPrice: number,
  floorPrice: number,
  decayRateSec: number,
  elapsedSec: number
): number {
  const priceDrop = elapsedSec * decayRateSec;
  const currentPrice = startPrice - priceDrop;
  return Math.max(floorPrice, Math.round(currentPrice));
}

/**
 * Computes collateral ratio in basis points (10000 = 100%)
 */
export function calculateCollateralRatio(collateral: number, debt: number): number {
  if (debt <= 0) return collateral > 0 ? 99999 : 0;
  return Math.round((collateral / debt) * 10000);
}
