pub use option_term_core::test_support::TestDir;

pub fn spin_until(ready: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !ready() && std::time::Instant::now() < deadline {
        gtk4::glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(ready(), "asynchronous operation did not finish");
}
