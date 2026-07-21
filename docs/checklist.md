# Checklist about Sketch Primitive Benchmark

Which is AQPBMV1

## Yes

- Multiple Distribution
    - Zipf, Uniform, Geometric, Poisson
- Multiple Data Type
    - integer (i64), float (f64), String
- Multiple input data constraint
    - finite element, range
- Data Generator with various output interface
    - keep in memory, save to file
- Various sketch from different library connectet into the BM-framework

## No

- Potentially more code refactor / removal
    - including plotting scripts
    - including removal of server
- match C++ BM to Rust BM