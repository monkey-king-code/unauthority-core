// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UNAUTHORITY (LOS) - METRICS MODULE
//
// Prometheus-compatible metrics for production monitoring.
// Exposes counters, gauges, and histograms via /metrics endpoint.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use prometheus::{
    Counter, Encoder, Gauge, Histogram, HistogramOpts, IntCounter, IntGauge, Opts, Registry,
    TextEncoder,
};
use std::sync::Arc;

/// Global metrics registry for all node metrics
pub struct LosMetrics {
    registry: Registry,

    // Blockchain metrics
    pub blocks_total: IntCounter,
    pub accounts_total: IntGauge,
    pub transactions_total: IntCounter,
    pub genesis_blocks_total: IntCounter,
    pub send_blocks_total: IntCounter,
    pub receive_blocks_total: IntCounter,
    pub mint_blocks_total: IntCounter,
    pub contract_blocks_total: IntCounter,

    // Database metrics
    pub db_size_bytes: Gauge,
    pub db_blocks_count: IntGauge,
    pub db_accounts_count: IntGauge,
    pub db_save_duration_seconds: Histogram,
    pub db_load_duration_seconds: Histogram,

    // Consensus metrics
    pub consensus_rounds_total: IntCounter,
    pub consensus_failures_total: IntCounter,
    pub consensus_latency_seconds: Histogram,
    pub active_validators: IntGauge,
    pub validator_votes_total: IntCounter,

    // Distribution metrics (PoW mining)
    pub mint_remaining_supply: Gauge,

    // Network metrics
    pub connected_peers: IntGauge,
    pub p2p_messages_received_total: IntCounter,
    pub p2p_messages_sent_total: IntCounter,
    pub p2p_bytes_received_total: Counter,
    pub p2p_bytes_sent_total: Counter,

    // API metrics
    pub api_requests_total: IntCounter,
    pub api_errors_total: IntCounter,
    pub api_request_duration_seconds: Histogram,
    pub grpc_requests_total: IntCounter,
    pub grpc_errors_total: IntCounter,

    // Rate limiter metrics
    pub rate_limit_rejections_total: IntCounter,
    pub rate_limit_active_ips: IntGauge,

    // Slashing metrics
    pub slashing_events_total: IntCounter,
    pub slashing_total_amount: Counter,

    // Smart contract metrics
    pub contracts_deployed_total: IntCounter,
    pub contract_executions_total: IntCounter,
    pub contract_gas_used_total: Counter,

    // Tor Hidden Service Health metrics
    /// 1 = own .onion address is reachable via Tor SOCKS5, 0 = unreachable
    pub tor_onion_reachable: IntGauge,
    /// Consecutive self-ping failures (resets to 0 on success)
    pub tor_consecutive_failures: IntGauge,
    /// Total self-ping attempts
    pub tor_self_ping_total: IntCounter,
    /// Total self-ping failures
    pub tor_self_ping_failures_total: IntCounter,
}

