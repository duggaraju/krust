# Getting Started.
* Use the nightly build of rust.
```bash
rustup override set nightly
```
* Add the x86_64-unknown-none target
```bash
rustup target add x86_64-unknown-none
```
* Install llvm tools
```bash
rustup component add llvm-tools-preview
```