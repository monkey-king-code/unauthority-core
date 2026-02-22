use serde::{Deserialize, Serialize};
/// Decentralized Oracle Consensus System — Fixed-Point Integer Math (u128)
///
/// Implements Byzantine Fault Tolerant oracle price consensus using median aggregation.
/// Prevents single validator from manipulating BTC/ETH prices for PoB distribution.
///
/// **MAINNET DETERMINISM:**
/// All prices are stored as micro-USD (u128): 1 USD = 1,000,000 micro-USD.
/// NO floating-point arithmetic (f64) is used anywhere in this module.
/// This guarantees cross-node deterministic consensus — every validator computes
/// the exact same median from identical price submissions.
///
/// **Security Model:**
/// - Minimum 2f+1 submissions required (f = faulty nodes)
/// - Median price resists outliers (cannot be manipulated by minority)
/// - Submission window: 60 seconds (configurable)
/// - Outlier detection: >2000 basis points (20%) deviation from median = flagged
///
/// **Workflow:**
/// 1. Each validator fetches ETH/BTC prices from external APIs
/// 2. Converts to micro-USD (u128) at the API boundary
/// 3. Broadcasts price submission via P2P: "ORACLE_SUBMIT:addr:eth_micro:btc_micro"
/// 4. All validators collect submissions within time window
/// 5. Calculate median (Byzantine-resistant, pure integer math)
/// 6. Use consensus price for PoB burn calculations
use std::collections::BTreeMap;

/// 1 USD = 1,000,000 micro-USD (6 decimal places of precision)
pub const MICRO_USD_PER_USD: u128 = 1_000_000;

/// Price submission from a validator (all prices in micro-USD)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceSubmission {
    pub validator_address: String,
    /// ETH price in micro-USD (e.g., $2500.00 = 2_500_000_000)
    pub eth_price_micro_usd: u128,
    /// BTC price in micro-USD (e.g., $83000.00 = 83_000_000_000)
    pub btc_price_micro_usd: u128,
    pub timestamp: u64,
}

/// Oracle consensus state (pure integer math — no f64)
pub struct OracleConsensus {
    /// Validator submissions: address -> PriceSubmission
    /// MAINNET: BTreeMap for deterministic iteration
    submissions: BTreeMap<String, PriceSubmission>,

    /// Submission window in seconds (default: 60s)
    submission_window_secs: u64,

    /// Minimum submissions required (2f+1 for BFT)
    min_submissions: usize,

    /// Outlier threshold in basis points (10000 = 100%, default 2000 = 20%)
    outlier_threshold_bp: u128,
}

impl Default for OracleConsensus {
    fn default() -> Self {
        Self::new()
    }
}

impl OracleConsensus {
    /// Create new oracle consensus with default settings
    pub fn new() -> Self {
        Self {
            submissions: BTreeMap::new(),
            submission_window_secs: 60,
            // SECURITY FIX C-18: min_submissions is now a floor that gets
            // dynamically raised by update_min_submissions() based on
            // the actual validator count (2f+1 where f = n/3).
            min_submissions: 2,
            outlier_threshold_bp: 2000, // 20% = 2000 basis points
        }
    }

    /// Create with custom configuration
    /// `outlier_threshold_bp`: basis points (10000 = 100%), e.g. 2000 = 20%
    pub fn with_config(
        submission_window_secs: u64,
        min_submissions: usize,
        outlier_threshold_bp: u128,
    ) -> Self {
        Self {
            submissions: BTreeMap::new(),
            submission_window_secs,
            min_submissions,
            outlier_threshold_bp,
        }
    }

