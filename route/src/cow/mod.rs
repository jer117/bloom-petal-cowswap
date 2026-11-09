//! CoW Protocol domain modules shared by this Petal's route components.

pub mod account_orders;
pub mod app_data;
pub mod chain;
pub mod confirm;
pub mod draft;
pub mod http;
pub mod network;
pub mod order;
pub mod params;
pub mod quote;

pub use network::CowNetwork;

use petal::sdk::SdkError;

/// `error`'s message, followed by Bloom's reason when the SDK call that just
/// failed recorded one. The SDK records reasons only with its
/// `host-error-detail` feature, and the next host call replaces the reason,
/// so call this straight after the failing call.
pub fn host_error_text(error: &SdkError) -> String {
    match (error, petal::sdk::last_host_reason()) {
        (SdkError::Host(status), Some((recorded, reason))) if *status == recorded => {
            format!("{}: {reason}", error.message())
        }
        _ => error.message(),
    }
}
