import React, { createContext, useContext, useState, useEffect } from 'react';
import {
  CreditLine,
  RiskFormulaConfig,
  ProtocolState,
  Auction,
  ContractEvent,
  CreditStatus
} from '../types';
import {
  INITIAL_CREDIT_LINES,
  INITIAL_RISK_CONFIG,
  INITIAL_PROTOCOL_STATE,
  INITIAL_AUCTIONS,
  INITIAL_EVENTS
} from '../data/initialData';
import { computeRateFromScore, calculateAccruedInterest, calculateCollateralRatio } from '../services/creditraSimulator';

interface ProtocolContextType {
  creditLines: CreditLine[];
  riskConfig: RiskFormulaConfig;
  protocolState: ProtocolState;
  auctions: Auction[];
  events: ContractEvent[];
  openCreditLine: (borrower: string, borrowerName: string, limit: number, riskScore: number, collateral: number) => { success: boolean; message: string };
  drawCredit: (borrower: string, amount: number) => { success: boolean; message: string };
  repayCredit: (borrower: string, amount: number) => { success: boolean; message: string };
  depositCollateral: (borrower: string, amount: number) => { success: boolean; message: string };
  withdrawCollateral: (borrower: string, amount: number) => { success: boolean; message: string };
  selfSuspendCreditLine: (borrower: string) => { success: boolean; message: string };
  reinstateCreditLine: (borrower: string) => { success: boolean; message: string };
  defaultCreditLine: (borrower: string) => { success: boolean; message: string };
  closeCreditLine: (borrower: string) => { success: boolean; message: string };
  updateRiskConfig: (newConfig: RiskFormulaConfig) => { success: boolean; message: string };
  togglePauseProtocol: () => void;
  toggleFreezeDraws: () => void;
  withdrawTreasury: (amount: number) => { success: boolean; message: string };
  placeAuctionBid: (auctionId: string, bidder: string, amount: number) => { success: boolean; message: string };
  settleDefaultLiquidation: (auctionId: string) => { success: boolean; message: string };
  resetSimulator: () => void;
  simulateTimePassage: (hours: number) => void;
}

const ProtocolContext = createContext<ProtocolContextType | undefined>(undefined);

