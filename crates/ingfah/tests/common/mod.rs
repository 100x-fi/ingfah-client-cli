#![allow(
    dead_code,
    reason = "each test binary uses a different subset of these helpers"
)]
#![expect(clippy::expect_used, reason = "test helpers panic on a missing binary")]

use assert_cmd::Command;
use wiremock::MockServer;

pub const KEY: &str = "sk-ing-v1-00000000000000000000000000000000000000000000000000000000000abcde";

/// The CLI pointed at the mock server, with a key flag and no keyring. assert_cmd gives
/// the child non-terminal stdin and stdout, so writes cannot prompt and output is JSON.
pub fn ingfah(server: &MockServer) -> Command {
    let mut cmd = Command::cargo_bin("ingfah").expect("binary built");
    cmd.env_clear()
        .env("INGFAH_TEST_DISABLE_KEYRING", "1")
        .env("XDG_CONFIG_HOME", "/nonexistent-ingfah-test-config")
        .args(["--base-url", &server.uri(), "--api-key", KEY]);
    cmd
}

/// The CLI with no key anywhere.
pub fn ingfah_without_key(server: &MockServer) -> Command {
    let mut cmd = Command::cargo_bin("ingfah").expect("binary built");
    cmd.env_clear()
        .env("INGFAH_TEST_DISABLE_KEYRING", "1")
        .env("XDG_CONFIG_HOME", "/nonexistent-ingfah-test-config")
        .args(["--base-url", &server.uri()]);
    cmd
}

pub async fn request_count(server: &MockServer) -> usize {
    server.received_requests().await.map_or(0, |r| r.len())
}
