---
title: 'QOMN: An Open-Source Domain-Specific Language and JIT Runtime for Deterministic Engineering Computation'
tags:
  - Rust
  - deterministic computing
  - IEEE-754
  - JIT compilation
  - Cranelift
  - WebAssembly
  - safety-critical systems
  - engineering calculations
  - domain-specific language
  - neuro-symbolic AI
  - NFPA
  - IEC 60364
  - AISC 360
authors:
  - name: Percy Rojas Masgo
    orcid: 0000-0000-0000-0000
    affiliation: 1
affiliations:
  - name: Condesi Perú / Qomni AI Lab, Lima, Peru
    index: 1
date: 19 April 2026
bibliography: paper.bib
---

# Summary

Engineering certification standards — including NFPA 13/20/72 (fire
protection), IEC 60364 and NEC (electrical), AISC 360 (structural),
and regulated domains such as medical equipment, clinical dosing, and
national payroll codes — require that computational results be
**mathematically reproducible**: given identical inputs, every
conforming runtime must produce bit-identical outputs under any load,
on any supported hardware, on any day. Spreadsheets, general-purpose
scripts, and large language models all fail this requirement in
different ways.

**QOMN** is an open-source domain-specific language (DSL) and
just-in-time (JIT) runtime that enforces bit-exact reproducibility as
a first-class property of the language, not an accident of the
runtime. It is distributed under Apache-2.0 as a reference
implementation for **certifiable AI computation** — the subcategory
of AI systems whose answers can be independently reproduced and
audited against a cited standard.

The artifact ships a validation library of **57 engineering plans
across 10 domains**, each citing the governing standard in source
(e.g. `// NFPA 20:2022 §4.26`). This is **a validation sample, not a
closed catalog**: the language imposes no upper bound on plan count,
and the intended scope is on the order of **thousands of plans**
organized into domain libraries (fire, structural, electrical,
clinical, legal/fiscal, aeronautical, pharmaceutical, nuclear,
maritime, geotechnical) maintained collaboratively by certified
professional engineers. Reaching that scale is an explicit call for
contribution that this paper accompanies.

On a commodity VPS (AMD EPYC 12-core, $80/month) the JIT backend
sustains **449–540 million scenario evaluations per second** (median
≈510M/s, 60–73% SIMD utilization) with **IEEE-754 bit-exact** output
across all runs. Every measurement in this paper is reproducible
through a public HTTP API at <https://desarrollador.xyz> without
credentials. Source code is at <https://github.com/condesi/qomn>.

# Vision and Scope

QOMN is **not only a compute engine**. It is the deterministic tier
of a broader architectural proposal for AI in regulated domains:

1. **The kernel** — the DSL, JIT runtime, unit-aware type system, and
   branchless oracle pattern documented in this paper. Apache-2.0
   open source, independently verifiable, ready today.
2. **The standard library at scale** — the present 57 plans are a
   sample that validates the architecture across heterogeneous
   formula classes; the architectural target is thousands of plans
   covering the full catalog of engineering codes (NFPA alone
   contains thousands of calculable provisions; AISC, ACI, IEC, NEC,
   ASHRAE, and clinical/fiscal regulations add thousands more). That
   scale is unreachable through a single author and is framed here
   as an open invitation for community contribution.
3. **The cognitive orchestrator above it** — a separate,
   complementary system (*Qomni Cognitive OS*) currently under active
   development is designed to compose QOMN with non-neural
   strategies — reflex caches, hyperdimensional memory (2048-bit
   hypervectors), mixture-of-experts retrievers, adversarial veto,
   and permanent indexed memory — explicitly **without any
   dependence on large language models**. QOMN functions as its
   fastest deterministic tier. Qomni Cognitive OS is not yet public
   and is not within the verifiability claims of this paper; it is
   noted to clarify the broader program of work and how QOMN fits
   into it.

The present paper commits only to what the public artifact delivers
today: the DSL, the JIT runtime, the 57 plans, and the public API.
Everything else is future work, flagged as such.

# Statement of Need

Three classes of tools widely used for engineering calculations fail
the certified reproducibility requirement:

1. **Large Language Models.** Stochastic sampling produces different
   numeric outputs on repeated queries even at temperature 0, varies
   across hardware, library versions, and batch composition, and is
   opaque to auditors. An LLM cannot cite a specific line of code or
   standard clause for a given answer.
2. **Python / NumPy / SciPy.** Floating-point results drift across
   NumPy versions, BLAS backends, compiler flags, and reduction
   orders [@goldberg1991]. Bit-exact reproducibility across
   environments is not guaranteed.
