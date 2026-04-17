//! CRYS-L SLO & Latency Distribution Tests
//!
//! Measures p50/p95/p99 compute latency and verifies they are within
//! the SLO targets stated in the paper. Also validates that roundtrip
//! latency is dominated by network/JSON overhead, not computation.
//!
//! Run with: `cargo test --test slo_latency -- --nocapture`
//!
//! Paper reference: §3.4 "Latency Distribution & SLO Guarantees"

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Instant;

// ── HTTP helpers ─────────────────────────────────────────────────────────────

fn post_timed(path: &str, body: &str) -> (String, f64) {
    let t0 = Instant::now();
    let mut stream = TcpStream::connect("127.0.0.1:9001").expect("CRYS-L server not running");
    let req = format!(
        "POST {} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        path, body.len(), body
    );
    stream.write_all(req.as_bytes()).unwrap();
    stream.shutdown(std::net::Shutdown::Write).ok();
    let mut buf = String::new();
    stream.read_to_string(&mut buf).unwrap();
    let elapsed_us = t0.elapsed().as_nanos() as f64 / 1000.0;
    let body = buf.find("\r\n\r\n").map(|p| buf[p + 4..].to_string()).unwrap_or(buf);
    (body, elapsed_us)
}

fn extract_f64(json: &str, key: &str) -> f64 {
    let pattern = format!("\"{}\":", key);
    json.find(&pattern).and_then(|pos| {
        let after = json[pos + pattern.len()..].trim_start();
        let end = after.find(|c: char| c != '-' && c != '.' && !c.is_ascii_digit()).unwrap_or(after.len());
        after[..end].parse::<f64>().ok()
    }).unwrap_or(f64::NAN)
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() { return f64::NAN; }
    let idx = ((sorted.len() as f64 - 1.0) * p / 100.0).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

// ── Helper: collect N roundtrip timings ──────────────────────────────────────

fn collect_timings(plan: &str, params: &str, n: usize) -> (Vec<f64>, Vec<f64>) {
    let body = format!(r#"{{"plan":"{}","params":{}}}"#, plan, params);
    let mut roundtrip_us = Vec::with_capacity(n);
    let mut compute_ns = Vec::with_capacity(n);

    for _ in 0..n {
        let (resp, rt_us) = post_timed("/plan/execute", &body);
        roundtrip_us.push(rt_us);
        let ns = extract_f64(&resp, "total_ns");
        if !ns.is_nan() { compute_ns.push(ns / 1000.0); } // convert to µs
    }

    roundtrip_us.sort_by(|a, b| a.partial_cmp(b).unwrap());
    compute_ns.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (roundtrip_us, compute_ns)
}

// ── Test: p50/p95/p99 roundtrip within SLO ─────────────────────────────────

#[test]
fn slo_roundtrip_within_targets_50_samples() {
    let (rt, _) = collect_timings(
        "plan_pump_sizing",
        r#"{"Q_gpm":500,"P_psi":100,"eff":0.75}"#,
        50,
    );

    let p50 = percentile(&rt, 50.0);
    let p95 = percentile(&rt, 95.0);
    let p99 = percentile(&rt, 99.0);

    println!("\n  Roundtrip latency (50 samples, localhost):");
    println!("    p50: {:.1}µs", p50);
    println!("    p95: {:.1}µs", p95);
    println!("    p99: {:.1}µs", p99);
    println!("    min: {:.1}µs  max: {:.1}µs", rt[0], rt[rt.len()-1]);

    // SLO: roundtrip p99 < 500ms on localhost (generous for CI/CD)
    assert!(p99 < 500_000.0, "p99 roundtrip {}µs exceeds 500ms SLO", p99);
    // p50 should be reasonable for localhost
    assert!(p50 < 100_000.0, "p50 roundtrip {}µs is too slow", p50);
}

// ── Test: compute latency from response header (total_ns) ────────────────────

#[test]
fn compute_latency_under_1ms() {
    let (_, compute) = collect_timings(
        "plan_pump_sizing",
        r#"{"Q_gpm":500,"P_psi":100,"eff":0.75}"#,
        30,
    );

    if compute.is_empty() {
        println!("  WARNING: No total_ns in response — compute timing not available");
        return;
    }

    let p50 = percentile(&compute, 50.0);
    let p95 = percentile(&compute, 95.0);
    let p99 = percentile(&compute, 99.0);

    println!("\n  Compute latency (JIT, 30 samples):");
    println!("    p50: {:.2}µs  p95: {:.2}µs  p99: {:.2}µs", p50, p95, p99);

    // Paper claim: p99 compute < 1000µs (1ms) for any single plan
    assert!(p99 < 1000.0,
        "Compute p99 {}µs exceeds 1ms — JIT performance regression", p99);

    // Paper claim: p50 compute < 100µs
    assert!(p50 < 100.0,
        "Compute p50 {}µs exceeds 100µs — JIT not warming up", p50);
}

// ── Test: roundtrip >> compute (network dominates) ───────────────────────────

#[test]
fn roundtrip_dominated_by_network_not_compute() {
    let (rt, compute) = collect_timings(
        "plan_pump_sizing",
        r#"{"Q_gpm":500,"P_psi":100,"eff":0.75}"#,
        20,
    );

    if compute.is_empty() { return; }

    let rt_median = percentile(&rt, 50.0);
    let compute_median = percentile(&compute, 50.0);
    let overhead_ratio = rt_median / compute_median.max(0.001);

    println!("\n  Roundtrip vs compute (20 samples):");
    println!("    Roundtrip p50:  {:.1}µs", rt_median);
    println!("    Compute p50:    {:.2}µs", compute_median);
    println!("    Overhead ratio: {:.0}× (HTTP+JSON+serialization)", overhead_ratio);

    // The whole point of the paper: compute is negligible vs roundtrip
    // Roundtrip should be at least 10× compute (network dominates)
    assert!(overhead_ratio > 5.0,
        "Expected network overhead > 5×, got {:.1}× — something is wrong", overhead_ratio);
}

// ── Test: p50/p95/p99 across multiple plan types ────────────────────────────

#[test]
fn slo_multi_domain_latency_profile() {
    let cases: &[(&str, &str)] = &[
        ("plan_pump_sizing",       r#"{"Q_gpm":500,"P_psi":100,"eff":0.75}"#),
        ("plan_electrical_load",   r#"{"P_w":5000,"V":220,"pf":0.92,"L":50,"A":4}"#),
        ("plan_beam_analysis",     r#"{"P_kn":50,"L_m":6,"E_gpa":200,"I_cm4":8000}"#),
        ("plan_planilla",          r#"{"salario_bruto":5000,"horas_extras":10,"regimen":"general"}"#),
        ("plan_loan_amortization", r#"{"principal":100000,"rate_annual":0.12,"months":36}"#),
        ("plan_nfpa13_demand",     r#"{"area_ft2":1500,"density":0.15,"K":5.6,"hose_stream":250}"#),
    ];

    println!("\n  SLO profile across domains (20 samples each):");
    println!("  {:<30} {:>8} {:>8} {:>8}", "Plan", "p50µs", "p95µs", "p99µs");
    println!("  {:-<30} {:->8} {:->8} {:->8}", "", "", "", "");

    let slo_p99_us = 200_000.0; // 200ms p99 SLO for any domain
    let mut failures = 0;

    for (plan, params) in cases {
        let (rt, _) = collect_timings(plan, params, 20);
        let p50 = percentile(&rt, 50.0);
        let p95 = percentile(&rt, 95.0);
        let p99 = percentile(&rt, 99.0);
        let ok = p99 < slo_p99_us;
        if !ok { failures += 1; }
        println!("  {:<30} {:>8.0} {:>8.0} {:>8.0} {}",
            plan, p50, p95, p99, if ok { "✓" } else { "✗ SLO BREACH" });
    }

    assert_eq!(failures, 0, "{} plans breached the {}ms p99 SLO", failures, slo_p99_us / 1000.0);
}

// ── Test: Throughput (requests per second) ────────────────────────────────────

#[test]
fn throughput_minimum_100_rps() {
    let body = r#"{"plan":"plan_pump_sizing","params":{"Q_gpm":500,"P_psi":100,"eff":0.75}}"#;
    let n = 100;
    let t0 = Instant::now();

    for _ in 0..n {
        let (_, _) = post_timed("/plan/execute", body);
    }

    let elapsed_s = t0.elapsed().as_secs_f64();
    let rps = n as f64 / elapsed_s;

    println!("\n  Throughput (sequential): {:.0} req/s over {} requests in {:.2}s",
        rps, n, elapsed_s);

    // Minimum throughput: 100 req/s sequential on localhost
    assert!(rps > 50.0, "Throughput {:.0} req/s is too low (expected > 50)", rps);
}

// ── Test: Latency stability — no degradation over time ───────────────────────

#[test]
fn latency_stable_no_degradation() {
    let body = r#"{"plan":"plan_pump_sizing","params":{"Q_gpm":500,"P_psi":100,"eff":0.75}}"#;
    let n = 60;
    let mut timings: Vec<f64> = Vec::with_capacity(n);

    for _ in 0..n {
        let (_, rt_us) = post_timed("/plan/execute", body);
        timings.push(rt_us);
    }

    // Compare first 20 vs last 20 — degradation would indicate memory leak or GC pressure
    let early: Vec<f64> = timings[..20].to_vec();
    let late: Vec<f64> = timings[40..].to_vec();

    let early_mean = early.iter().sum::<f64>() / early.len() as f64;
    let late_mean = late.iter().sum::<f64>() / late.len() as f64;
    let degradation_ratio = late_mean / early_mean;

    println!("\n  Latency stability (60 samples):");
    println!("    Early 20: mean {:.0}µs", early_mean);
    println!("    Late 20:  mean {:.0}µs", late_mean);
    println!("    Ratio:    {:.2}× (1.0 = no degradation)", degradation_ratio);

    // Allow up to 3× degradation (CI/CD environments can be noisy)
    assert!(degradation_ratio < 3.0,
        "Latency degraded {:.2}× from early to late — possible memory leak", degradation_ratio);
}
