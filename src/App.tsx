import React, { useState } from 'react';
import { ProtocolProvider } from './context/ProtocolContext';
import { Navbar } from './components/Navbar';
import { Sidebar, TabType } from './components/Sidebar';
import { DashboardView } from './components/DashboardView';
import { CreditLinesView } from './components/CreditLinesView';
import { RiskPricingLab } from './components/RiskPricingLab';
import { AuctionsView } from './components/AuctionsView';
import { ProtocolAdminView } from './components/ProtocolAdminView';
import { EventExplorerView } from './components/EventExplorerView';
import { LayoutDashboard, CreditCard, LineChart, Gavel, Shield, Terminal } from 'lucide-react';

export function AppContent() {
  const [activeTab, setActiveTab] = useState<TabType>('dashboard');

  return (
    <div className="min-h-screen bg-slate-950 text-slate-100 flex flex-col font-sans">
      <Navbar />

      {/* Mobile Top Navigation Tabs */}
      <div className="md:hidden flex overflow-x-auto border-b border-slate-800 bg-slate-950 p-2 gap-2 text-xs scrollbar-none">
        <button
          onClick={() => setActiveTab('dashboard')}
          className={`flex items-center gap-1.5 px-3 py-2 rounded-lg whitespace-nowrap cursor-pointer ${
            activeTab === 'dashboard' ? 'bg-cyan-500 text-slate-950 font-bold' : 'text-slate-400'
          }`}
        >
          <LayoutDashboard className="h-3.5 w-3.5" /> Dashboard
        </button>
        <button
          onClick={() => setActiveTab('credit_lines')}
          className={`flex items-center gap-1.5 px-3 py-2 rounded-lg whitespace-nowrap cursor-pointer ${
            activeTab === 'credit_lines' ? 'bg-cyan-500 text-slate-950 font-bold' : 'text-slate-400'
          }`}
        >
          <CreditCard className="h-3.5 w-3.5" /> Credit Lines
        </button>
        <button
          onClick={() => setActiveTab('risk_lab')}
          className={`flex items-center gap-1.5 px-3 py-2 rounded-lg whitespace-nowrap cursor-pointer ${
            activeTab === 'risk_lab' ? 'bg-cyan-500 text-slate-950 font-bold' : 'text-slate-400'
          }`}
        >
          <LineChart className="h-3.5 w-3.5" /> Risk Lab
        </button>
        <button
          onClick={() => setActiveTab('auctions')}
          className={`flex items-center gap-1.5 px-3 py-2 rounded-lg whitespace-nowrap cursor-pointer ${
            activeTab === 'auctions' ? 'bg-cyan-500 text-slate-950 font-bold' : 'text-slate-400'
          }`}
        >
          <Gavel className="h-3.5 w-3.5" /> Auctions
        </button>
        <button
          onClick={() => setActiveTab('admin')}
          className={`flex items-center gap-1.5 px-3 py-2 rounded-lg whitespace-nowrap cursor-pointer ${
            activeTab === 'admin' ? 'bg-cyan-500 text-slate-950 font-bold' : 'text-slate-400'
          }`}
        >
          <Shield className="h-3.5 w-3.5" /> Admin
        </button>
        <button
          onClick={() => setActiveTab('events')}
          className={`flex items-center gap-1.5 px-3 py-2 rounded-lg whitespace-nowrap cursor-pointer ${
            activeTab === 'events' ? 'bg-cyan-500 text-slate-950 font-bold' : 'text-slate-400'
          }`}
        >
          <Terminal className="h-3.5 w-3.5" /> Events
        </button>
      </div>

      <div className="flex-1 flex overflow-hidden">
        <Sidebar activeTab={activeTab} setActiveTab={setActiveTab} />

        <main className="flex-1 overflow-y-auto p-4 sm:p-6 lg:p-8 max-w-7xl mx-auto w-full">
          {activeTab === 'dashboard' && <DashboardView />}
          {activeTab === 'credit_lines' && <CreditLinesView />}
          {activeTab === 'risk_lab' && <RiskPricingLab />}
          {activeTab === 'auctions' && <AuctionsView />}
          {activeTab === 'admin' && <ProtocolAdminView />}
          {activeTab === 'events' && <EventExplorerView />}
        </main>
      </div>
    </div>
  );
}

export default function App() {
  return (
    <ProtocolProvider>
      <AppContent />
    </ProtocolProvider>
  );
}
