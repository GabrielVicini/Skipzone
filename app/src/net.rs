//! The app's only outbound network access, used by the validation harness.
//!
//! Kept to one file with one function so that "what does this program talk to"
//! has a single answer. The GUI and the solver never call it; the headless
//! harnesses in `src/bin` do, and only to fetch public measurement data
//! (WSPR spots, solar indices) that the run is scored against.
//!
//! Not compiled for wasm32 at all: the web build has no business making these
//! requests, and the dependency is gated out of it in `Cargo.toml`.

use std::fmt;
use std::time::Duration;

/// How long any single request may take before it is abandoned. A validation
/// run that hangs is worse than one that reports a timeout.
const TIMEOUT: Duration = Duration::from_secs(60);

/// Identifies this client to the services it queries. Both are volunteer-run
/// and ask that automated users be identifiable.
const USER_AGENT: &str = concat!(
    "skipzone-validate/",
    env!("CARGO_PKG_VERSION"),
    " (HF propagation model validation; https://github.com/)"
);

#[derive(Debug)]
pub enum NetError {
    /// The request itself failed: DNS, TLS, connection, timeout, HTTP status.
    Request(String),
    /// The request succeeded but the body was not what the caller expected.
    Data(String),
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(m) | Self::Data(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for NetError {}

/// Fetch a URL and return the body as text.
///
/// # Errors
/// Any transport failure, non-success status, or non-UTF-8 body.
pub fn get_text(url: &str) -> Result<String, NetError> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .user_agent(USER_AGENT)
        .build()
        .new_agent();

    let mut response = agent
        .get(url)
        .call()
        .map_err(|e| NetError::Request(format!("GET {url} failed: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        return Err(NetError::Request(format!("GET {url} returned {status}")));
    }
    response
        .body_mut()
        .read_to_string()
        .map_err(|e| NetError::Data(format!("GET {url} returned an unreadable body: {e}")))
}
