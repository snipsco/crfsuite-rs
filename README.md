# crfsuite-rs
[![Build Status](https://travis-ci.org/snipsco/crfsuite-rs.svg?branch=master)](https://travis-ci.org/snipsco/crfsuite-rs)
[![Build Status](https://ci.appveyor.com/api/projects/status/github/snipsco/crfsuite-rs?branch=master&svg=true)](https://ci.appveyor.com/project/snipsco/crfsuite-rs)

Rust bindings for CRFSuite

## Requirements

This lib uses [Bindgen](https://github.com/servo/rust-bindgen) to generate FFI bindings, hence you need to have clang installed

```bash
$ sudo apt-get install llvm-3.9-dev libclang-3.9-dev clang-3.9 # ubuntu, see http://apt.llvm.org/ before 16.10
$ sudo pacman -S clang # ArchLinux
$ brew install llvm@3.9 # macOS
```

## Building and testing

```bash
$ cargo build
$ cargo test
```

## Supported platforms

Was tested on various x86_64 linux distros, RaspberryPi, macOS, iOS and Android


## License

Note: files in the `crfsuite-sys/c` directory are copied from the
[crfsuite](https://github.com/chokkan/crfsuite) and
[liblbfgs](https://github.com/chokkan/liblbfgs) and are not covered by
the following licence statement.


All original work licensed under either of
 * Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.


