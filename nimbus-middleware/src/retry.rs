//! Retry interceptor with exponential backoff.

use std::time::Duration;

use nimbus_core::{Context, NimbusError};

use crate::interceptor::Interceptor;

/// Configuration for retry behavior.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts.
    pub max_retries: u32,
    /// Initial delay before first retry.
    pub initial_delay: Duration,
    /// Maximum delay between retries.
    pub max_delay: Duration,
    /// Multiplier for exponential backoff.
    pub backoff_multiplier: f64,
    /// Whether to add jitter to delays.
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            backoff_multiplier: 2.0,
            jitter: true,
        }
    }
}

impl RetryConfig {
    /// Create a new retry configuration.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum retry attempts.
    #[must_use]
    pub fn max_retries(mut self, max: u32) -> Self {
        self.max_retries = max;
        self
    }

    /// Set initial delay.
    #[must_use]
    pub fn initial_delay(mut self, delay: Duration) -> Self {
        self.initial_delay = delay;
        self
    }

    /// Set maximum delay.
    #[must_use]
    pub fn max_delay(mut self, delay: Duration) -> Self {
        self.max_delay = delay;
        self
    }

    /// Set backoff multiplier.
    #[must_use]
    pub fn backoff_multiplier(mut self, multiplier: f64) -> Self {
        self.backoff_multiplier = multiplier;
        self
    }

    /// Enable or disable jitter.
    #[must_use]
    pub fn jitter(mut self, enabled: bool) -> Self {
        self.jitter = enabled;
        self
    }

    /// Calculate the delay for a given attempt number.
    #[must_use]
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let base_delay =
            self.initial_delay.as_secs_f64() * self.backoff_multiplier.powi(attempt as i32);

        let delay = Duration::from_secs_f64(base_delay.min(self.max_delay.as_secs_f64()));

        if self.jitter {
            // Add up to 25% jitter
            let jitter_factor = 1.0 + (rand_simple() * 0.25);
            Duration::from_secs_f64(delay.as_secs_f64() * jitter_factor)
        } else {
            delay
        }
    }
}

/// Simple pseudo-random number generator for jitter.
/// This is not cryptographically secure but sufficient for jitter.
fn rand_simple() -> f64 {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    nanos as f64 / u32::MAX as f64
}

/// Interceptor that tracks retry state.
///
/// Note: This interceptor primarily provides configuration and tracking.
/// The actual retry logic is implemented in the transport layer which
/// uses this configuration.
pub struct RetryInterceptor {
    config: RetryConfig,
}

impl RetryInterceptor {
    /// Create a new retry interceptor with the given configuration.
    #[must_use]
    pub fn new(config: RetryConfig) -> Self {
        Self { config }
    }

    /// Get the retry configuration.
    #[must_use]
    pub fn config(&self) -> &RetryConfig {
        &self.config
    }

    /// Check if an error is retryable.
    #[must_use]
    pub fn is_retryable(error: &NimbusError) -> bool {
        error.is_retryable()
    }
}

impl Interceptor for RetryInterceptor {
    fn intercept_request(
        &self,
        ctx: &mut Context,
        _request: &[u8],
    ) -> Result<(), crate::interceptor::InterceptorError> {
        // Store retry config in metadata for transport layer
        ctx.metadata.insert(
            "nimbus.retry.max".to_string(),
            self.config.max_retries.to_string(),
        );
        Ok(())
    }

    fn on_error(&self, ctx: &Context, error: &NimbusError) {
        // Log retry-related information
        if Self::is_retryable(error) {
            let _ = ctx; // Could log attempt number from metadata
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exponential_backoff() {
        let config = RetryConfig::new()
            .initial_delay(Duration::from_millis(100))
            .backoff_multiplier(2.0)
            .jitter(false);

        assert_eq!(config.delay_for_attempt(0), Duration::from_millis(100));
        assert_eq!(config.delay_for_attempt(1), Duration::from_millis(200));
        assert_eq!(config.delay_for_attempt(2), Duration::from_millis(400));
    }

    #[test]
    fn test_max_delay_cap() {
        let config = RetryConfig::new()
            .initial_delay(Duration::from_secs(1))
            .max_delay(Duration::from_secs(5))
            .backoff_multiplier(10.0)
            .jitter(false);

        // Should be capped at max_delay
        let delay = config.delay_for_attempt(5);
        assert!(delay <= Duration::from_secs(5));
    }

    #[test]
    fn test_retryable_errors() {
        assert!(RetryInterceptor::is_retryable(&NimbusError::Timeout(
            Duration::from_secs(1)
        )));
        assert!(!RetryInterceptor::is_retryable(&NimbusError::Cancelled));
    }
}
