import React from 'react';
import { useProtocol } from '../context/ProtocolContext';
import { ShieldCheck, ShieldAlert, FastForward, RotateCcw, Cpu, Lock, Unlock } from 'lucide-react';

export const Navbar: React.FC = () => {
  const { protocolState, simulateTimePassage, resetSimulator } = useProtocol();

  return (
    <header className="sticky top-0 z-30 border-b border-slate-800 bg-slate-950/80 backdrop-blur-md px-6 py-3.5 flex items-center justify-between">
      <div className="flex items-center gap-3">
        <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-gradient-to-br from-cyan-500 to-blue-600 text-slate-950 shadow-lg shadow-cyan-500/20 font-bold">
          <Cpu className="h-6 w-6 text-slate-950" />
        </div>
        <div>
          <div className="flex items-center gap-2">
            <h1 className="text-lg font-bold tracking-tight text-white">Creditra Protocol</h1>
            <span className="rounded-md bg-cyan-950/80 px-2 py-0.5 text-xs font-medium text-cyan-400 border border-cyan-800/50 font-mono">
              Soroban WASM &lt; 50KB
            </span>
          </div>
          <p className="text-xs text-slate-400">Decentralized, Risk-Priced Credit on Stellar</p>
        </div>
      </div>

      <div className="flex items-center gap-4">
        {/* Protocol Status Indicators */}
        <div className="hidden md:flex items-center gap-2 text-xs font-medium">
          <div className={`flex items-center gap-1.5 px-3 py-1 rounded-full border ${
            protocolState.isPaused 
              ? 'bg-rose-950/60 border-rose-800 text-rose-300' 
              : 'bg-emerald-950/60 border-emerald-800 text-emerald-300'
          }`}>
            {protocolState.isPaused ? (
              <>
                <ShieldAlert className="h-3.5 w-3.5" />
                <span>Paused</span>
              </>
            ) : (
              <>
                <ShieldCheck className="h-3.5 w-3.5" />
                <span>Protocol Active</span>
              </>
            )}
          </div>

          <div className={`flex items-center gap-1.5 px-3 py-1 rounded-full border ${
            protocolState.isDrawsFrozen 
              ? 'bg-amber-950/60 border-amber-800 text-amber-300' 
              : 'bg-slate-900 border-slate-700 text-slate-300'
          }`}>
            {protocolState.isDrawsFrozen ? <Lock className="h-3.5 w-3.5" /> : <Unlock className="h-3.5 w-3.5 text-cyan-400" />}
            <span>{protocolState.isDrawsFrozen ? 'Draws Frozen' : 'Draws Normal'}</span>
          </div>
        </div>

        {/* Time Simulator Button */}
        <div className="flex items-center gap-1.5 bg-slate-900 border border-slate-800 rounded-lg p-1 text-xs">
          <button
            onClick={() => simulateTimePassage(24)}
            className="flex items-center gap-1 px-2.5 py-1 rounded-md bg-slate-800 hover:bg-slate-700 text-slate-200 font-medium transition cursor-pointer"
            title="Advance simulated clock by 24 hours to accrue interest"
          >
            <FastForward className="h-3.5 w-3.5 text-cyan-400" />
            <span>+24h Accrue</span>
          </button>
          
          <button
            onClick={() => simulateTimePassage(7 * 24)}
            className="flex items-center gap-1 px-2 py-1 rounded-md hover:bg-slate-800 text-slate-400 hover:text-slate-200 transition cursor-pointer"
            title="Advance by 7 days"
          >
            <span>+7d</span>
          </button>
        </div>

        {/* Reset Simulator */}
        <button
          onClick={resetSimulator}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg border border-slate-800 bg-slate-900/80 hover:bg-slate-800 text-slate-300 hover:text-white text-xs font-medium transition cursor-pointer"
          title="Reset simulator state"
        >
          <RotateCcw className="h-3.5 w-3.5 text-slate-400" />
          <span className="hidden sm:inline">Reset</span>
        </button>
      </div>
    </header>
  );
};
