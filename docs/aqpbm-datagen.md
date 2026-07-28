# `aqpbm-datagen` Design

`aqpbm-datagen` turns a description of a column into the values a benchmark ingests.
It depends on nothing else in the workspace.

## 1. Purpose

This crate produces one column of values from one description.

The **only** responsibility of this crate is data generation.
It does not know which statistic a benchmark will measure on that column.
It therefore supplies no exact answers, and a consumer needing ground truth computes it at its own cost.
Generating data and replaying recorded data are different operations, so reading a `.bin` or a `.pcap` belongs to `aqpbm-core`.

## 2. What a description says

A description makes four independent choices, and any combination of them is legal unless it contradicts itself.

- **How many values the column holds.**
- **What type the values are**, either a fixed-width numeric type or a string.
- **Which values appear**, drawn from a contiguous key range, a fixed categorical set, or a monotonically increasing sequence.
- **How often each value appears**, uniform or skewed (the distribution), with the skew carrying its own parameter.

A seed is mandatory, since the promise in §3 cannot hold without one.
Each of the four sets is open, and adding a member to one leaves the other three untouched.
Free combination admits descriptions that mean nothing, so a contradictory description is rejected before any value is generated.

## 3. What is promised

- **Determinism.** The same description always produces the same column.
- **Generation never runs inside a timed region.**
- By default the whole column exists before timing starts.
- In stream mode the caller sets a chunk size, and generation alternates with consumption.
- **Two destinations.** A column either stays in memory or goes to disk.

## 4. The on-disk format

Values are written as a raw little-endian byte sequence.
One metadata record states the value type for the whole file.
A fixed-width column carries its type once and spends no bytes per value.
A string serialises as a one-byte length followed by that many bytes.
That length caps a string at 255 bytes, and a description asking for longer strings is rejected under §2.
The metadata also records the count and the description that produced the file, which makes a stored column reproducible on its own.
Any change to the metadata layout breaks readers, so the layout carries a version and a reader refuses a version it does not know.

## 5. Contract

```rust
// The description, built in memory or read from a `.yaml`/`.yml`/`.json` file.
pub struct GenSpec {
    pub shape: Shape,
    pub size: usize,
    pub seed: u64,
    pub string: Option<StringOpts>,
}

impl GenSpec {
    pub fn from_path(path: &Path) -> Result<Self, SketchError>;
    pub fn generate<T: GenValue>(&self) -> Result<Vec<T>, SketchError>;
    pub fn generate_into<T: GenValue, S: Sink<T>>(
        &self,
        sink: &mut S,
        chunk: usize,
    ) -> Result<GenMeta, SketchError>;
}

pub enum SketchError {
    Io(std::io::Error),
    BadParam(String),
    SchemaVersion { file: u32, expected: u32 },
}
```

```rust
// A destination. Values arrive in emission order, and `flush` runs once after the last chunk.
pub trait Sink<T> {
    fn accept(&mut self, chunk: &[T]) -> Result<(), SketchError>;
    fn flush(&mut self) -> Result<(), SketchError> { Ok(()) }
}
```

```rust
// A value type a caller may name as `T`.
pub trait GenValue: Clone + Debug + PartialEq + 'static {
    type Cfg: Clone;
    const NAME: &'static str;                  // the tag recorded in the sidecar and the report
    const EXACT_INTEGER_LIMIT: Option<u64>;    // None when the type carries no such bound
    const SUPPORTS_MONOTONIC: bool;
    fn cfg(spec: &GenSpec) -> Result<Self::Cfg, SketchError>;
    fn from_draw(u: u64, cfg: &Self::Cfg) -> Self;
    fn from_acc(a: i128, cfg: &Self::Cfg) -> Result<Self, SketchError>;
    fn stat(&self) -> Option<f64>;
}

// The bound a value type meets to enter the `.bin` stream.
pub trait FixedWidth: GenValue {
    fn write_le<W: Write>(&self, w: &mut W) -> std::io::Result<()>;
}
```

`T` is named by the caller at the call site.
There is no run-time type tag, so adding a value type is one `GenValue` impl and nothing else.
`chunk` sets how many values are drawn at a time, which bounds peak memory and never changes the values produced.
`generate` is `generate_into` over a `MemorySink` at `DEFAULT_CHUNK`, so both paths are one draw.
Only the chunked path hands back `GenMeta`, the provenance record of §4.
`MemorySink` yields the column through `into_values`, and `BinSink::create` opens a `.bin` at a path.
`BadParam` covers a description that cannot be honoured, and `SchemaVersion` covers a sidecar whose version a reader does not know.

## 6. Open questions

- **Reproducing a recorded trace with a synthetic description.** Matching its frequency distribution is within reach, and matching its arrival order and burstiness needs vocabulary §2 does not have.
- One option extends §2 with an order rule, at the cost of a fifth choice that only one use case wants.
- The other option approximates a trace by its value distribution alone, at the cost of saying so wherever such a result is reported.
