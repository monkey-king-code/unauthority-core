// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BENCHMARK SUITE — los-crypto
//
// Measures performance of cryptographic operations.
// CRITICAL: These are the bottleneck for TPS (every tx needs sign+verify).
//
// ZERO production code changes — benchmark-only file.
// Run: cargo bench -p los-crypto
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use los_crypto::{
    generate_keypair, generate_keypair_from_seed, public_key_to_address,
    sign_message, validate_address, verify_signature,
};

// ─────────────────────────────────────────────────────────────────
// KEY GENERATION BENCHMARKS
// ─────────────────────────────────────────────────────────────────

fn bench_keypair_generation(c: &mut Criterion) {
    c.bench_function("crypto/generate_keypair (Dilithium5)", |b| {
        b.iter(|| black_box(generate_keypair()))
    });
}

fn bench_deterministic_keygen(c: &mut Criterion) {
    let seed = [42u8; 64]; // BIP39 seed (64 bytes)
    c.bench_function("crypto/generate_keypair_from_seed", |b| {
        b.iter(|| black_box(generate_keypair_from_seed(&seed)))
    });
}

// ─────────────────────────────────────────────────────────────────
// SIGNATURE BENCHMARKS (TPS bottleneck)
// ─────────────────────────────────────────────────────────────────

fn bench_sign(c: &mut Criterion) {
    let kp = generate_keypair();
    let mut group = c.benchmark_group("crypto/sign");

    for msg_size in [32, 256, 1024, 4096] {
        let message = vec![0xAB; msg_size];
        group.bench_with_input(
            BenchmarkId::new("Dilithium5", msg_size),
            &message,
            |b, msg| {
                b.iter(|| black_box(sign_message(msg, &kp.secret_key).unwrap()))
            },
        );
    }
    group.finish();
}

fn bench_verify(c: &mut Criterion) {
    let kp = generate_keypair();
    let mut group = c.benchmark_group("crypto/verify");

    for msg_size in [32, 256, 1024, 4096] {
        let message = vec![0xAB; msg_size];
        let sig = sign_message(&message, &kp.secret_key).unwrap();

        group.bench_with_input(
            BenchmarkId::new("Dilithium5", msg_size),
            &(message, sig),
            |b, (msg, signature)| {
                b.iter(|| black_box(verify_signature(msg, signature, &kp.public_key)))
            },
        );
    }
    group.finish();
}

// ─────────────────────────────────────────────────────────────────
// ADDRESS BENCHMARKS
// ─────────────────────────────────────────────────────────────────

fn bench_address_derivation(c: &mut Criterion) {
    let kp = generate_keypair();
    c.bench_function("crypto/public_key_to_address", |b| {
        b.iter(|| black_box(public_key_to_address(&kp.public_key)))
    });
}

fn bench_address_validation(c: &mut Criterion) {
    let kp = generate_keypair();
    let addr = public_key_to_address(&kp.public_key);

    let mut group = c.benchmark_group("crypto/validate_address");
    group.bench_function("valid", |b| {
        b.iter(|| black_box(validate_address(&addr)))
    });
    group.bench_function("invalid", |b| {
        b.iter(|| black_box(validate_address("LOSinvalid123456789")))
    });
    group.finish();
}

// ─────────────────────────────────────────────────────────────────
// THROUGHPUT ESTIMATE (Sign+Verify per second = theoretical max TPS)
// ─────────────────────────────────────────────────────────────────

fn bench_sign_verify_roundtrip(c: &mut Criterion) {
    let kp = generate_keypair();
    let message = vec![0xAB; 256]; // Typical transaction signing hash size

    c.bench_function("crypto/sign_then_verify (TPS estimate)", |b| {
        b.iter(|| {
            let sig = sign_message(&message, &kp.secret_key).unwrap();
            black_box(verify_signature(&message, &sig, &kp.public_key))
        })
    });
}

// ─────────────────────────────────────────────────────────────────

criterion_group!(
    benches,
    bench_keypair_generation,
    bench_deterministic_keygen,
    bench_sign,
    bench_verify,
    bench_address_derivation,
    bench_address_validation,
    bench_sign_verify_roundtrip,
);
criterion_main!(benches);
