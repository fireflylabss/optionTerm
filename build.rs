// Emit runtime library search paths (rpath) for the patched VTE build.
//
// The rpath cannot live in .cargo/config.toml's `[build] rustflags`: makepkg
// (and any other tool that sets RUSTFLAGS in the environment) overrides that
// key, silently dropping the rpath and making the binary resolve libvte to the
// system copy — which lacks the kitty graphics symbols.
//
// cargo:rustc-link-arg is emitted at build time and is not overridden by
// RUSTFLAGS, so it survives makepkg/paru builds.
fn main() {
    // Dev layout: vte-dist/ lives in the repo root.
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/../../vte-dist/lib");
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/../../../vte-dist/lib");
    // Packaged layouts: .deb installs the patched lib under /usr/lib/optionterm,
    // the AppImage bundles it in AppDir/usr/lib next to the binary.
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/../lib/optionterm");
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/../lib");
}