    /// Submit price from a validator (broadcast via P2P)
    /// Prices are in micro-USD (u128): 1 USD = 1,000,000
    ///
    /// SECURITY NOTE: Caller is responsible for authenticating the validator
    /// before calling this method (signature verification happens at P2P layer).
    pub fn submit_price(
        &mut self,
        validator_address: String,
        eth_price_micro_usd: u128,
        btc_price_micro_usd: u128,
    ) {
        // SECURITY P0-5: Reject zero prices (u128 cannot be NaN/Inf/negative)
        if eth_price_micro_usd == 0 || btc_price_micro_usd == 0 {
            println!(
                "🚨 Rejected invalid oracle price from {}: ETH={}, BTC={}",
                &validator_address[..std::cmp::min(12, validator_address.len())],
                eth_price_micro_usd,
                btc_price_micro_usd
            );
            return;
        }

        // SECURITY: Price bounds sanity check
        // ETH: reject if < $1 or > $1,000,000
        // BTC: reject if < $1 or > $10,000,000
        const MIN_PRICE_MICRO: u128 = MICRO_USD_PER_USD;
        const MAX_ETH_MICRO: u128 = 1_000_000 * MICRO_USD_PER_USD;
        const MAX_BTC_MICRO: u128 = 10_000_000 * MICRO_USD_PER_USD;

        if !(MIN_PRICE_MICRO..=MAX_ETH_MICRO).contains(&eth_price_micro_usd) {
            println!(
                "🚨 Rejected out-of-bounds ETH price from {}: ${}.{:02}",
                &validator_address[..std::cmp::min(12, validator_address.len())],
                eth_price_micro_usd / MICRO_USD_PER_USD,
                (eth_price_micro_usd % MICRO_USD_PER_USD) / 10_000,
            );
            return;
        }
        if !(MIN_PRICE_MICRO..=MAX_BTC_MICRO).contains(&btc_price_micro_usd) {
            println!(
                "🚨 Rejected out-of-bounds BTC price from {}: ${}.{:02}",
                &validator_address[..std::cmp::min(12, validator_address.len())],
                btc_price_micro_usd / MICRO_USD_PER_USD,
                (btc_price_micro_usd % MICRO_USD_PER_USD) / 10_000,
            );
            return;
        }

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let submission = PriceSubmission {
            validator_address: validator_address.clone(),
            eth_price_micro_usd,
            btc_price_micro_usd,
            timestamp,
        };

        self.submissions
            .insert(validator_address.clone(), submission);

        println!(
            "📊 Oracle submission from {}: ETH=${}.{:02}, BTC=${}.{:02}",
            &validator_address[..std::cmp::min(12, validator_address.len())],
            eth_price_micro_usd / MICRO_USD_PER_USD,
            (eth_price_micro_usd % MICRO_USD_PER_USD) / 10_000,
            btc_price_micro_usd / MICRO_USD_PER_USD,
            (btc_price_micro_usd % MICRO_USD_PER_USD) / 10_000,
        );
    }

    /// Get consensus price (median of recent submissions) in micro-USD
    /// Returns None if insufficient submissions
    pub fn get_consensus_price(&self) -> Option<(u128, u128)> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Filter recent submissions (within window)
        let recent: Vec<&PriceSubmission> = self
            .submissions
            .values()
            .filter(|s| now - s.timestamp < self.submission_window_secs)
            .collect();

        // Check if we have enough submissions (BFT requirement)
        if recent.len() < self.min_submissions {
            println!(
                "⚠️  Insufficient oracle submissions: {} (need ≥{})",
                recent.len(),
                self.min_submissions
            );
            return None;
        }

        // Extract prices (filter out zero)
        let mut eth_prices: Vec<u128> = recent
            .iter()
            .map(|s| s.eth_price_micro_usd)
            .filter(|p| *p > 0)
            .collect();
        let mut btc_prices: Vec<u128> = recent
            .iter()
            .map(|s| s.btc_price_micro_usd)
            .filter(|p| *p > 0)
            .collect();

        // Need at least min_submissions of valid prices
        if eth_prices.len() < self.min_submissions || btc_prices.len() < self.min_submissions {
            println!(
                "⚠️  Insufficient valid oracle prices after zero filtering: ETH={}, BTC={}",
                eth_prices.len(),
                btc_prices.len()
            );
            return None;
        }

        // Sort for median calculation (u128 has total ordering — no NaN issues)
        eth_prices.sort();
        btc_prices.sort();

        // Calculate median (Byzantine-resistant, pure integer math)
        let eth_median = Self::calculate_median(&eth_prices);
        let btc_median = Self::calculate_median(&btc_prices);

