//! [`SendAllowance`] over the shared rate limiter.

#[cfg(test)]
mod test;

use std::time::Duration;

use macro_user_id::user_id::MacroUserIdStr;
use rate_limit::{RateBudgetPort, RateLimitConfig, RateLimitKey};

use super::{AllowanceExceeded, SendAllowance};

/// Notifications each sender may send per [`WINDOW`], counted per recipient.
/// Far above ordinary use: it stops a program stuck in a loop and nothing
/// else.
pub(crate) const NOTIFICATIONS_PER_WINDOW: u64 = 1000;

pub(crate) const WINDOW: Duration = Duration::from_secs(60 * 60);

/// What the allowance's counters are filed under, beside the sender.
const KEY_SCOPE: &str = "manual-notification-recipients";

/// Keeps each sender's count for the current hour in the rate limiter's store.
pub(crate) struct HourlyAllowance<Budget>(pub Budget);

impl<Budget: RateBudgetPort> SendAllowance for HourlyAllowance<Budget> {
    async fn take(
        &self,
        sender: &MacroUserIdStr<'_>,
        notifications: u64,
    ) -> Result<Result<(), AllowanceExceeded>, rootcause::Report> {
        let key = RateLimitKey::builder(&KEY_SCOPE)
            .append(&sender.as_ref())
            .finish();
        let spent = self
            .0
            .spend(
                key,
                RateLimitConfig::new(NOTIFICATIONS_PER_WINDOW, WINDOW),
                notifications,
            )
            .await?;
        Ok(spent.map(|_| ()).map_err(|exceeded| AllowanceExceeded {
            limit: exceeded.max_count,
            retry_after: exceeded.retry_after,
        }))
    }
}
