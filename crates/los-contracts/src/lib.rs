//! # LOS Production Smart Contracts
//!
//! This crate contains the production `#![no_std]` WASM smart contracts
//! for the Unauthority (LOS) blockchain.
//!
//! ## Contracts
//!
//! | Contract       | Binary         | Description                                        |
//! |----------------|----------------|----------------------------------------------------|
//! | USP-01 Token   | `usp01_token`  | Native Fungible Token Standard (ERC-20 equivalent) |
//! | DEX AMM        | `dex_amm`      | Constant Product AMM (x·y=k) decentralized exchange|
//!
//! ## Compilation
//!
//! These contracts are compiled to `wasm32-unknown-unknown` for deployment on the UVM:
//!
//! ```bash
//! # Build all contracts
//! cargo build --target wasm32-unknown-unknown --release --manifest-path crates/los-contracts/Cargo.toml
//!
//! # Build individual contract
//! cargo build --target wasm32-unknown-unknown --release --manifest-path crates/los-contracts/Cargo.toml --bin usp01_token
//! cargo build --target wasm32-unknown-unknown --release --manifest-path crates/los-contracts/Cargo.toml --bin dex_amm
//! ```
//!
//! ## Architecture
//!
//! All contracts use `los-sdk` for host function interaction:
//! - Key-value state storage (decimal strings for numerics)
//! - Event emission for indexing
//! - Caller/context introspection
//! - Native CIL transfers
//!
//! **Important:** Numeric values are stored as decimal strings
//! (not LE bytes) to avoid `String::from_utf8_lossy` corruption
//! in `Contract.state: BTreeMap<String, String>`.

// ─────────────────────────────────────────────────────────────────
// Shared pure helper functions (tested natively, duplicated in bins)
// ─────────────────────────────────────────────────────────────────
// These helpers mirror the logic inside usp01_token.rs and dex_amm.rs.
// Unit tests below verify correctness of all pure arithmetic, string
// conversion, and JSON formatting used by both WASM contracts.
// ─────────────────────────────────────────────────────────────────

/// Parse a decimal string to u128 with overflow protection.
/// Returns 0 on empty string, non-digit chars, or overflow.
pub fn parse_u128(s: &str) -> u128 {
    let mut result: u128 = 0;
    for b in s.as_bytes() {
        if *b >= b'0' && *b <= b'9' {
            result = match result.checked_mul(10) {
                Some(v) => v,
                None => return 0,
            };
            result = match result.checked_add((*b - b'0') as u128) {
                Some(v) => v,
                None => return 0,
            };
        } else {
            return 0;
        }
    }
    result
}

/// Parse a decimal string to u64 with overflow protection.
pub fn parse_u64(s: &str) -> u64 {
    let mut result: u64 = 0;
    for b in s.as_bytes() {
        if *b >= b'0' && *b <= b'9' {
            result = match result.checked_mul(10) {
                Some(v) => v,
                None => return 0,
            };
            result = match result.checked_add((*b - b'0') as u64) {
                Some(v) => v,
                None => return 0,
            };
        } else {
            return 0;
        }
    }
    result
}

/// Convert u128 to decimal string without allocation (returns owned String).
pub fn u128_to_str(val: u128) -> String {
    if val == 0 {
        return String::from("0");
    }
    let mut buf = [0u8; 40];
    let mut pos = buf.len();
    let mut v = val;
    while v > 0 {
        pos -= 1;
        buf[pos] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    // SAFETY: we only write ASCII digits
    String::from_utf8(buf[pos..].to_vec()).unwrap_or_default()
}

/// Escape a string for JSON output (double-quote, backslash, newline).
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out
}

