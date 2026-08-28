import React, { useState } from 'react';
import { useProtocol } from '../context/ProtocolContext';
import { CreditLine, CreditStatus } from '../types';
import {
  CreditCard,
  Search,
  ArrowUpRight,
  ArrowDownLeft,
  Shield,
  AlertTriangle,
  Play,
  CheckCircle2,
  Lock,
  Coins
} from 'lucide-react';

export const CreditLinesView: React.FC = () => {
  const {
    creditLines,
    drawCredit,
    repayCredit,
    depositCollateral,
    withdrawCollateral,
    selfSuspendCreditLine,
    reinstateCreditLine,
    defaultCreditLine,
    closeCreditLine
  } = useProtocol();

  const [searchTerm, setSearchTerm] = useState('');
  const [statusFilter, setStatusFilter] = useState<string>('All');
  const [selectedBorrower, setSelectedBorrower] = useState<CreditLine | null>(creditLines[0] || null);

  const [actionType, setActionType] = useState<'draw' | 'repay' | 'deposit_col' | 'withdraw_col' | null>(null);
  const [inputAmount, setInputAmount] = useState('50000');
  const [feedback, setFeedback] = useState<{ msg: string; isErr: boolean } | null>(null);

  const filteredLines = creditLines.filter(line => {
    const matchesSearch =
      line.borrower.toLowerCase().includes(searchTerm.toLowerCase()) ||
      line.borrowerName.toLowerCase().includes(searchTerm.toLowerCase());
    const matchesStatus = statusFilter === 'All' || line.status === statusFilter;
    return matchesSearch && matchesStatus;
  });

  const handleAction = (e: React.FormEvent) => {
    e.preventDefault();
    if (!selectedBorrower) return;
    const amount = parseFloat(inputAmount);
    let res = { success: false, message: '' };

    if (actionType === 'draw') res = drawCredit(selectedBorrower.borrower, amount);
    else if (actionType === 'repay') res = repayCredit(selectedBorrower.borrower, amount);
    else if (actionType === 'deposit_col') res = depositCollateral(selectedBorrower.borrower, amount);
    else if (actionType === 'withdraw_col') res = withdrawCollateral(selectedBorrower.borrower, amount);

    setFeedback({ msg: res.message, isErr: !res.success });
    if (res.success) setActionType(null);
  };

  return (
    <div className="space-y-6">
      {/* Feedback Alert */}
      {feedback && (
        <div className={`p-4 rounded-xl border text-sm font-medium flex items-center justify-between ${
          feedback.isErr ? 'bg-rose-950/60 border-rose-800 text-rose-200' : 'bg-emerald-950/60 border-emerald-800 text-emerald-200'
        }`}>
          <span>{feedback.msg}</span>
          <button onClick={() => setFeedback(null)} className="text-xs underline cursor-pointer">Dismiss</button>
        </div>
      )}

      {/* Header Controls */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        <div>
          <h2 className="text-xl font-bold text-white tracking-tight">Credit Line Registry</h2>
          <p className="text-xs text-slate-400">Manage borrower accounts, draws, collateral ratios, and lifecycle states</p>
        </div>

        <div className="flex items-center gap-3">
          <div className="relative">
            <Search className="h-4 w-4 absolute left-3 top-2.5 text-slate-500" />
            <input
              type="text"
              placeholder="Search borrower or address..."
              value={searchTerm}
              onChange={e => setSearchTerm(e.target.value)}
              className="bg-slate-900 border border-slate-800 rounded-xl pl-9 pr-4 py-2 text-xs text-slate-200 focus:outline-none focus:border-cyan-500 w-64"
            />
          </div>

          <select
            value={statusFilter}
            onChange={e => setStatusFilter(e.target.value)}
            className="bg-slate-900 border border-slate-800 rounded-xl px-3 py-2 text-xs text-slate-200 focus:outline-none focus:border-cyan-500"
          >
            <option value="All">All Statuses</option>
            <option value="Active">Active</option>
            <option value="Suspended">Suspended</option>
            <option value="Defaulted">Defaulted</option>
            <option value="Closed">Closed</option>
          </select>
        </div>
      </div>

      {/* Main Grid: List & Detail Drawer */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        {/* Credit Lines List */}
        <div className="lg:col-span-2 space-y-3">
          {filteredLines.map(line => {
            const isSelected = selectedBorrower?.borrower === line.borrower;
            const available = line.limit - line.utilized;
            const colRatioPct = line.collateralRatioBps > 0 ? (line.collateralRatioBps / 100).toFixed(1) : 'N/A';

            return (
              <div
                key={line.borrower}
                onClick={() => setSelectedBorrower(line)}
                className={`p-4 rounded-2xl border cursor-pointer transition ${
                  isSelected
                    ? 'bg-slate-900 border-cyan-500/60 shadow-lg shadow-cyan-950/30'
                    : 'bg-slate-900/40 border-slate-800/80 hover:bg-slate-900/80 hover:border-slate-700'
                }`}
              >
                <div className="flex items-center justify-between mb-3">
                  <div className="flex items-center gap-2.5">
                    <div className="p-2 rounded-xl bg-slate-800 border border-slate-700 text-cyan-400">
                      <CreditCard className="h-4 w-4" />
                    </div>
                    <div>
                      <h4 className="text-sm font-semibold text-white">{line.borrowerName}</h4>
                      <p className="text-[11px] font-mono text-slate-500">{line.borrower}</p>
                    </div>
                  </div>

                  <span className={`px-2.5 py-0.5 rounded-full text-xs font-medium border ${
                    line.status === 'Active'
                      ? 'bg-emerald-950/60 border-emerald-800 text-emerald-300'
                      : line.status === 'Suspended'
                      ? 'bg-amber-950/60 border-amber-800 text-amber-300'
                      : line.status === 'Defaulted'
                      ? 'bg-rose-950/60 border-rose-800 text-rose-300'
                      : 'bg-slate-800 border-slate-700 text-slate-400'
                  }`}>
                    {line.status}
                  </span>
                </div>

                <div className="grid grid-cols-4 gap-2 pt-2 border-t border-slate-800/50 text-xs font-mono">
                  <div>
                    <span className="text-[10px] text-slate-500 block font-sans">Utilized / Limit</span>
                    <span className="text-slate-200 font-medium">
                      ${line.utilized.toLocaleString()} / <span className="text-slate-400">${line.limit.toLocaleString()}</span>
                    </span>
                  </div>
                  <div>
                    <span className="text-[10px] text-slate-500 block font-sans">Interest Rate</span>
                    <span className="text-amber-300 font-medium">{(line.interestRateBps / 100).toFixed(2)}%</span>
                  </div>
                  <div>
                    <span className="text-[10px] text-slate-500 block font-sans">Accrued Interest</span>
                    <span className="text-rose-300 font-medium">${line.accruedInterest.toLocaleString()}</span>
                  </div>
                  <div>
                    <span className="text-[10px] text-slate-500 block font-sans">Collateral Ratio</span>
                    <span className="text-cyan-300 font-medium">{colRatioPct}%</span>
                  </div>
                </div>
              </div>
            );
          })}
        </div>

        {/* Selected Borrower Inspector & Actions */}
        {selectedBorrower && (
          <div className="bg-slate-900 border border-slate-800 rounded-2xl p-6 space-y-6 h-fit sticky top-20">
            <div>
              <div className="flex items-center justify-between">
                <h3 className="text-lg font-bold text-white">{selectedBorrower.borrowerName}</h3>
                <span className="text-xs font-mono text-cyan-400">Risk Score: {selectedBorrower.riskScore}</span>
              </div>
              <p className="text-xs font-mono text-slate-500 mt-0.5">{selectedBorrower.borrower}</p>
            </div>

            {/* Quick Action Buttons */}
            <div className="grid grid-cols-2 gap-2">
              <button
                onClick={() => setActionType('draw')}
                disabled={selectedBorrower.status !== 'Active'}
                className="flex items-center justify-center gap-1.5 p-2.5 rounded-xl bg-cyan-500/10 border border-cyan-500/30 text-cyan-300 hover:bg-cyan-500/20 text-xs font-medium transition cursor-pointer disabled:opacity-40"
              >
                <ArrowUpRight className="h-4 w-4" />
                <span>Draw Liquidity</span>
              </button>

              <button
                onClick={() => setActionType('repay')}
                className="flex items-center justify-center gap-1.5 p-2.5 rounded-xl bg-emerald-500/10 border border-emerald-500/30 text-emerald-300 hover:bg-emerald-500/20 text-xs font-medium transition cursor-pointer"
              >
                <ArrowDownLeft className="h-4 w-4" />
                <span>Repay Debt</span>
              </button>

              <button
                onClick={() => setActionType('deposit_col')}
                className="flex items-center justify-center gap-1.5 p-2.5 rounded-xl bg-slate-800 border border-slate-700 text-slate-200 hover:bg-slate-700 text-xs font-medium transition cursor-pointer"
              >
                <Coins className="h-4 w-4 text-amber-400" />
                <span>Deposit Col.</span>
              </button>

              <button
                onClick={() => setActionType('withdraw_col')}
                className="flex items-center justify-center gap-1.5 p-2.5 rounded-xl bg-slate-800 border border-slate-700 text-slate-200 hover:bg-slate-700 text-xs font-medium transition cursor-pointer"
              >
                <Coins className="h-4 w-4 text-purple-400" />
                <span>Withdraw Col.</span>
              </button>
            </div>

            {/* Inline Action Form */}
            {actionType && (
              <form onSubmit={handleAction} className="bg-slate-950 border border-slate-800 p-4 rounded-xl space-y-3 text-xs">
                <div className="flex justify-between items-center text-slate-300 font-semibold uppercase tracking-wider text-[11px]">
                  <span>Action: {actionType.replace('_', ' ')}</span>
                  <button type="button" onClick={() => setActionType(null)} className="text-slate-500 hover:text-slate-300">✕</button>
                </div>
                <div>
                  <label className="block text-slate-400 mb-1">Amount ($)</label>
                  <input
                    type="number"
                    value={inputAmount}
                    onChange={e => setInputAmount(e.target.value)}
                    className="w-full bg-slate-900 border border-slate-700 rounded-lg px-3 py-2 text-slate-100 font-mono"
                    required
                  />
                </div>
                <button
                  type="submit"
                  className="w-full py-2 rounded-lg bg-cyan-500 hover:bg-cyan-400 text-slate-950 font-bold transition cursor-pointer"
                >
                  Execute Operation
                </button>
              </form>
            )}

            {/* Lifecycle State Controls */}
            <div className="border-t border-slate-800 pt-4 space-y-3">
              <h4 className="text-xs font-semibold text-slate-400 uppercase tracking-wider">Soroban Lifecycle Controls</h4>

              <div className="flex flex-col gap-2 text-xs">
                {selectedBorrower.status === 'Active' && (
                  <button
                    onClick={() => {
                      const res = selfSuspendCreditLine(selectedBorrower.borrower);
                      setFeedback({ msg: res.message, isErr: !res.success });
                    }}
                    className="w-full flex items-center justify-between p-2.5 rounded-xl border border-amber-800/60 bg-amber-950/30 text-amber-300 hover:bg-amber-950/60 transition cursor-pointer"
                  >
                    <div className="flex items-center gap-2">
                      <Lock className="h-4 w-4" />
                      <span>Self-Suspend Line</span>
                    </div>
                    <span className="text-[10px] text-amber-400/80">Borrower callable</span>
                  </button>
                )}

                {selectedBorrower.status === 'Suspended' && (
                  <button
                    onClick={() => {
                      const res = reinstateCreditLine(selectedBorrower.borrower);
                      setFeedback({ msg: res.message, isErr: !res.success });
                    }}
                    className="w-full flex items-center justify-between p-2.5 rounded-xl border border-emerald-800/60 bg-emerald-950/30 text-emerald-300 hover:bg-emerald-950/60 transition cursor-pointer"
                  >
                    <div className="flex items-center gap-2">
                      <Play className="h-4 w-4" />
                      <span>Reinstate Active State</span>
                    </div>
                    <span className="text-[10px] text-emerald-400/80">Re-enable draws</span>
                  </button>
                )}

                {selectedBorrower.status !== 'Defaulted' && selectedBorrower.status !== 'Closed' && (
                  <button
                    onClick={() => {
                      const res = defaultCreditLine(selectedBorrower.borrower);
                      setFeedback({ msg: res.message, isErr: !res.success });
                    }}
                    className="w-full flex items-center justify-between p-2.5 rounded-xl border border-rose-800/60 bg-rose-950/30 text-rose-300 hover:bg-rose-950/60 transition cursor-pointer"
                  >
                    <div className="flex items-center gap-2">
                      <AlertTriangle className="h-4 w-4" />
                      <span>Declare Default & Auction</span>
                    </div>
                    <span className="text-[10px] text-rose-400/80">Cross-contract handoff</span>
                  </button>
                )}

                {selectedBorrower.utilized === 0 && selectedBorrower.accruedInterest === 0 && selectedBorrower.status !== 'Closed' && (
                  <button
                    onClick={() => {
                      const res = closeCreditLine(selectedBorrower.borrower);
                      setFeedback({ msg: res.message, isErr: !res.success });
                    }}
                    className="w-full flex items-center justify-between p-2.5 rounded-xl border border-slate-700 bg-slate-800/50 text-slate-300 hover:bg-slate-800 transition cursor-pointer"
                  >
                    <div className="flex items-center gap-2">
                      <CheckCircle2 className="h-4 w-4 text-emerald-400" />
                      <span>Close Credit Line</span>
                    </div>
                    <span className="text-[10px] text-slate-400">Zero debt required</span>
                  </button>
                )}
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
};
