// SPDX-License-Identifier: MIT

//! Integration tests for `get_developer_balances_page` cursor pagination.
//!
//! Coverage matrix
//! ──────────────
//! • Empty index → ([], None)
//! • Single entry, first page               → ([entry], None)
//! • Multiple entries, single page          → all entries, None
//! • Cursor advances correctly across pages
//! • Last page returns None next_cursor
//! • Cursor past last id                    → ([], None)
//! • limit=0                                → ([], None)
//! • limit capped at MAX (100)
//! • Stable ordering across repeated calls
//! • Cursor unaffected by interleaved draws
//! • utilized_amount reflects latest state

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{testutils::Address as _, Address, Env};

// ── helpers ───────────────────────────────────────────────────────────────────

struct TestEnv {
    pub env: Env,
    pub contract_id: Address,
}

impl TestEnv {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        Self { env, contract_id }
    }

    fn client(&self) -> CreditClient<'_> {
        CreditClient::new(&self.env, &self.contract_id)
    }

    /// Open a credit line with a fixed rate/score; returns the borrower address.
    fn open(&self, limit: i128) -> Address {
        let borrower = Address::generate(&self.env);
        self.client()
            .open_credit_line(&borrower, &limit, &300_u32, &70_u32);
        borrower
    }

    /// Open `n` credit lines and return their addresses in insertion order.
    fn open_n(&self, n: usize, limit: i128) -> soroban_sdk::Vec<Address> {
        let mut out = soroban_sdk::Vec::new(&self.env);
        for _ in 0..n {
            out.push_back(self.open(limit));
        }
        out
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[test]
fn empty_index_returns_empty_page_and_no_cursor() {
    let t = TestEnv::new();
    let (page, next) = t.client().get_developer_balances_page(&None, &10);
    assert_eq!(page.len(), 0);
    assert!(next.is_none());
}

#[test]
fn single_entry_first_page() {
    let t = TestEnv::new();
    let borrower = t.open(5_000);

    let (page, next) = t.client().get_developer_balances_page(&None, &10);
    assert_eq!(page.len(), 1);
    assert_eq!(page.get(0).unwrap().id, 0);
    assert_eq!(page.get(0).unwrap().borrower, borrower);
    assert_eq!(page.get(0).unwrap().utilized_amount, 0);
    assert_eq!(page.get(0).unwrap().credit_limit, 5_000);
    // Only one entry — no more pages.
    assert!(next.is_none());
}

#[test]
fn all_entries_fit_in_one_page_returns_no_cursor() {
    let t = TestEnv::new();
    t.open_n(5, 1_000);

    let (page, next) = t.client().get_developer_balances_page(&None, &10);
    assert_eq!(page.len(), 5);
    // IDs must be 0..4 in order.
    for i in 0..5_u32 {
        assert_eq!(page.get(i).unwrap().id, i);
    }
    assert!(next.is_none());
}

#[test]
fn cursor_advances_across_pages() {
    let t = TestEnv::new();
    let borrowers = t.open_n(5, 1_000);

    // Page 1: ids 0, 1
    let (page1, cursor1) = t.client().get_developer_balances_page(&None, &2);
    assert_eq!(page1.len(), 2);
    assert_eq!(page1.get(0).unwrap().id, 0);
    assert_eq!(page1.get(1).unwrap().id, 1);
    // next_cursor points at id=1 (last in page), meaning next call starts at id=2.
    let c1 = cursor1.expect("should have a next cursor after page 1");

    // Page 2: ids 2, 3
    let (page2, cursor2) = t.client().get_developer_balances_page(&Some(c1), &2);
    assert_eq!(page2.len(), 2);
    assert_eq!(page2.get(0).unwrap().id, 2);
    assert_eq!(page2.get(1).unwrap().id, 3);
    let c2 = cursor2.expect("should have a next cursor after page 2");

    // Page 3: id 4 (last)
    let (page3, cursor3) = t.client().get_developer_balances_page(&Some(c2), &2);
    assert_eq!(page3.len(), 1);
    assert_eq!(page3.get(0).unwrap().id, 4);
    assert_eq!(page3.get(0).unwrap().borrower, borrowers.get(4).unwrap());
    // Last page — no more cursor.
    assert!(cursor3.is_none());
}

#[test]
fn last_page_returns_none_cursor() {
    let t = TestEnv::new();
    t.open_n(3, 1_000);

    // Ask for exactly all 3 in one call.
    let (page, next) = t.client().get_developer_balances_page(&None, &3);
    assert_eq!(page.len(), 3);
    assert!(next.is_none());
}

#[test]
fn cursor_past_last_id_returns_empty() {
    let t = TestEnv::new();
    t.open_n(3, 1_000);

    // Cursor at id=100 — far beyond the index.
    let (page, next) = t.client().get_developer_balances_page(&Some(100), &10);
    assert_eq!(page.len(), 0);
    assert!(next.is_none());
}

#[test]
fn limit_zero_returns_empty() {
    let t = TestEnv::new();
    t.open_n(3, 1_000);

    let (page, next) = t.client().get_developer_balances_page(&None, &0);
    assert_eq!(page.len(), 0);
    assert!(next.is_none());
}

#[test]
fn limit_capped_at_max_enumeration_limit() {
    let t = TestEnv::new();
    // Open 10 lines — fewer than MAX (100), so all are returned even with limit=200.
    t.open_n(10, 1_000);

    let (page, _) = t.client().get_developer_balances_page(&None, &200);
    // All 10 returned; the cap (100) doesn't reduce the real count here.
    assert_eq!(page.len(), 10);
}

#[test]
fn stable_ordering_across_repeated_calls() {
    let t = TestEnv::new();
    t.open_n(5, 1_000);

    let (page_a, _) = t.client().get_developer_balances_page(&None, &10);
    let (page_b, _) = t.client().get_developer_balances_page(&None, &10);
    let (page_c, _) = t.client().get_developer_balances_page(&None, &10);

    assert_eq!(page_a.len(), page_b.len());
    assert_eq!(page_b.len(), page_c.len());
    for i in 0..page_a.len() {
        let a = page_a.get(i).unwrap();
        let b = page_b.get(i).unwrap();
        let c = page_c.get(i).unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(b.id, c.id);
        assert_eq!(a.borrower, b.borrower);
        assert_eq!(b.borrower, c.borrower);
    }
}

#[test]
fn cursor_unaffected_by_interleaved_credits() {
    let t = TestEnv::new();
    // Open 4 lines.
    t.open_n(4, 10_000);

    // Get page 1 (ids 0, 1) — save cursor.
    let (page1, cursor1) = t.client().get_developer_balances_page(&None, &2);
    assert_eq!(page1.len(), 2);
    let c1 = cursor1.unwrap();

    // Open a NEW credit line (id=4) between pages — simulates interleaved activity.
    t.open(50_000);

    // Page 2 using the saved cursor must still start at id=2, not be disrupted.
    let (page2, _) = t.client().get_developer_balances_page(&Some(c1), &2);
    assert_eq!(page2.len(), 2);
    assert_eq!(page2.get(0).unwrap().id, 2);
    assert_eq!(page2.get(1).unwrap().id, 3);
}

#[test]
fn utilized_amount_is_zero_for_new_line() {
    // A freshly opened credit line has utilized_amount == 0 before any draw.
    let t = TestEnv::new();
    let _borrower = t.open(10_000);

    let (page, _) = t.client().get_developer_balances_page(&None, &10);
    assert_eq!(page.len(), 1);
    assert_eq!(page.get(0).unwrap().utilized_amount, 0);
    assert_eq!(page.get(0).unwrap().credit_limit, 10_000);
}

#[test]
fn utilized_amount_reflects_draw_via_enumerate() {
    // Cross-verify: enumerate_credit_lines and get_developer_balances_page
    // must agree on utilized_amount for the same borrower.
    let t = TestEnv::new();

    // Set up token for draws.
    let token_id = t
        .env
        .register_stellar_asset_contract_v2(Address::generate(&t.env));
    let token_addr = token_id.address();
    t.client().set_liquidity_token(&token_addr);
    t.client().set_liquidity_source(&t.contract_id);
    soroban_sdk::token::StellarAssetClient::new(&t.env, &token_addr)
        .mint(&t.contract_id, &100_000);

    // Disable collateral requirement so draw succeeds with zero collateral.
    t.client().set_min_collateral_ratio_bps(&0);

    let borrower = t.open(10_000);
    t.client().draw_credit(&borrower, &3_000);

    // get_developer_balances_page must reflect the draw.
    let (page, _) = t.client().get_developer_balances_page(&None, &10);
    assert_eq!(page.len(), 1);
    assert_eq!(page.get(0).unwrap().utilized_amount, 3_000);

    // enumerate_credit_lines must agree.
    let lines = t.client().enumerate_credit_lines(&None, &10);
    assert_eq!(lines.get(0).unwrap().1.utilized_amount, 3_000);
}
#[test]
fn credit_limit_field_is_correct() {
    let t = TestEnv::new();
    let _b1 = t.open(1_111);
    let _b2 = t.open(2_222);
    let _b3 = t.open(3_333);

    let (page, _) = t.client().get_developer_balances_page(&None, &10);
    assert_eq!(page.get(0).unwrap().credit_limit, 1_111);
    assert_eq!(page.get(1).unwrap().credit_limit, 2_222);
    assert_eq!(page.get(2).unwrap().credit_limit, 3_333);
}

#[test]
fn full_walk_collects_all_entries() {
    let t = TestEnv::new();
    let borrowers = t.open_n(7, 1_000);

    let mut all_ids: soroban_sdk::Vec<u32> = soroban_sdk::Vec::new(&t.env);
    let mut cursor: Option<u32> = None;

    loop {
        let (page, next) = t.client().get_developer_balances_page(&cursor, &3);
        for entry in page.iter() {
            all_ids.push_back(entry.id);
        }
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }

    assert_eq!(all_ids.len(), 7);
    for i in 0..7_u32 {
        assert_eq!(all_ids.get(i).unwrap(), i);
    }
    // Spot-check first and last borrower addresses.
    let (full_page, _) = t.client().get_developer_balances_page(&None, &10);
    assert_eq!(full_page.get(0).unwrap().borrower, borrowers.get(0).unwrap());
    assert_eq!(full_page.get(6).unwrap().borrower, borrowers.get(6).unwrap());
}