3. **Unsafe C++ with data-dependent branches.** Patterns such as
   `if (flow < 1.0) return NAN;` prevent SIMD vectorization,
   propagate NaN silently on invalid input, and yield
   compiler-specific codegen under `-O3`.

QOMN targets the specific subdomain of **closed-form engineering
formulas with established domain standards**, where bit-exact
reproducibility is a regulatory or safety requirement. It is
explicitly **not a replacement for LLMs** on open-ended problem
formulation, natural-language understanding, or design exploration;
it is the certifiable numeric tier intended to be composed with
other components by the caller.

# The Branchless Oracle Pattern

The central language-level innovation is expressing conditional
validation as a floating-point predicate, eliminating control-flow
branches entirely from oracle bodies:

```
oracle nfpa20_pump_hp(
    flow: float, head: float, eff: float) -> float:
  let valid = (flow >= 1.0) * (flow <= 50000.0) * (eff >= 0.10)
  ((flow * 0.06309 * head * 0.70307) / (eff * 76.04 + 0.0001)) * valid
```

`valid` evaluates to `1.0` when all constraints hold and `0.0`
otherwise. Invalid inputs produce a numeric zero rather than a
propagating NaN, and the absence of data-dependent branches allows
Cranelift to emit vectorized code that evaluates multiple scenarios
per AVX2/FMA instruction. The `cond(pred, a, b)` form compiles to a
branchless `select`, so piecewise oracles remain vectorizable.

A hardened **NaN-Shield** has been validated against a corpus of
**12.8 million adversarial inputs** (NaN, ±∞, denormals, signed
zero, out-of-range values) with zero runtime panics in the
`tests/adversarial.rs` suite.

# Language and Runtime Design

QOMN is implemented in **~27,900 lines of Rust** across 25 modules.
The compiler is a conventional pipeline — lexer → parser → type
checker (with physical-unit dimensions and NFPA/IEC range
validation) → HIR → bytecode IR — feeding **three interchangeable
backends** over a single stable IR:

- **Cranelift native x86-64** [@cranelift] — the default deployed
  backend. Compilation in milliseconds; adequate code quality for
  numerical kernels; FMA contraction explicitly disabled and
  frame-pointer emission suppressed for bit-exactness and for the
  2–3 ns/call saving relevant at tens-of-nanoseconds oracle latency.
- **LLVM IR 18** — optional AOT compilation of plans to shared
  libraries for offline or embedded deployment.
- **WebAssembly (WAT)** — sandboxed execution in browsers and edge
  environments.

To our knowledge no other engineering-oriented DSL runtime provides
all three backends behind a single bytecode IR.

A **tiered-JIT policy** interprets each oracle for the first $N=50$
invocations and JIT-compiles thereafter, avoiding compilation tax on
rarely used plans. The threshold is exposed as
`jit::JIT_THRESHOLD` for experimenters.

## Type system with physical units and regulatory ranges

Types such as `flow`, `pressure`, `voltage`, and `k_factor` are
first-class and carry physical dimensions. A call with
dimensionally inconsistent arguments is a **compile error**, not a
runtime exception. The type checker additionally consults an
NFPA/IEC range table to warn when literal inputs fall outside the
physical ranges recognized by the governing standard — a form of
regulatory range-checking at the compiler level that, to our
knowledge, no other unit-of-measure system integrates.

## Live determinism proof

The `/verify` endpoint executes a reference oracle $N$ times and
returns the FNV-1a hash of the IEEE-754 bit pattern of the result.
The hash is identical across runs on conforming hardware:

```bash
curl "https://desarrollador.xyz/verify?runs=20"
# {"variance":0.000000000000, "all_identical":true, "hash_match":true}
```

# Performance

Throughput is measured live on `/simulation/jitter_bench` over ten
consecutive sweeps on an AMD EPYC 12-core VPS ($80/month). Values
below are ranges from the deployed system and are reproducible
through the open API.

| Metric                   | Measured value                     |
|--------------------------|------------------------------------|
| Throughput (scenarios/s) | **449–540 M** (median ≈510 M)      |
| SIMD utilization         | 60–73 % (median ≈69 %)             |
| Determinism              | IEEE-754 exact; 3/3 bit-identical over repeated runs |
| Resident memory          | ~14 MB at rest                     |
| Release binary           | ~8.3 MB                            |
| Adversarial panics       | 0 over 12.8 M inputs               |

A coarse orientation against non-deterministic alternatives (these
systems compute different things; the comparison is illustrative, not
a direct speedup claim):

| System                | Throughput   | Determinism                   |
|-----------------------|--------------|-------------------------------|
| QOMN v3.2 (AVX2 JIT)  | 449–540 M/s  | IEEE-754 exact                |
| C++ GCC -O3           | ~5 M/s       | UB on NaN, branchy codegen    |
| Python / NumPy        | ~0.2 M/s     | Drift across versions/BLAS    |
| Stochastic LLMs       | <1 ans/s     | Non-reproducible              |

