import React from 'react';
import {
  LayoutDashboard,
  CreditCard,
  LineChart,
  Gavel,
  Shield,
  Terminal,
  FileCode2
} from 'lucide-react';

export type TabType = 'dashboard' | 'credit_lines' | 'risk_lab' | 'auctions' | 'admin' | 'events';

interface SidebarProps {
  activeTab: TabType;
  setActiveTab: (tab: TabType) => void;
}

export const Sidebar: React.FC<SidebarProps> = ({ activeTab, setActiveTab }) => {
  const navItems: { id: TabType; label: string; icon: React.ReactNode; badge?: string }[] = [
    { id: 'dashboard', label: 'Protocol Dashboard', icon: <LayoutDashboard className="h-4 w-4" /> },
    { id: 'credit_lines', label: 'Credit Lines', icon: <CreditCard className="h-4 w-4" /> },
    { id: 'risk_lab', label: 'Risk Pricing Lab', icon: <LineChart className="h-4 w-4" />, badge: 'r(k)' },
    { id: 'auctions', label: 'Default Auctions', icon: <Gavel className="h-4 w-4" /> },
    { id: 'admin', label: 'Circuit Breakers & Governance', icon: <Shield className="h-4 w-4" /> },
    { id: 'events', label: 'Soroban Event Log & Errors', icon: <Terminal className="h-4 w-4" />, badge: '38 Errors' }
  ];

  return (
    <aside className="w-64 shrink-0 border-r border-slate-800 bg-slate-950/60 p-4 flex flex-col justify-between hidden md:flex">
      <div className="space-y-1">
        <div className="px-3 py-2 text-xs font-semibold uppercase tracking-wider text-slate-500">
          Navigation
        </div>
        {navItems.map(item => {
          const isActive = activeTab === item.id;
          return (
            <button
              key={item.id}
              onClick={() => setActiveTab(item.id)}
              className={`w-full flex items-center justify-between px-3 py-2.5 rounded-lg text-sm font-medium transition cursor-pointer ${
                isActive
                  ? 'bg-cyan-950/70 border border-cyan-800/50 text-cyan-300 shadow-sm'
                  : 'text-slate-400 hover:text-slate-200 hover:bg-slate-900/60'
              }`}
            >
              <div className="flex items-center gap-3">
                {item.icon}
                <span>{item.label}</span>
              </div>
              {item.badge && (
                <span className={`text-[10px] font-mono px-1.5 py-0.5 rounded ${
                  isActive ? 'bg-cyan-800/60 text-cyan-200' : 'bg-slate-800 text-slate-400'
                }`}>
                  {item.badge}
                </span>
              )}
            </button>
          );
        })}
      </div>

      <div className="rounded-xl border border-slate-800/80 bg-slate-900/40 p-3.5 space-y-2">
        <div className="flex items-center gap-2 text-xs font-medium text-slate-300">
          <FileCode2 className="h-4 w-4 text-cyan-400" />
          <span>Soroban Architecture</span>
        </div>
        <p className="text-xs text-slate-400 leading-relaxed">
          Unsecured & risk-adjusted credit protocol with deterministic rate-formula $r(k)$ and two-sided replay-protected default handoffs.
        </p>
      </div>
    </aside>
  );
};
