# anyOS Math Library (libm) API Reference

The **libm** shared library provides hardware-accelerated math functions for f64 and f32 types, using x86-64 SSE2 and x87 FPU instructions for maximum performance.

**Format:** ELF64 shared object (.so), loaded on demand via `SYS_DLL_LOAD`
**Exports:** 56
**Client crate:** `libm_client` (uses `dynlink::dl_open` / `dl_sym`)

libm is a pure computation library — no heap allocation, no mutable state. Every function is thread-safe and reentrant.

---

## Getting Started

### Dependencies

```toml
[dependencies]
libm_client = { path = "../../libs/libm_client" }
```

### Example

```rust
use libm_client as math;

// Load the shared library (once at startup)
math::load();

// f64 math
let x = math::sin(1.0);
let y = math::sqrt(2.0);
let (s, c) = math::sincos(0.5);  // both at once, faster than separate calls

// f32 math
let xf = math::sinf(1.0f32);
let yf = math::sqrtf(2.0f32);
```

---

## Hardware Implementation

Each function uses the fastest available x86-64 hardware instruction:

| Instruction Set | Functions | Notes |
|----------------|-----------|-------|
| **SSE2** | `sqrt`, `fabs`, `fmin`, `fmax`, `copysign`, `trunc` | Scalar double/single-precision |
| **x87 FPU** | `sin`, `cos`, `sincos`, `tan`, `atan`, `atan2`, `asin`, `acos`, `pow`, `exp`, `exp2`, `log`, `log2`, `log10`, `floor`, `ceil`, `fmod`, `ldexp` | 80-bit internal precision |
| **Pure Rust** | `round`, `frexp`, `cbrt`, `hypot`, `copysign` | IEEE 754 bit manipulation / iteration |

---

## f64 Functions

### Trigonometric

#### `sin(x: f64) -> f64`

Sine via x87 `fsin`.

#### `cos(x: f64) -> f64`

Cosine via x87 `fcos`.

#### `sincos(x: f64) -> (f64, f64)`

Compute sine and cosine simultaneously via x87 `fsincos`. Returns `(sin, cos)`. More efficient than calling `sin()` and `cos()` separately — single FPU operation.

#### `tan(x: f64) -> f64`

Tangent via x87 `fptan`.

#### `asin(x: f64) -> f64`

Arcsine. Returns result in [-PI/2, PI/2]. Clamped for |x| >= 1.

#### `acos(x: f64) -> f64`

Arccosine. Returns result in [0, PI]. Clamped for |x| >= 1.

#### `atan(x: f64) -> f64`

Arctangent via x87 `fpatan`. Returns result in [-PI/2, PI/2].

#### `atan2(y: f64, x: f64) -> f64`

Two-argument arctangent via x87 `fpatan`. Returns the angle in radians between the positive x-axis and the point (x, y), in [-PI, PI].

| Parameter | Type | Description |
|-----------|------|-------------|
| y | `f64` | Y coordinate |
| x | `f64` | X coordinate |
| **Returns** | `f64` | Angle in radians |

---

### Exponential & Logarithmic

#### `pow(x: f64, y: f64) -> f64`

Power function x^y via x87 `fyl2x` + `f2xm1` + `fscale`.

Special cases:
- `pow(x, 0.0)` = 1.0 for all x (including 0 and NaN)
- `pow(1.0, y)` = 1.0 for all y
- `pow(0.0, y)` = 0.0 for y > 0, Inf for y < 0
- Negative base with non-integer exponent returns NaN
- Negative base with odd integer exponent returns negative result

#### `exp(x: f64) -> f64`

Exponential e^x via x87. Computed as 2^(x * log2(e)).

#### `exp2(x: f64) -> f64`

Base-2 exponential 2^x via x87 `f2xm1` + `fscale`.

#### `log(x: f64) -> f64`

Natural logarithm ln(x) via x87. Computed as log2(x) * ln(2) using `fyl2x`.

#### `log2(x: f64) -> f64`

Base-2 logarithm via x87 `fyl2x`.

#### `log10(x: f64) -> f64`

Base-10 logarithm via x87. Computed as log2(x) * log10(2) using `fyl2x`.

---

### Rounding

#### `floor(x: f64) -> f64`

Round toward negative infinity via x87 `frndint` with control word set to round-down mode.

#### `ceil(x: f64) -> f64`

Round toward positive infinity via x87 `frndint` with control word set to round-up mode.

#### `round(x: f64) -> f64`

Round to nearest integer, ties away from zero (C99 semantics). Implemented as `trunc(x + copysign(0.5, x))`.

#### `trunc(x: f64) -> f64`

Truncate toward zero via IEEE 754 bit manipulation. Values >= 2^52 are returned as-is (already integers in f64 representation).

---

### Basic Operations

#### `sqrt(x: f64) -> f64`

Square root (IEEE 754 exact) via SSE2 `sqrtsd`. Single-cycle latency on modern CPUs.

#### `fabs(x: f64) -> f64`

Absolute value via SSE2 `andpd` (clears the sign bit).

#### `fmod(x: f64, y: f64) -> f64`

Floating-point remainder via x87 `fprem`. Returns x - trunc(x/y) * y. Returns NaN if y is 0.

#### `fmin(x: f64, y: f64) -> f64`

Minimum of two values via SSE2 `minsd`. NaN-safe: if either argument is NaN, returns the other.

