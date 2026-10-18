//! The HTTP client the Hub uses (release list, downloads).

use std::time::Duration;

pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        // No total limit on the body: an editor download is a few hundred MB.
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .user_agent(concat!("oxide-hub/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}
