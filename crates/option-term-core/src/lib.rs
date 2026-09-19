//! GTK-free core shared by the optionTerm frontends.

pub mod codex;
pub mod commands;
pub mod config;
pub mod crash;
pub mod default_terminal;
pub mod keys;
pub mod launch;
pub mod pty;
pub mod session;
pub mod storage;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