#### `fmax(x: f64, y: f64) -> f64`

Maximum of two values via SSE2 `maxsd`. NaN-safe: if either argument is NaN, returns the other.

#### `hypot(x: f64, y: f64) -> f64`

Hypotenuse sqrt(x^2 + y^2) with overflow protection. Uses scaled computation: max * sqrt(1 + (min/max)^2).

#### `copysign(x: f64, y: f64) -> f64`

Copy sign of y onto magnitude of x via IEEE 754 bit manipulation.

#### `cbrt(x: f64) -> f64`

Cube root via Halley's method (3 iterations). Initial estimate from IEEE 754 exponent manipulation.

---

### Decomposition

#### `ldexp(x: f64, n: i32) -> f64`

Load exponent: returns x * 2^n via x87 `fscale`.

| Parameter | Type | Description |
|-----------|------|-------------|
| x | `f64` | Significand |
| n | `i32` | Exponent |
| **Returns** | `f64` | x * 2^n |

#### `frexp(x: f64) -> (f64, i32)`

Decompose into normalized fraction and exponent. Returns (frac, exp) where x = frac * 2^exp and frac is in [0.5, 1.0). Implemented via IEEE 754 bit manipulation (no FPU).

---

## f32 Functions

All f64 functions have f32 equivalents with an `f` suffix. SSE2 operations use single-precision instructions (`sqrtss`, `minss`, `maxss`). x87 operations load/store as 32-bit (`fld dword` / `fstp dword`) but compute internally with 80-bit precision.

| f64 | f32 | HW Difference |
|-----|-----|--------------|
| `sqrt` | `sqrtf` | `sqrtss` instead of `sqrtsd` |
| `sin` | `sinf` | `fld dword` instead of `fld qword` |
| `cos` | `cosf` | same |
| `sincos` | `sincosf` | same |
| `tan` | `tanf` | same |
| `asin` | `asinf` | same |
| `acos` | `acosf` | same |
| `atan` | `atanf` | same |
| `atan2` | `atan2f` | same |
| `pow` | `powf` | promotes to f64 internally for precision |
| `exp` | `expf` | promotes to f64 internally |
| `exp2` | `exp2f` | promotes to f64 internally |
| `log` | `logf` | same |
| `log2` | `log2f` | same |
| `log10` | `log10f` | same |
| `floor` | `floorf` | same |
| `ceil` | `ceilf` | same |
| `round` | `roundf` | same |
| `trunc` | `truncf` | IEEE 754 f32 bit manipulation |
| `fabs` | `fabsf` | f32 bit mask `0x7FFFFFFF` |
| `fmod` | `fmodf` | same |
| `fmin` | `fminf` | `minss` instead of `minsd` |
| `fmax` | `fmaxf` | `maxss` instead of `maxsd` |
| `hypot` | `hypotf` | same |
| `copysign` | `copysignf` | f32 bit manipulation |
| `ldexp` | `ldexpf` | same |
| `frexp` | `frexpf` | f32 bit manipulation |
| `cbrt` | `cbrtf` | 2 Halley iterations (vs 3 for f64) |

---

## C ABI Exports

For direct FFI usage without the client wrapper, the exported C symbols follow the naming convention `math_<name>` for f64 and `math_<name>f` for f32:

```c
// f64
double math_sqrt(double x);
double math_sin(double x);
double math_cos(double x);
void   math_sincos(double x, double *out_sin, double *out_cos);
double math_tan(double x);
double math_asin(double x);
double math_acos(double x);
double math_atan(double x);
double math_atan2(double y, double x);
double math_pow(double x, double y);
double math_exp(double x);
double math_exp2(double x);
double math_log(double x);
double math_log2(double x);
double math_log10(double x);
double math_floor(double x);
double math_ceil(double x);
double math_round(double x);
double math_trunc(double x);
double math_fabs(double x);
double math_fmod(double x, double y);
double math_fmin(double x, double y);
double math_fmax(double x, double y);
double math_hypot(double x, double y);
double math_copysign(double x, double y);
double math_ldexp(double x, int n);
double math_frexp(double x, int *exp);
double math_cbrt(double x);

// f32
float math_sqrtf(float x);
float math_sinf(float x);
// ... (same pattern with 'f' suffix)
```

---

## Architecture

- **libm** (`libs/libm/`) — the shared library, built as `staticlib` and linked by `anyld` into an ELF64 `.so`. Exports 56 `#[no_mangle] pub extern "C"` symbols. No heap, no dependencies (not even `libheap`).
- **libm_client** (`libs/libm_client/`) — client wrapper that resolves symbols via `dynlink::dl_open("/Libraries/libm.so")` + `dl_sym()`. Caches all 56 function pointers in a static `MathLib` struct. Provides ergonomic Rust wrappers (e.g. `sincos()` returns a tuple instead of writing through pointers).

### Source Files

| File | Description |
|------|-------------|
| `libs/libm/src/lib.rs` | C ABI exports + panic handler |
| `libs/libm/src/f64_ops.rs` | f64 implementations (SSE2 + x87 inline asm) |
| `libs/libm/src/f32_ops.rs` | f32 implementations (SSE2 + x87 inline asm) |
| `libs/libm/exports.def` | 56 exported symbols for anyld |
| `libs/libm_client/src/lib.rs` | Safe Rust wrappers via dl_open/dl_sym |
