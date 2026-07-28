import React, { useState } from 'react';
import { useProtocol } from '../context/ProtocolContext';
import { Shield, ShieldAlert, Lock, Unlock, DollarSign, Ban, Building, CheckCircle2 } from 'lucide-react';

export const ProtocolAdminView: React.FC = () => {
  const { protocolState, togglePauseProtocol, toggleFreezeDraws, withdrawTreasury } = useProtocol();

  const [withdrawAmount, setWithdrawAmount] = useState('5000');
  const [blockedAddress, setBlockedAddress] = useState('');
  const [feedback, setFeedback] = useState<{ msg: string; isErr: boolean } | null>(null);

  const handleWithdraw = (e: React.FormEvent) => {
    e.preventDefault();
    const res = withdrawTreasury(parseFloat(withdrawAmount));
    setFeedback({ msg: res.message, isErr: !res.success });
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

      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 bg-slate-900/60 border border-slate-800 rounded-2xl p-6">
        <div>
          <div className="flex items-center gap-2">
            <h2 className="text-xl font-bold text-white tracking-tight">Protocol Circuit Breakers & Governance</h2>
            <span className="px-2.5 py-0.5 rounded-md bg-rose-950/80 border border-rose-800 text-rose-300 text-xs font-mono">
              Admin & Multisig
            </span>
          </div>
          <p className="text-xs text-slate-400 mt-1">
            Emergency toggles, global limits, treasury fee reserves, and security filters.
          </p>
        </div>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
        {/* Emergency Circuit Breakers */}
        <div className="bg-slate-900/50 border border-slate-800/80 rounded-2xl p-6 space-y-5">
          <div className="flex items-center gap-2 pb-3 border-b border-slate-800">
            <Shield className="h-5 w-5 text-rose-400" />
            <h3 className="text-base font-semibold text-white">Emergency Protocol Toggles</h3>
          </div>

          <div className="space-y-4">
            {/* Pause Protocol Toggle */}
            <div className="flex items-center justify-between p-4 rounded-xl bg-slate-950 border border-slate-800">
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-semibold text-white">Pause Protocol</span>
                  {protocolState.isPaused && (
                    <span className="px-2 py-0.5 rounded text-[10px] font-mono bg-rose-950 text-rose-300 border border-rose-800">
                      PAUSED
                    </span>
                  )}
                </div>
                <p className="text-xs text-slate-400">
                  Halts all protocol actions except debt repayment (<code className="text-cyan-400">repay_credit</code>).
                </p>
              </div>

              <button
                onClick={togglePauseProtocol}
                className={`flex items-center gap-1.5 px-4 py-2 rounded-xl text-xs font-bold transition cursor-pointer ${
                  protocolState.isPaused
                    ? 'bg-emerald-500 hover:bg-emerald-400 text-slate-950'
                    : 'bg-rose-600 hover:bg-rose-500 text-white'
                }`}
              >
                {protocolState.isPaused ? (
                  <>
                    <CheckCircle2 className="h-4 w-4" />
                    <span>Unpause</span>
                  </>
                ) : (
                  <>
                    <ShieldAlert className="h-4 w-4" />
                    <span>Pause</span>
                  </>
                )}
              </button>
            </div>

            {/* Freeze Draws Toggle */}
            <div className="flex items-center justify-between p-4 rounded-xl bg-slate-950 border border-slate-800">
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-semibold text-white">Freeze Draws</span>
                  {protocolState.isDrawsFrozen && (
                    <span className="px-2 py-0.5 rounded text-[10px] font-mono bg-amber-950 text-amber-300 border border-amber-800">
                      FROZEN
                    </span>
                  )}
                </div>
                <p className="text-xs text-slate-400">
                  Blocks new draw requests while keeping collateral deposits & repayments enabled.
                </p>
              </div>

              <button
                onClick={toggleFreezeDraws}
                className={`flex items-center gap-1.5 px-4 py-2 rounded-xl text-xs font-bold transition cursor-pointer ${
                  protocolState.isDrawsFrozen
                    ? 'bg-emerald-500 hover:bg-emerald-400 text-slate-950'
                    : 'bg-amber-600 hover:bg-amber-500 text-white'
                }`}
              >
                {protocolState.isDrawsFrozen ? (
                  <>
                    <Unlock className="h-4 w-4" />
                    <span>Unfreeze</span>
                  </>
                ) : (
                  <>
                    <Lock className="h-4 w-4" />
                    <span>Freeze</span>
                  </>
                )}
              </button>
            </div>
          </div>
        </div>

        {/* Treasury Fee Management */}
        <div className="bg-slate-900/50 border border-slate-800/80 rounded-2xl p-6 space-y-5">
          <div className="flex items-center gap-2 pb-3 border-b border-slate-800">
            <DollarSign className="h-5 w-5 text-purple-400" />
            <h3 className="text-base font-semibold text-white">Treasury & Fee Reserves</h3>
          </div>

          <div className="bg-slate-950 border border-slate-800 p-4 rounded-xl space-y-2">
            <div className="flex justify-between items-center text-xs">
              <span className="text-slate-400">Protocol Fee Rate:</span>
              <span className="font-mono text-slate-200">{protocolState.protocolFeeBps / 100}% on accrued interest</span>
            </div>
            <div className="flex justify-between items-center text-xs pt-1 border-t border-slate-800 font-mono">
              <span className="text-slate-400">Accrued Treasury Reserve:</span>
              <span className="text-base font-bold text-purple-300">${protocolState.treasuryBalance.toLocaleString()}</span>
            </div>
          </div>

          <form onSubmit={handleWithdraw} className="space-y-3 text-xs">
            <div>
              <label className="block text-slate-400 mb-1">Withdrawal Amount ($)</label>
              <input
                type="number"
                value={withdrawAmount}
                onChange={e => setWithdrawAmount(e.target.value)}
                className="w-full bg-slate-950 border border-slate-800 rounded-lg px-3 py-2 text-slate-200 font-mono"
                required
              />
            </div>

            <button
              type="submit"
              className="w-full py-2.5 rounded-xl bg-purple-600 hover:bg-purple-500 text-white font-bold text-xs transition cursor-pointer"
            >
              Withdraw to Treasury Address (withdraw_treasury)
            </button>
          </form>
        </div>
      </div>
    </div>
  );
};
