import React, { useState } from 'react';
import { useProtocol } from '../context/ProtocolContext';
import { RiskFormulaConfig } from '../types';
import { computeRateFromScore } from '../services/creditraSimulator';
import { Sliders, Save, CheckCircle2, Calculator, Info } from 'lucide-react';
import { ResponsiveContainer, LineChart, Line, XAxis, YAxis, Tooltip, CartesianGrid } from 'recharts';

export const RiskPricingLab: React.FC = () => {
  const { riskConfig, updateRiskConfig, creditLines } = useProtocol();

  const [localConfig, setLocalConfig] = useState<RiskFormulaConfig>(riskConfig);
  const [testScore, setTestScore] = useState<number>(350);
  const [feedback, setFeedback] = useState<string | null>(null);

  // Generate dataset for $r(k)$ curve (scores from 0 to 1000 in steps of 50)
  const curveData = [];
  for (let score = 0; score <= 1000; score += 50) {
    const rateBps = computeRateFromScore(score, localConfig);
    curveData.push({
      score,
      ratePct: (rateBps / 100).toFixed(2),
      rawRatePct: ((localConfig.baseRateBps + score * localConfig.riskSensitivityBps) / 100).toFixed(2)
    });
  }

  const calculatedTestRate = (computeRateFromScore(testScore, localConfig) / 100).toFixed(2);

  const handleApply = () => {
    const res = updateRiskConfig(localConfig);
    setFeedback(res.message);
    setTimeout(() => setFeedback(null), 4000);
  };

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 bg-slate-900/60 border border-slate-800 rounded-2xl p-6">
        <div>
          <div className="flex items-center gap-2">
            <h2 className="text-xl font-bold text-white tracking-tight">Risk-Pricing Formula Lab</h2>
            <span className="px-2.5 py-0.5 rounded-md bg-amber-950/80 border border-amber-800 text-amber-300 text-xs font-mono">
              contracts/credit/src/risk.rs
            </span>
          </div>
          <p className="text-xs text-slate-400 mt-1">
            Simulate and calibrate the continuous interest rate function <code className="text-cyan-300 font-mono">r(k) = clamp(b + k · s, r_min, r_max)</code>
          </p>
        </div>

        <button
          onClick={handleApply}
          className="flex items-center gap-2 px-4 py-2.5 rounded-xl bg-cyan-500 hover:bg-cyan-400 text-slate-950 font-bold text-xs transition cursor-pointer shadow-lg shadow-cyan-500/20 shrink-0"
        >
          <Save className="h-4 w-4" />
          <span>Apply to Protocol State</span>
        </button>
      </div>

      {feedback && (
        <div className="p-4 rounded-xl bg-emerald-950/60 border border-emerald-800 text-emerald-200 text-xs font-medium flex items-center gap-2">
          <CheckCircle2 className="h-4 w-4 text-emerald-400" />
          <span>{feedback}</span>
        </div>
      )}

      {/* Main Grid: Controls & Curve Chart */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        {/* Parameter Sliders & Formula Controls */}
        <div className="bg-slate-900/50 border border-slate-800/80 rounded-2xl p-6 space-y-6">
          <div className="flex items-center gap-2 pb-3 border-b border-slate-800">
            <Sliders className="h-4 w-4 text-cyan-400" />
            <h3 className="text-base font-semibold text-white">Rate Formula Parameters</h3>
          </div>

          <div className="space-y-4 text-xs">
            {/* Base Rate b */}
            <div>
              <div className="flex justify-between font-mono mb-1">
                <span className="text-slate-300 font-semibold">Base Rate (b)</span>
                <span className="text-cyan-400">{(localConfig.baseRateBps / 100).toFixed(2)}% ({localConfig.baseRateBps} bps)</span>
              </div>
              <input
                type="range"
                min="0"
                max="1500"
                step="25"
                value={localConfig.baseRateBps}
                onChange={e => setLocalConfig({ ...localConfig, baseRateBps: parseInt(e.target.value) })}
                className="w-full accent-cyan-400 cursor-pointer"
              />
              <span className="text-[10px] text-slate-500">Fixed rate floor component for risk score = 0</span>
            </div>

            {/* Risk Sensitivity s */}
            <div>
              <div className="flex justify-between font-mono mb-1">
                <span className="text-slate-300 font-semibold">Risk Sensitivity (s)</span>
                <span className="text-cyan-400">{(localConfig.riskSensitivityBps / 100).toFixed(2)}% / pt ({localConfig.riskSensitivityBps} bps)</span>
              </div>
              <input
                type="range"
                min="1"
                max="50"
                step="1"
                value={localConfig.riskSensitivityBps}
                onChange={e => setLocalConfig({ ...localConfig, riskSensitivityBps: parseInt(e.target.value) })}
                className="w-full accent-cyan-400 cursor-pointer"
              />
              <span className="text-[10px] text-slate-500">Slope: rate increase per risk score unit</span>
            </div>

            {/* Rate Floor r_min */}
            <div>
              <div className="flex justify-between font-mono mb-1">
                <span className="text-slate-300 font-semibold">Rate Floor (r_min)</span>
                <span className="text-emerald-400">{(localConfig.rateFloorBps / 100).toFixed(2)}% ({localConfig.rateFloorBps} bps)</span>
              </div>
              <input
                type="range"
                min="100"
                max="1000"
                step="25"
                value={localConfig.rateFloorBps}
                onChange={e => setLocalConfig({ ...localConfig, rateFloorBps: parseInt(e.target.value) })}
                className="w-full accent-emerald-400 cursor-pointer"
              />
              <span className="text-[10px] text-slate-500">Minimum allowable interest rate</span>
            </div>

            {/* Rate Ceiling r_max */}
            <div>
              <div className="flex justify-between font-mono mb-1">
                <span className="text-slate-300 font-semibold">Rate Ceiling (r_max)</span>
                <span className="text-rose-400">{(localConfig.rateCeilingBps / 100).toFixed(2)}% ({localConfig.rateCeilingBps} bps)</span>
              </div>
              <input
                type="range"
                min="1000"
                max="4000"
                step="50"
                value={localConfig.rateCeilingBps}
                onChange={e => setLocalConfig({ ...localConfig, rateCeilingBps: parseInt(e.target.value) })}
                className="w-full accent-rose-400 cursor-pointer"
              />
              <span className="text-[10px] text-slate-500">Maximum rate cap (hard protocol cap 10,000 bps)</span>
            </div>
          </div>

          {/* Interactive Calculator */}
          <div className="pt-4 border-t border-slate-800 space-y-3">
            <div className="flex items-center gap-2 text-xs font-semibold text-slate-200">
              <Calculator className="h-4 w-4 text-amber-400" />
              <span>Score Calculator Test Bench</span>
            </div>

            <div className="bg-slate-950 border border-slate-800 rounded-xl p-3 space-y-2 text-xs">
              <div className="flex items-center justify-between">
                <span className="text-slate-400">Input Risk Score (k):</span>
                <input
                  type="number"
                  min="0"
                  max="1000"
                  value={testScore}
                  onChange={e => setTestScore(parseInt(e.target.value) || 0)}
                  className="w-20 bg-slate-900 border border-slate-700 rounded px-2 py-1 text-right text-cyan-300 font-mono"
                />
              </div>

              <div className="flex items-center justify-between font-mono pt-1 border-t border-slate-800">
                <span className="text-slate-400">Resulting Rate r(k):</span>
                <span className="text-sm font-bold text-amber-300">{calculatedTestRate}%</span>
              </div>
            </div>
          </div>
        </div>

        {/* Visual Rate Curve Graph */}
        <div className="lg:col-span-2 bg-slate-900/50 border border-slate-800/80 rounded-2xl p-6 flex flex-col justify-between space-y-4">
          <div>
            <div className="flex items-center justify-between">
              <h3 className="text-base font-semibold text-white">Interest Rate vs Risk Score Curve</h3>
              <span className="text-xs font-mono text-cyan-400">r(k) = clamp(b + k·s)</span>
            </div>
            <p className="text-xs text-slate-400 mt-1">
              Visual plot comparing raw linear score multiplication vs clamped protocol interest rate.
            </p>
          </div>

          <div className="h-72 w-full">
            <ResponsiveContainer width="100%" height="100%">
              <LineChart data={curveData} margin={{ top: 10, right: 20, left: 0, bottom: 0 }}>
                <CartesianGrid strokeDasharray="3 3" stroke="#1e293b" />
                <XAxis dataKey="score" stroke="#64748b" fontSize={11} label={{ value: 'Risk Score (k)', position: 'insideBottom', offset: -5, fill: '#64748b', fontSize: 11 }} />
                <YAxis stroke="#64748b" fontSize={11} unit="%" domain={[0, 35]} />
                <Tooltip
                  contentStyle={{ backgroundColor: '#0f172a', borderColor: '#334155', borderRadius: '0.75rem' }}
                  itemStyle={{ fontSize: '12px' }}
                />
                <Line type="monotone" dataKey="ratePct" name="Clamped Rate (%)" stroke="#06b6d4" strokeWidth={3} dot={false} />
                <Line type="monotone" dataKey="rawRatePct" name="Raw Rate (%)" stroke="#64748b" strokeWidth={1} strokeDasharray="5 5" dot={false} />
              </LineChart>
            </ResponsiveContainer>
          </div>

          {/* Impact on Active Borrowers */}
          <div className="bg-slate-950 border border-slate-800 rounded-xl p-4 space-y-2">
            <div className="flex items-center gap-1.5 text-xs font-medium text-slate-300">
              <Info className="h-4 w-4 text-cyan-400" />
              <span>Simulated Impact on Current Portfolio</span>
            </div>
            <div className="grid grid-cols-3 gap-2 text-xs font-mono">
              {creditLines.slice(0, 3).map(line => {
                const simulatedRate = (computeRateFromScore(line.riskScore, localConfig) / 100).toFixed(2);
                return (
                  <div key={line.borrower} className="bg-slate-900 p-2 rounded-lg border border-slate-800">
                    <span className="text-[10px] text-slate-400 truncate block font-sans">{line.borrowerName}</span>
                    <div className="flex justify-between items-center mt-1">
                      <span className="text-slate-500 text-[10px]">Score {line.riskScore}</span>
                      <span className="text-amber-300 font-bold">{simulatedRate}%</span>
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};