impl LosMetrics {
    /// Create new metrics registry with all LOS node metrics
    pub fn new() -> Result<Arc<Self>, Box<dyn std::error::Error>> {
        let registry = Registry::new();

        // Blockchain metrics
        let blocks_total = IntCounter::with_opts(Opts::new(
            "los_blocks_total",
            "Total number of blocks in blockchain",
        ))?;
        registry.register(Box::new(blocks_total.clone()))?;

        let accounts_total =
            IntGauge::with_opts(Opts::new("los_accounts_total", "Total number of accounts"))?;
        registry.register(Box::new(accounts_total.clone()))?;

        let transactions_total = IntCounter::with_opts(Opts::new(
            "los_transactions_total",
            "Total number of transactions processed",
        ))?;
        registry.register(Box::new(transactions_total.clone()))?;

        let genesis_blocks_total = IntCounter::with_opts(Opts::new(
            "los_genesis_blocks_total",
            "Number of genesis blocks",
        ))?;
        registry.register(Box::new(genesis_blocks_total.clone()))?;

        let send_blocks_total =
            IntCounter::with_opts(Opts::new("los_send_blocks_total", "Number of send blocks"))?;
        registry.register(Box::new(send_blocks_total.clone()))?;

        let receive_blocks_total = IntCounter::with_opts(Opts::new(
            "los_receive_blocks_total",
            "Number of receive blocks",
        ))?;
        registry.register(Box::new(receive_blocks_total.clone()))?;

        let mint_blocks_total = IntCounter::with_opts(Opts::new(
            "los_mint_blocks_total",
            "Number of mint blocks (PoW mining)",
        ))?;
        registry.register(Box::new(mint_blocks_total.clone()))?;

        let contract_blocks_total = IntCounter::with_opts(Opts::new(
            "los_contract_blocks_total",
            "Number of smart contract blocks",
        ))?;
        registry.register(Box::new(contract_blocks_total.clone()))?;

        // Database metrics
        let db_size_bytes =
            Gauge::with_opts(Opts::new("los_db_size_bytes", "Database size in bytes"))?;
        registry.register(Box::new(db_size_bytes.clone()))?;

        let db_blocks_count = IntGauge::with_opts(Opts::new(
            "los_db_blocks_count",
            "Number of blocks stored in database",
        ))?;
        registry.register(Box::new(db_blocks_count.clone()))?;

        let db_accounts_count = IntGauge::with_opts(Opts::new(
            "los_db_accounts_count",
            "Number of accounts stored in database",
        ))?;
        registry.register(Box::new(db_accounts_count.clone()))?;

        let db_save_duration_seconds = Histogram::with_opts(
            HistogramOpts::new(
                "los_db_save_duration_seconds",
                "Database save operation latency",
            )
            .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0]),
        )?;
        registry.register(Box::new(db_save_duration_seconds.clone()))?;

        let db_load_duration_seconds = Histogram::with_opts(
            HistogramOpts::new(
                "los_db_load_duration_seconds",
                "Database load operation latency",
            )
            .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0]),
        )?;
        registry.register(Box::new(db_load_duration_seconds.clone()))?;

        // Consensus metrics
        let consensus_rounds_total = IntCounter::with_opts(Opts::new(
            "los_consensus_rounds_total",
            "Total consensus rounds completed",
        ))?;
        registry.register(Box::new(consensus_rounds_total.clone()))?;

        let consensus_failures_total = IntCounter::with_opts(Opts::new(
            "los_consensus_failures_total",
            "Total consensus failures",
        ))?;
        registry.register(Box::new(consensus_failures_total.clone()))?;

        let consensus_latency_seconds = Histogram::with_opts(
            HistogramOpts::new(
                "los_consensus_latency_seconds",
                "Consensus finality latency",
            )
            .buckets(vec![0.5, 1.0, 2.0, 3.0, 5.0, 10.0]),
        )?;
        registry.register(Box::new(consensus_latency_seconds.clone()))?;

        let active_validators = IntGauge::with_opts(Opts::new(
            "los_active_validators",
            "Number of active validators",
        ))?;
        registry.register(Box::new(active_validators.clone()))?;

        let validator_votes_total = IntCounter::with_opts(Opts::new(
            "los_validator_votes_total",
            "Total validator votes cast",
        ))?;
        registry.register(Box::new(validator_votes_total.clone()))?;

        // Distribution metrics (PoW mining)
        let mint_remaining_supply = Gauge::with_opts(Opts::new(
            "los_mint_remaining_supply",
            "Remaining LOS supply for PoW mining distribution",
        ))?;
        registry.register(Box::new(mint_remaining_supply.clone()))?;

        // Network metrics
        let connected_peers = IntGauge::with_opts(Opts::new(
            "los_connected_peers",
            "Number of connected P2P peers",
        ))?;
        registry.register(Box::new(connected_peers.clone()))?;

        let p2p_messages_received_total = IntCounter::with_opts(Opts::new(
            "los_p2p_messages_received_total",
            "Total P2P messages received",
        ))?;
        registry.register(Box::new(p2p_messages_received_total.clone()))?;

        let p2p_messages_sent_total = IntCounter::with_opts(Opts::new(
            "los_p2p_messages_sent_total",
            "Total P2P messages sent",
        ))?;
        registry.register(Box::new(p2p_messages_sent_total.clone()))?;

        let p2p_bytes_received_total = Counter::with_opts(Opts::new(
            "los_p2p_bytes_received_total",
            "Total bytes received via P2P",
        ))?;
        registry.register(Box::new(p2p_bytes_received_total.clone()))?;

        let p2p_bytes_sent_total = Counter::with_opts(Opts::new(
            "los_p2p_bytes_sent_total",
            "Total bytes sent via P2P",
        ))?;
        registry.register(Box::new(p2p_bytes_sent_total.clone()))?;

        // API metrics
        let api_requests_total = IntCounter::with_opts(Opts::new(
            "los_api_requests_total",
            "Total REST API requests",
        ))?;
        registry.register(Box::new(api_requests_total.clone()))?;

        let api_errors_total =
            IntCounter::with_opts(Opts::new("los_api_errors_total", "Total REST API errors"))?;
        registry.register(Box::new(api_errors_total.clone()))?;

        let api_request_duration_seconds = Histogram::with_opts(
            HistogramOpts::new(
                "los_api_request_duration_seconds",
                "REST API request latency",
            )
            .buckets(vec![0.001, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0]),
        )?;
        registry.register(Box::new(api_request_duration_seconds.clone()))?;

        let grpc_requests_total =
            IntCounter::with_opts(Opts::new("los_grpc_requests_total", "Total gRPC requests"))?;
        registry.register(Box::new(grpc_requests_total.clone()))?;

        let grpc_errors_total =
            IntCounter::with_opts(Opts::new("los_grpc_errors_total", "Total gRPC errors"))?;
        registry.register(Box::new(grpc_errors_total.clone()))?;

        // Rate limiter metrics
        let rate_limit_rejections_total = IntCounter::with_opts(Opts::new(
            "los_rate_limit_rejections_total",
            "Total rate limit rejections",
        ))?;
        registry.register(Box::new(rate_limit_rejections_total.clone()))?;

        let rate_limit_active_ips = IntGauge::with_opts(Opts::new(
            "los_rate_limit_active_ips",
            "Number of IPs being tracked by rate limiter",
        ))?;
        registry.register(Box::new(rate_limit_active_ips.clone()))?;

        // Slashing metrics
        let slashing_events_total = IntCounter::with_opts(Opts::new(
            "los_slashing_events_total",
            "Total slashing events",
        ))?;
        registry.register(Box::new(slashing_events_total.clone()))?;

        let slashing_total_amount = Counter::with_opts(Opts::new(
            "los_slashing_total_amount",
            "Total LOS slashed (in CIL)",
        ))?;
        registry.register(Box::new(slashing_total_amount.clone()))?;

        // Smart contract metrics
        let contracts_deployed_total = IntCounter::with_opts(Opts::new(
            "los_contracts_deployed_total",
            "Total smart contracts deployed",
        ))?;
        registry.register(Box::new(contracts_deployed_total.clone()))?;

        let contract_executions_total = IntCounter::with_opts(Opts::new(
            "los_contract_executions_total",
            "Total contract executions",
        ))?;
        registry.register(Box::new(contract_executions_total.clone()))?;

        let contract_gas_used_total = Counter::with_opts(Opts::new(
            "los_contract_gas_used_total",
            "Total gas consumed by contracts",
        ))?;
        registry.register(Box::new(contract_gas_used_total.clone()))?;

        // Tor Hidden Service Health metrics
        let tor_onion_reachable = IntGauge::with_opts(Opts::new(
            "los_tor_onion_reachable",
            "Whether this node's own .onion hidden service is reachable (1=yes, 0=no)",
        ))?;
        registry.register(Box::new(tor_onion_reachable.clone()))?;

        let tor_consecutive_failures = IntGauge::with_opts(Opts::new(
            "los_tor_consecutive_failures",
            "Consecutive Tor self-ping failures (resets to 0 on success)",
        ))?;
        registry.register(Box::new(tor_consecutive_failures.clone()))?;

        let tor_self_ping_total = IntCounter::with_opts(Opts::new(
            "los_tor_self_ping_total",
            "Total Tor self-ping attempts",
        ))?;
        registry.register(Box::new(tor_self_ping_total.clone()))?;

        let tor_self_ping_failures_total = IntCounter::with_opts(Opts::new(
            "los_tor_self_ping_failures_total",
            "Total Tor self-ping failures",
        ))?;
        registry.register(Box::new(tor_self_ping_failures_total.clone()))?;

        Ok(Arc::new(Self {
            registry,
            blocks_total,
            accounts_total,
            transactions_total,
            genesis_blocks_total,
            send_blocks_total,
            receive_blocks_total,
            mint_blocks_total,
            contract_blocks_total,
            db_size_bytes,
            db_blocks_count,
            db_accounts_count,
            db_save_duration_seconds,
            db_load_duration_seconds,
            consensus_rounds_total,
            consensus_failures_total,
            consensus_latency_seconds,
            active_validators,
            validator_votes_total,
            mint_remaining_supply,
            connected_peers,
            p2p_messages_received_total,
            p2p_messages_sent_total,
            p2p_bytes_received_total,
            p2p_bytes_sent_total,
            api_requests_total,
            api_errors_total,
            api_request_duration_seconds,
            grpc_requests_total,
            grpc_errors_total,
            rate_limit_rejections_total,
            rate_limit_active_ips,
            slashing_events_total,
            slashing_total_amount,
            contracts_deployed_total,
            contract_executions_total,
            contract_gas_used_total,
            tor_onion_reachable,
            tor_consecutive_failures,
            tor_self_ping_total,
            tor_self_ping_failures_total,
        }))
    }

    /// Export all metrics in Prometheus text format
    pub fn export(&self) -> Result<String, Box<dyn std::error::Error>> {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut buffer = Vec::new();
        encoder.encode(&metric_families, &mut buffer)?;
        Ok(String::from_utf8(buffer)?)
    }

    /// Update blockchain metrics from current ledger state
    pub fn update_blockchain_metrics(&self, ledger: &los_core::Ledger) {
        self.blocks_total.reset();
        self.blocks_total.inc_by(ledger.blocks.len() as u64);

        self.accounts_total.set(ledger.accounts.len() as i64);

        // Count active validators (registered + staked above minimum)
        let validator_count = ledger
            .accounts
            .iter()
            .filter(|(_, a)| a.is_validator && a.balance >= los_core::MIN_VALIDATOR_STAKE_CIL)
            .count();
        self.active_validators.set(validator_count as i64);

        // Count block types
        let mut send_count = 0;
        let mut receive_count = 0;
        let mut mint_count = 0;

        for block in ledger.blocks.values() {
            match block.block_type {
                los_core::BlockType::Send => send_count += 1,
                los_core::BlockType::Receive => receive_count += 1,
                los_core::BlockType::Mint => mint_count += 1,
                los_core::BlockType::Change => {} // Skip change blocks for now
                los_core::BlockType::Slash => {} // Slash blocks counted separately via slashing manager
                los_core::BlockType::ContractDeploy => {} // Counted via contracts_deployed_total
                los_core::BlockType::ContractCall => {} // Counted via contract_executions_total
            }
        }

        self.send_blocks_total.reset();
        self.send_blocks_total.inc_by(send_count);

        self.receive_blocks_total.reset();
        self.receive_blocks_total.inc_by(receive_count);

        self.mint_blocks_total.reset();
        self.mint_blocks_total.inc_by(mint_count);

        // PoW mining distribution metrics
        self.mint_remaining_supply
            .set(ledger.distribution.remaining_supply as f64);
    }

    /// Update database metrics from database stats
    pub fn update_db_metrics(&self, stats: &crate::db::DatabaseStats) {
        self.db_size_bytes.set(stats.size_on_disk as f64);
        self.db_blocks_count.set(stats.blocks_count as i64);
        self.db_accounts_count.set(stats.accounts_count as i64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_creation() {
        let metrics = LosMetrics::new().unwrap();
        assert_eq!(metrics.blocks_total.get(), 0);
        assert_eq!(metrics.accounts_total.get(), 0);
    }

    #[test]
    fn test_metrics_export() {
        let metrics = LosMetrics::new().unwrap();
        metrics.blocks_total.inc_by(100);
        metrics.accounts_total.set(50);

        let output = metrics.export().unwrap();
        assert!(output.contains("los_blocks_total"));
        assert!(output.contains("los_accounts_total"));
        assert!(output.contains("100"));
        assert!(output.contains("50"));
    }

    #[test]
    fn test_counter_increment() {
        let metrics = LosMetrics::new().unwrap();

        metrics.transactions_total.inc();
        metrics.transactions_total.inc();
        metrics.transactions_total.inc_by(5);

        assert_eq!(metrics.transactions_total.get(), 7);
    }

    #[test]
    fn test_gauge_operations() {
        let metrics = LosMetrics::new().unwrap();

        metrics.connected_peers.set(10);
        assert_eq!(metrics.connected_peers.get(), 10);

        metrics.connected_peers.inc();
        assert_eq!(metrics.connected_peers.get(), 11);

        metrics.connected_peers.dec();
        assert_eq!(metrics.connected_peers.get(), 10);
    }

    #[test]
    fn test_histogram_observe() {
        let metrics = LosMetrics::new().unwrap();

        metrics.consensus_latency_seconds.observe(1.5);
        metrics.consensus_latency_seconds.observe(2.8);
        metrics.consensus_latency_seconds.observe(0.5);

        // Histogram should have recorded 3 observations
        let export = metrics.export().unwrap();
        assert!(export.contains("los_consensus_latency_seconds"));
    }
}
