# Getting Started.
* Add the x86_64-unknown-none target
```bash
rustup target add x86_64-unknown-none --toolchain nightly
```
* Install llvm tools
```bash
rustup component add llvm-tools-preview
```

* Build the code
```bash
cargo +nightly build -Z bindeps
```