        println!(
            "✅ Oracle consensus reached: ETH=${}.{:02}, BTC=${}.{:02} (from {} validators)",
            eth_median / MICRO_USD_PER_USD,
            (eth_median % MICRO_USD_PER_USD) / 10_000,
            btc_median / MICRO_USD_PER_USD,
            (btc_median % MICRO_USD_PER_USD) / 10_000,
            recent.len()
        );

        Some((eth_median, btc_median))
    }

    /// Calculate median of sorted u128 array (pure integer math)
    /// For even-length arrays, returns integer average of two middle values
    /// (loses at most 0.5 micro-USD = $0.0000005, negligible)
    fn calculate_median(sorted_values: &[u128]) -> u128 {
        let len = sorted_values.len();
        if len == 0 {
            return 0;
        }

        if len % 2 == 1 {
            // Odd number: return middle value (exact)
            sorted_values[len / 2]
        } else {
            // Even number: integer average of two middle values
            (sorted_values[len / 2 - 1] + sorted_values[len / 2]) / 2
        }
    }

    /// Detect outlier validators (possible price manipulation)
    /// Returns list of validator addresses with suspicious prices
    /// Uses basis points (10000 = 100%) for deviation threshold — no f64
    pub fn detect_outliers(&self) -> Vec<String> {
        let consensus = match self.get_consensus_price() {
            Some(p) => p,
            None => return vec![],
        };

        let (median_eth, median_btc) = consensus;
        let mut outliers = Vec::new();

        // Guard against division by zero
        if median_eth == 0 || median_btc == 0 {
            return vec![];
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        for (validator, submission) in &self.submissions {
            // Only check recent submissions
            if now - submission.timestamp >= self.submission_window_secs {
                continue;
            }

            // Deviation in basis points: |price - median| * 10000 / median
            let eth_diff = submission.eth_price_micro_usd.abs_diff(median_eth);
            let btc_diff = submission.btc_price_micro_usd.abs_diff(median_btc);

            let eth_deviation_bp = eth_diff.saturating_mul(10_000) / median_eth;
            let btc_deviation_bp = btc_diff.saturating_mul(10_000) / median_btc;

            // If deviation > threshold basis points, flag as outlier
            if eth_deviation_bp > self.outlier_threshold_bp
                || btc_deviation_bp > self.outlier_threshold_bp
            {
                println!(
                    "🚨 Oracle outlier detected: {} (ETH: {}bp, BTC: {}bp)",
                    &validator[..std::cmp::min(12, validator.len())],
                    eth_deviation_bp,
                    btc_deviation_bp
                );
                outliers.push(validator.clone());
            }
        }

        outliers
    }

    /// Cleanup old submissions (garbage collection)
    pub fn cleanup_old(&mut self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let cutoff = now - (self.submission_window_secs * 2);

        let before_count = self.submissions.len();
        self.submissions.retain(|_, s| s.timestamp > cutoff);
        let removed = before_count - self.submissions.len();

        if removed > 0 {
            println!("🧹 Oracle cleanup: removed {} old submissions", removed);
        }
    }

    /// Get current submission count
    pub fn submission_count(&self) -> usize {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        self.submissions
            .values()
            .filter(|s| now - s.timestamp < self.submission_window_secs)
            .count()
    }

    /// SECURITY FIX C-18: Update min_submissions dynamically based on validator count.
    /// For BFT: need 2f+1 submissions where f = floor(n/3) (max faulty tolerated).
    /// Minimum is always 2 for safety (never allow single-node oracle consensus).
    pub fn update_min_submissions(&mut self, validator_count: usize) {
        let f = validator_count / 3;
        let bft_min = 2 * f + 1;
        self.min_submissions = bft_min.max(2); // Never below 2
    }

    /// Get current min_submissions value
    pub fn min_submissions(&self) -> usize {
        self.min_submissions
    }

    /// Get all recent submissions (for debugging)
    pub fn get_recent_submissions(&self) -> Vec<PriceSubmission> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        self.submissions
            .values()
            .filter(|s| now - s.timestamp < self.submission_window_secs)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_median_calculation() {
        // Odd number of values (micro-USD)
        let odd = vec![
            10_000_000u128,
            20_000_000,
            30_000_000,
            40_000_000,
            50_000_000,
        ];
        assert_eq!(OracleConsensus::calculate_median(&odd), 30_000_000);

        // Even number of values — integer average
        let even = vec![10_000_000u128, 20_000_000, 30_000_000, 40_000_000];
        assert_eq!(OracleConsensus::calculate_median(&even), 25_000_000);

        // Single value
        let single = vec![42_000_000u128];
        assert_eq!(OracleConsensus::calculate_median(&single), 42_000_000);
    }

    #[test]
    fn test_byzantine_resistance() {
        let mut oracle = OracleConsensus::new();

        // 2 honest validators (prices in micro-USD)
        oracle.submit_price("VAL1".to_string(), 2_500_000_000, 83_000_000_000);
        oracle.submit_price("VAL2".to_string(), 2_510_000_000, 83_100_000_000);

        // 1 malicious validator (trying to manipulate 2x price)
        oracle.submit_price("VAL_EVIL".to_string(), 5_000_000_000, 166_000_000_000);

        let (eth, btc) = oracle.get_consensus_price().unwrap();

        // Median resists the outlier (should be ~$2510, not $5000)
        assert!((2_500_000_000..=2_520_000_000).contains(&eth));
        assert!((83_000_000_000..=83_200_000_000).contains(&btc));

        // Detect the outlier
        let outliers = oracle.detect_outliers();
        assert_eq!(outliers.len(), 1);
        assert!(outliers[0].contains("EVIL"));
    }

    #[test]
    fn test_insufficient_submissions() {
        let mut oracle = OracleConsensus::with_config(60, 3, 2000); // Require 3 submissions, 20% threshold

        // Only 1 submission (insufficient)
        oracle.submit_price("VAL1".to_string(), 2_500_000_000, 83_000_000_000);

        let result = oracle.get_consensus_price();
        assert!(result.is_none());
    }

    #[test]
    fn test_submission_window_expiry() {
        let mut oracle = OracleConsensus::with_config(1, 2, 2000); // 1 second window

        oracle.submit_price("VAL1".to_string(), 2_500_000_000, 83_000_000_000);
        oracle.submit_price("VAL2".to_string(), 2_510_000_000, 83_100_000_000);

        // Should work immediately
        assert!(oracle.get_consensus_price().is_some());

        // Wait for expiry (simulation - in real code this would sleep)
        // Since we can't sleep in tests, we manually set old timestamp
        for submission in oracle.submissions.values_mut() {
            submission.timestamp -= 2; // Make it 2 seconds old
        }

        // Should fail now (expired)
        assert!(oracle.get_consensus_price().is_none());
    }

    #[test]
    fn test_cleanup_old_submissions() {
        let mut oracle = OracleConsensus::with_config(60, 2, 2000);

        oracle.submit_price("VAL1".to_string(), 2_500_000_000, 83_000_000_000);
        oracle.submit_price("VAL2".to_string(), 2_510_000_000, 83_100_000_000);

        assert_eq!(oracle.submissions.len(), 2);

        // Make submissions very old
        for submission in oracle.submissions.values_mut() {
            submission.timestamp -= 200; // 200 seconds old
        }

        oracle.cleanup_old();
        assert_eq!(oracle.submissions.len(), 0);
    }

    #[test]
    fn test_outlier_detection() {
        let mut oracle = OracleConsensus::with_config(60, 2, 1000); // 10% threshold = 1000bp

        // Normal submissions (micro-USD)
        oracle.submit_price("VAL1".to_string(), 2_500_000_000, 83_000_000_000);
        oracle.submit_price("VAL2".to_string(), 2_520_000_000, 83_200_000_000);

        // Outlier (15% higher)
        oracle.submit_price("VAL3".to_string(), 2_875_000_000, 95_450_000_000);

        let outliers = oracle.detect_outliers();
        assert_eq!(outliers.len(), 1);
        assert_eq!(outliers[0], "VAL3");
    }
}
