// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BENCHMARK SUITE — los-consensus
//
// Measures performance of consensus operations.
// ZERO production code changes — benchmark-only file.
// Run: cargo bench -p los-consensus
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use los_consensus::abft::{ABFTConsensus, Block as AbftBlock};
use los_consensus::voting::{calculate_voting_power, ValidatorVote, VotingSystem};

// ─────────────────────────────────────────────────────────────────
// VOTING POWER BENCHMARKS
// ─────────────────────────────────────────────────────────────────

fn bench_calculate_voting_power(c: &mut Criterion) {
    let cil_per_los: u128 = 100_000_000_000;
    let stakes = vec![
        ("1_LOS", 1 * cil_per_los),
        ("1000_LOS", 1_000 * cil_per_los),
        ("100000_LOS", 100_000 * cil_per_los),
        ("1M_LOS", 1_000_000 * cil_per_los),
    ];

    let mut group = c.benchmark_group("voting/calculate_power");
    for (name, stake) in &stakes {
        group.bench_with_input(BenchmarkId::new("linear", name), stake, |b, &s| {
            b.iter(|| black_box(calculate_voting_power(s)))
        });
    }
    group.finish();
}

fn bench_validator_vote_creation(c: &mut Criterion) {
    let cil_per_los: u128 = 100_000_000_000;
    c.bench_function("voting/new_validator_vote", |b| {
        b.iter(|| {
            black_box(ValidatorVote::new(
                "LOSXtestValidator123456".to_string(),
                1_000 * cil_per_los,
                "propose_123".to_string(),
                true,
            ))
        })
    });
}

// ─────────────────────────────────────────────────────────────────
// aBFT BLOCK HASH BENCHMARKS
// ─────────────────────────────────────────────────────────────────

fn bench_abft_block_hash(c: &mut Criterion) {
    let mut group = c.benchmark_group("abft/block_hash");

    for data_size in [0, 256, 1024, 4096, 65536] {
        let block = AbftBlock {
            height: 1000,
            timestamp: 1_700_000_000,
            data: vec![0xAB; data_size],
            proposer: "validator_0".to_string(),
            parent_hash: "f".repeat(64),
        };

        group.bench_with_input(
            BenchmarkId::new("data_bytes", data_size),
            &data_size,
            |b, _| b.iter(|| black_box(block.calculate_hash())),
        );
    }
    group.finish();
}

// ─────────────────────────────────────────────────────────────────
// CONSENSUS CREATION BENCHMARKS
// ─────────────────────────────────────────────────────────────────

fn bench_consensus_new(c: &mut Criterion) {
    let mut group = c.benchmark_group("abft/consensus_new");

    for num_validators in [4, 10, 20, 50, 100] {
        group.bench_with_input(
            BenchmarkId::new("validators", num_validators),
            &num_validators,
            |b, &n| b.iter(|| black_box(ABFTConsensus::new("validator_0".to_string(), n))),
        );
    }
    group.finish();
}

// ─────────────────────────────────────────────────────────────────
// VOTING POWER SUMMARY (network-wide aggregation)
// ─────────────────────────────────────────────────────────────────

fn bench_voting_summary(c: &mut Criterion) {
    let cil_per_los: u128 = 100_000_000_000;
    let mut group = c.benchmark_group("voting/summary");

    for num_validators in [4usize, 20, 100, 500] {
        group.bench_function(BenchmarkId::new("validators", num_validators), |b| {
            b.iter(|| {
                let mut system = VotingSystem::new();
                for i in 0..num_validators {
                    let _ = system.register_validator(
                        format!("LOSval{:06}", i),
                        ((i as u128) + 1) * 1_000 * cil_per_los,
                        "propose_1".to_string(),
                        true,
                    );
                }
                black_box(system.get_summary())
            })
        });
    }
    group.finish();
}

// ─────────────────────────────────────────────────────────────────

criterion_group!(
    benches,
    bench_calculate_voting_power,
    bench_validator_vote_creation,
    bench_abft_block_hash,
    bench_consensus_new,
    bench_voting_summary,
);
criterion_main!(benches);
