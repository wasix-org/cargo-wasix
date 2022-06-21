#!/bin/bash -e

# Install pre-reqs
if ! apt -qq list python 2>/dev/null | grep -q installed &&
   ! apt -qq list python2 2>/dev/null | grep -q installed; then
  sudo apt install python
fi
if ! apt -qq list ninja-build 2>/dev/null | grep -q installed; then
  sudo apt install ninja-build
fi
if ! apt -qq list git 2>/dev/null | grep -q installed; then
  sudo apt install git
fi
rustup toolchain uninstall wasix || true

CUR_DIR=$(pwd)

# Download the RUST sourcecode
if [ ! -f /opt/wasix-rust/done.pulled ]; then
  cd /opt
  sudo mkdir -p wasix-rust
  sudo chmod 777 wasix-rust
  git clone --branch wasix --depth=1 https://github.com/john-sharratt/rust.git wasix-rust
  git config --global --add safe.directory /opt/wasix-rust
  cd wasix-rust
  git config -f .gitmodules submodule.src/rust-installer.shallow true
  git config -f .gitmodules submodule.src/doc/nomicon.shallow true
  git config -f .gitmodules submodule.src/tools/cargo.shallow true
  git config -f .gitmodules submodule.src/doc/reference.shallow true
  git config -f .gitmodules submodule.src/tools/rls.shallow true
  git config -f .gitmodules submodule.src/tool/miri.shallow true
  git config -f .gitmodules submodule.src/doc/rust-by-example.shallow true
  git config -f .gitmodules submodule.library/stdarch.shallow true
  git config -f .gitmodules submodule.src/doc/edition-guide.shallow true
  git config -f .gitmodules submodule.src/llvm-project.shallow true
  git config -f .gitmodules submodule.src/doc/embedded-book.shallow true
  git config -f .gitmodules submodule.src/tools/rust-analyzer.shallow true
  git config -f .gitmodules submodule.library/backtrace.shallow true
  git submodule update --init
  touch done.pulled
  cd ..
else
  cd /opt/wasix-rust
  git pull
fi

# Download the LIBC source code
cd $CUR_DIR
if [ ! -d /opt/wasix-libc/done.pulled ]; then
  cd /opt
  sudo mkdir -p wasix-libc
  sudo chmod 777 wasix-libc
  git clone --depth=1 https://github.com/john-sharratt/wasix-libc.git wasix-libc
  git config --global --add safe.directory /opt/wasix-libc
  cd wasix-libc
  git submodule update --init
  touch done.pulled
  cd ..
else
  cd /opt/wasix-libc
  git pull
fi
cd $CUR_DIR

# Copy the configuration file over
cat >/opt/wasix-rust/config.toml <<EOF
changelog-seen = 2

[build]
target = ["wasm32-wasmer-wasi", "wasm64-wasmer-wasi"]
extended = true
tools = [ "clippy", "rustfmt" ]
configure-args = []

[rust]
lld = true
llvm-tools = true

[target.wasm32-wasmer-wasi]
wasi-root = "../wasix-libc/sysroot32"

[target.wasm64-wasmer-wasi]
wasi-root = "../wasix-libc/sysroot64"
EOF

# Build the sysroots
cd /opt/wasix-libc
./build32.sh
./build64.sh
cd $CUR_DIR

# Run the build
cd /opt/wasix-rust
./x.py build
./x.py build --stage 2
rustup toolchain link wasix ./build/$(uname -m)-unknown-$OSTYPE/stage2
cd $CUR_DIR

echo "rustup toolchain link wasix ./build/$(uname -m)-unknown-$OSTYPE/stage2"

#rustup default wasix