FROM ubuntu:24.04 AS dependencies
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl git build-essential gcc-14 g++-14 pkg-config meson ninja-build libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev libwebkitgtk-6.0-dev libpango1.0-dev libcairo2-dev libgdk-pixbuf-2.0-dev xvfb dbus-daemon imagemagick desktop-file-utils file patchelf python3
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo
ENV PATH=/opt/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.95.0
RUN rustup component add rustfmt clippy
WORKDIR /work
COPY scripts/build-vte.sh scripts/build-vte.sh
RUN apt-get install -y --no-install-recommends liblz4-dev libgnutls28-dev
RUN bash scripts/build-vte.sh
RUN apt-get install -y --no-install-recommends xauth fonts-dejavu-core
RUN useradd --create-home --uid 1001 tester && chown -R tester:tester /work /opt/cargo
FROM dependencies AS rust_dependencies
COPY --chown=tester:tester Cargo.toml Cargo.lock build.rs ./
USER tester
RUN mkdir -p src && printf 'fn main() {}\n' > src/main.rs && cargo build --locked --release
FROM rust_dependencies AS verification
COPY --chown=tester:tester Cargo.toml Cargo.lock build.rs LICENSE NOTICE README.md CHANGELOG.md ./
COPY --chown=tester:tester src src
COPY --chown=tester:tester assets assets
COPY --chown=tester:tester scripts scripts
COPY --chown=tester:tester packaging packaging
USER tester
ENV GTK_A11Y=none GTK_USE_PORTAL=0 GIO_USE_VFS=local
RUN cargo clean --release -p optionterm && cargo build --locked --release && xvfb-run -a cargo test --locked --release && cargo clippy --locked --release --all-targets -- -D warnings
CMD ["bash"]
