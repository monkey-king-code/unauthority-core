// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BENCHMARK SUITE — los-core
//
// Measures performance of core blockchain operations.
// ZERO production code changes — benchmark-only file.
// Run: cargo bench -p los-core
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use los_core::pow_mint::{compute_mining_hash, count_leading_zero_bits, MiningState};
use los_core::{Block, BlockType, Ledger, AccountState, CIL_PER_LOS};

// ─────────────────────────────────────────────────────────────────
// BLOCK HASH BENCHMARKS
// ─────────────────────────────────────────────────────────────────

fn bench_signing_hash(c: &mut Criterion) {
    let block = Block {
        account: "LOSXtestBenchAccount123456789".to_string(),
        previous: "a".repeat(64),
        block_type: BlockType::Send,
        amount: 1_000 * CIL_PER_LOS,
        link: "LOSXtestBenchRecipient987654".to_string(),
        signature: "b".repeat(128),
        public_key: "c".repeat(128),
        work: 12345,
        timestamp: 1_700_000_000,
        fee: 100_000,
    };

    c.bench_function("block/signing_hash", |b| {
        b.iter(|| black_box(block.signing_hash()))
    });
}

fn bench_calculate_hash(c: &mut Criterion) {
    let block = Block {
        account: "LOSXtestBenchAccount123456789".to_string(),
        previous: "a".repeat(64),
        block_type: BlockType::Send,
        amount: 1_000 * CIL_PER_LOS,
        link: "LOSXtestBenchRecipient987654".to_string(),
        signature: "b".repeat(128),
        public_key: "c".repeat(128),
        work: 12345,
        timestamp: 1_700_000_000,
        fee: 100_000,
    };

    c.bench_function("block/calculate_hash", |b| {
        b.iter(|| black_box(block.calculate_hash()))
    });
}

// ─────────────────────────────────────────────────────────────────
// MINING HASH BENCHMARKS (PoW throughput — critical for miners)
// ─────────────────────────────────────────────────────────────────

fn bench_mining_hash(c: &mut Criterion) {
    let address = "LOSXbenchMiner123456789012345";
    let epoch = 100u64;

    c.bench_function("pow/compute_mining_hash", |b| {
        let mut nonce = 0u64;
        b.iter(|| {
            nonce += 1;
            black_box(compute_mining_hash(address, epoch, nonce))
        })
    });
}

fn bench_leading_zero_bits(c: &mut Criterion) {
    let hash = compute_mining_hash("LOSXbench123", 1, 42);

    c.bench_function("pow/count_leading_zero_bits", |b| {
        b.iter(|| black_box(count_leading_zero_bits(&hash)))
    });
}

fn bench_mining_epoch_reward(c: &mut Criterion) {
    let mut group = c.benchmark_group("pow/epoch_reward");
    for epoch in [0u64, 100, 8760, 17520, 43800, 87600] {
        group.bench_with_input(BenchmarkId::from_parameter(epoch), &epoch, |b, &e| {
            b.iter(|| black_box(MiningState::epoch_reward_cil(e)))
        });
    }
    group.finish();
}

// ─────────────────────────────────────────────────────────────────
// STATE ROOT BENCHMARKS (consensus-critical — all validators compute)
// ─────────────────────────────────────────────────────────────────

fn bench_state_root(c: &mut Criterion) {
    let mut group = c.benchmark_group("ledger/state_root");

    for num_accounts in [100, 1_000, 10_000, 50_000] {
        let mut ledger = Ledger::new();
        for i in 0..num_accounts {
            ledger.accounts.insert(
                format!("LOSaddr{:08}", i),
                AccountState {
                    head: format!("{:064x}", i),
                    balance: (i as u128 + 1) * CIL_PER_LOS,
                    block_count: i as u64,
                    is_validator: false,
                },
            );
        }

        group.bench_with_input(
            BenchmarkId::new("accounts", num_accounts),
            &num_accounts,
            |b, _| b.iter(|| black_box(ledger.compute_state_root())),
        );
    }
    group.finish();
}

// ─────────────────────────────────────────────────────────────────
// DIFFICULTY ADJUSTMENT BENCHMARK
// ─────────────────────────────────────────────────────────────────

fn bench_difficulty_adjustment(c: &mut Criterion) {
    c.bench_function("pow/advance_epoch_50_miners", |b| {
        b.iter(|| {
            let mut state = MiningState::new(1_700_000_000);
            for i in 0..50 {
                state.current_epoch_miners.insert(format!("LOSminer{}", i));
            }
            black_box(state.advance_epoch(1));
        })
    });
}

// ─────────────────────────────────────────────────────────────────

criterion_group!(
    benches,
    bench_signing_hash,
    bench_calculate_hash,
    bench_mining_hash,
    bench_leading_zero_bits,
    bench_mining_epoch_reward,
    bench_state_root,
    bench_difficulty_adjustment,
);
criterion_main!(benches);
