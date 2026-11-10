use std::sync::Mutex;
use std::time::Duration;

use macro_user_id::cowlike::CowLike;
use macro_user_id::user_id::MacroUserIdStr;
use rate_limit::{RateBudgetPort, RateLimitConfig, RateLimitExceeded, RateLimitKey};
use rootcause::Report;

use super::{HourlyAllowance, NOTIFICATIONS_PER_WINDOW, WINDOW};
use crate::manual::{AllowanceExceeded, SendAllowance};

/// A budget that counts in memory, one window that never ends.
#[derive(Default)]
struct CountingBudget {
    spent: Mutex<Vec<(String, u64)>>,
}

impl RateBudgetPort for CountingBudget {
    async fn spend(
        &self,
        key: RateLimitKey,
        config: RateLimitConfig,
        cost: u64,
    ) -> Result<Result<u64, RateLimitExceeded>, Report> {
        assert_eq!(config.window, WINDOW);
        let mut spent = self.spent.lock().unwrap();
        let current: u64 = spent
            .iter()
            .filter(|(spender, _)| *spender == key.to_hex_string())
            .map(|(_, cost)| cost)
            .sum();
        if current + cost > config.max_count {
            return Ok(Err(RateLimitExceeded {
                current_count: current,
                max_count: config.max_count,
                retry_after: Duration::from_secs(25 * 60),
            }));
        }
        spent.push((key.to_hex_string(), cost));
        Ok(Ok(current + cost))
    }
}

fn user(id: &str) -> MacroUserIdStr<'static> {
    MacroUserIdStr::parse_from_str(id).unwrap().into_owned()
}

#[tokio::test]
async fn counts_each_recipient_against_the_senders_own_hour() {
    let allowance = HourlyAllowance(CountingBudget::default());
    let sender = user("macro|sender@macro.com");
    let other = user("macro|other@macro.com");

    assert_eq!(
        allowance
            .take(&sender, NOTIFICATIONS_PER_WINDOW - 1)
            .await
            .unwrap(),
        Ok(())
    );
    assert_eq!(allowance.take(&sender, 1).await.unwrap(), Ok(()));

    // Full: one more does not fit, and says when to try again.
    assert_eq!(
        allowance.take(&sender, 1).await.unwrap(),
        Err(AllowanceExceeded {
            limit: NOTIFICATIONS_PER_WINDOW,
            retry_after: Duration::from_secs(25 * 60),
        })
    );

    // Another sender has an allowance of their own.
    assert_eq!(allowance.take(&other, 100).await.unwrap(), Ok(()));
}

#[tokio::test]
async fn takes_nothing_when_the_whole_send_does_not_fit() {
    let allowance = HourlyAllowance(CountingBudget::default());
    let sender = user("macro|sender@macro.com");

    assert_eq!(
        allowance
            .take(&sender, NOTIFICATIONS_PER_WINDOW - 5)
            .await
            .unwrap(),
        Ok(())
    );
    assert!(allowance.take(&sender, 10).await.unwrap().is_err());
    // The refused ten were not counted, so five still fit.
    assert_eq!(allowance.take(&sender, 5).await.unwrap(), Ok(()));
}
