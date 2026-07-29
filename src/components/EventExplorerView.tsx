import React, { useState } from 'react';
import { useProtocol } from '../context/ProtocolContext';
import { CONTRACT_ERRORS } from '../data/contractErrors';
import { Terminal, Search, Code, Filter, CheckCircle2, ChevronRight, ChevronDown } from 'lucide-react';

export const EventExplorerView: React.FC = () => {
  const { events } = useProtocol();

  const [activeTab, setActiveTab] = useState<'events' | 'errors'>('events');
  const [topicFilter, setTopicFilter] = useState('All');
  const [errorSearch, setErrorSearch] = useState('');
  const [moduleFilter, setModuleFilter] = useState('All');
  const [expandedEventId, setExpandedEventId] = useState<string | null>(null);

  const filteredEvents = events.filter(e => {
    if (topicFilter === 'All') return true;
    return e.topic === topicFilter;
  });

  const filteredErrors = CONTRACT_ERRORS.filter(err => {
    const matchesSearch =
      err.variant.toLowerCase().includes(errorSearch.toLowerCase()) ||
      err.code.toString().includes(errorSearch) ||
      err.summary.toLowerCase().includes(errorSearch.toLowerCase());
    const matchesModule = moduleFilter === 'All' || err.module === moduleFilter;
    return matchesSearch && matchesModule;
  });

  const uniqueTopics = ['All', ...Array.from(new Set(events.map(e => e.topic)))];

  return (
    <div className="space-y-6">
      {/* Header & Sub-tab switcher */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 bg-slate-900/60 border border-slate-800 rounded-2xl p-6">
        <div>
          <div className="flex items-center gap-2">
            <h2 className="text-xl font-bold text-white tracking-tight">Contract Event Stream & Error Spec</h2>
            <span className="px-2.5 py-0.5 rounded-md bg-cyan-950/80 border border-cyan-800 text-cyan-300 text-xs font-mono">
              Soroban Event Indexer
            </span>
          </div>
          <p className="text-xs text-slate-400 mt-1">
            Inspect live protocol event payloads and search the 38-variant Soroban ContractError catalog.
          </p>
        </div>

        {/* Tab Switcher */}
        <div className="flex items-center p-1 bg-slate-950 border border-slate-800 rounded-xl text-xs font-medium shrink-0">
          <button
            onClick={() => setActiveTab('events')}
            className={`px-4 py-2 rounded-lg transition cursor-pointer ${
              activeTab === 'events' ? 'bg-cyan-500 text-slate-950 font-bold' : 'text-slate-400 hover:text-slate-200'
            }`}
          >
            Soroban Events ({events.length})
          </button>
          <button
            onClick={() => setActiveTab('errors')}
            className={`px-4 py-2 rounded-lg transition cursor-pointer ${
              activeTab === 'errors' ? 'bg-cyan-500 text-slate-950 font-bold' : 'text-slate-400 hover:text-slate-200'
            }`}
          >
            Error Catalog (38)
          </button>
        </div>
      </div>

      {/* Events View */}
      {activeTab === 'events' && (
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <h3 className="text-base font-semibold text-white">Live Event Log</h3>

            <div className="flex items-center gap-2 text-xs">
              <Filter className="h-4 w-4 text-slate-500" />
              <select
                value={topicFilter}
                onChange={e => setTopicFilter(e.target.value)}
                className="bg-slate-900 border border-slate-800 rounded-xl px-3 py-1.5 text-slate-200"
              >
                {uniqueTopics.map(t => (
                  <option key={t} value={t}>{t}</option>
                ))}
              </select>
            </div>
          </div>

          <div className="space-y-3 font-mono text-xs">
            {filteredEvents.map(evt => {
              const isExpanded = expandedEventId === evt.id;
              const dateStr = new Date(evt.timestamp * 1000).toLocaleTimeString();

              return (
                <div
                  key={evt.id}
                  className="bg-slate-900/60 border border-slate-800 rounded-xl p-4 space-y-2 hover:border-slate-700 transition"
                >
                  <div
                    onClick={() => setExpandedEventId(isExpanded ? null : evt.id)}
                    className="flex items-center justify-between cursor-pointer"
                  >
                    <div className="flex items-center gap-3">
                      {isExpanded ? <ChevronDown className="h-4 w-4 text-cyan-400" /> : <ChevronRight className="h-4 w-4 text-slate-500" />}
                      <span className="px-2 py-0.5 rounded bg-cyan-950/80 border border-cyan-800 text-cyan-300 font-bold text-[11px]">
                        {evt.topic}
                      </span>
                      {evt.borrower && (
                        <span className="text-slate-400 text-[11px]">Borrower: {evt.borrower}</span>
                      )}
                    </div>

                    <span className="text-[10px] text-slate-500">{dateStr}</span>
                  </div>

                  {isExpanded && (
                    <div className="mt-3 pt-3 border-t border-slate-800/80 bg-slate-950 p-3 rounded-lg overflow-x-auto text-slate-300">
                      <pre className="text-[11px] leading-relaxed">
                        {JSON.stringify(evt.payload, null, 2)}
                      </pre>
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}

      {/* Errors Catalog View */}
      {activeTab === 'errors' && (
        <div className="space-y-4">
          <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
            <div className="relative flex-1">
              <Search className="h-4 w-4 absolute left-3 top-2.5 text-slate-500" />
              <input
                type="text"
                placeholder="Search error by variant name, code (e.g., 105), or description..."
                value={errorSearch}
                onChange={e => setErrorSearch(e.target.value)}
                className="w-full bg-slate-900 border border-slate-800 rounded-xl pl-9 pr-4 py-2 text-xs text-slate-200 focus:outline-none focus:border-cyan-500"
              />
            </div>

            <select
              value={moduleFilter}
              onChange={e => setModuleFilter(e.target.value)}
              className="bg-slate-900 border border-slate-800 rounded-xl px-3 py-2 text-xs text-slate-200"
            >
              <option value="All">All Modules</option>
              <option value="credit">credit</option>
              <option value="borrow">borrow</option>
              <option value="collateral">collateral</option>
              <option value="risk">risk</option>
              <option value="auction">auction</option>
              <option value="freeze">freeze</option>
            </select>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            {filteredErrors.map(err => (
              <div key={err.code} className="bg-slate-900/50 border border-slate-800 rounded-2xl p-4 space-y-2">
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-sm font-bold text-amber-300">{err.code}</span>
                    <span className="font-bold text-white text-xs">{err.variant}</span>
                  </div>
                  <span className="px-2 py-0.5 rounded text-[10px] font-mono bg-slate-800 text-slate-400 border border-slate-700">
                    {err.module}
                  </span>
                </div>

                <p className="text-xs text-slate-300 font-medium">{err.summary}</p>
                <p className="text-[11px] text-slate-400 leading-relaxed bg-slate-950 p-2.5 rounded-lg border border-slate-800/60">
                  {err.description}
                </p>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
};