/// Integer square root via Newton's method. Returns floor(√n).
/// Used by DEX AMM for initial LP token calculation.
pub fn isqrt(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = x.div_ceil(2); // safe: no overflow for u128::MAX
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Constant product swap output: `out = (in * reserve_out) / (reserve_in + in)`.
/// Returns 0 if any input is zero to prevent division by zero.
pub fn compute_output(amount_in: u128, reserve_in: u128, reserve_out: u128) -> u128 {
    if reserve_in == 0 || reserve_out == 0 || amount_in == 0 {
        return 0;
    }
    const PRECISION: u128 = 1_000_000_000_000;
    match (
        amount_in.checked_mul(reserve_out),
        reserve_in.checked_add(amount_in),
    ) {
        (Some(num), Some(den)) if den > 0 => num / den,
        _ => {
            // Overflow fallback: scaled division
            let ratio_scaled = (amount_in * PRECISION) / reserve_in.saturating_add(amount_in);
            (ratio_scaled * reserve_out) / PRECISION
        }
    }
}

/// Deduct fee from input amount. Returns (amount_after_fee, fee_amount).
pub fn deduct_fee(amount: u128, fee_bps: u128) -> (u128, u128) {
    const BPS_DENOMINATOR: u128 = 10_000;
    let fee = amount * fee_bps / BPS_DENOMINATOR;
    (amount - fee, fee)
}

/// Generate deterministic pool ID from token pair (sorted alphabetically).
pub fn make_pool_id(token_a: &str, token_b: &str) -> String {
    if token_a < token_b {
        format!("POOL:{}:{}", token_a, token_b)
    } else {
        format!("POOL:{}:{}", token_b, token_a)
    }
}

/// Generate USP-01 balance key for state storage.
pub fn bal_key(address: &str) -> String {
    format!("bal:{}", address)
}

/// Generate USP-01 allowance key for state storage.
pub fn allow_key(owner: &str, spender: &str) -> String {
    format!("allow:{}:{}", owner, spender)
}

// ─────────────────────────────────────────────────────────────────
// UNIT TESTS — verifies all pure logic used by WASM contracts
// ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_u128 ──────────────────────────────────────────────

    #[test]
    fn test_parse_u128_zero() {
        assert_eq!(parse_u128("0"), 0);
    }

    #[test]
    fn test_parse_u128_positive() {
        assert_eq!(parse_u128("12345"), 12345);
    }

    #[test]
    fn test_parse_u128_large() {
        assert_eq!(parse_u128("340282366920938463463374607431768211455"), u128::MAX);
    }

    #[test]
    fn test_parse_u128_overflow_returns_zero() {
        // u128::MAX + 1
        assert_eq!(parse_u128("340282366920938463463374607431768211456"), 0);
    }

    #[test]
    fn test_parse_u128_empty_string() {
        assert_eq!(parse_u128(""), 0);
    }

    #[test]
    fn test_parse_u128_non_digit() {
        assert_eq!(parse_u128("123abc"), 0);
    }

    #[test]
    fn test_parse_u128_negative() {
        assert_eq!(parse_u128("-1"), 0);
    }

    #[test]
    fn test_parse_u128_with_space() {
        assert_eq!(parse_u128("12 34"), 0);
    }

    #[test]
    fn test_parse_u128_leading_zeros() {
        assert_eq!(parse_u128("000123"), 123);
    }

    #[test]
    fn test_parse_u128_total_supply_cil() {
        // 21,936,236 LOS × 10^11 CIL = 2,193,623,600,000,000,000 CIL
        assert_eq!(parse_u128("2193623600000000000"), 2_193_623_600_000_000_000);
    }

    // ── parse_u64 ───────────────────────────────────────────────

    #[test]
    fn test_parse_u64_zero() {
        assert_eq!(parse_u64("0"), 0);
    }

    #[test]
    fn test_parse_u64_max() {
        assert_eq!(parse_u64("18446744073709551615"), u64::MAX);
    }

    #[test]
    fn test_parse_u64_overflow_returns_zero() {
        assert_eq!(parse_u64("18446744073709551616"), 0);
    }

    #[test]
    fn test_parse_u64_non_digit() {
        assert_eq!(parse_u64("abc"), 0);
    }

    // ── u128_to_str ─────────────────────────────────────────────

    #[test]
    fn test_u128_to_str_zero() {
        assert_eq!(u128_to_str(0), "0");
    }

    #[test]
    fn test_u128_to_str_positive() {
        assert_eq!(u128_to_str(12345), "12345");
    }

    #[test]
    fn test_u128_to_str_max() {
        assert_eq!(u128_to_str(u128::MAX), "340282366920938463463374607431768211455");
    }

    #[test]
    fn test_u128_to_str_one() {
        assert_eq!(u128_to_str(1), "1");
    }

    #[test]
    fn test_u128_to_str_roundtrip() {
        for val in [0, 1, 999, 100_000_000_000, u128::MAX / 2, u128::MAX] {
            let s = u128_to_str(val);
            assert_eq!(parse_u128(&s), val, "Roundtrip failed for {}", val);
        }
    }

    // ── json_escape ─────────────────────────────────────────────

    #[test]
    fn test_json_escape_no_special() {
        assert_eq!(json_escape("hello"), "hello");
    }

    #[test]
    fn test_json_escape_quotes() {
        assert_eq!(json_escape(r#"say "hi""#), r#"say \"hi\""#);
    }

    #[test]
    fn test_json_escape_backslash() {
        assert_eq!(json_escape(r"path\to"), r"path\\to");
    }

    #[test]
    fn test_json_escape_newline() {
        assert_eq!(json_escape("line1\nline2"), "line1\\nline2");
    }

    #[test]
    fn test_json_escape_combined() {
        assert_eq!(json_escape("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
    }

    #[test]
    fn test_json_escape_empty() {
        assert_eq!(json_escape(""), "");
    }

    #[test]
    fn test_json_escape_address() {
        // LOS addresses should pass through unchanged
        let addr = "LOSWa8rB7k5mz2XpGtH9nR4vY6jL3cF";
        assert_eq!(json_escape(addr), addr);
    }

    // ── isqrt ───────────────────────────────────────────────────

    #[test]
    fn test_isqrt_zero() {
        assert_eq!(isqrt(0), 0);
    }

    #[test]
    fn test_isqrt_one() {
        assert_eq!(isqrt(1), 1);
    }

    #[test]
    fn test_isqrt_perfect_squares() {
        assert_eq!(isqrt(4), 2);
        assert_eq!(isqrt(9), 3);
        assert_eq!(isqrt(16), 4);
        assert_eq!(isqrt(100), 10);
        assert_eq!(isqrt(10000), 100);
        assert_eq!(isqrt(1_000_000), 1_000);
    }

    #[test]
    fn test_isqrt_non_perfect_floors() {
        assert_eq!(isqrt(2), 1);
        assert_eq!(isqrt(3), 1);
        assert_eq!(isqrt(5), 2);
        assert_eq!(isqrt(8), 2);
        assert_eq!(isqrt(99), 9);
        assert_eq!(isqrt(101), 10);
    }

    #[test]
    fn test_isqrt_large_values() {
        // 10^18 * 10^18 = 10^36 — DEX-scale reserves
        let reserve_product: u128 = 1_000_000_000_000_000_000 * 1_000_000_000_000_000_000;
        assert_eq!(isqrt(reserve_product), 1_000_000_000_000_000_000);
    }

    #[test]
    fn test_isqrt_u128_max() {
        // floor(√(u128::MAX)) ≈ 1.844674407×10^19
        let root = isqrt(u128::MAX);
        assert!(root.checked_mul(root).map_or(false, |sq| sq <= u128::MAX));
        // (root+1)^2 must overflow or exceed u128::MAX
        let next = root + 1;
        match next.checked_mul(next) {
            None => {} // overflow — expected
            Some(sq) => assert!(sq > u128::MAX, "isqrt too small"),
        }
    }

    #[test]
    fn test_isqrt_minimum_liquidity_check() {
        // Initial LP = isqrt(amount_a * amount_b) - MINIMUM_LIQUIDITY(1000)
        // If isqrt(product) <= 1000, pool creation should fail
        let amount_a: u128 = 500;
        let amount_b: u128 = 500;
        let lp = isqrt(amount_a * amount_b); // sqrt(250000) = 500
        assert!(lp <= 1000, "LP tokens {} should be <= MINIMUM_LIQUIDITY", lp);
    }

    // ── compute_output ──────────────────────────────────────────

    #[test]
    fn test_compute_output_basic() {
        // 1000 in, reserve_in=10000, reserve_out=10000
        // out = 1000 * 10000 / (10000 + 1000) = 909
        let out = compute_output(1000, 10000, 10000);
        assert_eq!(out, 909);
    }

    #[test]
    fn test_compute_output_zero_amount() {
        assert_eq!(compute_output(0, 10000, 10000), 0);
    }

    #[test]
    fn test_compute_output_zero_reserve_in() {
        assert_eq!(compute_output(1000, 0, 10000), 0);
    }

    #[test]
    fn test_compute_output_zero_reserve_out() {
        assert_eq!(compute_output(1000, 10000, 0), 0);
    }

    #[test]
    fn test_compute_output_never_exceeds_reserve() {
        // Even with large input, output < reserve_out
        let out = compute_output(1_000_000_000_000, 1000, 1_000_000);
        assert!(out < 1_000_000, "Output {} should be < reserve", out);
    }

    #[test]
    fn test_compute_output_small_trade() {
        // Very small trade relative to reserves
        let out = compute_output(1, 1_000_000, 1_000_000);
        // Expected: 1 * 1_000_000 / 1_000_001 ≈ 0 (due to integer division)
        assert_eq!(out, 0);
    }

    #[test]
    fn test_compute_output_large_reserves() {
        // DEX-scale: billions of CIL in reserves
        let out = compute_output(
            1_000_000_000_000, // 10 LOS input
            100_000_000_000_000_000, // 1M LOS reserve
            50_000_000_000_000_000,  // 500K LOS reserve
        );
        // out ≈ 0.005% of reserve_out — price impact is tiny
        assert!(out > 0);
        assert!(out < 50_000_000_000_000_000);
    }

    // ── deduct_fee ──────────────────────────────────────────────

    #[test]
    fn test_deduct_fee_30_bps() {
        // 30 bps = 0.3% fee
        let (after, fee) = deduct_fee(10000, 30);
        assert_eq!(fee, 30); // 10000 * 30 / 10000 = 30
        assert_eq!(after, 9970);
        assert_eq!(after + fee, 10000);
    }

    #[test]
    fn test_deduct_fee_zero_fee() {
        let (after, fee) = deduct_fee(10000, 0);
        assert_eq!(fee, 0);
        assert_eq!(after, 10000);
    }

    #[test]
    fn test_deduct_fee_max_fee() {
        // 1000 bps = 10%
        let (after, fee) = deduct_fee(10000, 1000);
        assert_eq!(fee, 1000);
        assert_eq!(after, 9000);
    }

    #[test]
    fn test_deduct_fee_100_percent() {
        // 10000 bps = 100%
        let (after, fee) = deduct_fee(10000, 10000);
        assert_eq!(fee, 10000);
        assert_eq!(after, 0);
    }

    #[test]
    fn test_deduct_fee_small_amount() {
        // Fee rounds down for small amounts
        let (after, fee) = deduct_fee(100, 30);
        // 100 * 30 / 10000 = 0 (integer division)
        assert_eq!(fee, 0);
        assert_eq!(after, 100);
    }

    // ── make_pool_id ────────────────────────────────────────────

    #[test]
    fn test_make_pool_id_sorted() {
        let id1 = make_pool_id("LOS", "TOKEN_A");
        let id2 = make_pool_id("TOKEN_A", "LOS");
        assert_eq!(id1, id2, "Pool ID must be deterministic regardless of order");
    }

    #[test]
    fn test_make_pool_id_format() {
        let id = make_pool_id("LOS", "wBTC");
        assert_eq!(id, "POOL:LOS:wBTC");
    }

    #[test]
    fn test_make_pool_id_same_tokens() {
        let id = make_pool_id("LOS", "LOS");
        assert_eq!(id, "POOL:LOS:LOS");
    }

    // ── bal_key / allow_key ─────────────────────────────────────

    #[test]
    fn test_bal_key() {
        assert_eq!(bal_key("LOSaddr"), "bal:LOSaddr");
    }

    #[test]
    fn test_allow_key() {
        assert_eq!(allow_key("owner", "spender"), "allow:owner:spender");
    }

    // ── Integration: fee + swap pipeline ────────────────────────

    #[test]
    fn test_swap_pipeline_with_fee() {
        let amount_in: u128 = 1_000_000;
        let reserve_in: u128 = 100_000_000;
        let reserve_out: u128 = 100_000_000;
        let fee_bps: u128 = 30;

        // Step 1: deduct fee
        let (after_fee, fee) = deduct_fee(amount_in, fee_bps);
        assert_eq!(fee, 3_000); // 1_000_000 * 30 / 10_000 = 3_000
        assert_eq!(after_fee, 997_000);

        // Step 2: compute output
        let out = compute_output(after_fee, reserve_in, reserve_out);

        // Output should be less than input (slippage + fee)
        assert!(out < amount_in, "Output {} should be < input {}", out, amount_in);
        assert!(out > 0, "Output should be positive");

        // Step 3: verify LP tokens for pool creation
        let lp = isqrt(reserve_in * reserve_out);
        assert_eq!(lp, 100_000_000, "LP = sqrt(100M * 100M) = 100M");
    }

    #[test]
    fn test_roundtrip_u128_all_edge_cases() {
        let cases: Vec<u128> = vec![
            0,
            1,
            9,
            10,
            99,
            100,
            999,
            1000,
            u128::MAX / 2,
            u128::MAX - 1,
            u128::MAX,
        ];
        for val in cases {
            let s = u128_to_str(val);
            let back = parse_u128(&s);
            assert_eq!(back, val, "Roundtrip failed for {}", val);
        }
    }
}