All benchmarks are publicly verifiable:

```bash
curl https://desarrollador.xyz/simulation/jitter_bench
curl https://desarrollador.xyz/simulation/adversarial
curl https://desarrollador.xyz/benchmark/vs_llm
```

# Plan Coverage: A Sample, Not a Catalog

Version 3.2 distributes **57 validation plans across 10 domains**,
each citing its governing standard inline:

- **Fire protection** — NFPA 13, NFPA 20, NFPA 72
- **Electrical** — IEC 60364, NEC
- **Structural** — AISC 360
- **Hydraulics** — Hazen-Williams, Manning
- **HVAC** — ASHRAE cooling/heating loads
- **Financial / payroll** — national labor-code formulas
- **Medical equipment** — IEC 60601 dosing, flow, pressure
- **Cybersecurity** — password entropy, key strength
- **Statistics** — confidence intervals, regression summaries
- **Transport** — braking distance, fleet economics

**These 57 plans are a validation sample, not a design target.** The
architecture imposes no upper bound. Full faithful coverage of
mainstream engineering codes would require **thousands of plans**
distributed across specialized libraries maintained by domain
experts — NFPA alone contains thousands of calculable provisions,
and AISC, ACI, IEC, ASHRAE, clinical dosing, and fiscal codes each
add comparable depth. Reaching that scale is the central
community-contribution call of this work. Open questions for future
plan governance include:

- How to federate plan repositories so contributions are not
  bottlenecked on a single maintainer.
- How to version plans against versions of standards (e.g. NFPA
  13:2022 vs. NFPA 13:2025).
- How to formally verify that a plan remains faithful to its cited
  standard clause across revisions.

# Contribution Beyond the Compute Kernel

This work is relevant to several audiences simultaneously:

**Neuro-symbolic AI research.** QOMN is a concrete reference
implementation of the *deterministic compute tier* of a hybrid
architecture. Routing research (when to invoke the DSL vs. a neural
component) and confidence calibration can be evaluated against a
running, public system.

**AI developers in industry.** The pattern *DSL for known formulas,
LLM for open queries* reduces API cost and improves auditability.
Apache-2.0 licensing permits commercial incorporation without
copyleft friction.

**Practicing engineers.** Plans are plain-text source under version
control. A fire-protection calculation can be diffed, reviewed, and
signed off exactly like a pull request — and it carries its
governing-standard citation inline.

**Regulators and certification bodies.** The determinism policy
(explicit FMA, rounding, NaN canonicalization, denormal handling)
provides a concrete technical target for discussions of
*certifiable AI computation*. Independent verifiability requires no
vendor cooperation.

**Standards bodies.** Machine-readable encoding of selected
provisions of NFPA, IEC, AISC, and related standards opens a channel
for direct engagement with the bodies that maintain the source
standards.

# Availability and Reproducibility

Source code is Apache-2.0 at <https://github.com/condesi/qomn>. The
deployed runtime used for every measurement in this paper is live at
<https://desarrollador.xyz>; the `/verify`,
`/simulation/jitter_bench`, and `/benchmark/vs_llm` endpoints require
no authentication and exercise the same code paths as a local build.
A reproduction script (`scripts/reproduce.sh` in the paper
repository) replays all measurements and prints a diff against the
paper's recorded values. Five test suites ship with the runtime:
`golden`, `repeatability`, `adversarial`, `slo_latency`, and
`all_57_plans`, totaling over 1,873 SLOC of integration tests.

# Limitations

- **Coverage is a sample.** 57 plans represent well under 1% of
  formulas in professional practice. Scale-out depends on community
  contribution.
- **No natural-language understanding.** QOMN requires structured
  plan invocations. Pairing with an external front-end for
  natural-language dispatch is outside the scope of this paper.
- **No open design problems.** QOMN evaluates formulas; it does not
  choose which formula or load combination applies.
- **Single-server deployment tested.** Horizontally scaled
  multi-tenant behavior is future work.
- **Cross-ISA determinism not claimed.** Bit-exactness is
  guaranteed on x86-64 with AVX2. ARM support compiles but requires
  `QOMN_NO_FMA=1` and has not been performance-characterized.
- **No formal verification.** Plans have unit and golden tests;
  they do not have machine-checked proofs against their cited
  standards. Formal verification is explicit future work.
- **Single-author standard library.** Plans were drafted by the
  author and reviewed by professional engineers in the author's
  network; formal peer review by authorities-having-jurisdiction is
  future work.

# References