export const ProtocolProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [creditLines, setCreditLines] = useState<CreditLine[]>(INITIAL_CREDIT_LINES);
  const [riskConfig, setRiskConfig] = useState<RiskFormulaConfig>(INITIAL_RISK_CONFIG);
  const [protocolState, setProtocolState] = useState<ProtocolState>(INITIAL_PROTOCOL_STATE);
  const [auctions, setAuctions] = useState<Auction[]>(INITIAL_AUCTIONS);
  const [events, setEvents] = useState<ContractEvent[]>(INITIAL_EVENTS);

  // Helper to log contract events
  const emitEvent = (topic: string, borrower: string | undefined, payload: Record<string, any>) => {
    const newEvent: ContractEvent = {
      id: `evt-${Date.now().toString().slice(-6)}`,
      timestamp: Math.floor(Date.now() / 1000),
      topic,
      borrower,
      payload
    };
    setEvents(prev => [newEvent, ...prev]);
  };

  // Open Credit Line
  const openCreditLine = (
    borrower: string,
    borrowerName: string,
    limit: number,
    riskScore: number,
    collateral: number
  ) => {
    if (protocolState.isPaused) {
      return { success: false, message: 'ContractError::ProtocolPaused (107)' };
    }
    if (creditLines.some(c => c.borrower.toLowerCase() === borrower.toLowerCase())) {
      return { success: false, message: 'ContractError::CreditLineAlreadyExists (104)' };
    }
    if (protocolState.totalExposure + limit > protocolState.maxTotalExposure) {
      return { success: false, message: 'ContractError::ExceedsMaxTotalExposure (121)' };
    }

    const calculatedRate = computeRateFromScore(riskScore, riskConfig);
    const nowSec = Math.floor(Date.now() / 1000);
    const colRatio = calculateCollateralRatio(collateral, 0);

    const newLine: CreditLine = {
      borrower,
      borrowerName: borrowerName || 'Borrower ' + borrower.slice(0, 6),
      limit,
      utilized: 0,
      interestRateBps: calculatedRate,
      riskScore,
      collateral,
      collateralRatioBps: colRatio,
      status: 'Active',
      openTimestamp: nowSec,
      lastAccrualTimestamp: nowSec,
      accruedInterest: 0,
      isDelinquent: false,
      isBlocked: false,
    };

    setCreditLines(prev => [...prev, newLine]);
    setProtocolState(prev => ({
      ...prev,
      totalExposure: prev.totalExposure + limit
    }));

    emitEvent('open_credit_line', borrower, {
      creditLimit: limit,
      interestRateBps: calculatedRate,
      riskScore,
      collateral
    });

    return { success: true, message: `Credit line of $${limit.toLocaleString()} opened for ${borrower}` };
  };

  // Draw Credit
  const drawCredit = (borrower: string, amount: number) => {
    if (protocolState.isPaused) {
      return { success: false, message: 'ContractError::ProtocolPaused (107)' };
    }
    if (protocolState.isDrawsFrozen) {
      return { success: false, message: 'ContractError::DrawsFrozen (106)' };
    }
    if (amount <= 0) {
      return { success: false, message: 'ContractError::ZeroAmountInvalid (118)' };
    }

    const index = creditLines.findIndex(c => c.borrower === borrower);
    if (index === -1) {
      return { success: false, message: 'ContractError::CreditLineNotFound (103)' };
    }

    const line = creditLines[index];

    if (line.status === 'Suspended') {
      return { success: false, message: 'ContractError::BorrowerSelfSuspended (127)' };
    }
    if (line.status !== 'Active') {
      return { success: false, message: 'ContractError::InvalidStatusTransition (119)' };
    }
    if (line.isBlocked) {
      return { success: false, message: 'ContractError::BorrowerBlocked (116)' };
    }
    if (line.isDelinquent) {
      return { success: false, message: 'ContractError::DelinquentNoDraw (111)' };
    }

    const available = line.limit - line.utilized;
    if (amount > available) {
      return { success: false, message: 'ContractError::InsufficientAvailableLimit (105)' };
    }

    // Check utilization cap
    const newTotalUtilized = protocolState.totalUtilized + amount;
    const maxAllowedUtilized = (protocolState.totalExposure * protocolState.utilizationCapBps) / 10000;
    if (newTotalUtilized > maxAllowedUtilized) {
      return { success: false, message: 'ContractError::ExceedsUtilizationCap (109)' };
    }

    const newUtilized = line.utilized + amount;
    const totalDebt = newUtilized + line.accruedInterest;
    const newColRatio = calculateCollateralRatio(line.collateral, totalDebt);

    // If collateral exists and drops below min ratio
    if (line.collateral > 0 && newColRatio < protocolState.minCollateralRatioBps) {
      return { success: false, message: 'ContractError::MinCollateralViolation (108)' };
    }

    const updated = [...creditLines];
    updated[index] = {
      ...line,
      utilized: newUtilized,
      collateralRatioBps: newColRatio,
      lastAccrualTimestamp: Math.floor(Date.now() / 1000)
    };

    setCreditLines(updated);
    setProtocolState(prev => ({
      ...prev,
      totalUtilized: prev.totalUtilized + amount
    }));

    emitEvent('draw_credit', borrower, {
      drawAmount: amount,
      totalUtilized: newUtilized,
      availableLimit: line.limit - newUtilized
    });

    return { success: true, message: `Successfully drew $${amount.toLocaleString()} on ${borrower}` };
  };

  // Repay Credit
  const repayCredit = (borrower: string, amount: number) => {
    if (amount <= 0) {
      return { success: false, message: 'ContractError::ZeroAmountInvalid (118)' };
    }

    const index = creditLines.findIndex(c => c.borrower === borrower);
    if (index === -1) {
      return { success: false, message: 'ContractError::CreditLineNotFound (103)' };
    }

    const line = creditLines[index];
    let remainingPay = amount;
    let newInterest = line.accruedInterest;
    let newUtilized = line.utilized;
    let feeCollected = 0;

    // First pay accrued interest
    if (remainingPay <= newInterest) {
      newInterest -= remainingPay;
      feeCollected = (remainingPay * protocolState.protocolFeeBps) / 10000;
      remainingPay = 0;
    } else {
      feeCollected = (newInterest * protocolState.protocolFeeBps) / 10000;
      remainingPay -= newInterest;
      newInterest = 0;
      // Then pay principal
      const principalPaid = Math.min(remainingPay, newUtilized);
      newUtilized -= principalPaid;
    }

    const totalPaid = amount;
    const totalDebt = newUtilized + newInterest;
    const newColRatio = calculateCollateralRatio(line.collateral, totalDebt);

    const updated = [...creditLines];
    updated[index] = {
      ...line,
      utilized: newUtilized,
      accruedInterest: newInterest,
      collateralRatioBps: newColRatio,
      isDelinquent: newInterest === 0 ? false : line.isDelinquent
    };

    setCreditLines(updated);
    setProtocolState(prev => ({
      ...prev,
      totalUtilized: Math.max(0, prev.totalUtilized - (line.utilized - newUtilized)),
      treasuryBalance: prev.treasuryBalance + feeCollected
    }));

    emitEvent('repay_credit', borrower, {
      repayAmount: totalPaid,
      remainingUtilized: newUtilized,
      remainingInterest: newInterest,
      protocolFeeCollected: feeCollected
    });

    return { success: true, message: `Repaid $${amount.toLocaleString()} for borrower ${borrower}` };
  };

  // Deposit Collateral
  const depositCollateral = (borrower: string, amount: number) => {
    if (amount <= 0) return { success: false, message: 'ContractError::ZeroAmountInvalid (118)' };
    const index = creditLines.findIndex(c => c.borrower === borrower);
    if (index === -1) return { success: false, message: 'ContractError::CreditLineNotFound (103)' };

    const line = creditLines[index];
    const newCollateral = line.collateral + amount;
    const totalDebt = line.utilized + line.accruedInterest;
    const newColRatio = calculateCollateralRatio(newCollateral, totalDebt);

    const updated = [...creditLines];
    updated[index] = {
      ...line,
      collateral: newCollateral,
      collateralRatioBps: newColRatio
    };

    setCreditLines(updated);
    emitEvent('deposit_collateral', borrower, { amount, totalCollateral: newCollateral });
    return { success: true, message: `Deposited $${amount.toLocaleString()} collateral` };
  };

  // Withdraw Collateral
  const withdrawCollateral = (borrower: string, amount: number) => {
    if (amount <= 0) return { success: false, message: 'ContractError::ZeroAmountInvalid (118)' };
    const index = creditLines.findIndex(c => c.borrower === borrower);
    if (index === -1) return { success: false, message: 'ContractError::CreditLineNotFound (103)' };

    const line = creditLines[index];
    if (amount > line.collateral) {
      return { success: false, message: 'ContractError::MinCollateralViolation (108)' };
    }

    const newCollateral = line.collateral - amount;
    const totalDebt = line.utilized + line.accruedInterest;
    const newColRatio = calculateCollateralRatio(newCollateral, totalDebt);

    if (totalDebt > 0 && newColRatio < protocolState.minCollateralRatioBps) {
      return { success: false, message: 'ContractError::MinCollateralViolation (108)' };
    }

    const updated = [...creditLines];
    updated[index] = {
      ...line,
      collateral: newCollateral,
      collateralRatioBps: newColRatio
    };

    setCreditLines(updated);
    emitEvent('withdraw_collateral', borrower, { amount, remainingCollateral: newCollateral });
    return { success: true, message: `Withdrew $${amount.toLocaleString()} collateral` };
  };

  // Self Suspend
  const selfSuspendCreditLine = (borrower: string) => {
    const index = creditLines.findIndex(c => c.borrower === borrower);
    if (index === -1) return { success: false, message: 'ContractError::CreditLineNotFound (103)' };

    const line = creditLines[index];
    if (line.status !== 'Active') return { success: false, message: 'ContractError::InvalidStatusTransition (119)' };

    const updated = [...creditLines];
    updated[index] = { ...line, status: 'Suspended' };
    setCreditLines(updated);

    emitEvent('self_suspend_credit_line', borrower, { timestamp: Math.floor(Date.now() / 1000) });
    return { success: true, message: `Borrower ${borrower} self-suspended credit line` };
  };

  // Reinstate Line
  const reinstateCreditLine = (borrower: string) => {
    const index = creditLines.findIndex(c => c.borrower === borrower);
    if (index === -1) return { success: false, message: 'ContractError::CreditLineNotFound (103)' };

    const line = creditLines[index];
    if (line.status !== 'Suspended') return { success: false, message: 'ContractError::InvalidStatusTransition (119)' };

    const updated = [...creditLines];
    updated[index] = { ...line, status: 'Active' };
    setCreditLines(updated);

    emitEvent('reinstate_credit_line', borrower, { timestamp: Math.floor(Date.now() / 1000) });
    return { success: true, message: `Credit line reinstated for ${borrower}` };
  };

  // Default Credit Line
  const defaultCreditLine = (borrower: string) => {
    const index = creditLines.findIndex(c => c.borrower === borrower);
    if (index === -1) return { success: false, message: 'ContractError::CreditLineNotFound (103)' };

    const line = creditLines[index];
    if (line.status === 'Closed' || line.status === 'Defaulted') {
      return { success: false, message: 'ContractError::InvalidStatusTransition (119)' };
    }

    const updated = [...creditLines];
    updated[index] = { ...line, status: 'Defaulted', isDelinquent: true, isBlocked: true };
    setCreditLines(updated);

    // Auto-create a liquidation auction for the defaulted collateral
    const newAuction: Auction = {
      id: `AUC-2026-0${auctions.length + 1}`,
      borrower,
      mode: 'English',
      collateralAmount: line.collateral,
      debtToCover: line.utilized + line.accruedInterest,
      startTime: Math.floor(Date.now() / 1000),
      endTime: Math.floor(Date.now() / 1000) + 86400,
      minBid: Math.round(line.collateral * 0.7),
      currentHighBid: Math.round(line.collateral * 0.75),
      currentHighBidder: 'GLIQUIDATOR...11',
      status: 'Active',
      dutchStartPrice: line.collateral * 1.2,
      dutchFloorPrice: line.collateral * 0.6,
      dutchDecayRateSec: 10
    };

    setAuctions(prev => [newAuction, ...prev]);

    emitEvent('default_credit_line', borrower, {
      totalDebt: line.utilized + line.accruedInterest,
      collateralSeized: line.collateral,
      auctionId: newAuction.id
    });

    return { success: true, message: `Default declared on ${borrower}. Auction ${newAuction.id} created!` };
  };

  // Close Line
  const closeCreditLine = (borrower: string) => {
    const index = creditLines.findIndex(c => c.borrower === borrower);
    if (index === -1) return { success: false, message: 'ContractError::CreditLineNotFound (103)' };

    const line = creditLines[index];
    if (line.utilized > 0 || line.accruedInterest > 0) {
      return { success: false, message: 'Cannot close credit line with active utilization or debt' };
    }

    const updated = [...creditLines];
    updated[index] = { ...line, status: 'Closed' };
    setCreditLines(updated);

    setProtocolState(prev => ({
      ...prev,
      totalExposure: Math.max(0, prev.totalExposure - line.limit)
    }));

    emitEvent('close_credit_line', borrower, { timestamp: Math.floor(Date.now() / 1000) });
    return { success: true, message: `Credit line closed for ${borrower}` };
  };

  // Update Risk Config Parameters
  const updateRiskConfig = (newConfig: RiskFormulaConfig) => {
    setRiskConfig(newConfig);

    // Recalculate interest rates for all active credit lines
    setCreditLines(prev =>
      prev.map(line => {
        const newRate = computeRateFromScore(line.riskScore, newConfig);
        return { ...line, interestRateBps: newRate };
      })
    );

    emitEvent('update_risk_parameters', undefined, newConfig);
    return { success: true, message: 'Risk pricing formula parameters updated across protocol' };
  };

  // Toggle Circuit Breakers
  const togglePauseProtocol = () => {
    setProtocolState(prev => {
      const next = !prev.isPaused;
      emitEvent('pause_protocol', undefined, { isPaused: next });
      return { ...prev, isPaused: next };
    });
  };

  const toggleFreezeDraws = () => {
    setProtocolState(prev => {
      const next = !prev.isDrawsFrozen;
      emitEvent('freeze_draws', undefined, { isDrawsFrozen: next });
      return { ...prev, isDrawsFrozen: next };
    });
  };

  // Withdraw Treasury
  const withdrawTreasury = (amount: number) => {
    if (amount <= 0) return { success: false, message: 'ContractError::ZeroAmountInvalid (118)' };
    if (amount > protocolState.treasuryBalance) {
      return { success: false, message: 'ContractError::TreasuryBalanceInsufficient (125)' };
    }

    setProtocolState(prev => ({
      ...prev,
      treasuryBalance: prev.treasuryBalance - amount
    }));

    emitEvent('withdraw_treasury', undefined, { amountRequested: amount, remainingTreasury: protocolState.treasuryBalance - amount });
    return { success: true, message: `Successfully withdrew $${amount.toLocaleString()} to governance treasury` };
  };

  // Auction Bidding
  const placeAuctionBid = (auctionId: string, bidder: string, amount: number) => {
    const index = auctions.findIndex(a => a.id === auctionId);
    if (index === -1) return { success: false, message: 'ContractError::InvalidAuctionState (115)' };

    const auc = auctions[index];
    if (auc.status !== 'Active') return { success: false, message: 'ContractError::InvalidAuctionState (115)' };
    if (amount <= auc.currentHighBid) {
      return { success: false, message: 'ContractError::BidTooLow (122)' };
    }

    const updated = [...auctions];
    updated[index] = {
      ...auc,
      currentHighBid: amount,
      currentHighBidder: bidder
    };

    setAuctions(updated);
    emitEvent('place_bid', auc.borrower, { auctionId, bidder, bidAmount: amount });
    return { success: true, message: `Placed bid of $${amount.toLocaleString()} on ${auctionId}` };
  };

  // Cross-contract Settlement Handoff
  const settleDefaultLiquidation = (auctionId: string) => {
    const index = auctions.findIndex(a => a.id === auctionId);
    if (index === -1) return { success: false, message: 'ContractError::InvalidAuctionState (115)' };

    const auc = auctions[index];
    if (auc.status === 'Settled' || auc.status === 'Claimed') {
      return { success: false, message: 'ContractError::AlreadySettled (124)' };
    }

    const updatedAuctions = [...auctions];
    updatedAuctions[index] = {
      ...auc,
      status: 'Settled',
      settledAmount: auc.currentHighBid
    };

    setAuctions(updatedAuctions);

    // Apply settlement to defaulted line
    const lineIndex = creditLines.findIndex(c => c.borrower === auc.borrower);
    if (lineIndex !== -1) {
      const line = creditLines[lineIndex];
      const debtToCover = line.utilized + line.accruedInterest;
      const recovered = auc.currentHighBid;
      const writeOff = Math.max(0, debtToCover - recovered);

      const updatedLines = [...creditLines];
      updatedLines[lineIndex] = {
        ...line,
        utilized: 0,
        accruedInterest: 0,
        status: 'Closed'
      };
      setCreditLines(updatedLines);

      setProtocolState(prev => ({
        ...prev,
        totalUtilized: Math.max(0, prev.totalUtilized - line.utilized),
        totalExposure: Math.max(0, prev.totalExposure - line.limit)
      }));

      emitEvent('settle_default_liquidation', auc.borrower, {
        auctionId,
        recoveredDebt: recovered,
        writtenOffDebt: writeOff,
        crossContractHandshakeSuccess: true
      });
    }

    return { success: true, message: `Cross-contract settlement complete for ${auctionId}. Recovered $${auc.currentHighBid.toLocaleString()}` };
  };

  // Time simulation for interest accrual
  const simulateTimePassage = (hours: number) => {
    const secondsPass = hours * 3600;
    setCreditLines(prev =>
      prev.map(line => {
        if (line.status !== 'Active' || line.utilized <= 0) return line;
        const interest = calculateAccruedInterest(line.utilized, line.interestRateBps, secondsPass);
        const newAccrued = line.accruedInterest + interest;
        const totalDebt = line.utilized + newAccrued;
        const newRatio = calculateCollateralRatio(line.collateral, totalDebt);
        return {
          ...line,
          accruedInterest: newAccrued,
          collateralRatioBps: newRatio,
          lastAccrualTimestamp: line.lastAccrualTimestamp + secondsPass
        };
      })
    );
  };

  // Reset simulator
  const resetSimulator = () => {
    setCreditLines(INITIAL_CREDIT_LINES);
    setRiskConfig(INITIAL_RISK_CONFIG);
    setProtocolState(INITIAL_PROTOCOL_STATE);
    setAuctions(INITIAL_AUCTIONS);
    setEvents(INITIAL_EVENTS);
  };

  return (
    <ProtocolContext.Provider
      value={{
        creditLines,
        riskConfig,
        protocolState,
        auctions,
        events,
        openCreditLine,
        drawCredit,
        repayCredit,
        depositCollateral,
        withdrawCollateral,
        selfSuspendCreditLine,
        reinstateCreditLine,
        defaultCreditLine,
        closeCreditLine,
        updateRiskConfig,
        togglePauseProtocol,
        toggleFreezeDraws,
        withdrawTreasury,
        placeAuctionBid,
        settleDefaultLiquidation,
        resetSimulator,
        simulateTimePassage
      }}
    >
      {children}
    </ProtocolContext.Provider>
  );
};

export const useProtocol = () => {
  const context = useContext(ProtocolContext);
  if (!context) throw new Error('useProtocol must be used within a ProtocolProvider');
  return context;
};
