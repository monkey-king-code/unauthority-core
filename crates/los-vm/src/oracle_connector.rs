// Oracle Connector for External Price Feeds (Exchange Integration)
// Allows smart contracts to fetch real-time LOS price from exchanges
//
// MAINNET SAFETY: All prices stored as integer micro-USD (u64).
// 1 USD = 1_000_000 micro-USD. This ensures deterministic behavior
// across all CPU architectures (no f64 rounding differences).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Micro-USD per 1 USD (10^6 precision)
pub const MICRO_USD: u64 = 1_000_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangePrice {
    pub exchange: String,     // "binance", "coinbase", "kraken"
    pub pair: String,         // "LOS/USDT", "LOS/BTC"
    pub price_micro_usd: u64, // Price in micro-USD (1 USD = 1_000_000)
    pub volume_24h_usd: u64,  // 24h volume in whole USD
    pub timestamp: u64,       // Last update timestamp
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleConsensusPrice {
    pub median_price_micro_usd: u64, // Byzantine-resistant median in micro-USD
    pub sources: Vec<ExchangePrice>,
    pub confidence_bps: u16, // 0-10000 basis points (0.00%-100.00%)
}

/// Smart Contract Oracle Interface
/// This is what payment smart contracts will call
pub trait PriceOracle {
    /// Get current LOS price in micro-USD (median from multiple exchanges)
    fn get_los_price_micro_usd(&self) -> Result<u64, String>;

    /// Get LOS price from specific exchange in micro-USD
    fn get_los_price_from_exchange(&self, exchange: &str) -> Result<u64, String>;

    /// Get full consensus data (all sources + median)
    fn get_oracle_consensus(&self) -> Result<OracleConsensusPrice, String>;

    /// Verify if price is within acceptable deviation (anti-manipulation)
    /// Returns true if price_micro_usd is within 10% of oracle consensus
    fn verify_price_sanity(&self, price_micro_usd: u64) -> Result<bool, String>;
}

/// Implementation (used by UVM when contract calls oracle)
pub struct ExchangeOracle {
    price_feeds: BTreeMap<String, ExchangePrice>,
    last_update: u64,
}

impl ExchangeOracle {
    pub fn new() -> Self {
        Self {
            price_feeds: BTreeMap::new(),
            last_update: 0,
        }
    }

    /// Fetch prices from multiple exchanges (called by background worker)
    pub async fn fetch_exchange_prices(&mut self) -> Result<(), String> {
        // Example: Fetch from Binance API
        let binance_price = self.fetch_from_binance().await?;
        self.price_feeds
            .insert("binance".to_string(), binance_price);

        // Example: Fetch from Coinbase API
        let coinbase_price = self.fetch_from_coinbase().await?;
        self.price_feeds
            .insert("coinbase".to_string(), coinbase_price);

        // Example: Fetch from Kraken API
        let kraken_price = self.fetch_from_kraken().await?;
        self.price_feeds.insert("kraken".to_string(), kraken_price);

        self.last_update = chrono::Utc::now().timestamp() as u64;
        Ok(())
    }

    async fn fetch_from_binance(&self) -> Result<ExchangePrice, String> {
        // SECURITY: On mainnet builds, stub oracles are disabled.
        // The node-level oracle in main.rs fetches real prices from CoinGecko/CryptoCompare/Kraken.
        // These VM-level stubs exist only for testnet contract testing.
        #[cfg(feature = "mainnet")]
        return Err("VM oracle stubs disabled on mainnet. Use node-level oracle.".to_string());

        #[cfg(not(feature = "mainnet"))]
        Ok(ExchangePrice {
            exchange: "binance".to_string(),
            pair: "LOS/USDT".to_string(),
            price_micro_usd: 10_000, // 0.01 USD = 10,000 micro-USD
            volume_24h_usd: 1_000_000,
            timestamp: chrono::Utc::now().timestamp() as u64,
        })
    }

    async fn fetch_from_coinbase(&self) -> Result<ExchangePrice, String> {
        #[cfg(feature = "mainnet")]
        return Err("VM oracle stubs disabled on mainnet. Use node-level oracle.".to_string());

        #[cfg(not(feature = "mainnet"))]
        Ok(ExchangePrice {
            exchange: "coinbase".to_string(),
            pair: "LOS-USD".to_string(),
            price_micro_usd: 9_900, // 0.0099 USD
            volume_24h_usd: 500_000,
            timestamp: chrono::Utc::now().timestamp() as u64,
        })
    }

    async fn fetch_from_kraken(&self) -> Result<ExchangePrice, String> {
        #[cfg(feature = "mainnet")]
        return Err("VM oracle stubs disabled on mainnet. Use node-level oracle.".to_string());

        #[cfg(not(feature = "mainnet"))]
        Ok(ExchangePrice {
            exchange: "kraken".to_string(),
            pair: "LOSUSD".to_string(),
            price_micro_usd: 10_100, // 0.0101 USD
            volume_24h_usd: 750_000,
            timestamp: chrono::Utc::now().timestamp() as u64,
        })
    }
}

impl PriceOracle for ExchangeOracle {
    fn get_los_price_micro_usd(&self) -> Result<u64, String> {
        if self.price_feeds.is_empty() {
            return Err("No price feeds available".to_string());
        }

        // Calculate median price (Byzantine-resistant) — integer only
        let mut prices: Vec<u64> = self
            .price_feeds
            .values()
            .map(|p| p.price_micro_usd)
            .filter(|p| *p > 0) // Filter zero prices
            .collect();

        if prices.is_empty() {
            return Err("No valid prices available".to_string());
        }
        prices.sort_unstable();
        let median = prices[prices.len() / 2];

        Ok(median)
    }

    fn get_los_price_from_exchange(&self, exchange: &str) -> Result<u64, String> {
        self.price_feeds
            .get(exchange)
            .map(|p| p.price_micro_usd)
            .ok_or_else(|| format!("Exchange {} not found", exchange))
    }

    fn get_oracle_consensus(&self) -> Result<OracleConsensusPrice, String> {
        let median = self.get_los_price_micro_usd()?;

        if median == 0 {
            return Err("Invalid median price: zero".to_string());
        }

        // Calculate confidence — integer math with basis points
        let prices: Vec<u64> = self
            .price_feeds
            .values()
            .map(|p| p.price_micro_usd)
            .filter(|p| *p > 0)
            .collect();
        let max_price = prices.iter().copied().max().unwrap_or(0);
        let min_price = prices.iter().copied().min().unwrap_or(0);

        // deviation_bps = ((max - min) * 10000) / median, clamped to u16 range
        let deviation_bps = if median > 0 {
            ((max_price.saturating_sub(min_price) as u128 * 10_000) / median as u128).min(10_000)
                as u16
        } else {
            10_000 // 100% deviation if median is 0
        };
        // confidence = 10000 - deviation (capped at 0)
        let confidence_bps = 10_000u16.saturating_sub(deviation_bps);

        Ok(OracleConsensusPrice {
            median_price_micro_usd: median,
            sources: self.price_feeds.values().cloned().collect(),
            confidence_bps,
        })
    }

    fn verify_price_sanity(&self, price_micro_usd: u64) -> Result<bool, String> {
        let median = self.get_los_price_micro_usd()?;
        if median == 0 {
            return Err("Invalid median price: zero".to_string());
        }

        // Calculate deviation in basis points (integer math)
        let diff = price_micro_usd.abs_diff(median);
        let deviation_bps = (diff as u128 * 10_000 / median as u128) as u64;

        // Reject if price deviates more than 10% (1000 bps) from oracle consensus
        if deviation_bps > 1_000 {
            return Ok(false);
        }

        Ok(true)
    }
}

impl Default for ExchangeOracle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_median_price_calculation() {
        let mut oracle = ExchangeOracle::new();

        oracle.price_feeds.insert(
            "binance".to_string(),
            ExchangePrice {
                exchange: "binance".to_string(),
                pair: "LOS/USDT".to_string(),
                price_micro_usd: 10_000, // 0.01 USD
                volume_24h_usd: 1_000_000,
                timestamp: 0,
            },
        );

        oracle.price_feeds.insert(
            "coinbase".to_string(),
            ExchangePrice {
                exchange: "coinbase".to_string(),
                pair: "LOS-USD".to_string(),
                price_micro_usd: 11_000, // 0.011 USD
                volume_24h_usd: 500_000,
                timestamp: 0,
            },
        );

        oracle.price_feeds.insert(
            "kraken".to_string(),
            ExchangePrice {
                exchange: "kraken".to_string(),
                pair: "LOSUSD".to_string(),
                price_micro_usd: 10_500, // 0.0105 USD
                volume_24h_usd: 750_000,
                timestamp: 0,
            },
        );

        let median = oracle.get_los_price_micro_usd().unwrap();
        assert_eq!(median, 10_500); // Median of [10000, 10500, 11000]
    }

    #[test]
    fn test_price_sanity_check() {
        let mut oracle = ExchangeOracle::new();

        oracle.price_feeds.insert(
            "binance".to_string(),
            ExchangePrice {
                exchange: "binance".to_string(),
                pair: "LOS/USDT".to_string(),
                price_micro_usd: 10_000,
                volume_24h_usd: 1_000_000,
                timestamp: 0,
            },
        );

        // Test within range: 10500 vs median 10000 = 5% → should pass
        assert!(oracle.verify_price_sanity(10_500).unwrap());

        // Test outside range: 20000 vs median 10000 = 100% → should fail
        assert!(!oracle.verify_price_sanity(20_000).unwrap());
    }

    #[test]
    fn test_oracle_consensus() {
        let mut oracle = ExchangeOracle::new();

        // Add similar prices (high confidence expected)
        oracle.price_feeds.insert(
            "binance".to_string(),
            ExchangePrice {
                exchange: "binance".to_string(),
                pair: "LOS/USDT".to_string(),
                price_micro_usd: 10_000,
                volume_24h_usd: 1_000_000,
                timestamp: 0,
            },
        );

        oracle.price_feeds.insert(
            "coinbase".to_string(),
            ExchangePrice {
                exchange: "coinbase".to_string(),
                pair: "LOS-USD".to_string(),
                price_micro_usd: 10_100,
                volume_24h_usd: 500_000,
                timestamp: 0,
            },
        );

        let consensus = oracle.get_oracle_consensus().unwrap();
        assert!(consensus.confidence_bps > 9000); // >90% confidence
        assert_eq!(consensus.sources.len(), 2);
    }
}
