import React, { useState } from 'react';
import { useProtocol } from '../context/ProtocolContext';
import {
  DollarSign,
  TrendingUp,
  CreditCard,
  Building,
  PlusCircle,
  ArrowUpRight,
  ArrowDownLeft,
  AlertCircle
} from 'lucide-react';
import { ResponsiveContainer, PieChart, Pie, Cell, Tooltip } from 'recharts';

export const DashboardView: React.FC = () => {
  const { creditLines, protocolState, openCreditLine, drawCredit, repayCredit } = useProtocol();

  // Modals state
  const [showOpenModal, setShowOpenModal] = useState(false);
  const [showDrawModal, setShowDrawModal] = useState(false);
  const [showRepayModal, setShowRepayModal] = useState(false);
  const [feedback, setFeedback] = useState<{ msg: string; isErr: boolean } | null>(null);

  // Form states
  const [openBorrower, setOpenBorrower] = useState('GNEW...881');
  const [openName, setOpenName] = useState('Stellar Voyager Fund');
  const [openLimit, setOpenLimit] = useState('350000');
  const [openRiskScore, setOpenRiskScore] = useState('180');
  const [openCollateral, setOpenCollateral] = useState('200000');

  const [selectedBorrower, setSelectedBorrower] = useState(creditLines[0]?.borrower || '');
  const [actionAmount, setActionAmount] = useState('10000');

  const activeLines = creditLines.filter(c => c.status === 'Active');
  const avgRate = activeLines.length > 0 
    ? (activeLines.reduce((acc, c) => acc + c.interestRateBps, 0) / activeLines.length / 100).toFixed(2)
    : '0.00';

  const utilizationRatio = ((protocolState.totalUtilized / protocolState.totalExposure) * 100).toFixed(1);

  // Risk Distribution Data for Recharts
  const riskGroups = [
    { name: 'Low Risk (0-200)', count: creditLines.filter(c => c.riskScore <= 200).length, color: '#10b981' },
    { name: 'Medium Risk (201-500)', count: creditLines.filter(c => c.riskScore > 200 && c.riskScore <= 500).length, color: '#06b6d4' },
    { name: 'High Risk (501-750)', count: creditLines.filter(c => c.riskScore > 500 && c.riskScore <= 750).length, color: '#f59e0b' },
    { name: 'Critical (>750)', count: creditLines.filter(c => c.riskScore > 750).length, color: '#ef4444' }
  ];

  const handleOpenSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const res = openCreditLine(
      openBorrower,
      openName,
      parseFloat(openLimit),
      parseInt(openRiskScore),
      parseFloat(openCollateral)
    );
    setFeedback({ msg: res.message, isErr: !res.success });
    if (res.success) setShowOpenModal(false);
  };

  const handleDrawSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const res = drawCredit(selectedBorrower, parseFloat(actionAmount));
    setFeedback({ msg: res.message, isErr: !res.success });
    if (res.success) setShowDrawModal(false);
  };

  const handleRepaySubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const res = repayCredit(selectedBorrower, parseFloat(actionAmount));
    setFeedback({ msg: res.message, isErr: !res.success });
    if (res.success) setShowRepayModal(false);
  };

  return (
    <div className="space-y-6">
      {/* Feedback Banner */}
      {feedback && (
        <div className={`flex items-center justify-between p-4 rounded-xl border text-sm font-medium ${
          feedback.isErr
            ? 'bg-rose-950/60 border-rose-800 text-rose-200'
            : 'bg-emerald-950/60 border-emerald-800 text-emerald-200'
        }`}>
          <div className="flex items-center gap-2">
            <AlertCircle className="h-5 w-5 shrink-0" />
            <span>{feedback.msg}</span>
          </div>
          <button onClick={() => setFeedback(null)} className="text-xs underline cursor-pointer">
            Dismiss
          </button>
        </div>
      )}

      {/* Top Banner & Quick Actions */}
      <div className="flex flex-col lg:flex-row lg:items-center lg:justify-between gap-4 bg-slate-900/60 border border-slate-800 rounded-2xl p-6">
        <div>
          <h2 className="text-xl font-bold text-white tracking-tight">Creditra Protocol Overview</h2>
          <p className="text-sm text-slate-400 mt-1">
            Real-time status of Soroban credit lines, risk scores, and liquidity reserve utilization.
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-3">
          <button
            onClick={() => setShowOpenModal(true)}
            className="flex items-center gap-2 px-4 py-2 rounded-xl bg-cyan-500 hover:bg-cyan-400 text-slate-950 font-semibold text-sm transition cursor-pointer shadow-md shadow-cyan-500/20"
          >
            <PlusCircle className="h-4 w-4" />
            <span>Open Credit Line</span>
          </button>

          <button
            onClick={() => setShowDrawModal(true)}
            className="flex items-center gap-2 px-4 py-2 rounded-xl bg-slate-800 hover:bg-slate-700 text-slate-100 font-medium text-sm border border-slate-700 transition cursor-pointer"
          >
            <ArrowUpRight className="h-4 w-4 text-cyan-400" />
            <span>Draw Credit</span>
          </button>

          <button
            onClick={() => setShowRepayModal(true)}
            className="flex items-center gap-2 px-4 py-2 rounded-xl bg-slate-800 hover:bg-slate-700 text-slate-100 font-medium text-sm border border-slate-700 transition cursor-pointer"
          >
            <ArrowDownLeft className="h-4 w-4 text-emerald-400" />
            <span>Repay Debt</span>
          </button>
        </div>
      </div>

      {/* KPI Cards */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
        <div className="bg-slate-900/50 border border-slate-800/80 rounded-2xl p-5 space-y-3">
          <div className="flex items-center justify-between text-slate-400 text-xs font-medium">
            <span>Total Exposure</span>
            <Building className="h-4 w-4 text-cyan-400" />
          </div>
          <div className="text-2xl font-bold text-white font-mono">
            ${protocolState.totalExposure.toLocaleString()}
          </div>
          <div className="text-xs text-slate-400">
            Cap: <span className="font-mono text-slate-200">${protocolState.maxTotalExposure.toLocaleString()}</span>
          </div>
        </div>

        <div className="bg-slate-900/50 border border-slate-800/80 rounded-2xl p-5 space-y-3">
          <div className="flex items-center justify-between text-slate-400 text-xs font-medium">
            <span>Total Utilized</span>
            <DollarSign className="h-4 w-4 text-emerald-400" />
          </div>
          <div className="text-2xl font-bold text-emerald-400 font-mono">
            ${protocolState.totalUtilized.toLocaleString()}
          </div>
          <div className="text-xs text-slate-400">
            Utilization Rate: <span className="font-mono text-emerald-300 font-medium">{utilizationRatio}%</span>
          </div>
        </div>

        <div className="bg-slate-900/50 border border-slate-800/80 rounded-2xl p-5 space-y-3">
          <div className="flex items-center justify-between text-slate-400 text-xs font-medium">
            <span>Avg Risk Interest Rate</span>
            <TrendingUp className="h-4 w-4 text-amber-400" />
          </div>
          <div className="text-2xl font-bold text-amber-300 font-mono">
            {avgRate}%
          </div>
          <div className="text-xs text-slate-400">
            Formula: <span className="font-mono text-slate-300">r(k) = clamp(b + k·s)</span>
          </div>
        </div>

        <div className="bg-slate-900/50 border border-slate-800/80 rounded-2xl p-5 space-y-3">
          <div className="flex items-center justify-between text-slate-400 text-xs font-medium">
            <span>Treasury Reserve</span>
            <CreditCard className="h-4 w-4 text-purple-400" />
          </div>
          <div className="text-2xl font-bold text-purple-300 font-mono">
            ${protocolState.treasuryBalance.toLocaleString()}
          </div>
          <div className="text-xs text-slate-400">
            Protocol Fee: <span className="font-mono text-slate-300">{protocolState.protocolFeeBps / 100}% on interest</span>
          </div>
        </div>
      </div>

      {/* Exposure Progress & Chart */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        {/* Utilization Bar */}
        <div className="lg:col-span-2 bg-slate-900/50 border border-slate-800/80 rounded-2xl p-6 space-y-5">
          <div className="flex items-center justify-between">
            <div>
              <h3 className="text-base font-semibold text-white">Pool Utilization & Capacity</h3>
              <p className="text-xs text-slate-400">Current draws against configured utilization cap ({protocolState.utilizationCapBps / 100}%)</p>
            </div>
            <span className="text-xs font-mono font-medium px-2.5 py-1 rounded-md bg-slate-800 text-cyan-300 border border-slate-700">
              {utilizationRatio}% Utilized
            </span>
          </div>

          <div className="space-y-2">
            <div className="flex justify-between text-xs font-mono text-slate-400">
              <span>$0</span>
              <span>Utilized: ${protocolState.totalUtilized.toLocaleString()}</span>
              <span>Max Cap: ${((protocolState.totalExposure * protocolState.utilizationCapBps) / 10000).toLocaleString()}</span>
            </div>
            <div className="h-4 w-full bg-slate-800 rounded-full overflow-hidden p-0.5 border border-slate-700/50">
              <div
                className="h-full bg-gradient-to-r from-cyan-500 to-emerald-400 rounded-full transition-all duration-500"
                style={{ width: `${Math.min(100, (protocolState.totalUtilized / protocolState.totalExposure) * 100)}%` }}
              />
            </div>
          </div>

          <div className="grid grid-cols-3 gap-4 pt-2 border-t border-slate-800/60 text-xs">
            <div>
              <span className="text-slate-500 block">Active Credit Lines</span>
              <span className="text-sm font-semibold text-slate-200 font-mono">{activeLines.length}</span>
            </div>
            <div>
              <span className="text-slate-500 block">Min Collateral Floor</span>
              <span className="text-sm font-semibold text-slate-200 font-mono">{protocolState.minCollateralRatioBps / 100}%</span>
            </div>
            <div>
              <span className="text-slate-500 block">Draw Cooldown</span>
              <span className="text-sm font-semibold text-slate-200 font-mono">{protocolState.drawMinIntervalSeconds}s</span>
            </div>
          </div>
        </div>

        {/* Risk Distribution Chart */}
        <div className="bg-slate-900/50 border border-slate-800/80 rounded-2xl p-6 flex flex-col justify-between">
          <div>
            <h3 className="text-base font-semibold text-white">Risk Profile Distribution</h3>
            <p className="text-xs text-slate-400">Borrower risk scores across active credit lines</p>
          </div>

          <div className="h-44 w-full my-2">
            <ResponsiveContainer width="100%" height="100%">
              <PieChart>
                <Pie
                  data={riskGroups}
                  dataKey="count"
                  nameKey="name"
                  innerRadius={45}
                  outerRadius={70}
                  paddingAngle={4}
                >
                  {riskGroups.map((entry, index) => (
                    <Cell key={`cell-${index}`} fill={entry.color} />
                  ))}
                </Pie>
                <Tooltip
                  contentStyle={{ backgroundColor: '#0f172a', borderColor: '#334155', borderRadius: '0.75rem' }}
                  itemStyle={{ color: '#f8fafc', fontSize: '12px' }}
                />
              </PieChart>
            </ResponsiveContainer>
          </div>

          <div className="grid grid-cols-2 gap-2 text-[11px]">
            {riskGroups.map(group => (
              <div key={group.name} className="flex items-center gap-1.5 text-slate-300">
                <span className="h-2 w-2 rounded-full shrink-0" style={{ backgroundColor: group.color }} />
                <span className="truncate">{group.name} ({group.count})</span>
              </div>
            ))}
          </div>
        </div>
      </div>

      {/* Credit Lines Quick Summary Table */}
      <div className="bg-slate-900/50 border border-slate-800/80 rounded-2xl p-6 space-y-4">
        <div className="flex items-center justify-between">
          <h3 className="text-base font-semibold text-white">Borrower Portfolio Summary</h3>
          <span className="text-xs text-slate-400">{creditLines.length} Total Lines Registered</span>
        </div>

        <div className="overflow-x-auto">
          <table className="w-full text-left text-xs">
            <thead>
              <tr className="border-b border-slate-800 text-slate-400 uppercase tracking-wider font-semibold">
                <th className="pb-3 px-3">Borrower / Identity</th>
                <th className="pb-3 px-3">Status</th>
                <th className="pb-3 px-3">Limit</th>
                <th className="pb-3 px-3">Utilized</th>
                <th className="pb-3 px-3">Interest Rate</th>
                <th className="pb-3 px-3">Risk Score</th>
                <th className="pb-3 px-3">Collateral</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-slate-800/50">
              {creditLines.map(line => (
                <tr key={line.borrower} className="hover:bg-slate-800/30 transition">
                  <td className="py-3 px-3">
                    <div className="font-semibold text-slate-200">{line.borrowerName}</div>
                    <div className="font-mono text-slate-500 text-[11px]">{line.borrower}</div>
                  </td>
                  <td className="py-3 px-3">
                    <span className={`px-2.5 py-1 rounded-full text-[11px] font-medium border ${
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
                  </td>
                  <td className="py-3 px-3 font-mono font-medium text-slate-200">
                    ${line.limit.toLocaleString()}
                  </td>
                  <td className="py-3 px-3 font-mono text-emerald-400">
                    ${line.utilized.toLocaleString()}
                  </td>
                  <td className="py-3 px-3 font-mono text-amber-300">
                    {(line.interestRateBps / 100).toFixed(2)}%
                  </td>
                  <td className="py-3 px-3 font-mono text-slate-300">
                    {line.riskScore} / 1000
                  </td>
                  <td className="py-3 px-3 font-mono text-slate-300">
                    ${line.collateral.toLocaleString()}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>

      {/* Open Credit Line Modal */}
      {showOpenModal && (
        <div className="fixed inset-0 z-50 bg-slate-950/80 backdrop-blur-sm flex items-center justify-center p-4">
          <div className="bg-slate-900 border border-slate-800 rounded-2xl w-full max-w-md p-6 space-y-4">
            <h3 className="text-lg font-bold text-white">Open New Soroban Credit Line</h3>
            <form onSubmit={handleOpenSubmit} className="space-y-3 text-xs">
              <div>
                <label className="block text-slate-400 mb-1">Borrower Address (Public Key)</label>
                <input
                  type="text"
                  value={openBorrower}
                  onChange={e => setOpenBorrower(e.target.value)}
                  className="w-full bg-slate-950 border border-slate-800 rounded-lg px-3 py-2 text-slate-200 font-mono"
                  required
                />
              </div>
              <div>
                <label className="block text-slate-400 mb-1">Borrower Name / Alias</label>
                <input
                  type="text"
                  value={openName}
                  onChange={e => setOpenName(e.target.value)}
                  className="w-full bg-slate-950 border border-slate-800 rounded-lg px-3 py-2 text-slate-200"
                  required
                />
              </div>
              <div className="grid grid-cols-2 gap-3">
                <div>
                  <label className="block text-slate-400 mb-1">Credit Limit ($)</label>
                  <input
                    type="number"
                    value={openLimit}
                    onChange={e => setOpenLimit(e.target.value)}
                    className="w-full bg-slate-950 border border-slate-800 rounded-lg px-3 py-2 text-slate-200 font-mono"
                    required
                  />
                </div>
                <div>
                  <label className="block text-slate-400 mb-1">Risk Score (0..1000)</label>
                  <input
                    type="number"
                    min="0"
                    max="1000"
                    value={openRiskScore}
                    onChange={e => setOpenRiskScore(e.target.value)}
                    className="w-full bg-slate-950 border border-slate-800 rounded-lg px-3 py-2 text-slate-200 font-mono"
                    required
                  />
                </div>
              </div>
              <div>
                <label className="block text-slate-400 mb-1">Initial Collateral Floor ($)</label>
                <input
                  type="number"
                  value={openCollateral}
                  onChange={e => setOpenCollateral(e.target.value)}
                  className="w-full bg-slate-950 border border-slate-800 rounded-lg px-3 py-2 text-slate-200 font-mono"
                  required
                />
              </div>

              <div className="flex justify-end gap-2 pt-3">
                <button
                  type="button"
                  onClick={() => setShowOpenModal(false)}
                  className="px-4 py-2 rounded-xl bg-slate-800 hover:bg-slate-700 text-slate-300 cursor-pointer"
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  className="px-4 py-2 rounded-xl bg-cyan-500 hover:bg-cyan-400 text-slate-950 font-semibold cursor-pointer"
                >
                  Confirm Open
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* Draw Credit Modal */}
      {showDrawModal && (
        <div className="fixed inset-0 z-50 bg-slate-950/80 backdrop-blur-sm flex items-center justify-center p-4">
          <div className="bg-slate-900 border border-slate-800 rounded-2xl w-full max-w-md p-6 space-y-4">
            <h3 className="text-lg font-bold text-white">Draw Credit Liquidity</h3>
            <form onSubmit={handleDrawSubmit} className="space-y-3 text-xs">
              <div>
                <label className="block text-slate-400 mb-1">Select Borrower</label>
                <select
                  value={selectedBorrower}
                  onChange={e => setSelectedBorrower(e.target.value)}
                  className="w-full bg-slate-950 border border-slate-800 rounded-lg px-3 py-2 text-slate-200"
                >
                  {creditLines.map(c => (
                    <option key={c.borrower} value={c.borrower}>
                      {c.borrowerName} (Avail: ${(c.limit - c.utilized).toLocaleString()})
                    </option>
                  ))}
                </select>
              </div>
              <div>
                <label className="block text-slate-400 mb-1">Draw Amount ($)</label>
                <input
                  type="number"
                  value={actionAmount}
                  onChange={e => setActionAmount(e.target.value)}
                  className="w-full bg-slate-950 border border-slate-800 rounded-lg px-3 py-2 text-slate-200 font-mono"
                  required
                />
              </div>

              <div className="flex justify-end gap-2 pt-3">
                <button
                  type="button"
                  onClick={() => setShowDrawModal(false)}
                  className="px-4 py-2 rounded-xl bg-slate-800 hover:bg-slate-700 text-slate-300 cursor-pointer"
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  className="px-4 py-2 rounded-xl bg-cyan-500 hover:bg-cyan-400 text-slate-950 font-semibold cursor-pointer"
                >
                  Confirm Draw
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* Repay Debt Modal */}
      {showRepayModal && (
        <div className="fixed inset-0 z-50 bg-slate-950/80 backdrop-blur-sm flex items-center justify-center p-4">
          <div className="bg-slate-900 border border-slate-800 rounded-2xl w-full max-w-md p-6 space-y-4">
            <h3 className="text-lg font-bold text-white">Repay Debt & Accrued Interest</h3>
            <form onSubmit={handleRepaySubmit} className="space-y-3 text-xs">
              <div>
                <label className="block text-slate-400 mb-1">Select Borrower</label>
                <select
                  value={selectedBorrower}
                  onChange={e => setSelectedBorrower(e.target.value)}
                  className="w-full bg-slate-950 border border-slate-800 rounded-lg px-3 py-2 text-slate-200"
                >
                  {creditLines.map(c => (
                    <option key={c.borrower} value={c.borrower}>
                      {c.borrowerName} (Debt: ${(c.utilized + c.accruedInterest).toLocaleString()})
                    </option>
                  ))}
                </select>
              </div>
              <div>
                <label className="block text-slate-400 mb-1">Repayment Amount ($)</label>
                <input
                  type="number"
                  value={actionAmount}
                  onChange={e => setActionAmount(e.target.value)}
                  className="w-full bg-slate-950 border border-slate-800 rounded-lg px-3 py-2 text-slate-200 font-mono"
                  required
                />
              </div>

              <div className="flex justify-end gap-2 pt-3">
                <button
                  type="button"
                  onClick={() => setShowRepayModal(false)}
                  className="px-4 py-2 rounded-xl bg-slate-800 hover:bg-slate-700 text-slate-300 cursor-pointer"
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  className="px-4 py-2 rounded-xl bg-emerald-500 hover:bg-emerald-400 text-slate-950 font-semibold cursor-pointer"
                >
                  Confirm Repay
                </button>
              </div>
            </form>
          </div>
        </div>
      )}
    </div>
  );
};
