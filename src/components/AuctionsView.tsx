import React, { useState } from 'react';
import { useProtocol } from '../context/ProtocolContext';
import { Auction } from '../types';
import { calculateDutchAuctionPrice } from '../services/creditraSimulator';
import { Gavel, Clock, TrendingDown, ArrowRightLeft, ShieldCheck, DollarSign } from 'lucide-react';

export const AuctionsView: React.FC = () => {
  const { auctions, placeAuctionBid, settleDefaultLiquidation } = useProtocol();

  const [selectedAuction, setSelectedAuction] = useState<Auction | null>(auctions[0] || null);
  const [bidAmount, setBidAmount] = useState('260000');
  const [bidder, setBidder] = useState('GBIDDER_RESCUE...99');
  const [feedback, setFeedback] = useState<{ msg: string; isErr: boolean } | null>(null);

  const handleBidSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!selectedAuction) return;
    const res = placeAuctionBid(selectedAuction.id, bidder, parseFloat(bidAmount));
    setFeedback({ msg: res.message, isErr: !res.success });
  };

  const handleSettle = () => {
    if (!selectedAuction) return;
    const res = settleDefaultLiquidation(selectedAuction.id);
    setFeedback({ msg: res.message, isErr: !res.success });
  };

  return (
    <div className="space-y-6">
      {/* Feedback Banner */}
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
            <h2 className="text-xl font-bold text-white tracking-tight">Default Liquidation Auctions</h2>
            <span className="px-2.5 py-0.5 rounded-md bg-purple-950/80 border border-purple-800 text-purple-300 text-xs font-mono">
              gateway-contract/contracts/auction_contract
            </span>
          </div>
          <p className="text-xs text-slate-400 mt-1">
            One-shot, replay-protected cross-contract handoff settling defaulted borrower debt with English & Dutch auctions.
          </p>
        </div>
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        {/* Auctions List */}
        <div className="lg:col-span-2 space-y-4">
          <h3 className="text-base font-semibold text-white">Active Collateral Liquidation Pools</h3>

          {auctions.map(auc => {
            const isSelected = selectedAuction?.id === auc.id;
            const nowSec = Math.floor(Date.now() / 1000);
            const elapsed = Math.max(0, nowSec - auc.startTime);
            const currentDutchPrice = calculateDutchAuctionPrice(
              auc.dutchStartPrice,
              auc.dutchFloorPrice,
              auc.dutchDecayRateSec,
              elapsed
            );

            return (
              <div
                key={auc.id}
                onClick={() => {
                  setSelectedAuction(auc);
                  setBidAmount((auc.currentHighBid + 10000).toString());
                }}
                className={`p-5 rounded-2xl border cursor-pointer transition ${
                  isSelected
                    ? 'bg-slate-900 border-purple-500/60 shadow-lg shadow-purple-950/30'
                    : 'bg-slate-900/40 border-slate-800/80 hover:bg-slate-900/80 hover:border-slate-700'
                }`}
              >
                <div className="flex items-center justify-between mb-3">
                  <div className="flex items-center gap-3">
                    <div className="p-2.5 rounded-xl bg-purple-950/80 border border-purple-800 text-purple-300">
                      <Gavel className="h-5 w-5" />
                    </div>
                    <div>
                      <div className="flex items-center gap-2">
                        <h4 className="text-sm font-bold text-white font-mono">{auc.id}</h4>
                        <span className="px-2 py-0.5 rounded text-[10px] font-mono bg-slate-800 text-slate-300 border border-slate-700">
                          {auc.mode} Mode
                        </span>
                      </div>
                      <p className="text-xs text-slate-400 font-mono">Borrower: {auc.borrower}</p>
                    </div>
                  </div>

                  <span className={`px-2.5 py-1 rounded-full text-xs font-medium border ${
                    auc.status === 'Active'
                      ? 'bg-emerald-950/60 border-emerald-800 text-emerald-300'
                      : 'bg-purple-950/60 border-purple-800 text-purple-300'
                  }`}>
                    {auc.status}
                  </span>
                </div>

                <div className="grid grid-cols-4 gap-2 pt-3 border-t border-slate-800/60 text-xs font-mono">
                  <div>
                    <span className="text-[10px] text-slate-500 block font-sans">Collateral Seized</span>
                    <span className="text-slate-200 font-medium">${auc.collateralAmount.toLocaleString()}</span>
                  </div>
                  <div>
                    <span className="text-[10px] text-slate-500 block font-sans">Debt to Cover</span>
                    <span className="text-rose-400 font-medium">${auc.debtToCover.toLocaleString()}</span>
                  </div>
                  <div>
                    <span className="text-[10px] text-slate-500 block font-sans">
                      {auc.mode === 'English' ? 'Highest Bid' : 'Current Price'}
                    </span>
                    <span className="text-cyan-300 font-bold">
                      ${auc.mode === 'English' ? auc.currentHighBid.toLocaleString() : currentDutchPrice.toLocaleString()}
                    </span>
                  </div>
                  <div>
                    <span className="text-[10px] text-slate-500 block font-sans">High Bidder</span>
                    <span className="text-slate-400 truncate block">{auc.currentHighBidder || 'None'}</span>
                  </div>
                </div>
              </div>
            );
          })}
        </div>

        {/* Selected Auction Detail & Bidding */}
        {selectedAuction && (
          <div className="bg-slate-900 border border-slate-800 rounded-2xl p-6 space-y-6 h-fit sticky top-20">
            <div>
              <div className="flex items-center justify-between">
                <h3 className="text-lg font-bold text-white font-mono">{selectedAuction.id}</h3>
                <span className="text-xs font-mono text-purple-400">{selectedAuction.mode} Auction</span>
              </div>
              <p className="text-xs text-slate-400 mt-1">Defaulted Borrower: <span className="font-mono text-slate-300">{selectedAuction.borrower}</span></p>
            </div>

            {/* Bidding Form */}
            {selectedAuction.status === 'Active' && (
              <form onSubmit={handleBidSubmit} className="bg-slate-950 border border-slate-800 p-4 rounded-xl space-y-3 text-xs">
                <div className="flex items-center gap-1.5 text-slate-200 font-semibold">
                  <DollarSign className="h-4 w-4 text-cyan-400" />
                  <span>Submit Liquidation Bid</span>
                </div>

                <div>
                  <label className="block text-slate-400 mb-1">Bidder Public Address</label>
                  <input
                    type="text"
                    value={bidder}
                    onChange={e => setBidder(e.target.value)}
                    className="w-full bg-slate-900 border border-slate-700 rounded-lg px-3 py-2 text-slate-200 font-mono"
                    required
                  />
                </div>

                <div>
                  <label className="block text-slate-400 mb-1">Bid Amount ($)</label>
                  <input
                    type="number"
                    value={bidAmount}
                    onChange={e => setBidAmount(e.target.value)}
                    className="w-full bg-slate-900 border border-slate-700 rounded-lg px-3 py-2 text-slate-200 font-mono"
                    required
                  />
                  <span className="text-[10px] text-slate-500 mt-1 block">
                    Must exceed current high bid of ${selectedAuction.currentHighBid.toLocaleString()}
                  </span>
                </div>

                <button
                  type="submit"
                  className="w-full py-2.5 rounded-lg bg-cyan-500 hover:bg-cyan-400 text-slate-950 font-bold transition cursor-pointer"
                >
                  Place Bid (place_bid)
                </button>
              </form>
            )}

            {/* Cross-Contract Settlement Handoff */}
            <div className="border-t border-slate-800 pt-4 space-y-3">
              <div className="flex items-center gap-1.5 text-xs font-semibold text-purple-300">
                <ArrowRightLeft className="h-4 w-4 text-purple-400" />
                <span>Cross-Contract Settlement Handoff</span>
              </div>

              <p className="text-xs text-slate-400 leading-relaxed">
                Invokes <code className="text-purple-300 font-mono">settle_default_liquidation</code>: transfers auction proceeds back to Credit contract and writes off remaining debt.
              </p>

              <button
                onClick={handleSettle}
                disabled={selectedAuction.status === 'Settled'}
                className="w-full flex items-center justify-center gap-2 p-3 rounded-xl bg-purple-600 hover:bg-purple-500 text-white font-bold text-xs transition cursor-pointer disabled:opacity-40"
              >
                <ShieldCheck className="h-4 w-4" />
                <span>{selectedAuction.status === 'Settled' ? 'Already Settled' : 'Trigger Cross-Contract Settlement'}</span>
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
};
